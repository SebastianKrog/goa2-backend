//! Offline policy comparison. All moves and transitions run in Rust.
use super::{
    BotPlanningScope, DecisionMove, EngineError, ExecutionBudget, NativeSession, OpponentPolicy,
    PlanningRolloutSample, PlanningSearchOptions, Result, SearchEvaluation, SearchLimits,
    TeamKnowledgeMode,
};
use crate::codec::Phase;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflinePolicy {
    pub depth: u8,
    pub max_nodes: usize,
    pub time_ms: u64,
    pub rollout_turns: Option<u8>,
    #[serde(default)]
    pub sample_choices: bool,
    #[serde(default)]
    pub evaluation: SearchEvaluation,
}

impl OfflinePolicy {
    fn limits(self) -> SearchLimits {
        SearchLimits {
            max_depth: self.depth,
            max_nodes: self.max_nodes,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: self.evaluation,
        }
    }

    fn budget(self) -> ExecutionBudget<'static> {
        ExecutionBudget {
            deadline: Some(Instant::now() + Duration::from_millis(self.time_ms)),
            ..ExecutionBudget::default()
        }
    }
}

#[derive(Default, serde::Serialize)]
pub struct OfflinePolicyStats {
    planning: usize,
    resolution: usize,
    forced: usize,
    fallback: usize,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    resolution_fallbacks: BTreeMap<String, usize>,
}

#[derive(serde::Serialize)]
pub struct OfflineMatch {
    round: i64,
    turn: i64,
    phase: Phase,
    winner_team: Option<String>,
    winner_hero: Option<String>,
    lives: Vec<(String, i64)>,
    policies: [OfflinePolicyStats; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    trace: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<Value>,
}

impl NativeSession {
    pub fn play_offline_match(
        &mut self,
        turns: u8,
        seed: u64,
        policies: [OfflinePolicy; 2],
        capture_trace: bool,
    ) -> Result<OfflineMatch> {
        if !(1..=64).contains(&turns)
            || policies.iter().any(|policy| {
                policy.depth > 64
                    || (policy.sample_choices && policy.depth != 0)
                    || policy.max_nodes == 0
                    || !(1..=60_000).contains(&policy.time_ms)
                    || policy
                        .rollout_turns
                        .is_some_and(|turns| !(1..=2).contains(&turns))
            })
        {
            return Err(EngineError::InvalidInput(
                "invalid offline match limits".into(),
            ));
        }
        let budget = ExecutionBudget::default();
        let cutoff = self.search_turn_index() + i128::from(turns);
        let mut trace = capture_trace.then(Vec::new);
        let mut stats = [OfflinePolicyStats::default(), OfflinePolicyStats::default()];
        let team_index = |team: &str| match team {
            "RED" => Ok(0),
            "BLUE" => Ok(1),
            _ => Err(EngineError::Unsupported(
                "offline match requires red and blue teams".into(),
            )),
        };
        for revision in 0..usize::from(turns) * 128 {
            if self.planning.phase == Phase::GameOver || self.search_turn_index() >= cutoff {
                let outcome = self.world.combat.outcome;
                let name = |id| self.symbols.name(id).to_owned();
                return Ok(OfflineMatch {
                    round: self.round.round,
                    turn: self.world.combat.turn,
                    phase: self.planning.phase,
                    winner_team: outcome.and_then(|row| row.team).map(name),
                    winner_hero: outcome.and_then(|row| row.hero).map(name),
                    lives: self
                        .world
                        .combat
                        .team_lives()
                        .map(|(team, lives)| (name(team), lives))
                        .collect(),
                    policies: stats,
                    trace,
                    state: capture_trace.then(|| self.semantic_state()).transpose()?,
                });
            }
            let choice_index = |hero: crate::data::Symbol, choices| {
                super::loadout::sample_index(
                    seed ^ (revision as u64).rotate_left(32) ^ hero.0 as u64,
                    choices,
                )
            };
            let epoch_seed = seed.wrapping_add(self.search_turn_index() as u64);
            if self.planning.phase == Phase::Planning {
                let mut joint = Vec::new();
                for hero in &self.world.heroes {
                    let Some(fallback) = self.first_planning_commitment(hero.id)? else {
                        continue;
                    };
                    let index = team_index(self.symbols.name(hero.team))?;
                    let policy = policies[index];
                    stats[index].planning += 1;
                    let plan = if policy.sample_choices {
                        let choices = self.hero_planning_commitments(hero.id)?;
                        choices[choice_index(hero.id, choices.len())]
                    } else if policy.depth == 0 {
                        fallback
                    } else {
                        match self.search_bot_team_planning(
                            hero.id,
                            BotPlanningScope {
                                bots: &[hero.id],
                                mode: TeamKnowledgeMode::TrueRules,
                                worlds: 2,
                                seed: epoch_seed,
                            },
                            4,
                            policy.limits(),
                            PlanningSearchOptions {
                                rollout_turns: policy.rollout_turns,
                                workers: 1,
                                upgrade_checkpoints: None,
                            },
                            &policy.budget(),
                        ) {
                            Ok(result) if result.matrix.score.is_some() => result.commitments[0],
                            Ok(_) | Err(EngineError::Unsupported(_) | EngineError::Deadline) => {
                                stats[index].fallback += 1;
                                fallback
                            }
                            Err(error) => return Err(error),
                        }
                    };
                    joint.push(plan);
                }
                if let Some(trace) = &mut trace {
                    let commitments = joint
                        .iter()
                        .map(|plan| {
                            let printed = |card| self.printed_observation_card(card);
                            Ok(json!({
                                "hero": self.symbols.name(plan.hero),
                                "first": plan.first.map(printed).transpose()?,
                                "second": plan.second.map(printed).transpose()?,
                            }))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    trace.push(json!({"kind": "planning", "commitments": commitments}));
                }
                self.apply_joint_planning_ordered(&joint)?;
            } else {
                self.advance_to_decision(&budget)?;
                if matches!(self.planning.phase, Phase::Planning | Phase::GameOver)
                    || self.search_turn_index() >= cutoff
                {
                    continue;
                }
                let (owner, team) = self.search_player()?.ok_or_else(|| {
                    EngineError::Unsupported("offline match stalled outside planning".into())
                })?;
                let index = team_index(self.symbols.name(team))?;
                let policy = policies[index];
                stats[index].resolution += 1;
                let fallback = |session: &mut Self| {
                    session.first_resolution_choice(owner)?.ok_or_else(|| {
                        EngineError::Unsupported("offline match has no legal input".into())
                    })
                };
                let decision = if policy.sample_choices {
                    let mut choices = self.resolution_choices(owner)?;
                    if choices.is_empty() {
                        return Err(EngineError::Unsupported(
                            "offline match has no legal input".into(),
                        ));
                    }
                    let index = choice_index(owner, choices.len());
                    choices.swap_remove(index)
                } else if policy.depth == 0 {
                    fallback(self)?
                } else if let [only] = self.resolution_branch_probe(owner)?.as_slice() {
                    stats[index].forced += 1;
                    only.clone()
                } else {
                    let sample = PlanningRolloutSample {
                        seed: epoch_seed,
                        upgrade_checkpoints: None,
                    };
                    let searched = self.search_resolution_worlds(
                        owner,
                        (sample, 2, 1),
                        policy.limits(),
                        policy.rollout_turns,
                        policy.rollout_turns.map(|_| sample),
                        &policy.budget(),
                    );
                    match searched {
                        Ok(result) if result.score.is_some() => result.decision,
                        other => {
                            let reason = match other {
                                Ok(result) => format!("{:?} before a scored move", result.stop),
                                Err(EngineError::Unsupported(reason)) => reason,
                                Err(EngineError::Deadline) => "search deadline".into(),
                                Err(error) => return Err(error),
                            };
                            let stats = &mut stats[index];
                            stats.fallback += 1;
                            if capture_trace {
                                let prompt = self
                                    .last_request
                                    .as_ref()
                                    .map_or("unknown prompt", |request| {
                                        self.symbols.name(request.prompt)
                                    });
                                *stats
                                    .resolution_fallbacks
                                    .entry(format!("{prompt}: {reason}"))
                                    .or_default() += 1;
                            }
                            fallback(self)?
                        }
                    }
                };
                let DecisionMove::Input {
                    request: Some(token),
                    selection,
                    ..
                } = decision
                else {
                    return Err(EngineError::Unsupported(
                        "offline match expected a live input".into(),
                    ));
                };
                if let Some(trace) = &mut trace {
                    trace.push(json!({
                        "kind": "resolution", "hero": self.symbols.name(owner),
                        "selection": self.selection_json(&selection),
                    }));
                }
                self.submit(token, selection)?;
                self.last_request = None;
            }
        }
        Err(EngineError::Unsupported(
            "offline match exceeded its decision limit".into(),
        ))
    }
}
