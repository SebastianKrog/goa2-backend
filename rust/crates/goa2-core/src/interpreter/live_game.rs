//! Authoritative game sessions shared by the Lab and web adapters.
use super::{
    DecisionMove, EngineError, ExecutionBudget, NativeSession, Result,
    loadout::PublicUpgradeHistory,
};
use crate::{codec::Phase, data::Symbol, geometry::Hex, world::CardLifecycle};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

mod bot;
mod checkpoint;
mod clock;
mod lane_progress;
mod patch;
mod rollback;
mod seed;
mod setup;
mod unstick;
mod upgrade_preview;
mod view;
pub use bot::BotDecision;
pub use checkpoint::GameCheckpoint;
pub use clock::{PauseVoteChange, TimeControlConfig};
pub use patch::{CardPatchZone, PatchCommand};
pub use setup::{GameContent, GameMode, GameSetup, SetupTeam};
pub use unstick::UnstickCommand;

/// The transport authenticates the actor separately from this payload.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlayerCommand {
    Commit {
        card_id: String,
    },
    Uncommit,
    Pass,
    FinishPlanning,
    StartingPosition {
        operation: StartingPositionCommand,
    },
    Rollback,
    CheatGold {
        hero_id: String,
        amount: i64,
    },
    Ready {
        ready: bool,
    },
    Input {
        request_id: String,
        selection: Value,
    },
}

/// Opening deployment actions use the same consent checks as the frontend.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StartingPositionCommand {
    Move { destination: crate::geometry::Hex },
    RequestSwap { target: String },
    RespondSwap { request_id: String, accept: bool },
    Cancel,
}

/// A visible table location. Never contains the card identity.
#[derive(Deserialize, Serialize)]
#[serde(tag = "zone", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TableCardSlot {
    Current,
    Extra,
    Played { index: usize },
    Discard { index: usize },
    Ultimate,
    Cast { index: usize },
}

#[derive(Debug, PartialEq, Serialize)]
pub struct GamePosition {
    pub revision: u64,
    pub phase: Phase,
    pub round: i64,
    pub turn: i64,
    pub outcome: Option<GameResult>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct GameResult {
    pub winner_team: Option<String>,
    pub winner_hero: Option<String>,
    pub condition: String,
}

/// Internal result for the adapter. Events require viewer filtering before delivery.
#[derive(Debug, PartialEq)]
pub struct GameTransition {
    pub position: GamePosition,
    pub events: Vec<Value>,
    previous_phase: Phase,
}

pub struct LiveGame {
    session: NativeSession,
    revision: u64,
    upgrade_history: PublicUpgradeHistory,
    rollback: Option<Arc<rollback::ActionAnchor>>,
    lane_progress: lane_progress::LaneProgress,
    clock: Option<clock::TimeControl>,
}

impl LiveGame {
    pub fn position(&self) -> GamePosition {
        GamePosition {
            revision: self.revision,
            phase: self.session.planning.phase,
            round: self.session.round.round,
            turn: self.session.world.combat.turn,
            outcome: self.session.world.combat.outcome.map(|result| {
                let name = |id| self.session.symbols.name(id).to_owned();
                GameResult {
                    winner_team: result.team.map(name),
                    winner_hero: result.hero.map(name),
                    condition: name(result.condition),
                }
            }),
        }
    }

    pub fn accepts_table_ping(&self) -> bool {
        !matches!(
            self.session.planning.phase,
            Phase::Resolution | Phase::LevelUp
        )
    }

    pub fn table_contains_hex(&self, hex: Hex) -> bool {
        self.session.world.on_map(hex)
    }

    pub fn table_contains_card(&self, hero_id: &str, slot: &TableCardSlot) -> bool {
        let Ok(id) = self.hero(hero_id) else {
            return false;
        };
        let world = &self.session.world;
        let Some(hero) = world.hero(id) else {
            return false;
        };
        match *slot {
            TableCardSlot::Current => hero.current.is_some(),
            TableCardSlot::Extra => {
                let planning = &self.session.planning;
                let (first, second) = planning.committed_cards(id);
                if planning.phase == Phase::Planning && second.is_some() {
                    first.is_some()
                } else {
                    hero.extra.is_some()
                }
            }
            TableCardSlot::Played { index } => hero.played.get(index).is_some_and(Option::is_some),
            TableCardSlot::Discard { index } => index < hero.discard.len(),
            TableCardSlot::Ultimate => hero.level >= 8 && hero.ultimate.is_some(),
            TableCardSlot::Cast { index } => hero
                .spells
                .iter()
                .filter(|id| {
                    world
                        .card(**id)
                        .is_some_and(|card| card.lifecycle != CardLifecycle::Spellbook)
                })
                .nth(index)
                .is_some(),
        }
    }

    /// A trusted adapter calls this with the authenticated hero. The prompt
    /// never includes other players' simultaneous upgrade options.
    pub fn request_for(&self, hero_id: &str) -> Option<Value> {
        let hero = self.hero(hero_id).ok()?;
        let request = self.session.last_request.as_ref()?;
        if !self.session.resolution_request_addresses(request, hero) {
            return None;
        }
        let mut wire = self.session.request_json(request);
        if request.owner == hero && self.can_rollback() {
            wire["can_rollback"] = Value::Bool(true);
        }
        if let super::RequestChoices::Upgrades(_) = request.choices {
            let context = wire.get_mut("context")?.as_object_mut()?;
            let players = context.get_mut("players")?.as_object_mut()?;
            players.retain(|id, _| id == hero_id);
        }
        Some(wire)
    }

    /// Commit only after the command and automatic rule advancement both succeed.
    /// The caller serializes access to a game and persists before acknowledging.
    pub fn submit(
        &mut self,
        actor_id: &str,
        expected_revision: u64,
        command: PlayerCommand,
        budget: &ExecutionBudget<'_>,
    ) -> Result<GameTransition> {
        let (next, transition) =
            self.prepare_submission(actor_id, expected_revision, command, budget)?;
        *self = next;
        Ok(transition)
    }

    /// Produce a candidate revision for durable storage before publication.
    pub fn prepare_submission(
        &self,
        actor_id: &str,
        expected_revision: u64,
        command: PlayerCommand,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(Self, GameTransition)> {
        self.prepare_submission_at(
            actor_id,
            expected_revision,
            command,
            budget,
            self.clock_timestamp(),
        )
    }

    pub fn prepare_submission_at(
        &self,
        actor_id: &str,
        expected_revision: u64,
        command: PlayerCommand,
        budget: &ExecutionBudget<'_>,
        now_ms: u64,
    ) -> Result<(Self, GameTransition)> {
        self.prepare_timed_submission(actor_id, expected_revision, command, budget, now_ms, true)
    }

    /// Bot choices use the same rules and clock settlement as player choices.
    pub fn prepare_bot_submission_at(
        &self,
        actor_id: &str,
        expected_revision: u64,
        command: PlayerCommand,
        budget: &ExecutionBudget<'_>,
        now_ms: u64,
    ) -> Result<(Self, GameTransition)> {
        self.prepare_timed_submission(actor_id, expected_revision, command, budget, now_ms, false)
    }

    fn prepare_timed_submission(
        &self,
        actor_id: &str,
        expected_revision: u64,
        command: PlayerCommand,
        budget: &ExecutionBudget<'_>,
        now_ms: u64,
        human: bool,
    ) -> Result<(Self, GameTransition)> {
        if self.revision != expected_revision {
            return Err(invalid("stale game revision"));
        }
        budget.check(0)?;
        let actor = self.hero(actor_id)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("game revision exhausted"))?;
        let mut next = Self {
            session: self.session.clone(),
            revision,
            upgrade_history: self.upgrade_history.clone(),
            rollback: self.rollback.clone(),
            lane_progress: self.lane_progress.clone(),
            clock: self.clock.clone(),
        };
        let ready = match &command {
            PlayerCommand::Ready { ready } => Some(*ready),
            _ => None,
        };
        if matches!(
            command,
            PlayerCommand::Commit { .. }
                | PlayerCommand::Uncommit
                | PlayerCommand::Pass
                | PlayerCommand::FinishPlanning
        ) && self.session.planning.phase == Phase::Planning
            && self.planning_timed_out(actor_id)
        {
            return Err(invalid("Planning was locked by the clock"));
        }
        let deployment = matches!(command, PlayerCommand::StartingPosition { .. });
        let gold_cheat = matches!(command, PlayerCommand::CheatGold { .. });
        let mut cheat_event = None;
        if let Some(ready) = ready {
            next.set_ready(actor_id, ready, now_ms)?;
        } else {
            next.prepare_clock_action(now_ms, human, gold_cheat)?;
            match command {
                PlayerCommand::Rollback => next.rewind(actor)?,
                PlayerCommand::CheatGold { hero_id, amount } => {
                    cheat_event = Some(next.apply_gold_cheat(&hero_id, amount)?);
                }
                _ => next.apply(actor, command)?,
            }
        }
        let mut transition = next.advance(budget)?;
        transition.events.extend(cheat_event);
        if !deployment && ready.is_none() {
            next.update_rollback()?;
        }
        next.reconcile_clock(now_ms)?;
        transition.previous_phase = self.session.planning.phase;
        Ok((next, transition))
    }

    pub fn players(&self) -> impl Iterator<Item = (&str, &str)> {
        self.session.world.heroes.iter().map(|hero| {
            let id = self.session.symbols.name(hero.id);
            let name = self
                .session
                .world
                .entity(hero.id)
                .map_or(id, |entity| self.session.symbols.name(entity.name));
            (id, name)
        })
    }

    fn hero(&self, id: &str) -> Result<Symbol> {
        self.session
            .symbols
            .lookup(id)
            .filter(|id| {
                self.session
                    .world
                    .hero(*id)
                    .is_some_and(|hero| hero.id == *id)
            })
            .ok_or_else(|| invalid("unknown game hero"))
    }

    pub fn cheats_enabled(&self) -> bool {
        self.session.source.state["cheats_enabled"]
            .as_bool()
            .unwrap_or(false)
    }

    fn apply_gold_cheat(&mut self, hero_id: &str, amount: i64) -> Result<Value> {
        if !self.cheats_enabled() {
            return Err(invalid("Cheats are not enabled for this game"));
        }
        if self.session.planning.phase != Phase::Planning {
            return Err(invalid("Gold cheats require the planning phase"));
        }
        if amount <= 0 {
            return Err(invalid("Amount must be a positive integer"));
        }
        let hero = self.hero(hero_id)?;
        self.session.grant_test_gold(hero, amount)?;
        Ok(json!({
            "event_type": "GOLD_GAINED", "actor_id": hero_id, "target_id": null,
            "from_hex": null, "to_hex": null, "metadata": {"amount": amount, "reason": "cheat"},
        }))
    }

    fn apply(&mut self, actor: Symbol, command: PlayerCommand) -> Result<()> {
        let session = &mut self.session;
        if let PlayerCommand::Input {
            request_id,
            selection,
        } = command
        {
            let token = session
                .last_request
                .as_ref()
                .filter(|request| session.request_token(request.id) == request_id)
                .map(|request| request.id)
                .ok_or_else(|| invalid("stale or unissued input request"))?;
            let selection = session.import_selection(&selection)?;
            session.validate_sequence_input(actor, Some(token), &selection)?;
            session.submit(token, selection)?;
            session.last_request = None;
            return Ok(());
        }

        if session.planning.phase != Phase::Planning
            && !matches!(command, PlayerCommand::StartingPosition { .. })
        {
            return Err(invalid("planning command outside planning"));
        }
        match command {
            PlayerCommand::Commit { card_id } => {
                let card = session
                    .planning_choices(actor)?
                    .into_iter()
                    .find_map(|choice| {
                        let DecisionMove::Card { card, .. } = choice else {
                            return None;
                        };
                        let row = session.world.card(card)?;
                        let printed = session.world.definitions.cards[row.definition].printed_id;
                        (session.symbols.name(printed) == card_id).then_some(card)
                    })
                    .ok_or_else(|| invalid("card is not available to commit"))?;
                session.commit_card(actor, card)
            }
            PlayerCommand::Uncommit => session.uncommit_card(actor),
            PlayerCommand::Pass => session.pass_planning(actor),
            PlayerCommand::FinishPlanning => session.finish_planning(actor),
            PlayerCommand::StartingPosition { operation } => session.change_starting_position(
                actor,
                operation,
                format!("starting-position-{}", self.revision),
            ),
            PlayerCommand::Input { .. }
            | PlayerCommand::Rollback
            | PlayerCommand::CheatGold { .. }
            | PlayerCommand::Ready { .. } => {
                unreachable!("resolution commands handled above")
            }
        }
    }

    fn advance(&mut self, budget: &ExecutionBudget<'_>) -> Result<GameTransition> {
        let previous_phase = self.session.planning.phase;
        let mut remaining = budget.max_steps.min(1000);
        let mut events = Vec::new();
        while self.session.last_request.is_none()
            && !matches!(
                self.session.planning.phase,
                Phase::Planning | Phase::GameOver
            )
        {
            let bounded = ExecutionBudget {
                max_steps: remaining,
                ..*budget
            };
            let result = self.session.advance(&bounded)?;
            events.extend(
                result
                    .events
                    .iter()
                    .map(|event| self.session.event_json(event)),
            );
            if result.steps == 0
                && self.session.last_request.is_none()
                && !matches!(
                    self.session.planning.phase,
                    Phase::Planning | Phase::GameOver
                )
            {
                return Err(EngineError::Unsupported(
                    "game stopped outside a decision boundary".into(),
                ));
            }
            remaining -= result.steps;
        }
        match self.session.planning.phase {
            Phase::Planning => self
                .upgrade_history
                .planning_boundary(&self.session, self.revision)?,
            Phase::LevelUp => self.upgrade_history.begin_batch(&self.session)?,
            _ => {}
        }
        self.lane_progress.observe(&self.session);
        Ok(GameTransition {
            position: self.position(),
            events,
            previous_phase,
        })
    }
}

fn invalid(message: &str) -> EngineError {
    EngineError::InvalidInput(message.into())
}
