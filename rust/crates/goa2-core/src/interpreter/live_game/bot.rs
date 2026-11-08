//! The arena baseline at the live game's authenticated decision boundary.
use super::{DecisionMove, EngineError, ExecutionBudget, LiveGame, PlayerCommand, Result, invalid};
use crate::{
    codec::Phase,
    data::Symbol,
    interpreter::{
        BotPlanningScope, ForecastNodeShare, NativeSession, OpponentPolicy, PlanningRolloutSample,
        PlanningSearchOptions, SearchEvaluation, SearchLimits, TeamKnowledgeMode,
    },
};

const HORIZON: Option<u8> = Some(2);
const WORLDS: usize = 2;

pub struct BotDecision {
    pub command: PlayerCommand,
    pub used_fallback: bool,
    pub fallback: Option<PlayerCommand>,
}

impl LiveGame {
    /// Recommend one legal command using true-rules information for this seat.
    /// A speculative result is always tied to this immutable game revision.
    pub fn recommend_bot(
        &self,
        actor_id: &str,
        budget: &ExecutionBudget<'_>,
    ) -> Result<Option<BotDecision>> {
        let player = self.hero(actor_id)?;
        let position = self.position();
        let seed = self.session.source.state["rng_seed"]
            .as_u64()
            .unwrap_or_default()
            .wrapping_add((position.round as u64).wrapping_mul(4))
            .wrapping_add(position.turn as u64);
        let checkpoints = self.upgrade_history.checkpoints();
        let mut worker;
        let (session, chosen, fallback, searched) = if position.phase == Phase::Planning {
            let choices = self.session.planning_choices(player)?;
            let Some(fallback) = choices.first() else {
                return Ok(None);
            };
            if choices.len() == 1 {
                (&self.session, None, fallback.clone(), false)
            } else {
                let options = PlanningSearchOptions {
                    rollout_turns: HORIZON,
                    workers: 1,
                    upgrade_checkpoints: checkpoints,
                    reply_cap: Some(4),
                    reply_probe_cap: None,
                    greedy_card_order: true,
                    tactical_forecast: false,
                    forecast_node_share: ForecastNodeShare::default(),
                    legacy_fixed_followup: false,
                    fixed_future_planning: false,
                    branch_future_planning: false,
                };
                let result = self.session.search_bot_team_planning(
                    player,
                    BotPlanningScope {
                        seed,
                        bots: &[player],
                        worlds: WORLDS,
                        mode: TeamKnowledgeMode::TrueRules,
                    },
                    4,
                    limits(262_144),
                    options,
                    budget,
                );
                let plan = optional_search(result)?
                    .filter(|plan| {
                        plan.matrix.score.is_some()
                            || plan.shallow_score.is_some()
                            || plan.seed_score.is_some()
                    })
                    .and_then(|plan| {
                        plan.commitments
                            .into_iter()
                            .find(|choice| choice.hero == player)
                    });
                let chosen = plan
                    .and_then(|plan| {
                        // An open second slot gets its own decision, with the first
                        // card fixed in the next search.
                        if let Some(card) = plan.first {
                            Some(DecisionMove::Card { hero: player, card })
                        } else {
                            choices
                                .iter()
                                .find(|choice| {
                                    matches!(
                                        choice,
                                        DecisionMove::Pass { .. }
                                            | DecisionMove::FinishPlanning { .. }
                                    )
                                })
                                .cloned()
                        }
                    })
                    .filter(|choice| choices.contains(choice));
                (&self.session, chosen, fallback.clone(), true)
            }
        } else {
            let Some(request) = self.session.last_request.as_ref() else {
                return Ok(None);
            };
            if !self.session.resolution_request_addresses(request, player) {
                return Ok(None);
            }
            worker = self.session.clone();
            let offered = worker.resolution_branch_probe(player)?;
            let fallback = offered
                .first()
                .ok_or_else(|| invalid("Bot has no legal input at the live prompt"))?;
            if offered.len() == 1 || position.phase == Phase::LevelUp {
                // Upgrade requests can expose other players' unrevealed choices
                // to a full-state continuation. Use only this seat's options.
                (&worker, None, fallback.clone(), false)
            } else {
                let sample = PlanningRolloutSample {
                    seed,
                    upgrade_checkpoints: checkpoints,
                };
                let result = worker.search_resolution_worlds(
                    player,
                    (sample, WORLDS, 1),
                    limits(32_768),
                    HORIZON,
                    Some(sample),
                    budget,
                );
                let chosen = optional_search(result)?
                    .filter(|row| row.score.is_some())
                    .map(|row| row.decision)
                    .filter(|choice| input_is_legal(&mut worker, player, choice));
                (&worker, chosen, fallback.clone(), true)
            }
        };
        if matches!(budget.check(0), Err(EngineError::Cancelled)) {
            return Err(EngineError::Cancelled);
        }

        let alternate = if chosen.as_ref().is_some_and(|choice| *choice != fallback) {
            Some(command_for(session, fallback.clone())?)
        } else {
            None
        };
        Ok(Some(BotDecision {
            used_fallback: searched && chosen.is_none(),
            command: command_for(session, chosen.unwrap_or(fallback))?,
            fallback: alternate,
        }))
    }
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

fn limits(max_nodes: usize) -> SearchLimits {
    SearchLimits {
        max_depth: 2,
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
