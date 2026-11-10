//! Pinned rules session for matches between separate policy workers.
use super::{
    DecisionMove, EngineError, ExecutionBudget, NativeSession, PlanningCommitment, Result,
    loadout::PublicUpgradeHistory,
};
use crate::{
    cards::CardId,
    codec::{Phase, StateEnvelope},
    data::Datum,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArenaChoice {
    Planning {
        commitments: Vec<ArenaCommitment>,
    },
    Resolution {
        player_id: String,
        request_id: String,
        selection: Value,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaCommitment {
    player_id: String,
    first_card: Option<u32>,
    second_card: Option<u32>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedArenaChoice {
    Planning {
        commitments: Vec<RecordedCommitment>,
        legal_choices_sha256: String,
    },
    Resolution {
        hero: String,
        selection: Value,
        legal_choices_sha256: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedCommitment {
    hero: String,
    first: Option<String>,
    second: Option<String>,
}

#[derive(Clone)]
pub struct ArenaMatch {
    session: NativeSession,
    history: PublicUpgradeHistory,
    cutoff: i128,
    revision: u64,
    max_decisions: u64,
}

impl ArenaMatch {
    pub fn start(envelope: StateEnvelope, turns: u8) -> Result<Self> {
        if !(1..=64).contains(&turns) {
            return Err(invalid("arena requires a turn limit in 1..64"));
        }
        let session = NativeSession::compile(envelope)?;
        let mut game = Self {
            history: PublicUpgradeHistory::new(&session)?,
            cutoff: session.search_turn_index() + i128::from(turns),
            session,
            revision: 0,
            max_decisions: u64::from(turns) * 128,
        };
        game.advance()?;
        Ok(game)
    }

    fn finished(&self) -> bool {
        self.session.planning.phase == Phase::GameOver
            || self.session.search_turn_index() >= self.cutoff
    }

    fn advance(&mut self) -> Result<()> {
        self.session
            .advance_to_decision(&ExecutionBudget::default())?;
        match self.session.planning.phase {
            Phase::Planning => self.history.planning_boundary(&self.session, self.revision),
            Phase::LevelUp if !self.finished() => self.history.begin_batch(&self.session),
            _ => Ok(()),
        }
    }

    fn boundary(&self) -> Value {
        json!({
            "revision": self.revision,
            "round": self.session.round.round,
            "turn": self.session.world.combat.turn,
            "phase": self.session.planning.phase,
        })
    }

    fn endpoint(&self) -> Result<Value> {
        let outcome = self.session.world.combat.outcome;
        let name = |id| self.session.symbols.name(id);
        let mut boundary = self.boundary();
        boundary["kind"] = json!("finished");
        boundary["winner_team"] = json!(outcome.and_then(|row| row.team).map(name));
        boundary["winner_hero"] = json!(outcome.and_then(|row| row.hero).map(name));
        boundary["state"] = self.session.semantic_state()?;
        boundary["public_upgrade_batches"] = json!(self.history.batches);
        Ok(boundary)
    }

    pub fn position(&mut self) -> Result<Value> {
        if self.finished() {
            return self.endpoint();
        }
        let mut boundary = self.boundary();
        if self.session.planning.phase == Phase::Planning {
            let mut slots = Vec::new();
            for hero in &self.session.world.heroes {
                if let Some(plan) = self.session.first_planning_commitment(hero.id)? {
                    slots.push(json!({
                        "player_id": self.session.symbols.name(hero.id),
                        "team": self.session.symbols.name(hero.team),
                        "fallback": {
                            "player_id": self.session.symbols.name(hero.id),
                            "first_card": plan.first.map(|card| card.0),
                            "second_card": plan.second.map(|card| card.0),
                        },
                    }));
                }
            }
            if slots.is_empty() {
                return Err(EngineError::Unsupported(
                    "arena planning has no open slots".into(),
                ));
            }
            boundary["kind"] = json!("planning");
            boundary["slots"] = json!(slots);
        } else {
            let (player, team) = self
                .session
                .search_player()?
                .ok_or_else(|| EngineError::Unsupported("arena stalled outside planning".into()))?;
            let moves = self.session.resolution_branch_probe(player)?;
            let Some(DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            }) = moves.first()
            else {
                return Err(EngineError::Unsupported(
                    "arena has no legal resolution move".into(),
                ));
            };
            boundary["kind"] = json!("resolution");
            boundary["team"] = json!(self.session.symbols.name(team));
            boundary["forced"] = json!(moves.len() == 1);
            boundary["fallback"] = json!({
                "kind": "resolution",
                "player_id": self.session.symbols.name(player),
                "request_id": self.session.request_token(*token),
                "selection": self.session.selection_json(selection),
            });
        }
        boundary["legal_choices"] = self.session.arena_legal_choices()?;
        boundary["upgrade_checkpoints"] = json!(self.history.checkpoints());
        boundary["envelope"] = json!(self.session.export_envelope()?);
        Ok(boundary)
    }

    pub fn replay(
        envelope: StateEnvelope,
        turns: u8,
        choices: Vec<RecordedArenaChoice>,
        mut emit: impl FnMut(Value) -> std::result::Result<(), String>,
    ) -> std::result::Result<(), String> {
        let describe = |error: EngineError| error.to_string();
        let mut game = Self::start(envelope, turns).map_err(describe)?;
        for recorded in choices {
            game.check_revision(game.revision).map_err(describe)?;
            let descriptor = game.session.arena_legal_choices().map_err(describe)?;
            let (choice, recorded_hash) = game.restore_choice(recorded).map_err(describe)?;
            emit(json!({
                "legal_choices": descriptor,
                "recorded_sha256": recorded_hash,
            }))?;
            game.apply(choice).map_err(describe)?;
            game.revision += 1;
            game.advance().map_err(describe)?;
        }
        if !game.finished() {
            return Err("arena replay ended before its terminal or capped boundary".into());
        }
        emit(game.endpoint().map_err(describe)?)
    }

    fn restore_choice(&mut self, recorded: RecordedArenaChoice) -> Result<(ArenaChoice, String)> {
        let session = &mut self.session;
        match recorded {
            RecordedArenaChoice::Planning {
                commitments,
                legal_choices_sha256,
            } => {
                let mut restored = Vec::with_capacity(commitments.len());
                for row in commitments {
                    let hero = player_symbol(session, row.hero.clone())?;
                    let mut matching = None;
                    for plan in session.hero_planning_commitments(hero)? {
                        let printed = |card: Option<CardId>| {
                            card.map(|id| session.printed_observation_card(id))
                                .transpose()
                        };
                        if printed(plan.first)? == row.first
                            && printed(plan.second)? == row.second
                            && matching.replace(plan).is_some()
                        {
                            return Err(invalid("ambiguous arena replay commitment"));
                        }
                    }
                    let plan =
                        matching.ok_or_else(|| invalid("illegal arena replay commitment"))?;
                    restored.push(ArenaCommitment {
                        player_id: row.hero,
                        first_card: plan.first.map(|card| card.0),
                        second_card: plan.second.map(|card| card.0),
                    });
                }
                Ok((
                    ArenaChoice::Planning {
                        commitments: restored,
                    },
                    legal_choices_sha256,
                ))
            }
            RecordedArenaChoice::Resolution {
                hero,
                selection,
                legal_choices_sha256,
            } => {
                let request = session
                    .last_request
                    .as_ref()
                    .ok_or_else(|| invalid("no current arena replay request"))?;
                Ok((
                    ArenaChoice::Resolution {
                        player_id: hero,
                        request_id: session.request_token(request.id),
                        selection,
                    },
                    legal_choices_sha256,
                ))
            }
        }
    }

    fn check_revision(&self, revision: u64) -> Result<()> {
        if self.finished() || revision != self.revision {
            return Err(invalid("stale or finished arena decision"));
        }
        if revision >= self.max_decisions {
            return Err(EngineError::Unsupported(
                "arena exceeded the decision limit".into(),
            ));
        }
        Ok(())
    }

    pub fn submit(&mut self, revision: u64, choice: ArenaChoice) -> Result<Value> {
        self.check_revision(revision)?;
        let mut next = self.clone();
        let decision = next.apply(choice)?;
        next.revision += 1;
        next.advance()?;
        let position = next.position()?;
        *self = next;
        Ok(json!({ "decision": decision, "position": position }))
    }

    fn apply(&mut self, choice: ArenaChoice) -> Result<Value> {
        let session = &mut self.session;
        match choice {
            ArenaChoice::Planning { commitments } => {
                let plans = commitments
                    .into_iter()
                    .map(|row| {
                        Ok(PlanningCommitment {
                            hero: player_symbol(session, row.player_id)?,
                            first: row.first_card.map(CardId),
                            second: row.second_card.map(CardId),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let ordered = session.validate_joint_planning(&plans)?;
                let decision = session.planning_trace_json(&ordered)?;
                session.apply_joint_planning_ordered(&ordered)?;
                Ok(decision)
            }
            ArenaChoice::Resolution {
                player_id,
                request_id,
                selection,
            } => {
                let player = player_symbol(session, player_id.clone())?;
                if session.search_player()?.map(|(owner, _)| owner) != Some(player) {
                    return Err(invalid("arena move belongs to another player"));
                }
                let token = session.import_request_token(&request_id);
                let selection = session.import_selection(&selection)?;
                let token = session.validate_sequence_input(player, Some(token), &selection)?;
                let decision = json!({
                    "kind": "resolution", "hero": player_id,
                    "selection": session.selection_json(&selection),
                });
                session.submit(token, selection)?;
                session.last_request = None;
                Ok(decision)
            }
        }
    }
}

fn player_symbol(session: &mut NativeSession, player_id: String) -> Result<crate::data::Symbol> {
    match session.import_selection(&Value::String(player_id))? {
        Datum::Text(id) => Ok(id),
        _ => Err(invalid("arena player ID must be text")),
    }
}

fn invalid(message: &str) -> EngineError {
    EngineError::InvalidInput(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_round_trips_unsigned_setup_seed() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let envelope: StateEnvelope = serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let baseline = ArenaMatch::start(envelope.clone(), 1)
            .unwrap()
            .position()
            .unwrap();
        for seed in [i64::MAX as u64 + 1, u64::MAX] {
            let mut start = envelope.clone();
            start.state["rng_seed"] = json!(seed);
            let position = ArenaMatch::start(start, 1).unwrap().position().unwrap();
            assert_eq!(position["legal_choices"], baseline["legal_choices"]);
            assert_eq!(position["envelope"]["state"]["rng_seed"], json!(seed));
            let restored: StateEnvelope =
                serde_json::from_value(position["envelope"].clone()).unwrap();
            let session = NativeSession::compile(restored).unwrap();
            assert_eq!(session.semantic_state().unwrap()["rng_seed"], json!(seed));
        }
    }

    #[test]
    fn recorded_arena_replay_requires_a_complete_trace_and_matches_the_endpoint() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let envelope: StateEnvelope = serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let mut arena = ArenaMatch::start(envelope.clone(), 1).unwrap();
        let mut position = arena.position().unwrap();
        let mut trace = Vec::new();
        let mut descriptors = Vec::new();
        while position["kind"] != "finished" {
            let choice = if position["kind"] == "planning" {
                let slots = position["slots"].as_array().unwrap();
                json!({
                    "kind": "planning",
                    "commitments": slots.iter().map(|slot| &slot["fallback"]).collect::<Vec<_>>(),
                })
            } else {
                position["fallback"].clone()
            };
            descriptors.push(position["legal_choices"].clone());
            let revision = position["revision"].as_u64().unwrap();
            let response = arena
                .submit(revision, serde_json::from_value(choice).unwrap())
                .unwrap();
            let mut decision = response["decision"].clone();
            decision["legal_choices_sha256"] = json!("unused by the rules session");
            trace.push(decision);
            position = response["position"].clone();
        }

        let mut stream = Vec::new();
        let choices = serde_json::from_value(json!(trace)).unwrap();
        ArenaMatch::replay(envelope.clone(), 1, choices, |record| {
            stream.push(record);
            Ok(())
        })
        .unwrap();
        assert_eq!(stream.pop(), Some(position));
        assert_eq!(
            stream
                .into_iter()
                .map(|row| row["legal_choices"].clone())
                .collect::<Vec<_>>(),
            descriptors,
        );

        trace.pop();
        let incomplete = serde_json::from_value(json!(trace)).unwrap();
        let error = ArenaMatch::replay(envelope, 1, incomplete, |_| Ok(())).unwrap_err();
        assert!(error.contains("ended before"));
    }

    #[test]
    fn rejected_arena_submissions_preserve_the_pending_decision() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let envelope = serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let mut arena = ArenaMatch::start(envelope, 1).unwrap();
        let planning = arena.position().unwrap();
        let commitments = planning["slots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slot| slot["fallback"].clone())
            .collect::<Vec<_>>();
        let choice = |commitments| {
            serde_json::from_value(json!({"kind": "planning", "commitments": commitments})).unwrap()
        };
        assert!(arena.submit(0, choice(&commitments[1..])).is_err());
        assert_eq!(arena.position().unwrap(), planning);

        let position = arena.submit(0, choice(&commitments)).unwrap()["position"].clone();
        assert_eq!(position["kind"], "resolution");
        assert!(arena.submit(0, choice(&commitments)).is_err());
        let mut stale = position["fallback"].clone();
        stale["request_id"] = json!("wrong");
        assert!(
            arena
                .submit(1, serde_json::from_value(stale).unwrap())
                .is_err()
        );
        let mut illegal = position["fallback"].clone();
        illegal["selection"] = json!("unoffered");
        assert!(
            arena
                .submit(1, serde_json::from_value(illegal).unwrap())
                .is_err()
        );
        assert_eq!(arena.position().unwrap(), position);
        let valid = serde_json::from_value(position["fallback"].clone()).unwrap();
        assert_eq!(arena.submit(1, valid).unwrap()["position"]["revision"], 2);
    }
}
