//! Bounded alpha-beta over live resolution decisions.
type PlanningRows = Vec<Vec<PlanningCommitment>>;
type NarrowedPlanningRows = (PlanningRows, Option<(usize, PlanningCommitment)>);

use super::{
    DecisionMove, EngineError, ExecutionBudget, NativeSession, PlanningCommitment, RequestChoices,
    Result, SearchNodeBudget,
};
use crate::{codec::Phase, data::Symbol};

use std::{
    cmp::Reverse,
    sync::atomic::{AtomicUsize, Ordering},
};

mod diagnostics;
mod evaluation;
mod generation;
mod hidden_world;
mod reuse;
mod rollout;
mod stall;
mod types;

use hidden_world::opponent_draw_seed;

pub use types::{
    BotPlanningScope, BotTeamPlanningSearch, ForecastNodeShare, GameTurnRollout, JointPlans,
    OpponentPolicy, PlanningRolloutSample, PlanningSearch, PlanningSearchOptions,
    ResolutionReuseCensus, ResolutionScoreKind, ResolutionSearch, ResolutionStop,
    SampledPlanningSearch, SearchEvaluation, SearchLimits,
};

const SEARCH_WORKER_STACK: usize = 8 * 1024 * 1024;
pub(super) const MAX_SEARCH_ROLLOUT_TURNS: u8 = 4;

#[derive(Clone, Copy)]
enum JointPlanOrder {
    PlayerFirst,
    Balanced,
    OpponentReplies(u64),
}

type HeroCardScores = Vec<(Option<crate::cards::CardId>, i64)>;
type GreedyCardScores = Vec<(Symbol, HeroCardScores)>;
type GreedyForecasts = Vec<(Symbol, super::GreedySearch)>;

// In the evaluator's normalized score scale, including terminal scores.
const LIVE_CHALLENGER_MARGIN: i64 = 35_000;
const LIVE_STABLE_MARGIN: i64 = 8_000;
const ARENA_SEARCH_CEILING: usize = usize::MAX;
const ARENA_FORECAST_WORK: usize = 8_192;

struct LiveResolution<'a> {
    bots: &'a [Symbol],
    incumbent: Option<&'a DecisionMove>,
    ranking: &'a mut Vec<(usize, i64)>,
    probabilistic_hand_responses: bool,
}

#[derive(Clone, Copy)]
struct HandBelief<'a> {
    known: &'a [Symbol],
    upgrade_checkpoints:
        Option<&'a std::collections::BTreeMap<String, Vec<super::UpgradeCheckpoint>>>,
    turn: i128,
}

struct PlanningWorld {
    session: NativeSession,
    replies: Vec<Vec<PlanningCommitment>>,
    forecasts: GreedyForecasts,
    fixed_opponent_seed: Option<u64>,
}

#[derive(Clone, Copy)]
struct PlanningPartition<'a> {
    rollout_turns: Option<u8>,
    controlled: Option<&'a [Symbol]>,
    shared_nodes: Option<&'a AtomicUsize>,
    greedy_forecasts: &'a [(Symbol, super::GreedySearch)],
    future_branching: Option<bool>,
    legacy_fixed_followup: bool,
    fixed_opponent_seed: Option<u64>,
}

// The implied weight is exp(-cost / TEMPERATURE). The floor also covers
// legal cards for which the greedy pass did not finish a rollout.
pub(super) const GREEDY_CARD_TEMPERATURE: u64 = 100;
const GREEDY_CARD_MAX_COST: u64 = 4 * GREEDY_CARD_TEMPERATURE;

pub(super) fn greedy_card_cost(best: i64, score: Option<i64>) -> u64 {
    score.map_or(GREEDY_CARD_MAX_COST, |score| {
        best.abs_diff(score).min(GREEDY_CARD_MAX_COST)
    })
}

fn shared_row_likelihoods(
    rows: &[Vec<PlanningCommitment>],
    scores: &[(Symbol, HeroCardScores)],
) -> Vec<f64> {
    let scored_heroes: Vec<_> = scores
        .iter()
        .filter_map(|(hero, cards)| {
            let best = cards.iter().map(|(_, score)| *score).max()?;
            Some((hero, cards, best))
        })
        .collect();
    let mut weights: Vec<_> = rows
        .iter()
        .map(|row| {
            let cost: u64 = row
                .iter()
                .filter_map(|commitment| {
                    let (_, cards, best) = scored_heroes
                        .iter()
                        .find(|(hero, _, _)| **hero == commitment.hero)?;
                    let score = cards
                        .iter()
                        .find_map(|(card, score)| (*card == commitment.first).then_some(*score));
                    Some(greedy_card_cost(*best, score))
                })
                .sum();
            (-(cost as f64) / GREEDY_CARD_TEMPERATURE as f64).exp()
        })
        .collect();
    let total: f64 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= total;
    }
    weights
}

// A row is eligible only after it has a worst reply in every sampled world.
// Compare exact sums so integer rounding cannot change the chosen row.
// Enumerate the most likely joint commitments without materializing the
// Cartesian product. Sum negative log weights to compare their products.
fn ranked_reply_indices(
    choices: &[Vec<PlanningCommitment>],
    strides: &[usize],
    costs: &[Option<Vec<u64>>],
    cap: usize,
) -> Vec<usize> {
    use std::{
        cmp::Reverse,
        collections::{BinaryHeap, HashSet},
    };

    let mut queue = BinaryHeap::from([Reverse((0u64, 0usize, 0usize))]);
    let mut seen = HashSet::from([0]);
    let mut indices = Vec::with_capacity(cap);
    while indices.len() < cap {
        let Some(Reverse((cost, rank, index))) = queue.pop() else {
            break;
        };
        indices.push(index);
        if indices.len() == cap {
            break;
        }
        for ((dimension, &stride), costs) in choices.iter().zip(strides).zip(costs) {
            let digit = index / stride % dimension.len();
            if digit + 1 < dimension.len() {
                let next = index + stride;
                if seen.insert(next) {
                    let delta = costs.as_ref().map_or(0, |row| row[digit + 1] - row[digit]);
                    queue.push(Reverse((cost + delta, rank + 1, next)));
                }
            }
        }
    }
    indices
}

struct PlanningWorldOptions<'a> {
    limits: SearchLimits,
    workers: usize,
    partition: PlanningPartition<'a>,
    budget: &'a ExecutionBudget<'a>,
    min_live_rows: Option<usize>,
}

fn search_planning_worlds(
    player: Symbol,
    rows: &[Vec<PlanningCommitment>],
    samples: &mut [PlanningWorld],
    options: PlanningWorldOptions<'_>,
) -> Result<PlanningSearch> {
    let PlanningWorldOptions {
        limits,
        workers,
        partition,
        budget,
        min_live_rows,
    } = options;
    let mut result = PlanningSearch {
        row: 0,
        score: None,
        completed_rows: 0,
        pruned_rows: 0,
        evaluated_cells: 0,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
        total_cells: rows.len()
            * samples
                .iter()
                .map(|sample| sample.replies.len())
                .sum::<usize>(),
        nodes: 0,
    };
    let sample_count = samples.len() as i128;
    let margin = i128::from(LIVE_CHALLENGER_MARGIN) * sample_count;
    let mut best = None;
    let mut runner_up = None;
    'rows: for (index, row) in rows.iter().enumerate() {
        if min_live_rows.is_some_and(|minimum| index >= minimum)
            && !live_roots_uncertain(best, runner_up, 0, margin)
        {
            break;
        }
        let mut sum = 0i128;
        let mut worlds_left = sample_count;
        for group in samples.chunks_mut(workers) {
            // Even a win in every unscored world cannot beat this incumbent.
            // As in the single-world matrix, the earlier row wins ties.
            let upper = sum + worlds_left * i128::from(WIN);
            if best.is_some_and(|(_, score)| upper <= score) {
                runner_up = Some(runner_up.map_or(upper, |score| score.max(upper)));
                result.pruned_rows += 1;
                continue 'rows;
            }
            worlds_left -= group.len() as i128;
            let remaining = limits.max_nodes - result.nodes;
            if remaining == 0 {
                break 'rows;
            }
            let evaluate = |sample: &mut PlanningWorld, shared_nodes| {
                sample.session.search_planning_matrix_partition(
                    player,
                    std::slice::from_ref(row),
                    &sample.replies,
                    SearchLimits {
                        max_nodes: remaining,
                        ..limits
                    },
                    PlanningPartition {
                        shared_nodes,
                        greedy_forecasts: &sample.forecasts,
                        fixed_opponent_seed: sample.fixed_opponent_seed,
                        ..partition
                    },
                    budget,
                )
            };
            let evaluations = if group.len() == 1 {
                vec![evaluate(&mut group[0], None)]
            } else {
                let shared_nodes = AtomicUsize::new(remaining);
                let shared_nodes = &shared_nodes;
                std::thread::scope(|threads| {
                    let jobs = group
                        .iter_mut()
                        .map(|sample| {
                            std::thread::Builder::new()
                                .name("planning-score".into())
                                .stack_size(SEARCH_WORKER_STACK)
                                .spawn_scoped(threads, move || evaluate(sample, Some(shared_nodes)))
                                .map_err(|error| {
                                    EngineError::Unsupported(format!(
                                        "could not spawn planning scoring worker: {error}",
                                    ))
                                })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(jobs
                        .into_iter()
                        .map(|job| {
                            job.join().unwrap_or_else(|_| {
                                Err(EngineError::Unsupported(
                                    "planning scoring worker panicked".into(),
                                ))
                            })
                        })
                        .collect::<Vec<_>>())
                })?
            };
            let mut incomplete = false;
            for evaluation in evaluations {
                let evaluation = evaluation?;
                result.nodes += evaluation.nodes;
                result.evaluated_cells += evaluation.evaluated_cells;
                result.forecast_changes += evaluation.forecast_changes;
                result.forecast_checks += evaluation.forecast_checks;
                result.forecast_matches += evaluation.forecast_matches;
                if let Some(score) = evaluation.score {
                    sum += i128::from(score);
                } else {
                    incomplete = true;
                }
            }
            if incomplete {
                break 'rows;
            }
        }
        result.completed_rows += 1;
        if best.is_none_or(|(_, score)| sum > score) {
            runner_up = best.map(|(_, score)| score);
            best = Some((index, sum));
            result.row = index;
            result.score = Some((sum / sample_count) as i64);
        } else {
            runner_up = Some(runner_up.map_or(sum, |score| score.max(sum)));
        }
    }
    Ok(result)
}

pub(super) const WIN: i64 = 1_000_000;

impl NativeSession {
    /// Search generated plans on one clean sampled world. The returned
    /// commitment belongs to the requesting hero and is valid on the live root.
    pub fn search_sampled_planning(
        &self,
        player: Symbol,
        seed: u64,
        cap: usize,
        limits: SearchLimits,
        budget: &ExecutionBudget<'_>,
    ) -> Result<SampledPlanningSearch> {
        let mut worker = self.sampled_planning_world(player, seed)?;
        worker.search_plans_on_sample(
            player,
            seed,
            cap,
            limits,
            PlanningSearchOptions {
                rollout_turns: None,
                workers: 1,
                upgrade_checkpoints: None,
                reply_cap: None,
                reply_probe_cap: None,
                greedy_card_order: false,
                tactical_forecast: false,
                forecast_node_share: ForecastNodeShare::default(),
                legacy_fixed_followup: false,
                fixed_future_planning: false,
                branch_future_planning: false,
            },
            budget,
        )
    }

    pub fn search_sampled_planning_with_options(
        &mut self,
        player: Symbol,
        seed: u64,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions<'_>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<SampledPlanningSearch> {
        if options.workers == 1 {
            self.with_sampled_planning_checkpoints(
                player,
                seed,
                options.upgrade_checkpoints,
                |worker| worker.search_plans_on_sample(player, seed, cap, limits, options, budget),
            )
        } else {
            let mut worker = self.sampled_planning_world_with_checkpoints(
                player,
                seed,
                options.upgrade_checkpoints,
            )?;
            worker.search_plans_on_sample(player, seed, cap, limits, options, budget)
        }
    }

    fn greedy_seed_budget<'a>(budget: &ExecutionBudget<'a>) -> ExecutionBudget<'a> {
        let now = std::time::Instant::now();
        ExecutionBudget {
            deadline: budget
                .deadline
                .map(|end| now + end.saturating_duration_since(now) / 4),
            ..*budget
        }
    }

    fn greedy_planning_scores(
        &mut self,
        seed: u64,
        node_cap: usize,
        tactical_replies: bool,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(GreedyCardScores, usize, GreedyForecasts)> {
        let max_nodes = node_cap / self.world.heroes.len().max(1);
        if max_nodes == 0 {
            return Ok((Vec::new(), 0, Vec::new()));
        }
        let heroes: Vec<_> = self.world.heroes.iter().map(|hero| hero.id).collect();
        let mut scores = Vec::new();
        let mut forecasts = Vec::new();
        let mut nodes = 0;
        let now = std::time::Instant::now();
        let seed_time = budget
            .deadline
            .map(|deadline| deadline.saturating_duration_since(now));
        for hero in heroes {
            if self.hero_planning_commitments(hero)?.len() < 2 {
                continue;
            }
            let hero_budget = ExecutionBudget {
                deadline: seed_time.zip(budget.deadline).map(|(time, end)| {
                    (std::time::Instant::now() + time / self.world.heroes.len().max(1) as u32)
                        .min(end)
                }),
                ..*budget
            };
            let result = self.search_greedy_card_scores(
                hero,
                super::GreedySearchOptions {
                    controlled: &[hero],
                    turns: 4,
                    fixed_card_order: false,
                    defer_upgrades: true,
                    positional_ordering: true,
                    position_first: true,
                    tactical_replies,
                    reuse_movement: true,
                    profile: false,
                    max_nodes,
                    seed,
                    evaluation: SearchEvaluation::MaterialPosition,
                },
                &hero_budget,
            )?;
            nodes += result.nodes;
            let row = result.turn_card_scores(0);
            if !row.is_empty() {
                scores.push((hero, row));
                forecasts.push((hero, result));
            }
        }
        Ok((scores, nodes, forecasts))
    }

    fn fixed_planning_reply(
        &self,
        player: Symbol,
        controlled: Option<&[Symbol]>,
        forecasts: &[(Symbol, super::GreedySearch)],
        seed: u64,
    ) -> Result<JointPlans> {
        let team = self
            .world
            .hero(player)
            .ok_or_else(|| EngineError::InvalidInput("unknown search player".into()))?
            .team;
        let mut reply = Vec::new();
        for hero in &self.world.heroes {
            if controlled.map_or(hero.team == team, |ids| ids.contains(&hero.id)) {
                continue;
            }
            let choice = self.sample_forecast_commitment(hero.id, forecasts, &[], seed)?;
            if let Some(choice) = choice {
                reply.push(choice);
            } else if let Some(choice) =
                self.greedy_opponent_commitment(hero.id, opponent_draw_seed(self, seed, hero.id))?
            {
                reply.push(choice);
            }
        }
        Ok(JointPlans {
            plans: vec![reply],
            total: 1,
        })
    }

    fn search_plans_on_sample(
        &mut self,
        player: Symbol,
        seed: u64,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions<'_>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<SampledPlanningSearch> {
        let (greedy_scores, greedy_nodes, forecasts) = if options.greedy_card_order {
            self.greedy_planning_scores(
                seed,
                options.forecast_node_share.allowance(limits.max_nodes),
                options.tactical_forecast,
                &Self::greedy_seed_budget(budget),
            )?
        } else {
            (Vec::new(), 0, Vec::new())
        };
        let limits = SearchLimits {
            max_nodes: limits.max_nodes - greedy_nodes,
            ..limits
        };
        let fixed_opponent_seed = options.fixed_opponent_seed(seed);
        let solo = [player];
        let controlled = fixed_opponent_seed.map(|_| solo.as_slice());
        let friendly = self.joint_plans_with_reply_order(
            player,
            true,
            cap,
            controlled,
            JointPlanOrder::PlayerFirst,
            &greedy_scores,
        )?;
        let opposing = if let Some(seed) = fixed_opponent_seed {
            self.fixed_planning_reply(player, controlled, &forecasts, seed)?
        } else {
            self.joint_plans_with_reply_order(
                player,
                false,
                options.reply_cap.unwrap_or(cap),
                None,
                JointPlanOrder::OpponentReplies(seed),
                &greedy_scores,
            )?
        };
        let partition = PlanningPartition {
            rollout_turns: options.rollout_turns,
            controlled,
            shared_nodes: None,
            greedy_forecasts: &forecasts,
            future_branching: options.future_branching(),
            legacy_fixed_followup: options.legacy_fixed_followup,
            fixed_opponent_seed,
        };
        let mut matrix = if options.workers == 1 {
            self.search_planning_matrix_partition(
                player,
                &friendly.plans,
                &opposing.plans,
                limits,
                partition,
                budget,
            )?
        } else {
            self.search_planning_matrix_parallel_partition(
                player,
                &friendly.plans,
                &opposing.plans,
                limits,
                options.workers,
                partition,
                budget,
            )?
        };
        matrix.nodes += greedy_nodes;
        let commitment = friendly.plans[matrix.row]
            .iter()
            .find(|plan| plan.hero == player)
            .copied()
            .ok_or_else(|| {
                EngineError::InvalidInput("search player has no open planning slot".into())
            })?;
        Ok(SampledPlanningSearch {
            commitment,
            matrix,
            total_friendly_plans: friendly.total,
            total_opposing_plans: opposing.total,
        })
    }

    /// Plan for authorized bots only. Uncontrolled open slots are evaluated
    /// as uncertain replies, including allied humans in the pre-reveal epoch.
    pub fn search_bot_team_planning(
        &self,
        player: Symbol,
        scope: BotPlanningScope<'_>,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions<'_>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<BotTeamPlanningSearch> {
        self.search_bot_team_planning_with_seed_pass(
            player, scope, cap, limits, options, budget, false,
        )
    }

    // Keep explicit budget, scope and deterministic sampling inputs until the search API split (C09/C10).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search_bot_team_planning_with_seed_pass(
        &self,
        player: Symbol,
        scope: BotPlanningScope<'_>,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions<'_>,
        budget: &ExecutionBudget<'_>,
        seed_pass: bool,
    ) -> Result<BotTeamPlanningSearch> {
        self.search_bot_team_planning_inner(
            player, scope, cap, limits, options, budget, seed_pass, None,
        )
    }

    /// Arena v2 runs the same fixed stages at every requested node budget.
    /// The shared meter interrupts whichever stage is active at the limit.
    pub fn search_node_limited_planning(
        &self,
        player: Symbol,
        scope: BotPlanningScope<'_>,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions<'_>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<BotTeamPlanningSearch> {
        self.search_turn_cutoff(options.rollout_turns)?;
        let meter = arena_search_meter(limits, budget)?;
        let budget = ExecutionBudget {
            search_nodes: Some(&meter),
            ..*budget
        };
        let run = |cap, max_depth, max_nodes, options, incumbent: Option<&[PlanningCommitment]>| {
            self.search_bot_team_planning_inner(
                player,
                scope,
                cap,
                SearchLimits {
                    max_depth,
                    max_nodes,
                    ..limits
                },
                options,
                &budget,
                false,
                incumbent,
            )
        };
        let mut best = run(
            cap.min(4),
            0,
            512,
            PlanningSearchOptions {
                rollout_turns: None,
                greedy_card_order: false,
                reply_cap: Some(1),
                reply_probe_cap: None,
                ..options
            },
            None,
        )?;
        if meter.remaining() > 0 && best.matrix.score != Some(WIN) {
            let shorter = PlanningSearchOptions {
                rollout_turns: options.rollout_turns.map(|turns| turns.min(2)),
                ..options
            };
            if let Some(result) = retain_arena_backup(run(
                cap.min(4),
                limits.max_depth.min(2),
                16_384,
                shorter,
                None,
            ))? && planning_has_score(&result)
            {
                best = result;
            }
        }
        if meter.remaining() > 0
            && best.matrix.score != Some(WIN)
            && let Some(deeper) = retain_arena_backup(run(
                cap,
                limits.max_depth,
                ARENA_SEARCH_CEILING,
                options,
                planning_has_score(&best).then_some(best.commitments.as_slice()),
            ))?
            && (deeper.matrix.score.is_some()
                || (!planning_has_score(&best) && planning_has_score(&deeper)))
        {
            best = deeper;
        }
        best.matrix.nodes = limits.max_nodes - meter.remaining();
        Ok(best)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Retain explicit search operands until the planned search API split"
    )]
    pub(super) fn search_live_bot_team_planning(
        &self,
        player: Symbol,
        scope: BotPlanningScope<'_>,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions<'_>,
        budget: &ExecutionBudget<'_>,
        incumbent: Option<&[PlanningCommitment]>,
    ) -> Result<BotTeamPlanningSearch> {
        self.search_bot_team_planning_inner(
            player, scope, cap, limits, options, budget, false, incumbent,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Retain explicit search operands until the planned search API split"
    )]
    fn search_bot_team_planning_inner(
        &self,
        player: Symbol,
        scope: BotPlanningScope<'_>,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions<'_>,
        budget: &ExecutionBudget<'_>,
        seed_pass: bool,
        live_incumbent: Option<&[PlanningCommitment]>,
    ) -> Result<BotTeamPlanningSearch> {
        let seed = scope.seed;
        let reply_cap = options.reply_cap.unwrap_or(cap);
        let probe_cap = options.reply_probe_cap.unwrap_or(reply_cap);
        if !(1..=4096).contains(&reply_cap) || !(1..=4096).contains(&probe_cap) {
            return Err(EngineError::InvalidInput(
                "planning reply caps must be in 1..4096".into(),
            ));
        }

        if !(1..=4).contains(&options.workers) {
            return Err(EngineError::InvalidInput(
                "scoped bot planning requires 1..4 workers".into(),
            ));
        }
        if !(1..=8).contains(&scope.worlds) {
            return Err(EngineError::InvalidInput(
                "scoped bot planning requires 1..8 worlds".into(),
            ));
        }
        if limits.max_depth > 64 || limits.max_nodes == 0 {
            return Err(EngineError::InvalidInput(
                "invalid scoped planning limits".into(),
            ));
        }
        self.search_turn_cutoff(options.rollout_turns)?;
        let team = self.knowledge_team(player, scope.bots, true)?;
        let controlled = self
            .world
            .heroes
            .iter()
            .filter(|hero| {
                hero.id == player
                    || (hero.team == team
                        && scope.mode != super::TeamKnowledgeMode::TrueRules
                        && scope.bots.contains(&hero.id))
            })
            .map(|hero| hero.id)
            .collect::<Vec<_>>();
        let mut rows = self.joint_plans_partition(player, true, cap, Some(&controlled))?;
        if rows.plans[0].is_empty() {
            return Err(EngineError::InvalidInput(
                "no open controlled planning slots".into(),
            ));
        }
        let partition = PlanningPartition {
            rollout_turns: options.rollout_turns,
            controlled: Some(&controlled),
            shared_nodes: None,
            greedy_forecasts: &[],
            future_branching: options.future_branching(),
            legacy_fixed_followup: options.legacy_fixed_followup,
            fixed_opponent_seed: None,
        };
        let probe_budget = ExecutionBudget {
            deadline: budget.deadline.map(|deadline| {
                let now = std::time::Instant::now();
                now + deadline.saturating_duration_since(now) / 2
            }),
            ..*budget
        };
        let seed_budget = Self::greedy_seed_budget(budget);
        let seed_slice = seed_budget.deadline.map(|end| {
            end.saturating_duration_since(std::time::Instant::now())
                / scope.worlds.div_ceil(options.workers) as u32
        });
        let forecast_work = if budget.search_nodes.is_some() {
            ARENA_FORECAST_WORK.min(limits.max_nodes)
        } else {
            limits.max_nodes
        };
        let prepare = |index: usize| {
            budget.check(0)?;
            let sample_seed = seed.wrapping_add((index as u64).wrapping_mul(0x9e3779b97f4a7c15));
            let mut world = self.sampled_bot_team_world(
                player,
                scope.bots,
                scope.mode,
                sample_seed,
                options.upgrade_checkpoints,
            )?;
            let (greedy_scores, greedy_nodes, forecasts) = if options.greedy_card_order {
                let world_budget = ExecutionBudget {
                    deadline: seed_budget
                        .deadline
                        .zip(seed_slice)
                        .map(|(end, slice)| end.min(std::time::Instant::now() + slice)),
                    ..seed_budget
                };
                world.greedy_planning_scores(
                    sample_seed,
                    options.forecast_node_share.allowance(forecast_work) / scope.worlds,
                    options.tactical_forecast,
                    &world_budget,
                )?
            } else {
                (Vec::new(), 0, Vec::new())
            };
            let fixed_opponent_seed = options.fixed_opponent_seed(sample_seed);
            let mut replies = if let Some(seed) = fixed_opponent_seed {
                world
                    .fixed_planning_reply(player, Some(&controlled), &forecasts, seed)?
                    .plans
            } else {
                world
                    .joint_plans_with_reply_order(
                        player,
                        false,
                        probe_cap.max(reply_cap),
                        Some(&controlled),
                        JointPlanOrder::OpponentReplies(sample_seed),
                        &greedy_scores,
                    )?
                    .plans
            };
            let probe_nodes = forecast_work / 4 / scope.worlds;
            // Each cell consumes at least one node. Screening cannot change
            // the shortlist unless it scores more than reply_cap full columns.
            let can_change_shortlist = probe_nodes / rows.plans.len() > reply_cap;
            let probe = if replies.len() > reply_cap && can_change_shortlist {
                world.screen_planning_replies(
                    player,
                    &rows.plans,
                    &mut replies,
                    reply_cap,
                    SearchLimits {
                        max_nodes: probe_nodes,
                        ..limits
                    },
                    partition,
                    &probe_budget,
                )?
            } else {
                (0, 0)
            };
            replies.truncate(reply_cap);
            let weights = options
                .greedy_card_order
                .then(|| shared_row_likelihoods(&rows.plans, &greedy_scores));
            let card_scores = greedy_scores
                .iter()
                .map(|(_, cards)| cards.len())
                .sum::<usize>();
            Ok((
                PlanningWorld {
                    session: world,
                    replies,
                    forecasts,
                    fixed_opponent_seed,
                },
                probe,
                greedy_nodes,
                card_scores,
                weights,
            ))
        };
        let mut samples = Vec::with_capacity(scope.worlds);
        let mut probe_nodes = 0;
        let mut screened_replies = 0;
        let mut greedy_nodes = 0;
        let mut greedy_tactical_probes = 0;
        let mut greedy_card_scores = 0;
        let mut row_weights = options
            .greedy_card_order
            .then(|| vec![0.0; rows.plans.len()]);
        for start in (0..scope.worlds).step_by(options.workers) {
            let end = (start + options.workers).min(scope.worlds);
            let batch = if end - start == 1 {
                vec![prepare(start)]
            } else {
                std::thread::scope(|threads| {
                    let jobs = (start..end)
                        .map(|index| {
                            if options.greedy_card_order {
                                std::thread::Builder::new()
                                    .name("planning-prepare".into())
                                    .stack_size(SEARCH_WORKER_STACK)
                                    .spawn_scoped(threads, move || prepare(index))
                                    .map_err(|error| EngineError::Unsupported(error.to_string()))
                            } else {
                                Ok(threads.spawn(move || prepare(index)))
                            }
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok::<_, EngineError>(
                        jobs.into_iter()
                            .map(|job| {
                                job.join().unwrap_or_else(|_| {
                                    Err(EngineError::Unsupported(
                                        "planning sample worker panicked".into(),
                                    ))
                                })
                            })
                            .collect(),
                    )
                })?
            };
            let mut interrupted = false;
            for sample in batch {
                match sample {
                    Ok((sample, (used, screened), seeded, card_scores, weights)) => {
                        greedy_card_scores += card_scores;
                        if let (Some(totals), Some(weights)) = (&mut row_weights, weights) {
                            for (total, weight) in totals.iter_mut().zip(weights) {
                                *total += weight;
                            }
                        }
                        greedy_nodes += seeded;
                        greedy_tactical_probes += sample
                            .forecasts
                            .iter()
                            .map(|(_, forecast)| forecast.tactical_probes)
                            .sum::<usize>();
                        probe_nodes += used;
                        screened_replies += screened;
                        samples.push(sample);
                    }
                    Err(EngineError::Deadline | EngineError::Cancelled) => interrupted = true,
                    Err(error) => return Err(error),
                }
            }
            if interrupted {
                return Ok(BotTeamPlanningSearch {
                    requested_worlds: scope.worlds,
                    total_rows: rows.plans.len(),
                    probe_nodes,
                    greedy_nodes,
                    greedy_tactical_probes,
                    shallow_nodes: 0,
                    seed_nodes: 0,
                    seed_score: None,
                    greedy_card_scores,
                    screened_replies,
                    prepared_worlds: samples.len(),
                    commitments: rows.plans[0].clone(),
                    shallow_score: None,
                    matrix: PlanningSearch {
                        row: 0,
                        score: None,
                        completed_rows: 0,
                        pruned_rows: 0,
                        evaluated_cells: 0,
                        forecast_changes: 0,
                        forecast_checks: 0,
                        forecast_matches: 0,
                        total_cells: 0,
                        nodes: probe_nodes + greedy_nodes,
                    },
                });
            }
        }
        if let Some(weights) = row_weights {
            let mut weighted_rows: Vec<_> = weights.into_iter().zip(rows.plans).collect();
            weighted_rows.sort_by(|(left, _), (right, _)| right.total_cmp(left));
            rows.plans = weighted_rows.into_iter().map(|(_, row)| row).collect();
        }
        let live_incumbent =
            live_incumbent.and_then(|incumbent| rows.plans.iter().position(|row| row == incumbent));
        if let Some(index) = live_incumbent {
            rows.plans[..=index].rotate_right(1);
        }
        let remaining = limits.max_nodes - probe_nodes - greedy_nodes;
        let shallow_nodes = if options.rollout_turns.is_some() {
            (remaining / 16).min(512)
        } else {
            0
        };
        let seed_allowance = if seed_pass && limits.max_depth > 0 && options.rollout_turns.is_some()
        {
            (remaining / 16).min(4096)
        } else {
            0
        };
        let mut seed_used = 0;
        let mut seed_score = None;
        if seed_allowance > 0 {
            let seed_budget = ExecutionBudget {
                deadline: budget.deadline.map(|end| {
                    let now = std::time::Instant::now();
                    now + (end.saturating_duration_since(now) / 10)
                        .min(std::time::Duration::from_millis(250))
                }),
                ..*budget
            };
            let seed = search_planning_worlds(
                player,
                &rows.plans[..rows.plans.len().min(4)],
                &mut samples,
                PlanningWorldOptions {
                    limits: SearchLimits {
                        max_nodes: seed_allowance,
                        max_depth: 0,
                        ..limits
                    },
                    workers: options.workers,
                    partition: PlanningPartition {
                        future_branching: Some(false),
                        ..partition
                    },
                    budget: &seed_budget,
                    min_live_rows: None,
                },
            )?;
            seed_used = seed.nodes;
            if seed.score.is_some() {
                rows.plans[..=seed.row].rotate_right(1);
                seed_score = seed.score;
            }
        }
        let deep_limits = SearchLimits {
            max_nodes: remaining - shallow_nodes - seed_used,
            ..limits
        };
        let deep_budget = ExecutionBudget {
            deadline: budget.deadline.map(|end| {
                let spare = end.saturating_duration_since(std::time::Instant::now()) / 10;
                end - if shallow_nodes > 0 {
                    spare.min(std::time::Duration::from_millis(100))
                } else {
                    std::time::Duration::ZERO
                }
            }),
            ..*budget
        };
        let mut matrix = if let [sample] = samples.as_mut_slice() {
            sample.session.search_planning_matrix_partition(
                player,
                &rows.plans,
                &sample.replies,
                deep_limits,
                PlanningPartition {
                    greedy_forecasts: &sample.forecasts,
                    fixed_opponent_seed: sample.fixed_opponent_seed,
                    ..partition
                },
                &deep_budget,
            )?
        } else {
            search_planning_worlds(
                player,
                &rows.plans,
                &mut samples,
                PlanningWorldOptions {
                    limits: deep_limits,
                    workers: options.workers,
                    partition,
                    budget: &deep_budget,
                    min_live_rows: live_incumbent.map(|_| 4),
                },
            )?
        };
        // A seed winner was moved to row zero. Keep it only when no deep
        // row finishes, and never compare scores from different depths.
        let mut selected_row = matrix.row;
        let mut shallow_score = None;
        let mut shallow_used = 0;
        if matrix.score.is_none() && seed_score.is_none() && shallow_nodes > 0 {
            // Score the leading rows over the same replies and worlds without
            // rolling into another turn. Keep shallow and deep scores separate.
            let shallow = search_planning_worlds(
                player,
                &rows.plans[..rows.plans.len().min(4)],
                &mut samples,
                PlanningWorldOptions {
                    limits: SearchLimits {
                        max_nodes: shallow_nodes,
                        max_depth: 0,
                        ..limits
                    },
                    workers: options.workers,
                    partition: PlanningPartition {
                        rollout_turns: None,
                        future_branching: Some(false),
                        ..partition
                    },
                    budget,
                    min_live_rows: None,
                },
            )?;
            shallow_used = shallow.nodes;
            matrix.nodes += shallow_used;
            if shallow.score.is_some() {
                selected_row = shallow.row;
                shallow_score = shallow.score;
            }
        }
        matrix.nodes += probe_nodes + greedy_nodes + seed_used;
        Ok(BotTeamPlanningSearch {
            probe_nodes,
            greedy_nodes,
            greedy_tactical_probes,
            shallow_nodes: shallow_used,
            seed_nodes: seed_used,
            seed_score,
            greedy_card_scores,
            screened_replies,
            requested_worlds: scope.worlds,
            total_rows: rows.plans.len(),
            prepared_worlds: scope.worlds,
            commitments: rows.plans[selected_row].clone(),
            shallow_score,
            matrix,
        })
    }

    // Keep explicit budget, scope and deterministic sampling inputs until the search API split (C09/C10).
    #[allow(clippy::too_many_arguments)]
    fn screen_planning_replies(
        &mut self,
        player: Symbol,
        references: &[Vec<PlanningCommitment>],
        replies: &mut Vec<Vec<PlanningCommitment>>,
        reply_cap: usize,
        limits: SearchLimits,
        partition: PlanningPartition<'_>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(usize, usize)> {
        let mut used = 0;
        let mut screened = 0;
        let mut rankings = vec![Vec::new(); references.len()];
        let mut scores = Vec::with_capacity(references.len());
        'replies: for (index, reply) in replies.iter().enumerate() {
            scores.clear();
            for reference in references {
                if used == limits.max_nodes {
                    break 'replies;
                }
                let probe = self.search_planning_matrix_partition(
                    player,
                    std::slice::from_ref(reference),
                    std::slice::from_ref(reply),
                    SearchLimits {
                        max_depth: 1,
                        max_nodes: limits.max_nodes - used,
                        ..limits
                    },
                    PlanningPartition {
                        rollout_turns: Some(1),
                        ..partition
                    },
                    budget,
                )?;
                used += probe.nodes;
                let Some(score) = probe.score else {
                    break 'replies;
                };
                scores.push(score);
            }
            for (ranked, &score) in rankings.iter_mut().zip(&scores) {
                ranked.push((score, index));
            }
            screened += 1;
        }
        // Below the shortlist width, ranking a defense prefix cannot add any
        // replies. Preserve its original order in that case.
        if screened <= reply_cap {
            return Ok((used, screened));
        }
        for ranked in &mut rankings {
            ranked.sort_unstable();
        }
        // Give each friendly row a chance to contribute its worst reply.
        // Deduplicate without disturbing the stable rank and row ordering.
        let mut chosen = Vec::with_capacity(reply_cap);
        let mut included = vec![false; replies.len()];
        for rank in 0..screened {
            for ranked in &rankings {
                let (_, index) = ranked[rank];
                if !std::mem::replace(&mut included[index], true) {
                    chosen.push(replies[index].clone());
                    if chosen.len() == reply_cap {
                        *replies = chosen;
                        return Ok((used, screened));
                    }
                }
            }
        }
        Ok((used, screened))
    }

    /// Evaluate the supplied simultaneous choices on one sampled world.
    /// A row is eligible only after all its opponent replies finish. The
    /// reported score is the maximum of those complete rows' minimum payoffs.
    pub fn search_planning_matrix(
        &mut self,
        player: Symbol,
        rows: &[Vec<PlanningCommitment>],
        columns: &[Vec<PlanningCommitment>],
        limits: SearchLimits,
        budget: &ExecutionBudget<'_>,
    ) -> Result<PlanningSearch> {
        self.search_planning_matrix_with_rollout(player, rows, columns, limits, None, budget)
    }

    /// Evaluate independent row groups in worker-local sessions. Each group
    /// has a deterministic share of the node budget and the common deadline.
    pub fn search_planning_matrix_parallel(
        &self,
        player: Symbol,
        rows: &[Vec<PlanningCommitment>],
        columns: &[Vec<PlanningCommitment>],
        limits: SearchLimits,
        workers: usize,
        budget: &ExecutionBudget<'_>,
    ) -> Result<PlanningSearch> {
        self.search_planning_matrix_parallel_partition(
            player,
            rows,
            columns,
            limits,
            workers,
            PlanningPartition {
                rollout_turns: None,
                controlled: None,
                shared_nodes: None,
                greedy_forecasts: &[],
                future_branching: None,
                legacy_fixed_followup: false,
                fixed_opponent_seed: None,
            },
            budget,
        )
    }

    // Keep explicit budget, scope and deterministic sampling inputs until the search API split (C09/C10).
    #[allow(clippy::too_many_arguments)]
    fn search_planning_matrix_parallel_partition(
        &self,
        player: Symbol,
        rows: &[Vec<PlanningCommitment>],
        columns: &[Vec<PlanningCommitment>],
        limits: SearchLimits,
        workers: usize,
        partition: PlanningPartition<'_>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<PlanningSearch> {
        if !(1..=4).contains(&workers) || rows.is_empty() || limits.max_nodes == 0 {
            return Err(EngineError::InvalidInput(
                "parallel planning requires 1..4 workers, rows and a positive node limit".into(),
            ));
        }
        let workers = workers.min(rows.len()).min(limits.max_nodes);
        let chunk_size = rows.len().div_ceil(workers);
        let groups = rows.len().div_ceil(chunk_size);
        let total_cells = rows
            .len()
            .checked_mul(columns.len())
            .ok_or_else(|| EngineError::InvalidInput("planning matrix is too large".into()))?;
        let parts = std::thread::scope(|scope| {
            let mut jobs = Vec::with_capacity(groups);
            for (group, rows) in rows.chunks(chunk_size).enumerate() {
                let mut session = self.clone();
                let max_nodes =
                    limits.max_nodes / groups + usize::from(group < limits.max_nodes % groups);
                let job = std::thread::Builder::new()
                    .name("planning-matrix".into())
                    .stack_size(SEARCH_WORKER_STACK)
                    .spawn_scoped(scope, move || {
                        session.search_planning_matrix_partition(
                            player,
                            rows,
                            columns,
                            SearchLimits {
                                max_nodes,
                                ..limits
                            },
                            partition,
                            budget,
                        )
                    })
                    .map_err(|error| {
                        EngineError::Unsupported(format!(
                            "could not spawn planning matrix worker: {error}",
                        ))
                    })?;
                jobs.push(job);
            }
            jobs.into_iter()
                .map(|job| {
                    job.join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect::<Result<Vec<_>>>()
        })?;
        let mut result = PlanningSearch {
            row: 0,
            score: None,
            completed_rows: 0,
            pruned_rows: 0,
            evaluated_cells: 0,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
            total_cells,
            nodes: 0,
        };
        for (group, part) in parts.into_iter().enumerate() {
            if let Some(score) = part.score
                && result.score.is_none_or(|best| score > best)
            {
                result.score = Some(score);
                result.row = group * chunk_size + part.row;
            }
            result.completed_rows += part.completed_rows;
            result.pruned_rows += part.pruned_rows;
            result.evaluated_cells += part.evaluated_cells;
            result.forecast_changes += part.forecast_changes;
            result.forecast_checks += part.forecast_checks;
            result.forecast_matches += part.forecast_matches;
            result.nodes += part.nodes;
        }
        Ok(result)
    }

    fn search_planning_matrix_with_rollout(
        &mut self,
        player: Symbol,
        rows: &[Vec<PlanningCommitment>],
        columns: &[Vec<PlanningCommitment>],
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<PlanningSearch> {
        self.search_planning_matrix_partition(
            player,
            rows,
            columns,
            limits,
            PlanningPartition {
                rollout_turns,
                controlled: None,
                shared_nodes: None,
                greedy_forecasts: &[],
                future_branching: None,
                legacy_fixed_followup: false,
                fixed_opponent_seed: None,
            },
            budget,
        )
    }

    fn search_planning_matrix_partition(
        &mut self,
        player: Symbol,
        rows: &[Vec<PlanningCommitment>],
        columns: &[Vec<PlanningCommitment>],
        limits: SearchLimits,
        partition: PlanningPartition<'_>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<PlanningSearch> {
        let rollout_turns = partition.rollout_turns;
        let turn_cutoff = self.search_turn_cutoff(rollout_turns)?;
        let invalid = |reason: &str| EngineError::InvalidInput(reason.into());
        if self.planning.phase != Phase::Planning
            || rows.is_empty()
            || columns.is_empty()
            || limits.max_depth > 64
            || limits.max_nodes == 0
        {
            return Err(invalid(
                "planning matrix requires a planning state, rows, columns and valid limits",
            ));
        }
        let team = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| invalid("search player is not a hero"))?
            .team;
        for (choices, friendly) in [(rows, true), (columns, false)] {
            if choices.iter().flatten().any(|choice| {
                !self.world.hero(choice.hero).is_some_and(|hero| {
                    hero.id == choice.hero
                        && partition
                            .controlled
                            .map_or(hero.team == team, |ids| ids.contains(&hero.id))
                            == friendly
                })
            }) {
                return Err(invalid("planning choice belongs to the wrong side"));
            }
        }
        let total_cells = rows
            .len()
            .checked_mul(columns.len())
            .ok_or_else(|| invalid("planning matrix is too large"))?;
        let mut result = PlanningSearch {
            row: 0,
            score: None,
            completed_rows: 0,
            pruned_rows: 0,
            evaluated_cells: 0,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
            total_cells,
            nodes: 0,
        };
        let mut search = Search {
            player,
            team,
            budget,
            nodes: 0,
            max_nodes: limits.max_nodes,
            opponent_policy: limits.opponent_policy,
            evaluation: limits.evaluation,
            turn_cutoff,
            controlled: partition.controlled,
            shared_nodes: partition.shared_nodes,
            guard_hidden: false,
            allow_simulated_upgrades: false,
            hand_belief: None,
            planning_sample: None,
            future_planning: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
        };
        let mut joint = Vec::new();
        'rows: for (index, row) in rows.iter().enumerate() {
            let mut worst = i64::MAX;
            for (column_index, column) in columns.iter().enumerate() {
                match budget.check(0) {
                    Ok(()) => {}
                    Err(EngineError::Deadline | EngineError::Cancelled) => break 'rows,
                    Err(error) => return Err(error),
                }
                joint.clear();
                joint.extend_from_slice(row);
                joint.extend_from_slice(column);
                if !search.reserve_node() {
                    break 'rows;
                }
                let expand = partition.future_branching.unwrap_or(
                    partition.fixed_opponent_seed.is_some()
                        || !partition.greedy_forecasts.is_empty(),
                );
                if expand
                    || !partition.greedy_forecasts.is_empty()
                    || partition.fixed_opponent_seed.is_some()
                {
                    search.future_planning = Some(FuturePlanning {
                        forecasts: partition.greedy_forecasts,
                        root: joint.clone(),
                        fixed_opponent_seed: partition.fixed_opponent_seed,
                        legacy_fixed_followup: partition.legacy_fixed_followup,
                        expand,
                    });
                }
                let value = self.with_joint_planning(&joint, |worker| {
                    worker.advance_to_decision(budget)?;
                    search.value(worker, limits.max_depth, i64::MIN, i64::MAX)
                });
                match value {
                    Ok(Some(value)) => {
                        result.evaluated_cells += 1;
                        worst = worst.min(value.score);
                        // Later replies can only lower this row's value. An
                        // earlier incumbent also wins an exact tie.
                        if turn_cutoff.is_some()
                            && column_index + 1 < columns.len()
                            && result.score.is_some_and(|best| worst <= best)
                        {
                            result.pruned_rows += 1;
                            continue 'rows;
                        }
                    }
                    Ok(None) | Err(EngineError::Deadline | EngineError::Cancelled) => break 'rows,
                    Err(error) => return Err(error),
                }
            }
            result.completed_rows += 1;
            if result.score.is_none_or(|best| worst > best) {
                result.score = Some(worst);
                result.row = index;
            }
        }
        result.nodes = search.nodes;
        result.forecast_changes = search.forecast_changes;
        result.forecast_checks = search.forecast_checks;
        result.forecast_matches = search.forecast_matches;
        Ok(result)
    }

    /// Search the current resolution using team minimax with material and position scoring.
    /// Stops at planning, where a separate hidden-information policy is needed.
    pub fn search_resolution(
        &mut self,
        player: Symbol,
        limits: SearchLimits,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        self.search_resolution_with_turn_rollout(player, limits, None, budget)
    }

    /// Experimental single-world resolution search at an ordinary action,
    /// defense, initiative or confirmation prompt.
    pub fn search_resolution_in_sampled_world(
        &mut self,
        player: Symbol,
        sample: PlanningRolloutSample<'_>,
        limits: SearchLimits,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        self.search_sampled_resolution_with_turn_rollout(player, sample, limits, None, None, budget)
    }

    /// Sample the resolution root and optionally reconstruct later planning
    /// worlds from each branch's public state.
    pub fn search_sampled_resolution_with_turn_rollout(
        &mut self,
        player: Symbol,
        sample: PlanningRolloutSample<'_>,
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        planning_sample: Option<PlanningRolloutSample<'_>>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        if self.planning.phase == Phase::LevelUp {
            return self.search_resolution_inner(player, limits, rollout_turns, None, budget, true);
        }
        budget.check(0)?;
        self.with_sampled_resolution(player, sample, |worker| {
            worker.search_resolution_inner(
                player,
                limits,
                rollout_turns,
                planning_sample,
                budget,
                true,
            )
        })
    }

    /// Compare root decisions across the same sampled worlds at each depth.
    pub fn search_resolution_worlds(
        &mut self,
        player: Symbol,
        (sample, worlds, workers): (PlanningRolloutSample<'_>, usize, usize),
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        planning_sample: Option<PlanningRolloutSample<'_>>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        self.search_resolution_worlds_inner(
            player,
            (sample, worlds, workers),
            limits,
            rollout_turns,
            planning_sample,
            None,
            budget,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Retain explicit search operands until the planned search API split"
    )]
    pub fn search_node_limited_resolution(
        &mut self,
        player: Symbol,
        sample: PlanningRolloutSample<'_>,
        worlds: usize,
        bots: &[Symbol],
        probabilistic_hand_responses: bool,
        limits: SearchLimits,
        rollout_turns: u8,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        self.search_turn_cutoff(Some(rollout_turns))?;
        let meter = arena_search_meter(limits, budget)?;
        let budget = ExecutionBudget {
            search_nodes: Some(&meter),
            ..*budget
        };
        if self.planning.phase == Phase::LevelUp {
            return self.search_upgrade_choices(player, limits, &budget);
        }
        let mut ranking = Vec::new();
        let mut best = self.search_live_bot_resolution(
            player,
            sample,
            worlds,
            bots,
            None,
            &mut ranking,
            probabilistic_hand_responses,
            SearchLimits {
                max_depth: limits.max_depth.min(2),
                max_nodes: 8_192,
                ..limits
            },
            rollout_turns.min(2),
            &budget,
        )?;
        if meter.remaining() > 0
            && best.score != Some(WIN)
            && let Some(deeper) = retain_arena_backup(self.search_live_bot_resolution(
                player,
                sample,
                worlds,
                bots,
                best.score.map(|_| &best.decision),
                &mut ranking,
                probabilistic_hand_responses,
                SearchLimits {
                    max_nodes: ARENA_SEARCH_CEILING,
                    ..limits
                },
                rollout_turns,
                &budget,
            ))?
            && (deeper.score_kind == Some(ResolutionScoreKind::TurnRollout)
                || (best.score.is_none() && deeper.score.is_some()))
        {
            best = deeper;
        }
        best.nodes = limits.max_nodes - meter.remaining();
        if meter.remaining() == 0 {
            best.stop = ResolutionStop::NodeLimit;
        }
        Ok(best)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Retain explicit search operands until the planned search API split"
    )]
    pub(super) fn search_live_bot_resolution(
        &mut self,
        player: Symbol,
        sample: PlanningRolloutSample<'_>,
        worlds: usize,
        bots: &[Symbol],
        incumbent: Option<&DecisionMove>,
        ranking: &mut Vec<(usize, i64)>,
        probabilistic_hand_responses: bool,
        limits: SearchLimits,
        rollout_turns: u8,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        self.knowledge_team(player, bots, true)?;
        // Keep each sampled world consistent at future planning boundaries.
        self.search_resolution_worlds_inner(
            player,
            (sample, worlds, 1),
            limits,
            Some(rollout_turns),
            None,
            Some(LiveResolution {
                bots,
                incumbent,
                ranking,
                probabilistic_hand_responses,
            }),
            budget,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Retain explicit search operands until the planned search API split"
    )]
    fn search_resolution_worlds_inner(
        &mut self,
        player: Symbol,
        (sample, worlds, workers): (PlanningRolloutSample<'_>, usize, usize),
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        planning_sample: Option<PlanningRolloutSample<'_>>,
        mut live: Option<LiveResolution<'_>>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        if !(1..=8).contains(&worlds)
            || !(1..=4.min(worlds)).contains(&workers)
            || !(1..=64).contains(&limits.max_depth)
            || limits.max_nodes == 0
        {
            return Err(EngineError::InvalidInput(
                "sampled search requires 1..8 worlds, up to four workers (at most one per world), depth 1..64 and a positive node limit".into(),
            ));
        }
        if self.planning.phase == Phase::LevelUp {
            return self.search_resolution_inner(player, limits, rollout_turns, None, budget, true);
        }
        if worlds == 1 && live.is_none() {
            return self.search_sampled_resolution_with_turn_rollout(
                player,
                sample,
                limits,
                rollout_turns,
                planning_sample,
                budget,
            );
        }

        let turn_cutoff = self.search_turn_cutoff(rollout_turns)?;
        let team = self
            .world
            .hero(player)
            .ok_or_else(|| EngineError::InvalidInput("search player is not a hero".into()))?
            .team;
        let mut moves = self.resolution_choices(player)?;
        self.order_movement_roots(player, &mut moves);
        let mut result = ResolutionSearch {
            decision: moves
                .first()
                .ok_or_else(|| {
                    EngineError::InvalidInput("no live resolution move for this player".into())
                })?
                .clone(),
            score: None,
            score_kind: None,
            completed_depth: 0,
            resolved: false,
            stop: ResolutionStop::DepthLimit,
            nodes: 0,
        };
        let shared_nodes = AtomicUsize::new(limits.max_nodes);
        let mut search = Search {
            player,
            team,
            budget,
            nodes: 0,
            max_nodes: limits.max_nodes,
            opponent_policy: limits.opponent_policy,
            evaluation: limits.evaluation,
            turn_cutoff,
            controlled: None,
            shared_nodes: (workers > 1).then_some(&shared_nodes),
            guard_hidden: true,
            allow_simulated_upgrades: live.is_some(),
            hand_belief: live
                .as_ref()
                .filter(|scope| scope.probabilistic_hand_responses)
                .map(|scope| HandBelief {
                    known: scope.bots,
                    upgrade_checkpoints: sample.upgrade_checkpoints,
                    turn: self.search_turn_index(),
                }),
            planning_sample,
            future_planning: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
        };
        let mut sampled = match self.sampled_resolution_workers(
            player,
            sample,
            worlds,
            live.as_ref().map_or(&[], |scope| scope.bots),
            budget,
        ) {
            Ok(workers) => workers,
            Err(EngineError::Deadline) => {
                result.stop = ResolutionStop::Deadline;
                return Ok(result);
            }
            Err(EngineError::Cancelled) => {
                result.stop = ResolutionStop::Cancelled;
                return Ok(result);
            }
            Err(error) => return Err(error),
        };

        // A scored backup from a shorter pass makes another warm-up redundant.
        let incumbent = live
            .as_ref()
            .and_then(|scope| scope.incumbent)
            .and_then(|decision| moves.iter().position(|candidate| candidate == decision));
        let mut root_order = (0..moves.len()).collect::<Vec<_>>();
        let root_width = live.as_ref().and_then(|scope| {
            incumbent?;
            // Both live passes enumerate the same legal roots in native order.
            let mut scores = vec![None; moves.len()];
            for &(index, score) in scope.ranking.iter() {
                scores[index] = Some(score);
            }
            root_order.sort_by_key(|&index| Reverse(scores[index]));
            Some(live_root_width(
                scope.ranking.iter().map(|(_, score)| *score),
                moves.len(),
            ))
        });
        let margin = i128::from(LIVE_CHALLENGER_MARGIN) * worlds as i128;
        let mut depth_scores = live.as_ref().map(|_| vec![None; moves.len()]);
        let mut previous_sum = None;
        let mut preferred = incumbent.unwrap_or(0);
        let mut resolved_scores = vec![vec![None; worlds]; moves.len()];
        if incumbent.is_none()
            && let Some(cutoff) = turn_cutoff
            && limits.max_nodes >= worlds
        {
            // A comparable fallback must have a score in every sampled world.
            // Keep this pass serial and charge it to the shared search budget.
            let warmup_moves = self.resolution_warmup_moves(limits.max_nodes, worlds);
            let mut best_sum = None;
            'warmup: for (index, decision) in moves.iter().take(warmup_moves).enumerate() {
                let mut sum = 0i128;
                for (worker, cached) in sampled.iter_mut().zip(&mut resolved_scores[index]) {
                    match search.score_immediate_root(worker, decision, cutoff) {
                        Ok(Some((score, resolved))) => {
                            if resolved {
                                *cached = Some(score);
                            }
                            sum += i128::from(score);
                        }
                        Ok(None) | Err(EngineError::Deadline | EngineError::Cancelled) => {
                            break 'warmup;
                        }
                        Err(error) => return Err(error),
                    }
                }
                if best_sum.is_none_or(|best| sum > best) {
                    best_sum = Some(sum);
                    preferred = index;
                    result.decision = decision.clone();
                    result.score = Some((sum / worlds as i128) as i64);
                    result.score_kind = Some(ResolutionScoreKind::Immediate);
                }
            }
        }

        let short = if incumbent.is_none() && resolved_scores.iter().flatten().any(Option::is_none)
        {
            search.seed_short_turn(&mut sampled, &moves)?
        } else {
            None
        };
        if let Some((index, score)) = short {
            preferred = index;
            result.decision = moves[index].clone();
            result.score = Some(score);
            result.score_kind = Some(ResolutionScoreKind::ShortTurnRollout);
        }
        for depth in 1..=limits.max_depth {
            let remaining = depth - u8::from(moves.len() > 1);
            let mut best_sum = None;
            let mut runner_up = None;
            let mut all_resolved = true;
            let mut restricted = false;
            if let Some(scores) = &mut depth_scores {
                scores.fill(None);
            }
            let ordered = std::iter::once(preferred).chain(
                root_order
                    .iter()
                    .copied()
                    .filter(|&index| index != preferred),
            );
            'roots: for (rank, index) in ordered.enumerate() {
                if let Some(width) = root_width
                    && rank >= width
                    && (rank >= width + 4
                        || !live_roots_uncertain(best_sum, runner_up, preferred, margin))
                {
                    restricted = true;
                    break;
                }
                let decision = &moves[index];
                let mut sum = 0i128;
                let mut worlds_left = worlds as i128;
                for (group, cache) in sampled
                    .chunks_mut(workers)
                    .zip(resolved_scores[index].chunks_mut(workers))
                {
                    // No unscored world can contribute more than a win.
                    // Respect native root order when the bound ties.
                    let upper = sum + worlds_left * i128::from(WIN);
                    if best_sum.is_some_and(|(best_index, best)| {
                        upper < best || (upper == best && index > best_index)
                    }) {
                        runner_up = Some(runner_up.map_or(upper, |second| second.max(upper)));
                        continue 'roots;
                    }
                    worlds_left -= group.len() as i128;
                    let values = if group.len() == 1 {
                        vec![search.score_sampled_root(
                            &mut group[0],
                            &mut cache[0],
                            decision,
                            remaining,
                        )]
                    } else {
                        let (values, nodes) = std::thread::scope(|threads| {
                            let jobs = group
                                .iter_mut()
                                .zip(cache)
                                .map(|(worker, cached)| {
                                    let mut local = Search {
                                        nodes: 0,
                                        future_planning: None,
                                        ..search
                                    };
                                    std::thread::Builder::new()
                                        .name("resolution-score".into())
                                        .stack_size(SEARCH_WORKER_STACK)
                                        .spawn_scoped(threads, move || {
                                            let value = local.score_sampled_root(
                                                worker, cached, decision, remaining,
                                            );
                                            (value, local.nodes)
                                        })
                                        .map_err(|error| EngineError::Unsupported(format!(
                                            "could not spawn resolution scoring worker: {error}",
                                        )))
                                })
                                .collect::<Result<Vec<_>>>()?;
                            let mut values = Vec::with_capacity(jobs.len());
                            let mut nodes = 0;
                            for job in jobs {
                                let (value, used) = job.join().map_err(|_| {
                                    EngineError::Unsupported(
                                        "resolution scoring worker panicked".into(),
                                    )
                                })?;
                                nodes += used;
                                values.push(value);
                            }
                            Ok((values, nodes))
                        })?;
                        search.nodes += nodes;
                        values
                    };
                    let mut stop = None;
                    for value in values {
                        match value {
                            Ok(Some(value)) => {
                                all_resolved &= value.resolved;
                                sum += i128::from(value.score);
                            }
                            Ok(None) => {
                                stop.get_or_insert(ResolutionStop::NodeLimit);
                            }
                            Err(EngineError::Deadline) => {
                                stop = Some(ResolutionStop::Deadline);
                            }
                            Err(EngineError::Cancelled) => {
                                stop = Some(ResolutionStop::Cancelled);
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    if let Some(stop) = stop {
                        if live.is_some()
                            && let Some((index, sum)) = best_sum
                        {
                            // These roots completed in every sampled world at
                            // this depth. Ignore the interrupted root's score.
                            result.decision = moves[index].clone();
                            result.score = Some((sum / worlds as i128) as i64);
                            result.score_kind = Some(search.depth_score_kind());
                            if result.completed_depth == 0
                                && let Some((scope, scores)) =
                                    live.as_mut().zip(depth_scores.as_ref())
                            {
                                record_live_root_scores(scope.ranking, scores, worlds);
                            }
                        }
                        result.stop = stop;
                        result.nodes = search.nodes;
                        return Ok(result);
                    }
                }
                if let Some(scores) = &mut depth_scores {
                    scores[index] = Some(sum);
                }
                if best_sum.is_none_or(|(best_index, best)| {
                    sum > best || (sum == best && index < best_index)
                }) {
                    runner_up = best_sum.map(|(_, best)| best);
                    best_sum = Some((index, sum));
                    if result.completed_depth == 0 && short.is_none() {
                        result.decision = decision.clone();
                        result.score = Some((sum / worlds as i128) as i64);
                        result.score_kind = Some(search.depth_score_kind());
                    }
                } else {
                    runner_up = Some(runner_up.map_or(sum, |second| second.max(sum)));
                }
            }
            let (index, sum) = best_sum.expect("root has legal choices");
            let stable = root_width.is_some()
                && !live_roots_uncertain(best_sum, runner_up, preferred, margin)
                && previous_sum.is_some_and(|prior: i128| {
                    (prior - sum).abs() <= i128::from(LIVE_STABLE_MARGIN) * worlds as i128
                });
            previous_sum = Some(sum);
            preferred = index;
            result.decision = moves[index].clone();
            result.score = Some((sum / worlds as i128) as i64);
            result.score_kind = Some(search.depth_score_kind());
            if !restricted {
                result.completed_depth = depth;
            }
            if let Some((scope, scores)) = live.as_mut().zip(depth_scores.as_ref()) {
                record_live_root_scores(scope.ranking, scores, worlds);
                // Keep the earlier ordering among unscored roots. A later
                // pass can widen if the new leading move is less convincing.
                root_order.sort_by_key(|&index| Reverse(scores[index]));
            }
            if all_resolved {
                result.resolved = !restricted;
                if result.resolved {
                    result.stop = ResolutionStop::Resolved;
                }
                break;
            }
            if stable {
                break;
            }
        }
        result.nodes = search.nodes;
        Ok(result)
    }

    /// Search resolution choices with an optional fixed-policy leaf continuation.
    pub fn search_resolution_with_turn_rollout(
        &mut self,
        player: Symbol,
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        self.search_resolution_with_sampled_turn_rollout(
            player,
            limits,
            rollout_turns,
            None,
            budget,
        )
    }

    /// Optionally reconstruct hidden hands at future planning epochs using
    /// the same seed per epoch across alternatives.
    pub fn search_resolution_with_sampled_turn_rollout(
        &mut self,
        player: Symbol,
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        planning_sample: Option<PlanningRolloutSample<'_>>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        self.search_resolution_inner(
            player,
            limits,
            rollout_turns,
            planning_sample,
            budget,
            false,
        )
    }

    /// Score only the acting hero's issued choices. Other heroes may
    /// already have submitted upgrades that are still private.
    fn search_upgrade_choices(
        &mut self,
        player: Symbol,
        limits: SearchLimits,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        let moves = self.resolution_choices(player)?;
        let row = self
            .last_request
            .as_ref()
            .and_then(|request| {
                if let RequestChoices::Upgrades(players) = &request.choices {
                    players.iter().find(|row| row.hero == player)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                EngineError::InvalidInput("no live upgrade choice for this hero".into())
            })?;
        let mut result = ResolutionSearch {
            decision: moves
                .first()
                .ok_or_else(|| {
                    EngineError::InvalidInput("no live upgrade choice for this hero".into())
                })?
                .clone(),
            score: None,
            score_kind: None,
            completed_depth: 0,
            resolved: false,
            stop: ResolutionStop::DepthLimit,
            nodes: 0,
        };
        if row
            .options
            .iter()
            .map(|option| option.pair.len())
            .sum::<usize>()
            != moves.len()
        {
            return Err(EngineError::Unsupported(
                "upgrade menu and moves disagree".into(),
            ));
        }
        let definition = |id| {
            self.world
                .card(id)
                .map(|card| &self.world.definitions.cards[card.definition])
                .ok_or_else(|| EngineError::Unsupported("upgrade card missing".into()))
        };
        for (decision, (option, chosen)) in moves.into_iter().zip(
            row.options
                .iter()
                .flat_map(|option| option.pair.iter().map(move |chosen| (option, *chosen))),
        ) {
            if result.nodes >= limits.max_nodes {
                result.stop = ResolutionStop::NodeLimit;
                return Ok(result);
            }
            match budget.check(0) {
                Err(EngineError::Deadline) => {
                    result.stop = ResolutionStop::Deadline;
                    return Ok(result);
                }
                Err(EngineError::Cancelled) => {
                    result.stop = ResolutionStop::Cancelled;
                    return Ok(result);
                }
                other => other?,
            }
            if !budget.reserve_search_nodes(1) {
                result.stop = ResolutionStop::NodeLimit;
                return Ok(result);
            }
            let card = definition(chosen)?;
            let mut score = i128::from(card.initiative)
                + i128::from(card.primary_value.unwrap_or(0))
                + card
                    .secondary
                    .iter()
                    .map(|(_, value)| i128::from(*value))
                    .sum::<i128>();
            if let Some(sibling) = option.pair.iter().find(|&&id| id != chosen)
                && let Some(item) = definition(*sibling)?.item
            {
                score += if item == crate::stats::Stat::Defense {
                    2
                } else {
                    1
                };
            }
            let score = score.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
            result.nodes += 1;
            if result.score.is_none_or(|best| score > best) {
                result.decision = decision;
                result.score = Some(score);
                result.score_kind = Some(ResolutionScoreKind::Immediate);
            }
        }
        result.completed_depth = 1;
        Ok(result)
    }

    fn search_resolution_inner(
        &mut self,
        player: Symbol,
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        planning_sample: Option<PlanningRolloutSample<'_>>,
        budget: &ExecutionBudget<'_>,
        guard_hidden: bool,
    ) -> Result<ResolutionSearch> {
        let turn_cutoff = self.search_turn_cutoff(rollout_turns)?;
        if limits.max_depth == 0 || limits.max_depth > 64 || limits.max_nodes == 0 {
            return Err(EngineError::InvalidInput(
                "search requires depth 1..64 and a positive node limit".into(),
            ));
        }
        if self.planning.phase == Phase::LevelUp {
            return self.search_upgrade_choices(player, limits, budget);
        }
        let team = self
            .world
            .hero(player)
            .ok_or_else(|| EngineError::InvalidInput("search player is not a hero".into()))?
            .team;
        let mut moves = self.resolution_choices(player)?;
        self.order_movement_roots(player, &mut moves);
        let fallback = moves.first().ok_or_else(|| {
            EngineError::InvalidInput("no live resolution move for this player".into())
        })?;
        self.with_rollback(|worker| {
            let mut result = ResolutionSearch {
                decision: fallback.clone(),
                score: None,
                score_kind: None,
                completed_depth: 0,
                resolved: false,
                stop: ResolutionStop::DepthLimit,
                nodes: 0,
            };
            let mut search = Search {
                player,
                team,
                budget,
                nodes: 0,
                max_nodes: limits.max_nodes,
                opponent_policy: limits.opponent_policy,
                evaluation: limits.evaluation,
                turn_cutoff,
                controlled: None,
                shared_nodes: None,
                guard_hidden,
                allow_simulated_upgrades: false,
                hand_belief: None,
                planning_sample,
                future_planning: None,
                forecast_changes: 0,
                forecast_checks: 0,
                forecast_matches: 0,
            };
            let mut preferred = 0;
            let mut resolved_scores = vec![None; moves.len()];
            if let Some(cutoff) = turn_cutoff {
                // Reserve most nodes for the requested horizon. Score quick
                // alternatives first so even an interrupted rollout has a fallback.
                let warmup_nodes = worker.resolution_warmup_moves(limits.max_nodes, 1);
                for (index, decision) in moves.iter().take(warmup_nodes).enumerate() {
                    let score = search.score_immediate_root(worker, decision, cutoff);
                    match score {
                        Ok(Some((score, resolved))) => {
                            if resolved {
                                resolved_scores[index] = Some(score);
                            }
                            if result.score.is_none_or(|best| score > best) {
                                result.decision = decision.clone();
                                result.score = Some(score);
                                result.score_kind = Some(ResolutionScoreKind::Immediate);
                                preferred = index;
                            }
                        }
                        Ok(None) | Err(EngineError::Deadline | EngineError::Cancelled) => break,
                        Err(error) => return Err(error),
                    }
                }
            }
            let short = if resolved_scores.iter().any(Option::is_none) {
                search.seed_short_turn(std::slice::from_mut(worker), &moves)?
            } else {
                None
            };
            if let Some((index, score)) = short {
                preferred = index;
                result.decision = moves[index].clone();
                result.score = Some(score);
                result.score_kind = Some(ResolutionScoreKind::ShortTurnRollout);
            }
            for depth in 1..=limits.max_depth {
                let iteration =
                    search.choose(worker, &moves, depth, preferred, &mut resolved_scores)?;
                if let Some((index, score)) = iteration.best
                    && (iteration.interrupted.is_none()
                        || (result.completed_depth == 0 && short.is_none()))
                {
                    preferred = index;
                    result.decision = moves[index].clone();
                    result.score = Some(score);
                    result.score_kind = Some(search.depth_score_kind());
                }
                if let Some(stop) = iteration.interrupted {
                    result.stop = stop;
                    break;
                }
                result.completed_depth = depth;
                if iteration.resolved {
                    result.resolved = true;
                    result.stop = ResolutionStop::Resolved;
                    break;
                }
            }
            result.nodes = search.nodes;
            Ok(result)
        })
    }

    /// Evaluate a fixed first-legal continuation through one or two game turns.
    /// The root and all commitments are restored on success or interruption.
    pub fn rollout_game_turns(
        &mut self,
        player: Symbol,
        turns: u8,
        max_nodes: usize,
        budget: &ExecutionBudget<'_>,
    ) -> Result<Option<GameTurnRollout>> {
        self.rollout_game_turns_diagnostic(player, turns, max_nodes, budget, false, false)
    }

    pub fn rollout_game_turns_diagnostic(
        &mut self,
        player: Symbol,
        turns: u8,
        max_nodes: usize,
        budget: &ExecutionBudget<'_>,
        capture_state: bool,
        lab_evaluation: bool,
    ) -> Result<Option<GameTurnRollout>> {
        if !(1..=2).contains(&turns) || max_nodes == 0 {
            return Err(EngineError::InvalidInput(
                "rollout requires one or two game turns and a positive node limit".into(),
            ));
        }
        let team = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| EngineError::InvalidInput("rollout player is not a hero".into()))?
            .team;
        let cutoff = self.search_turn_index() + i128::from(turns);
        let mut search = Search {
            player,
            team,
            budget,
            nodes: 0,
            max_nodes,
            opponent_policy: OpponentPolicy::FirstLegal,
            evaluation: SearchEvaluation::MaterialPosition,
            turn_cutoff: None,
            controlled: None,
            shared_nodes: None,
            guard_hidden: false,
            allow_simulated_upgrades: false,
            hand_belief: None,
            planning_sample: None,
            future_planning: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
        };
        self.with_rollback(|worker| {
            worker.advance_to_decision(budget)?;
            search.rollout(worker, cutoff, capture_state, lab_evaluation)
        })
    }

    fn resolution_warmup_moves(&self, max_nodes: usize, worlds: usize) -> usize {
        let moves = (max_nodes / 8 / worlds).max(1);
        if self.movement_root_actor().is_some() {
            // Movement roots already interleave approach and retreat choices.
            // Seed a fallback from that prefix and spend the rest on rollouts.
            moves.min(8)
        } else {
            moves
        }
    }

    fn search_turn_cutoff(&self, turns: Option<u8>) -> Result<Option<i128>> {
        if turns.is_some_and(|turns| !(1..=MAX_SEARCH_ROLLOUT_TURNS).contains(&turns)) {
            return Err(EngineError::InvalidInput(format!(
                "rollout_turns must be in 1..={MAX_SEARCH_ROLLOUT_TURNS}"
            )));
        }
        Ok(turns.map(|turns| self.search_turn_index() + i128::from(turns)))
    }

    pub(super) fn search_turn_index(&self) -> i128 {
        i128::from(self.round.round) * 4 + i128::from(self.world.combat.turn)
    }

    pub(super) fn search_player(&self) -> Result<Option<(Symbol, Symbol)>> {
        self.search_player_preferring(None)
    }

    fn search_player_preferring(
        &self,
        controlled: Option<&[Symbol]>,
    ) -> Result<Option<(Symbol, Symbol)>> {
        let Some(request) = &self.last_request else {
            return Ok(None);
        };
        let player = match &request.choices {
            RequestChoices::Upgrades(players) => players
                .iter()
                .find(|player| !player.options.is_empty())
                .map(|player| player.hero),
            _ => {
                let eligible = || {
                    self.world
                        .heroes
                        .iter()
                        .filter(|hero| self.resolution_request_addresses(request, hero.id))
                };
                controlled
                    .and_then(|ids| eligible().find(|hero| ids.contains(&hero.id)))
                    .or_else(|| eligible().next())
                    .map(|hero| hero.id)
            }
        };
        let Some((player, hero)) =
            player.and_then(|player| self.world.hero(player).map(|hero| (player, hero)))
        else {
            return Err(EngineError::Unsupported(
                "search cannot resolve decision owner".into(),
            ));
        };
        Ok(Some((player, hero.team)))
    }
}

struct FuturePlanning<'a> {
    forecasts: &'a [(Symbol, super::GreedySearch)],
    root: Vec<PlanningCommitment>,
    fixed_opponent_seed: Option<u64>,
    legacy_fixed_followup: bool,
    expand: bool,
}

struct Search<'a, 'b> {
    player: Symbol,
    team: Symbol,
    budget: &'a ExecutionBudget<'b>,
    nodes: usize,
    max_nodes: usize,
    opponent_policy: OpponentPolicy,
    evaluation: SearchEvaluation,
    turn_cutoff: Option<i128>,
    controlled: Option<&'a [Symbol]>,
    shared_nodes: Option<&'a AtomicUsize>,
    planning_sample: Option<PlanningRolloutSample<'a>>,
    guard_hidden: bool,
    allow_simulated_upgrades: bool,
    hand_belief: Option<HandBelief<'a>>,
    forecast_changes: usize,
    forecast_checks: usize,
    forecast_matches: usize,
    future_planning: Option<FuturePlanning<'a>>,
}

struct Iteration {
    best: Option<(usize, i64)>,
    interrupted: Option<ResolutionStop>,
    resolved: bool,
}

struct ValueBound {
    score: i64,
    resolved: bool,
}

fn arena_search_meter(
    limits: SearchLimits,
    budget: &ExecutionBudget<'_>,
) -> Result<SearchNodeBudget> {
    if limits.max_nodes == 0 || budget.deadline.is_some() || budget.search_nodes.is_some() {
        return Err(EngineError::InvalidInput(
            "arena v2 requires one positive node budget and no deadline".into(),
        ));
    }
    Ok(SearchNodeBudget::new(limits.max_nodes))
}

fn retain_arena_backup<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(result) => Ok(Some(result)),
        Err(EngineError::Unsupported(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn planning_has_score(result: &BotTeamPlanningSearch) -> bool {
    result.matrix.score.is_some() || result.shallow_score.is_some() || result.seed_score.is_some()
}

fn live_root_width(mut scores: impl ExactSizeIterator<Item = i64>, count: usize) -> usize {
    if scores.len() < 2 {
        return count.min(6);
    }
    let best = scores.next().unwrap();
    // Keep several alternatives and widen the first pass for near ties.
    let contenders = 1 + scores
        .take_while(|&score| best - score <= LIVE_CHALLENGER_MARGIN)
        .count();
    contenders.clamp(3, 8).min(count)
}

fn live_roots_uncertain(
    best: Option<(usize, i128)>,
    runner_up: Option<i128>,
    previous: usize,
    margin: i128,
) -> bool {
    best.is_none_or(|(index, score)| {
        index != previous || runner_up.is_none_or(|second| score - second <= margin)
    })
}

fn record_live_root_scores(
    ranking: &mut Vec<(usize, i64)>,
    scores: &[Option<i128>],
    worlds: usize,
) {
    ranking.clear();
    ranking.extend(
        scores
            .iter()
            .enumerate()
            .filter_map(|(index, score)| score.map(|sum| (index, (sum / worlds as i128) as i64))),
    );
    ranking.sort_by_key(|(_, score)| Reverse(*score));
}

fn root_move_order(count: usize, preferred: usize) -> impl Iterator<Item = usize> {
    std::iter::once(preferred).chain((0..count).filter(move |&index| index != preferred))
}

impl Search<'_, '_> {
    /// Establish a complete one-turn incumbent before a longer rollout.
    /// Keep its scores out of the longer horizon's exact-value cache.
    fn seed_short_turn(
        &mut self,
        worlds: &mut [NativeSession],
        moves: &[DecisionMove],
    ) -> Result<Option<(usize, i64)>> {
        let cutoff = worlds[0].search_turn_index() + 1;
        let allowance = self.max_nodes / 8;
        if moves.len() < 2
            || !self.turn_cutoff.is_some_and(|end| cutoff < end)
            || allowance < moves.len().saturating_mul(worlds.len())
        {
            return Ok(None);
        }
        let now = std::time::Instant::now();
        let budget = ExecutionBudget {
            deadline: self
                .budget
                .deadline
                .map(|end| now + end.saturating_duration_since(now) / 8),
            ..*self.budget
        };
        let mut short = Search {
            budget: &budget,
            turn_cutoff: Some(cutoff),
            max_nodes: self.nodes.saturating_add(allowance).min(self.max_nodes),
            future_planning: None,
            ..*self
        };
        let mut best = None;
        for (index, decision) in moves.iter().enumerate() {
            let mut sum = 0i128;
            for worker in worlds.iter_mut() {
                let leaf = worker.with_consumed_request(|branch| {
                    if !short.apply_input(branch, decision.clone())? {
                        return Ok(None);
                    }
                    if branch.last_request.is_none()
                        && !matches!(branch.planning.phase, Phase::Planning | Phase::GameOver)
                        && branch.search_turn_index() < cutoff
                        && short.pending_hand_branch(branch)?.is_none()
                    {
                        return Ok(None);
                    }
                    short.rollout(branch, cutoff, false, false)
                });
                match leaf {
                    Ok(Some(leaf)) if leaf.reached_cutoff(cutoff) => {
                        sum += i128::from(leaf.score);
                    }
                    Ok(_) | Err(EngineError::Deadline | EngineError::Cancelled) => {
                        self.nodes = short.nodes;
                        return Ok(None);
                    }
                    Err(error) => return Err(error),
                }
            }
            if best.is_none_or(|(_, best_sum)| sum > best_sum) {
                best = Some((index, sum));
            }
        }
        self.nodes = short.nodes;
        Ok(best.map(|(index, sum)| (index, (sum / worlds.len() as i128) as i64)))
    }

    fn score_immediate_root(
        &mut self,
        worker: &mut NativeSession,
        decision: &DecisionMove,
        cutoff: i128,
    ) -> Result<Option<(i64, bool)>> {
        worker.with_consumed_request(|branch| {
            if !self.apply_input(branch, decision.clone())? {
                return Ok(None);
            }
            let resolved =
                branch.planning.phase == Phase::GameOver || branch.search_turn_index() >= cutoff;
            Ok(Some((self.score(branch)?, resolved)))
        })
    }

    fn depth_score_kind(&self) -> ResolutionScoreKind {
        if self.turn_cutoff.is_some() {
            ResolutionScoreKind::TurnRollout
        } else {
            ResolutionScoreKind::DecisionDepth
        }
    }

    fn score(&self, worker: &NativeSession) -> Result<i64> {
        worker.search_score(self.evaluation, self.team, self.player)
    }

    fn reserve_node(&mut self) -> bool {
        if self.nodes == self.max_nodes
            || self.shared_nodes.is_some_and(|remaining| {
                remaining
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |nodes| {
                        nodes.checked_sub(1)
                    })
                    .is_err()
            })
        {
            return false;
        }
        if !self.budget.reserve_search_nodes(1) {
            return false;
        }
        self.nodes += 1;
        true
    }

    fn branch(
        &mut self,
        worker: &mut NativeSession,
        decision: DecisionMove,
        depth: u8,
        alpha: i64,
        beta: i64,
    ) -> Result<Option<ValueBound>> {
        worker.with_consumed_request(|branch| {
            if !self.apply_input(branch, decision)? {
                return Ok(None);
            }
            self.value(branch, depth, alpha, beta)
        })
    }

    fn apply_input(&mut self, worker: &mut NativeSession, decision: DecisionMove) -> Result<bool> {
        self.budget.check(0)?;
        if !self.reserve_node() {
            return Ok(false);
        }
        let DecisionMove::Input {
            request: Some(token),
            selection,
            ..
        } = decision
        else {
            return Err(EngineError::Unsupported(
                "resolution search requires a live input".into(),
            ));
        };
        worker.submit(token, selection)?;
        worker.last_request = None;
        self.advance(worker)?;
        Ok(true)
    }

    fn advance(&self, worker: &mut NativeSession) -> Result<()> {
        if !self.guard_hidden {
            return worker.advance_to_decision(self.budget);
        }
        let mut remaining = self.budget.max_steps.min(1000);
        while worker.last_request.is_none()
            && !matches!(worker.planning.phase, Phase::Planning | Phase::GameOver)
            && (worker.planning.phase != Phase::LevelUp || self.allow_simulated_upgrades)
        {
            let budget = ExecutionBudget {
                max_steps: remaining,
                ..*self.budget
            };
            let simulated_upgrade = self.allow_simulated_upgrades
                && worker.stack.last().is_some_and(|frame| {
                    matches!(
                        worker.steps[frame.instruction].instruction,
                        super::Instruction::Flow(crate::turn::FlowStep::Upgrades)
                    )
                });
            let advance = if simulated_upgrade {
                // This prompt is generated on a sampled future position. It has
                // no pending real upgrade choices to reveal.
                worker.advance_objects(&budget, Some(1), false)?
            } else if let Some(belief) = self
                .hand_belief
                .filter(|belief| belief.turn == worker.search_turn_index())
            {
                worker.advance_search_objects_before_hand(&budget, self.player, belief.known)?
            } else {
                worker.advance_search_objects(&budget, self.player)?
            };
            if advance.steps == 0 {
                break;
            }
            remaining -= advance.steps;
        }
        Ok(())
    }

    fn movement_leaf(
        &mut self,
        worker: &mut NativeSession,
        player: Symbol,
        cutoff: i128,
        beta: i64,
    ) -> Result<Option<ValueBound>> {
        let mut moves = worker.resolution_choices(player)?;
        let Some(fallback) = moves.first().cloned() else {
            return Err(EngineError::Unsupported(
                "movement search has no native choices".into(),
            ));
        };
        let is_hex = |choice: &DecisionMove| {
            matches!(
                choice,
                DecisionMove::Input {
                    selection: super::Datum::Object(_),
                    ..
                }
            )
        };
        let skip = moves.iter().find(|choice| !is_hex(choice)).cloned();
        worker.order_movement_roots(player, &mut moves);
        let mut shortlist: Vec<_> = moves.into_iter().filter(is_hex).take(3).collect();
        for extra in std::iter::once(fallback).chain(skip) {
            if !shortlist.contains(&extra) {
                shortlist.push(extra);
            }
        }

        let mut best = i64::MIN;
        for choice in shortlist {
            let leaf = worker.with_consumed_request(|branch| {
                if !self.apply_input(branch, choice)? {
                    return Ok(None);
                }
                self.rollout(branch, cutoff, false, false)
            })?;
            let Some(leaf) = leaf else { return Ok(None) };
            best = best.max(leaf.score);
            if best >= beta {
                break;
            }
        }
        Ok(Some(ValueBound {
            score: best,
            resolved: false,
        }))
    }

    // The caller owns the rollback scope for this path.
    fn value(
        &mut self,
        worker: &mut NativeSession,
        mut depth: u8,
        mut alpha: i64,
        mut beta: i64,
    ) -> Result<Option<ValueBound>> {
        let (team, moves) = loop {
            if let Some(cutoff) = self.turn_cutoff {
                if worker.search_turn_index() >= cutoff {
                    return Ok(Some(ValueBound {
                        score: self.score(worker)?,
                        resolved: true,
                    }));
                }
                if worker.planning.phase == Phase::Planning {
                    return Ok(self.rollout(worker, cutoff, false, false)?.map(|leaf| {
                        ValueBound {
                            score: leaf.score,
                            resolved: true,
                        }
                    }));
                }
            }
            if let Some(target) = self.pending_hand_branch(worker)? {
                return self.weighted_hand_response(worker, target, |search, branch| {
                    search.value(branch, depth, i64::MIN, i64::MAX)
                });
            }
            let terminal = worker.last_request.is_none()
                || matches!(worker.planning.phase, Phase::Planning | Phase::GameOver);
            if terminal {
                return Ok(Some(ValueBound {
                    score: self.score(worker)?,
                    resolved: true,
                }));
            }
            let (player, team) = worker
                .search_player_preferring(self.controlled)?
                .expect("live request has a search owner");
            let controlled = self
                .controlled
                .map_or(team == self.team, |ids| ids.contains(&player));
            if !controlled
                && let Some(seed) = self
                    .future_planning
                    .as_ref()
                    .and_then(|rollout| rollout.fixed_opponent_seed)
            {
                let decision = worker
                    .greedy_opponent_input(player, opponent_draw_seed(worker, seed, player))?
                    .ok_or_else(|| {
                        EngineError::Unsupported("fixed opponent has no native choices".into())
                    })?;
                if !self.apply_input(worker, decision)? {
                    return Ok(None);
                }
                continue;
            }
            let fixed_policy = if team == self.team {
                self.controlled.is_some_and(|ids| !ids.contains(&player))
            } else {
                matches!(self.opponent_policy, OpponentPolicy::FirstLegal)
            };
            if depth == 0
                && controlled
                && matches!(
                    self.opponent_policy,
                    OpponentPolicy::AdversarialMovementTargets
                )
                && let Some(cutoff) = self.turn_cutoff
                && worker.last_request.as_ref().is_some_and(|request| {
                    matches!(&request.choices, RequestChoices::Hexes(hexes)
                        if hexes.len() + usize::from(request.can_skip) > 1)
                })
                && worker
                    .movement_root_actor()
                    .is_some_and(|actor| worker.world.hero_identity(actor) == player)
            {
                return self.movement_leaf(worker, player, cutoff, beta);
            }
            let extend_attack = depth == 0
                && !fixed_policy
                && matches!(
                    self.opponent_policy,
                    OpponentPolicy::AdversarialAttackTargets
                )
                && worker.pending_attack_target();
            let mut moves = if (depth == 0 && !extend_attack) || fixed_policy {
                worker.resolution_branch_probe(player)?
            } else {
                worker.resolution_choices(player)?
            };
            if moves.is_empty() {
                return Err(EngineError::Unsupported(
                    "search decision has no native choices".into(),
                ));
            }
            if moves.len() > 1 {
                if extend_attack {
                    // Spend the extra layer on this target choice only.
                    depth = 1;
                }
                if depth == 0 || !fixed_policy {
                    break (team, moves);
                }
                // This policy commits to one continuation. The caller's
                // rollback scope already covers it, so only consume depth.
                depth -= 1;
            }
            if !self.apply_input(worker, moves.swap_remove(0))? {
                return Ok(None);
            }
        };
        if depth == 0 {
            if let Some(cutoff) = self.turn_cutoff {
                return Ok(self
                    .rollout(worker, cutoff, false, false)?
                    .map(|leaf| ValueBound {
                        score: leaf.score,
                        resolved: false,
                    }));
            }
            return Ok(Some(ValueBound {
                score: self.score(worker)?,
                resolved: false,
            }));
        }
        let maximizing = team == self.team;
        let mut best = if maximizing { i64::MIN } else { i64::MAX };
        let mut resolved = true;
        let count = moves.len();
        for (index, decision) in moves.into_iter().enumerate() {
            let Some(value) = self.branch(worker, decision, depth - 1, alpha, beta)? else {
                return Ok(None);
            };
            resolved &= value.resolved;
            let score = value.score;
            if maximizing {
                best = best.max(score);
                alpha = alpha.max(best);
            } else {
                best = best.min(score);
                beta = beta.min(best);
            }
            if alpha >= beta && index + 1 < count {
                // Unvisited siblings may still contain depth-limited leaves.
                resolved = false;
                break;
            }
        }
        Ok(Some(ValueBound {
            score: best,
            resolved,
        }))
    }
}

#[cfg(test)]
mod tests;
