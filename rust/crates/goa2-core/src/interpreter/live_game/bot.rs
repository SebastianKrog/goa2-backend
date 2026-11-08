//! Budgeted team planning and resolution for the authenticated live bot seats.
use super::{DecisionMove, EngineError, ExecutionBudget, LiveGame, PlayerCommand, Result, invalid};
use crate::interpreter::search::WIN;
use crate::{
    codec::Phase,
    data::Symbol,
    interpreter::{
        BotPlanningScope, ForecastNodeShare, NativeSession, OpponentPolicy, PlanningCommitment,
        PlanningRolloutSample, PlanningSearchOptions, ResolutionScoreKind, SearchEvaluation,
        SearchLimits, TeamKnowledgeMode,
    },
};
use std::time::{Duration, Instant};

const LIVE_HORIZON: u8 = 4;
const REFERENCE_HORIZON: u8 = 2;
const WORLDS: usize = 2;

pub struct BotDecision {
    pub command: PlayerCommand,
    pub used_fallback: bool,
    pub fallback: Option<PlayerCommand>,
}

impl LiveGame {
    /// Group authorized bot seats by team, including seats that already committed.
    pub fn bot_teams<'a>(&self, bots: &[&'a str]) -> Result<Vec<Vec<&'a str>>> {
        let mut teams: Vec<(Symbol, Vec<&str>)> = Vec::new();
        for &id in bots {
            let hero = self.hero(id)?;
            let team = self.session.world.hero(hero).unwrap().team;
            if let Some((_, members)) = teams.iter_mut().find(|(color, _)| *color == team) {
                members.push(id);
            } else {
                teams.push((team, vec![id]));
            }
        }
        Ok(teams.into_iter().map(|(_, members)| members).collect())
    }

    /// A planning team can use a human's actual selection as soon as it exists.
    pub fn bot_team_has_open_human(&self, bots: &[&str]) -> Result<bool> {
        let Some(first) = bots.first() else {
            return Ok(false);
        };
        let team = self.session.world.hero(self.hero(first)?).unwrap().team;
        for hero in &self.session.world.heroes {
            if hero.team == team
                && !bots.contains(&self.session.symbols.name(hero.id))
                && !self.session.planning_choices(hero.id)?.is_empty()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn bot_seed(&self) -> u64 {
        let position = self.position();
        self.session.source.state["rng_seed"]
            .as_u64()
            .unwrap_or_default()
            .wrapping_add((position.round as u64).wrapping_mul(4))
            .wrapping_add(position.turn as u64)
    }

    /// Search all open slots for one team jointly. The roster is supplied by
    /// the server and authorizes exact bot hands and allied human commitments.
    pub fn recommend_bot_team_plan(
        &self,
        bots: &[&str],
        budget: &ExecutionBudget<'_>,
    ) -> Result<Vec<(String, BotDecision)>> {
        let bots = bots
            .iter()
            .map(|id| self.hero(id))
            .collect::<Result<Vec<_>>>()?;
        let mut slots = Vec::new();
        for &hero in &bots {
            let choices = self.session.planning_choices(hero)?;
            if !choices.is_empty() {
                slots.push((hero, choices));
            }
        }
        let Some((player, _)) = slots.first() else {
            return Ok(Vec::new());
        };
        let search = |horizon,
                      cap,
                      limits,
                      budget: &ExecutionBudget<'_>,
                      incumbent: Option<&[PlanningCommitment]>| {
            optional_search(self.session.search_live_bot_team_planning(
                *player,
                BotPlanningScope {
                    seed: self.bot_seed(),
                    bots: &bots,
                    worlds: WORLDS,
                    mode: TeamKnowledgeMode::CoordinatedKnowSameTeamHumans,
                },
                cap,
                limits,
                PlanningSearchOptions {
                    rollout_turns: Some(horizon),
                    workers: 1,
                    upgrade_checkpoints: self.upgrade_history.checkpoints(),
                    reply_cap: Some(4),
                    reply_probe_cap: None,
                    greedy_card_order: true,
                    tactical_forecast: false,
                    forecast_node_share: ForecastNodeShare::default(),
                    legacy_fixed_followup: false,
                    fixed_future_planning: false,
                    branch_future_planning: false,
                },
                budget,
                incumbent,
            ))
        };
        let searched = slots.iter().any(|(_, choices)| choices.len() > 1);
        let mut selected = None;
        if searched {
            selected = search(
                REFERENCE_HORIZON,
                4,
                limits(2, 262_144),
                &reference_budget(budget),
                None,
            )?
            .filter(scored_plan);
            if can_refine(budget)
                && selected
                    .as_ref()
                    .is_none_or(|plan| plan.matrix.score != Some(WIN))
                && let Some(deeper) = search(
                    LIVE_HORIZON,
                    8,
                    limits(3, 1_048_576),
                    budget,
                    selected.as_ref().map(|plan| plan.commitments.as_slice()),
                )?
                && (deeper.matrix.score.is_some() || (selected.is_none() && scored_plan(&deeper)))
            {
                selected = Some(deeper);
            }
        }
        check_cancelled(budget)?;

        let mut decisions = Vec::new();
        for (hero, choices) in slots {
            let plan = selected
                .as_ref()
                .and_then(|plan| plan.commitments.iter().find(|row| row.hero == hero));
            let fallback = &choices[0];
            let chosen = plan
                .and_then(|plan| planning_move(plan, &choices))
                .filter(|choice| choices.contains(choice));
            let first = chosen.as_ref().unwrap_or(fallback);
            let actor = self.session.symbols.name(hero).to_owned();
            let alternate = chosen
                .as_ref()
                .filter(|choice| *choice != fallback)
                .map(|_| command_for(&self.session, fallback.clone()))
                .transpose()?;
            decisions.push((
                actor.clone(),
                BotDecision {
                    command: command_for(&self.session, first.clone())?,
                    used_fallback: searched && chosen.is_none(),
                    fallback: alternate,
                },
            ));
            if matches!(first, DecisionMove::Card { .. })
                && !choices.contains(&DecisionMove::FinishPlanning { hero })
                && self.session.planning.two_cards(&self.session.world, hero)
                && self
                    .session
                    .world
                    .hero(hero)
                    .is_some_and(|row| row.hand.len() > 1)
            {
                let finish = PlayerCommand::FinishPlanning;
                let second = plan
                    .filter(|_| chosen.is_some())
                    .and_then(|row| row.second)
                    .map(|card| DecisionMove::Card { hero, card })
                    .filter(|choice| choice != first && choices.contains(choice))
                    .map(|choice| command_for(&self.session, choice))
                    .transpose()?;
                decisions.push((
                    actor,
                    BotDecision {
                        fallback: second.as_ref().map(|_| finish.clone()),
                        command: second.unwrap_or(finish),
                        used_fallback: false,
                    },
                ));
            }
        }
        Ok(decisions)
    }

    /// Recommend a single bot command, including the next slot of a solo plan.
    pub fn recommend_bot(
        &self,
        actor_id: &str,
        budget: &ExecutionBudget<'_>,
    ) -> Result<Option<BotDecision>> {
        if self.position().phase == Phase::Planning {
            return Ok(self
                .recommend_bot_team_plan(&[actor_id], budget)?
                .into_iter()
                .next()
                .map(|(_, decision)| decision));
        }
        self.recommend_bot_resolution(actor_id, &[actor_id], budget)
    }

    /// Search an open prompt from the authorized bot's side of the table.
    pub fn recommend_bot_resolution(
        &self,
        actor_id: &str,
        bots: &[&str],
        budget: &ExecutionBudget<'_>,
    ) -> Result<Option<BotDecision>> {
        let player = self.hero(actor_id)?;
        let phase = self.position().phase;
        let Some(request) = self.session.last_request.as_ref() else {
            return Ok(None);
        };
        if !self.session.resolution_request_addresses(request, player) {
            return Ok(None);
        }
        let mut worker = self.session.clone();
        let offered = worker.resolution_branch_probe(player)?;
        let fallback = offered
            .first()
            .ok_or_else(|| invalid("Bot has no legal input at the live prompt"))?;
        let searched = offered.len() > 1 && phase != Phase::LevelUp;
        let chosen = if phase == Phase::LevelUp && offered.len() > 1 {
            // Select a defense item if available from the bot's own upgrade prompt.
            worker.greedy_opponent_input(player, self.bot_seed())?
        } else if searched {
            let bots = bots
                .iter()
                .map(|id| self.hero(id))
                .collect::<Result<Vec<_>>>()?;
            let sample = PlanningRolloutSample {
                seed: self.bot_seed(),
                upgrade_checkpoints: self.upgrade_history.checkpoints(),
            };
            let mut ranking = Vec::new();
            let mut search = |horizon,
                              limits,
                              incumbent: Option<&DecisionMove>,
                              budget: &ExecutionBudget<'_>|
             -> Result<_> {
                let result = optional_search(worker.search_live_bot_resolution(
                    player,
                    sample,
                    WORLDS,
                    &bots,
                    incumbent,
                    &mut ranking,
                    limits,
                    horizon,
                    budget,
                ))?;
                Ok(result.filter(|row| {
                    row.score.is_some() && input_is_legal(&mut worker, player, &row.decision)
                }))
            };
            let mut best = if phase == Phase::Resolution {
                search(
                    REFERENCE_HORIZON,
                    limits(2, 32_768),
                    None,
                    &reference_budget(budget),
                )?
            } else {
                search(1, limits(2, 8_192), None, budget)?
            };
            if phase == Phase::Resolution
                && can_refine(budget)
                && best.as_ref().is_none_or(|search| search.score != Some(WIN))
                && let Some(deeper) = search(
                    LIVE_HORIZON,
                    limits(5, 262_144),
                    best.as_ref().map(|row| &row.decision),
                    budget,
                )?
                && (deeper.score_kind == Some(ResolutionScoreKind::TurnRollout) || best.is_none())
            {
                best = Some(deeper);
            }
            best.map(|row| row.decision)
        } else {
            None
        };
        check_cancelled(budget)?;

        let alternate = chosen
            .as_ref()
            .filter(|choice| *choice != fallback)
            .map(|_| command_for(&worker, fallback.clone()))
            .transpose()?;
        Ok(Some(BotDecision {
            used_fallback: searched && chosen.is_none(),
            command: command_for(&worker, chosen.unwrap_or_else(|| fallback.clone()))?,
            fallback: alternate,
        }))
    }
}

fn scored_plan(plan: &crate::interpreter::BotTeamPlanningSearch) -> bool {
    plan.matrix.score.is_some() || plan.shallow_score.is_some() || plan.seed_score.is_some()
}

fn planning_move(plan: &PlanningCommitment, choices: &[DecisionMove]) -> Option<DecisionMove> {
    if let Some(card) = plan.first {
        Some(DecisionMove::Card {
            hero: plan.hero,
            card,
        })
    } else {
        choices
            .iter()
            .find(|choice| {
                matches!(
                    choice,
                    DecisionMove::Pass { .. } | DecisionMove::FinishPlanning { .. }
                )
            })
            .cloned()
    }
}

fn reference_budget<'a>(budget: &ExecutionBudget<'a>) -> ExecutionBudget<'a> {
    ExecutionBudget {
        deadline: budget.deadline.map(|end| {
            let now = Instant::now();
            now + (end.saturating_duration_since(now) / 2).min(Duration::from_secs(2))
        }),
        ..*budget
    }
}

fn can_refine(budget: &ExecutionBudget<'_>) -> bool {
    budget.check(0).is_ok()
        && budget.deadline.is_none_or(|end| {
            end.saturating_duration_since(Instant::now()) >= Duration::from_millis(250)
        })
}

fn check_cancelled(budget: &ExecutionBudget<'_>) -> Result<()> {
    if matches!(budget.check(0), Err(EngineError::Cancelled)) {
        return Err(EngineError::Cancelled);
    }
    Ok(())
}

pub(super) fn command_for(session: &NativeSession, choice: DecisionMove) -> Result<PlayerCommand> {
    Ok(match choice {
        DecisionMove::Card { card, .. } => PlayerCommand::Commit {
            card_id: session.printed_observation_card(card)?,
        },
        DecisionMove::Pass { .. } => PlayerCommand::Pass,
        DecisionMove::FinishPlanning { .. } => PlayerCommand::FinishPlanning,
        DecisionMove::Input {
            request: Some(token),
            selection,
            ..
        } => PlayerCommand::Input {
            request_id: session.request_token(token),
            selection: session.selection_json(&selection),
        },
        _ => return Err(invalid("Bot choice does not address an issued request")),
    })
}

fn limits(max_depth: u8, max_nodes: usize) -> SearchLimits {
    SearchLimits {
        max_depth,
        max_nodes,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::LanePressureV3,
    }
}

fn optional_search<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(EngineError::Cancelled) => Err(EngineError::Cancelled),
        Err(
            EngineError::Deadline
            | EngineError::StepLimit
            | EngineError::Unsupported(_)
            | EngineError::InvalidInput(_),
        ) => Ok(None),
        Err(error) => Err(error),
    }
}

fn input_is_legal(worker: &mut NativeSession, player: Symbol, choice: &DecisionMove) -> bool {
    let DecisionMove::Input {
        player: owner,
        request: Some(token),
        selection,
    } = choice
    else {
        return false;
    };
    *owner == player
        && worker
            .validate_sequence_input(player, Some(*token), selection)
            .is_ok()
}
