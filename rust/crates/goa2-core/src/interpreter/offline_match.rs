//! Offline policy comparison. All moves and transitions run in Rust.
use super::loadout::{PublicUpgradeBatch, PublicUpgradeHistory};
use super::search::MAX_SEARCH_ROLLOUT_TURNS;
use super::{
    BotPlanningScope, DecisionMove, EngineError, ExecutionBudget, NativeSession, OpponentPolicy,
    PlanningRolloutSample, PlanningSearchOptions, Result, SearchEvaluation, SearchLimits,
    TeamKnowledgeMode, WorkBudget,
};
use crate::codec::Phase;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::Instant,
};

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflinePolicy {
    pub depth: u8,
    pub max_nodes: usize,
    pub resolution_max_nodes: Option<usize>,
    pub time_ms: Option<u64>,
    pub work_budget: Option<WorkBudget>,
    pub rollout_turns: Option<u8>,
    pub planning_workers: Option<usize>,
    pub sampled_worlds: Option<usize>,
    pub reply_cap: Option<usize>,
    #[serde(default)]
    pub greedy_card_order: bool,
    #[serde(default)]
    pub tactical_forecast: bool,
    #[serde(default)]
    pub coordinated_planning: bool,
    #[serde(default)]
    pub forecast_node_share: super::ForecastNodeShare,
    #[serde(default)]
    pub legacy_fixed_followup: bool,
    #[serde(default)]
    pub planning_seed_pass: bool,
    #[serde(default)]
    pub fixed_future_planning: bool,
    #[serde(default)]
    pub branch_future_planning: bool,
    #[serde(default)]
    pub opponent_policy: OpponentPolicy,
    #[serde(default)]
    pub sample_choices: bool,
    #[serde(default)]
    pub evaluation: SearchEvaluation,
}

impl OfflinePolicy {
    fn sampled_worlds(self) -> usize {
        self.sampled_worlds.unwrap_or(2)
    }

    fn limits(self, max_nodes: usize) -> SearchLimits {
        SearchLimits {
            max_depth: self.depth,
            max_nodes,
            opponent_policy: self.opponent_policy,
            evaluation: self.evaluation,
        }
    }

    fn resolution_nodes(self) -> usize {
        self.resolution_max_nodes.unwrap_or(self.max_nodes)
    }

    fn budget_from(self, started: Instant) -> Result<ExecutionBudget<'static>> {
        if self.work_budget == Some(WorkBudget::NodesV2) {
            return Err(EngineError::InvalidInput(
                "nodes_v2 uses the versioned arena policy worker".into(),
            ));
        }
        ExecutionBudget::for_search(
            started, self.time_ms, self.work_budget, self.planning_workers.unwrap_or(1),
        )
    }
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

#[derive(Default, serde::Serialize)]
pub struct OfflinePolicyStats {
    planning: usize,
    resolution: usize,
    forced: usize,
    fallback: usize,
    planning_best_so_far: usize,
    planning_shallow_scores: usize,
    planning_nodes: usize,
    planning_greedy_nodes: usize,
    #[serde(skip_serializing_if = "is_zero")]
    planning_greedy_tactical_probes: usize,
    planning_probe_nodes: usize,
    planning_shallow_nodes: usize,
    planning_seed_nodes: usize,
    planning_seed_choices: usize,
    planning_seed_scored: usize,
    planning_seed_deep_agreement: usize,
    planning_completed_rows: usize,
    planning_pruned_rows: usize,
    planning_total_rows: usize,
    planning_with_greedy_scores: usize,
    planning_greedy_card_scores: usize,
    planning_forecast_changes: usize,
    planning_forecast_checks: usize,
    planning_forecast_matches: usize,
    resolution_best_so_far: usize,
    resolution_nodes: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    planning_micros: Vec<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    resolution_micros: Vec<u64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    resolution_fallbacks: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    resolution_fallback_trace: BTreeMap<usize, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    planning_fallbacks: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    planning_fallback_trace: BTreeMap<usize, Vec<String>>,
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
    work_budget: Option<WorkBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_upgrade_batches: Option<Vec<PublicUpgradeBatch>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<Value>,
}

impl NativeSession {
    pub(super) fn planning_trace_json(&self, plans: &[super::PlanningCommitment]) -> Result<Value> {
        let commitments = plans.iter().map(|plan| {
            let printed = |card| self.printed_observation_card(card);
            Ok(json!({
                "hero": self.symbols.name(plan.hero),
                "first": plan.first.map(printed).transpose()?,
                "second": plan.second.map(printed).transpose()?,
            }))
        }).collect::<Result<Vec<_>>>()?;
        Ok(json!({"kind": "planning", "commitments": commitments}))
    }

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
                    || !(1..=8).contains(&policy.sampled_worlds())
                    || policy.planning_workers.is_some_and(|workers| {
                        !(1..=2.min(policy.sampled_worlds())).contains(&workers)
                    })
                    || policy.reply_cap.is_some_and(|cap| !(1..=4096).contains(&cap))
                    || policy.max_nodes == 0
                    || policy.resolution_nodes() == 0
                    || policy
                        .rollout_turns
                        .is_some_and(|turns| !(1..=MAX_SEARCH_ROLLOUT_TURNS).contains(&turns))
            })
        {
            return Err(EngineError::InvalidInput(
                "invalid offline match limits".into(),
            ));
        }
        let [red, blue] = policies;
        for policy in policies {
            policy.budget_from(Instant::now())?;
        }
        if red.work_budget != blue.work_budget
            || (red.work_budget.is_some()
                && (red.max_nodes != blue.max_nodes || red.resolution_nodes() != blue.resolution_nodes()))
        {
            return Err(EngineError::InvalidInput(
                "work-budget matches require the same accounting and node caps on both sides".into(),
            ));
        }
        let budget = ExecutionBudget::default();
        let cutoff = self.search_turn_index() + i128::from(turns);
        let mut trace = capture_trace.then(Vec::new);
        let mut stats = [OfflinePolicyStats::default(), OfflinePolicyStats::default()];
        let mut upgrades = policies
            .iter()
            .any(|policy| policy.depth > 0)
            .then(|| PublicUpgradeHistory::new(self))
            .transpose()?;
        let team_index = |team: &str| match team {
            "RED" => Ok(0),
            "BLUE" => Ok(1),
            _ => Err(EngineError::Unsupported(
                "offline match requires red and blue teams".into(),
            )),
        };
        for revision in 0..usize::from(turns) * 128 {
            if self.planning.phase == Phase::Planning
                && let Some(history) = &mut upgrades
            {
                history.planning_boundary(self, revision as u64)?;
            }
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
                    work_budget: red.work_budget,
                    public_upgrade_batches: if capture_trace {
                        upgrades.map(|history| history.batches)
                    } else {
                        None
                    },
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
                let mut coordinated = [Vec::<super::PlanningCommitment>::new(), Vec::new()];
                for hero in &self.world.heroes {
                    let started = Instant::now();
                    let Some(fallback) = self.first_planning_commitment(hero.id)? else {
                        continue;
                    };
                    let index = team_index(self.symbols.name(hero.team))?;
                    let policy = policies[index];
                    if let Some(plan) = coordinated[index].iter().find(|plan| plan.hero == hero.id) {
                        joint.push(*plan);
                        continue;
                    }
                    stats[index].planning += 1;
                    let plan = if policy.sample_choices {
                        let choices = self.hero_planning_commitments(hero.id)?;
                        choices[choice_index(hero.id, choices.len())]
                    } else if policy.depth == 0 {
                        fallback
                    } else {
                        let team_bots;
                        let solo = [hero.id];
                        let (bots, mode) = if policy.coordinated_planning {
                            team_bots = self.world.heroes.iter()
                                .filter(|ally| ally.team == hero.team)
                                .map(|ally| ally.id)
                                .collect::<Vec<_>>();
                            (team_bots.as_slice(), TeamKnowledgeMode::Coordinated)
                        } else {
                            (solo.as_slice(), TeamKnowledgeMode::TrueRules)
                        };
                        let search = self.search_bot_team_planning_with_seed_pass(
                            hero.id,
                            BotPlanningScope {
                                bots,
                                mode,
                                worlds: policy.sampled_worlds(),
                                seed: epoch_seed,
                            },
                            4,
                            policy.limits(policy.max_nodes),
                            PlanningSearchOptions {
                                rollout_turns: policy.rollout_turns,
                                workers: policy.planning_workers.unwrap_or(1),
                                upgrade_checkpoints: upgrades.as_ref().and_then(PublicUpgradeHistory::checkpoints),
                                reply_cap: policy.reply_cap,
                                reply_probe_cap: None,
                                greedy_card_order: policy.greedy_card_order,
                                tactical_forecast: policy.tactical_forecast,
                                forecast_node_share: policy.forecast_node_share,
                                legacy_fixed_followup: policy.legacy_fixed_followup,
                                fixed_future_planning: policy.fixed_future_planning,
                                branch_future_planning: policy.branch_future_planning,
                            },
                            &policy.budget_from(started)?,
                            policy.planning_seed_pass,
                        );
                        if let Ok(result) = &search {
                            stats[index].planning_nodes += result.matrix.nodes;
                            stats[index].planning_greedy_nodes += result.greedy_nodes;
                            stats[index].planning_greedy_tactical_probes += result.greedy_tactical_probes;
                            stats[index].planning_probe_nodes += result.probe_nodes;
                            stats[index].planning_shallow_nodes += result.shallow_nodes;
                            stats[index].planning_seed_nodes += result.seed_nodes;
                            if result.seed_score.is_some() {
                                stats[index].planning_seed_scored += 1;
                                if result.matrix.score.is_some() && result.matrix.row == 0 {
                                    stats[index].planning_seed_deep_agreement += 1;
                                }
                            }
                            stats[index].planning_completed_rows += result.matrix.completed_rows;
                            stats[index].planning_pruned_rows += result.matrix.pruned_rows;
                            stats[index].planning_total_rows += result.total_rows;
                            stats[index].planning_with_greedy_scores +=
                                usize::from(result.greedy_card_scores > 0);
                            stats[index].planning_greedy_card_scores += result.greedy_card_scores;
                            stats[index].planning_forecast_changes += result.matrix.forecast_changes;
                            stats[index].planning_forecast_checks += result.matrix.forecast_checks;
                            stats[index].planning_forecast_matches += result.matrix.forecast_matches;
                        }
                        match search {
                            Ok(result) if result.matrix.score.is_some()
                                || result.seed_score.is_some() || result.shallow_score.is_some() => {
                                if result.matrix.score.is_none() && result.seed_score.is_some() {
                                    stats[index].planning_seed_choices += 1;
                                }
                                if result.shallow_score.is_some() {
                                    stats[index].planning_shallow_scores += 1;
                                } else if result.matrix.score.is_some()
                                    && result.matrix.completed_rows + result.matrix.pruned_rows
                                    < result.total_rows
                                {
                                    stats[index].planning_best_so_far += 1;
                                }
                                let chosen = result.commitments.iter().find(|plan| plan.hero == hero.id)
                                    .copied().ok_or_else(|| {
                                        EngineError::Unsupported("offline team plan omitted its requesting hero".into())
                                    })?;
                                if policy.coordinated_planning {
                                    coordinated[index] = result.commitments;
                                }
                                chosen
                            },
                            other => {
                                let work = match &other {
                                    Ok(result) => format!(
                                        "; {} of {} nodes, {} probe nodes, {} of {} scored cells, {} of {} worlds",
                                        result.matrix.nodes, policy.max_nodes, result.probe_nodes,
                                        result.matrix.evaluated_cells, result.matrix.total_cells,
                                        result.prepared_worlds, result.requested_worlds,
                                    ),
                                    Err(_) => String::new(),
                                };
                                let reason = match other {
                                    Ok(result) if result.prepared_worlds < result.requested_worlds => {
                                        "interrupted while preparing sampled worlds".into()
                                    }
                                    Ok(_) => "no scored planning row".into(),
                                    Err(EngineError::Unsupported(reason)) => reason,
                                    Err(EngineError::Deadline) => "search deadline".into(),
                                    Err(error) => return Err(error),
                                };
                                let stats = &mut stats[index];
                                stats.fallback += 1;
                                if capture_trace {
                                    if let Some(trace) = &trace {
                                        let detail = if policy.time_ms.is_some() {
                                            format!(" ({} ms{work})", started.elapsed().as_millis())
                                        } else {
                                            work
                                        };
                                        stats.planning_fallback_trace.entry(trace.len()).or_default().push(
                                            format!("{}: {reason}{detail}", self.symbols.name(hero.id)),
                                        );
                                    }
                                    *stats.planning_fallbacks.entry(reason).or_default() += 1;
                                }
                                if policy.coordinated_planning {
                                    for &bot in bots {
                                        if let Some(plan) = self.first_planning_commitment(bot)? {
                                            coordinated[index].push(plan);
                                        }
                                    }
                                }
                                fallback
                            }
                        }
                    };
                    if capture_trace && policy.depth > 0 && policy.time_ms.is_some() {
                        stats[index].planning_micros.push(started.elapsed().as_micros() as u64);
                    }
                    joint.push(plan);
                }
                if let Some(trace) = &mut trace {
                    trace.push(self.planning_trace_json(&joint)?);
                }
                self.apply_joint_planning_ordered(&joint)?;
            } else {
                self.advance_to_decision(&budget)?;
                if matches!(self.planning.phase, Phase::Planning | Phase::GameOver)
                    || self.search_turn_index() >= cutoff
                {
                    continue;
                }
                if self.planning.phase == Phase::LevelUp
                    && let Some(history) = &mut upgrades
                {
                    history.begin_batch(self)?;
                }
                let (owner, team) = self.search_player()?.ok_or_else(|| {
                    EngineError::Unsupported("offline match stalled outside planning".into())
                })?;
                let index = team_index(self.symbols.name(team))?;
                let policy = policies[index];
                stats[index].resolution += 1;
                let started = Instant::now();
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
                        upgrade_checkpoints: upgrades.as_ref().and_then(PublicUpgradeHistory::checkpoints),
                    };
                    let searched = self.search_resolution_worlds(
                        owner,
                        (sample, policy.sampled_worlds(), 1),
                        policy.limits(policy.resolution_nodes()),
                        policy.rollout_turns,
                        policy.rollout_turns.map(|_| sample),
                        &policy.budget_from(started)?,
                    );
                    if let Ok(result) = &searched {
                        stats[index].resolution_nodes += result.nodes;
                    }
                    match searched {
                        Ok(result) if result.score.is_some() => {
                            if !result.resolved && result.completed_depth < policy.depth {
                                stats[index].resolution_best_so_far += 1;
                            }
                            result.decision
                        },
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
                                let description = format!("{prompt}: {reason}");
                                if let Some(trace) = &trace {
                                    stats.resolution_fallback_trace.insert(trace.len(), description.clone());
                                }
                                *stats.resolution_fallbacks.entry(description).or_default() += 1;
                            }
                            fallback(self)?
                        }
                    }
                };
                if capture_trace && policy.depth > 0 && policy.time_ms.is_some() {
                    stats[index].resolution_micros.push(started.elapsed().as_micros() as u64);
                }
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
        let tail = trace.as_deref().unwrap_or_default();
        let recent = &tail[tail.len().saturating_sub(6)..];
        Err(EngineError::Unsupported(format!(
            "offline match exceeded its decision limit at round {}, turn {}, phase {:?}. Recent decisions: {recent:?}",
            self.round.round, self.world.combat.turn, self.planning.phase,
        )))
    }
}
