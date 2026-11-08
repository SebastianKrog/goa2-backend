//! Bounded alpha-beta over live resolution decisions.
use super::{
    DecisionMove, EngineError, ExecutionBudget, NativeSession, PlanningCommitment, RequestChoices,
    Result, SearchNodeBudget,
};
use crate::{
    cards::HeroRecord,
    codec::Phase,
    data::Symbol,
    geometry::Hex,
    world::{ActionKind, EntityKind, MinionKind},
};

use std::{
    cmp::Reverse,
    sync::atomic::{AtomicUsize, Ordering},
};

const SEARCH_WORKER_STACK: usize = 8 * 1024 * 1024;
pub(super) const MAX_SEARCH_ROLLOUT_TURNS: u8 = 4;

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpponentPolicy {
    #[default]
    Adversarial,
    AdversarialAttackTargets,
    AdversarialMovementTargets,
    FirstLegal,
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchEvaluation {
    #[default]
    MaterialPosition,
    LabAttackRangeV1,
    LanePressureV1,
    LanePressureV2,
    LanePressureV3,
    LanePressureV3AccumulatedGold,
    WaveBreakthrough,
    LanePressureV4,
    LanePressureV5,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub max_depth: u8,
    pub max_nodes: usize,
    pub opponent_policy: OpponentPolicy,
    pub evaluation: SearchEvaluation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionScoreKind {
    Immediate,
    DecisionDepth,
    ShortTurnRollout,
    TurnRollout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStop {
    Resolved,
    DepthLimit,
    NodeLimit,
    Deadline,
    Cancelled,
}

#[derive(Debug)]
pub struct ResolutionSearch {
    pub decision: DecisionMove,
    /// A scored root move may be available even before depth one is complete.
    pub score: Option<i64>,
    pub score_kind: Option<ResolutionScoreKind>,
    pub completed_depth: u8,
    pub resolved: bool,
    pub stop: ResolutionStop,
    pub nodes: usize,
}

#[derive(Debug, PartialEq)]
pub struct GameTurnRollout {
    pub score: i64,
    pub nodes: usize,
    pub round: i64,
    pub turn: i64,
    pub phase: Phase,
    pub state: Option<serde_json::Value>,
    pub lab_value: Option<f64>,
}

impl GameTurnRollout {
    fn reached_cutoff(&self, cutoff: i128) -> bool {
        self.phase == Phase::GameOver
            || i128::from(self.round) * 4 + i128::from(self.turn) >= cutoff
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PlanningSearch {
    pub row: usize,
    pub score: Option<i64>,
    pub completed_rows: usize,
    pub pruned_rows: usize,
    pub evaluated_cells: usize,
    pub total_cells: usize,
    pub nodes: usize,
    pub forecast_changes: usize,
    pub forecast_checks: usize,
    pub forecast_matches: usize,
}

#[derive(Debug)]
pub struct JointPlans {
    pub plans: Vec<Vec<PlanningCommitment>>,
    pub total: usize,
}

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
    upgrade_checkpoints: Option<&'a std::collections::BTreeMap<String, Vec<super::UpgradeCheckpoint>>>,
    turn: i128,
}

struct PlanningWorld {
    session: NativeSession,
    replies: Vec<Vec<PlanningCommitment>>,
    forecasts: GreedyForecasts,
    fixed_opponent_seed: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastNodeShare {
    #[default]
    Quarter,
    Eighth,
    Zero,
}

impl ForecastNodeShare {
    fn allowance(self, nodes: usize) -> usize {
        match self {
            Self::Quarter => nodes / 4,
            Self::Eighth => nodes / 8,
            Self::Zero => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlanningSearchOptions<'a> {
    pub rollout_turns: Option<u8>,
    pub workers: usize,
    pub upgrade_checkpoints:
        Option<&'a std::collections::BTreeMap<String, Vec<super::UpgradeCheckpoint>>>,
    pub reply_cap: Option<usize>,
    pub reply_probe_cap: Option<usize>,
    pub greedy_card_order: bool,
    pub tactical_forecast: bool,
    pub forecast_node_share: ForecastNodeShare,
    pub legacy_fixed_followup: bool,
    pub fixed_future_planning: bool,
    pub branch_future_planning: bool,
}

impl PlanningSearchOptions<'_> {
    fn fixed_opponent_seed(self, seed: u64) -> Option<u64> {
        (self.greedy_card_order && !self.fixed_future_planning && !self.branch_future_planning)
            .then_some(seed)
    }

    fn future_branching(self) -> Option<bool> {
        if self.fixed_future_planning {
            Some(false)
        } else {
            self.branch_future_planning.then_some(true)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlanningRolloutSample<'a> {
    pub seed: u64,
    pub upgrade_checkpoints:
        Option<&'a std::collections::BTreeMap<String, Vec<super::UpgradeCheckpoint>>>,
}

#[derive(Debug)]
pub struct SampledPlanningSearch {
    pub commitment: PlanningCommitment,
    pub matrix: PlanningSearch,
    pub total_friendly_plans: usize,
    pub total_opposing_plans: usize,
}

#[derive(Clone, Copy)]
pub struct BotPlanningScope<'a> {
    pub seed: u64,
    pub bots: &'a [Symbol],
    pub worlds: usize,
    pub mode: super::TeamKnowledgeMode,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BotTeamPlanningSearch {
    pub prepared_worlds: usize,
    pub requested_worlds: usize,
    pub total_rows: usize,
    pub probe_nodes: usize,
    pub greedy_nodes: usize,
    pub greedy_tactical_probes: usize,
    pub shallow_nodes: usize,
    pub seed_nodes: usize,
    pub seed_score: Option<i64>,
    pub greedy_card_scores: usize,
    pub screened_replies: usize,
    pub commitments: Vec<PlanningCommitment>,
    pub shallow_score: Option<i64>,
    pub matrix: PlanningSearch,
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
    let scored_heroes: Vec<_> = scores.iter().filter_map(|(hero, cards)| {
        let best = cards.iter().map(|(_, score)| *score).max()?;
        Some((hero, cards, best))
    }).collect();
    let mut weights: Vec<_> = rows.iter().map(|row| {
        let cost: u64 = row.iter().filter_map(|commitment| {
            let (_, cards, best) = scored_heroes.iter()
                .find(|(hero, _, _)| **hero == commitment.hero)?;
            let score = cards.iter().find_map(|(card, score)| {
                (*card == commitment.first).then_some(*score)
            });
            Some(greedy_card_cost(*best, score))
        }).sum();
        (-(cost as f64) / GREEDY_CARD_TEMPERATURE as f64).exp()
    }).collect();
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
    use std::{cmp::Reverse, collections::{BinaryHeap, HashSet}};

    let mut queue = BinaryHeap::from([Reverse((0u64, 0usize, 0usize))]);
    let mut seen = HashSet::from([0]);
    let mut indices = Vec::with_capacity(cap);
    while indices.len() < cap {
        let Some(Reverse((cost, rank, index))) = queue.pop() else { break };
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

fn search_planning_worlds(
    player: Symbol,
    rows: &[Vec<PlanningCommitment>],
    samples: &mut [PlanningWorld],
    limits: SearchLimits,
    workers: usize,
    partition: PlanningPartition<'_>,
    budget: &ExecutionBudget<'_>,
    min_live_rows: Option<usize>,
) -> Result<PlanningSearch> {
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
            let evaluate =
                |sample: &mut PlanningWorld, shared_nodes| {
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
                                .map_err(|error| EngineError::Unsupported(format!(
                                    "could not spawn planning scoring worker: {error}",
                                )))
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

#[derive(Default, serde::Serialize)]
pub struct ResolutionReuseCensus {
    pub visited: usize,
    pub internal: usize,
    pub repeated_internal: usize,
    pub repeated_leaf: usize,
    pub repeated_across_depths: usize,
    pub world_only_internal: usize,
    pub world_only_leaf: usize,
    pub planning_boundaries: usize,
    pub world_only_planning: usize,
    pub planning_pairs_checked: usize,
    pub planning_pairs_diverged: usize,
    pub planning_pairs_crossed_round: usize,
    pub planning_pairs_ended_game: usize,
    pub planning_pairs_capped: usize,
    pub key_nanoseconds: u128,
}

enum PlanningPair {
    Diverged,
    NextRound,
    GameOver,
    Capped,
}

#[derive(Default)]
struct ResolutionCensusKeys {
    by_depth: std::collections::HashSet<(u128, u128, u8)>,
    exact: std::collections::HashSet<(u128, u128)>,
    world: std::collections::HashSet<(u128, u8)>,
    planning: std::collections::HashMap<(u128, u8), (u128, NativeSession)>,
}

impl NativeSession {
    /// Count same-depth key matches in the unpruned native resolution tree.
    /// Depth counts submitted inputs, including forced choices.
    /// This diagnostic never substitutes a cached result for a live branch.
    pub fn resolution_reuse_census(
        &mut self,
        depth: u8,
        max_nodes: usize,
    ) -> Result<ResolutionReuseCensus> {
        if depth > 8 || !(1..=20_000).contains(&max_nodes) {
            return Err(EngineError::InvalidInput(
                "reuse census requires depth at most 8 and 1 to 20000 nodes".into(),
            ));
        }
        let mut census = ResolutionReuseCensus::default();
        self.census_resolution_node(
            depth, max_nodes, &mut Default::default(), &mut census,
        )?;
        Ok(census)
    }

    fn census_resolution_node(
        &mut self,
        depth: u8,
        max_nodes: usize,
        seen: &mut ResolutionCensusKeys,
        census: &mut ResolutionReuseCensus,
    ) -> Result<()> {
        census.visited += 1;
        let started = std::time::Instant::now();
        let world = self.world_state_fingerprint();
        let execution = self.execution_fingerprint();
        census.key_nanoseconds += started.elapsed().as_nanos();
        let repeated = !seen.by_depth.insert((world, execution, depth));
        let exact_seen = !seen.exact.insert((world, execution));
        let world_seen = !seen.world.insert((world, depth));
        census.repeated_across_depths += usize::from(exact_seen && !repeated);
        let world_only = world_seen && !repeated;
        if self.planning.phase == Phase::Planning {
            census.planning_boundaries += 1;
            census.world_only_planning += usize::from(world_only);
            if census.planning_pairs_checked < 8 {
                let key = (world, depth);
                if let Some((previous_execution, previous)) = seen.planning.get(&key) {
                    if *previous_execution != execution {
                        census.planning_pairs_checked += 1;
                        match previous.clone().compare_planning_continuation(self.clone())? {
                            PlanningPair::Diverged => census.planning_pairs_diverged += 1,
                            PlanningPair::NextRound => census.planning_pairs_crossed_round += 1,
                            PlanningPair::GameOver => census.planning_pairs_ended_game += 1,
                            PlanningPair::Capped => census.planning_pairs_capped += 1,
                        }
                    }
                } else if seen.planning.len() < 64 {
                    seen.planning.insert(key, (execution, self.clone()));
                }
            }
        }
        let mut choices = Vec::new();
        if depth > 0
            && !matches!(self.planning.phase, Phase::Planning | Phase::GameOver)
            && let Some((owner, _)) = self.search_player()?
        {
            choices = self.resolution_choices(owner)?;
        }
        if choices.is_empty() {
            census.repeated_leaf += usize::from(repeated);
            census.world_only_leaf += usize::from(world_only);
            return Ok(());
        }
        census.internal += 1;
        census.repeated_internal += usize::from(repeated);
        census.world_only_internal += usize::from(world_only);
        for choice in choices {
            if census.visited == max_nodes {
                break;
            }
            let DecisionMove::Input { request: Some(token), selection, .. } = choice else {
                return Err(EngineError::Unsupported("census requires a live input".into()));
            };
            self.with_consumed_request(|branch| {
                branch.submit(token, selection)?;
                branch.last_request = None;
                branch.advance_to_decision(&ExecutionBudget::default())?;
                branch.census_resolution_node(depth - 1, max_nodes, seen, census)
            })?;
        }
        Ok(())
    }

    fn compare_planning_continuation(&mut self, mut other: Self) -> Result<PlanningPair> {
        let initial_round = self.round.round;
        let budget = ExecutionBudget::default();
        for _ in 0..256 {
            if self.world_state_fingerprint() != other.world_state_fingerprint()
                || self.search_turn_index() != other.search_turn_index()
                || self.planning.phase != other.planning.phase
            {
                return Ok(PlanningPair::Diverged);
            }
            if self.planning.phase == Phase::GameOver {
                return Ok(PlanningPair::GameOver);
            }
            if self.planning.phase == Phase::Planning {
                let mut joint = Vec::new();
                for hero in &self.world.heroes {
                    let legal = self.hero_planning_commitments(hero.id)?;
                    if legal != other.hero_planning_commitments(hero.id)? {
                        return Ok(PlanningPair::Diverged);
                    }
                    joint.extend(legal.first().copied());
                }
                if self.round.round > initial_round {
                    return Ok(PlanningPair::NextRound);
                }
                self.apply_joint_planning_ordered(&joint)?;
                other.apply_joint_planning_ordered(&joint)?;
            } else {
                let owner = self.search_player()?;
                if owner != other.search_player()? {
                    return Ok(PlanningPair::Diverged);
                }
                let Some((owner, _)) = owner else {
                    return Ok(PlanningPair::Capped);
                };
                let left = self.resolution_choices(owner)?;
                let right = other.resolution_choices(owner)?;
                let comparable = |choice: &DecisionMove| match choice {
                    DecisionMove::Input { player, selection, .. } => Some((*player, selection.clone())),
                    _ => None,
                };
                if !left.iter().map(comparable).eq(right.iter().map(comparable)) {
                    return Ok(PlanningPair::Diverged);
                }
                for (worker, choices) in [(self as &mut Self, left), (&mut other, right)] {
                    let Some(DecisionMove::Input { request: Some(token), selection, .. }) =
                        choices.into_iter().next()
                    else {
                        return Ok(PlanningPair::Capped);
                    };
                    worker.submit(token, selection)?;
                    worker.last_request = None;
                }
            }
            self.advance_to_decision(&budget)?;
            other.advance_to_decision(&budget)?;
        }
        Ok(PlanningPair::Capped)
    }

    /// Generate up to cap joint plans. The requesting hero's choices vary
    /// first so a bounded friendly set covers their options before teammates'.
    pub fn joint_plans(&self, player: Symbol, friendly: bool, cap: usize) -> Result<JointPlans> {
        self.joint_plans_partition(player, friendly, cap, None)
    }

    pub(super) fn joint_plans_partition(
        &self,
        player: Symbol,
        friendly: bool,
        cap: usize,
        controlled: Option<&[Symbol]>,
    ) -> Result<JointPlans> {
        self.joint_plans_with_reply_order(player, friendly, cap, controlled, JointPlanOrder::PlayerFirst, &[])
    }

    fn joint_plans_with_reply_order(
        &self,
        player: Symbol,
        friendly: bool,
        cap: usize,
        controlled: Option<&[Symbol]>,
        order: JointPlanOrder,
        greedy_scores: &[(Symbol, HeroCardScores)],
    ) -> Result<JointPlans> {
        let invalid = |reason: &str| EngineError::InvalidInput(reason.into());
        if self.planning.phase != Phase::Planning || !(1..=4096).contains(&cap) {
            return Err(invalid(
                "joint plan generation requires planning and a cap in 1..4096",
            ));
        }
        let team = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| invalid("search player is not a hero"))?
            .team;
        let mut dimensions = Vec::new();
        let mut strides = Vec::new();
        let mut costs = Vec::new();
        let mut total = 1usize;
        let mut heroes = self
            .world
            .heroes
            .iter()
            .filter(|hero| {
                controlled.map_or(hero.team == team, |ids| ids.contains(&hero.id)) == friendly
            })
            .collect::<Vec<_>>();
        heroes.sort_by_key(|hero| hero.id != player);
        for hero in heroes {
            let mut choices = self.hero_planning_commitments(hero.id)?;
            if choices.is_empty() {
                continue;
            }
            if let JointPlanOrder::OpponentReplies(seed) = order
                && hero.team != team
            {
                let preferred = self.greedy_opponent_commitment(hero.id, seed)?;
                let mut ranks = std::collections::HashMap::new();
                for card in choices.iter().filter_map(|choice| choice.first) {
                    if let std::collections::hash_map::Entry::Vacant(entry) = ranks.entry(card) {
                        entry.insert(self.greedy_defense_rank(hero.id, card)?);
                    }
                }
                choices.sort_by_key(|choice| (
                    Some(*choice) != preferred,
                    choice.first.map_or((true, i64::MAX), |card| ranks[&card]),
                    choice.second.is_some(),
                ));
            }
            if let Some((_, scores)) = greedy_scores.iter().find(|(id, _)| *id == hero.id) {
                let best = scores.iter().map(|(_, score)| *score).max().unwrap();
                let cost = |choice: &PlanningCommitment| {
                    let score = scores.iter().find_map(|(card, score)| {
                        (*card == choice.first).then_some(*score)
                    });
                    greedy_card_cost(best, score)
                };
                choices.sort_by_key(cost);
                costs.push(Some(choices.iter().map(cost).collect()));
            } else {
                costs.push(None);
            }
            strides.push(total);
            total = total
                .checked_mul(choices.len())
                .ok_or_else(|| invalid("joint plan count overflow"))?;
            dimensions.push(choices);
        }
        let indices = if !matches!(order, JointPlanOrder::PlayerFirst) || !greedy_scores.is_empty() {
            ranked_reply_indices(&dimensions, &strides, &costs, total.min(cap))
        } else {
            (0..total.min(cap)).collect()
        };
        let plans = indices.into_iter()
            .map(|mut index| {
                dimensions
                    .iter()
                    .map(|choices| {
                        let choice = choices[index % choices.len()];
                        index /= choices.len();
                        choice
                    })
                    .collect()
            })
            .collect();
        Ok(JointPlans { plans, total })
    }

    pub(super) fn hero_planning_commitments(
        &self,
        hero: Symbol,
    ) -> Result<Vec<PlanningCommitment>> {
        let invalid = |reason: &str| EngineError::InvalidInput(reason.into());
        let moves = self.planning_choices(hero)?;
        let cards = moves
            .iter()
            .filter_map(|decision| match decision {
                DecisionMove::Card { card, .. } => Some(*card),
                _ => None,
            })
            .collect::<Vec<_>>();
        let second_slot = moves.contains(&DecisionMove::FinishPlanning { hero });
        let mut choices = Vec::new();
        for decision in moves {
            let first = match decision {
                DecisionMove::Card { card, .. } => Some(card),
                DecisionMove::Pass { .. } | DecisionMove::FinishPlanning { .. } => None,
                _ => return Err(invalid("unexpected joint planning choice")),
            };
            choices.push(PlanningCommitment {
                hero,
                first,
                second: None,
            });
        }
        if !second_slot && self.planning.two_cards(&self.world, hero) {
            // Put single-card plans first so capped sets still see every
            // possible first card before considering second-card variants.
            for &first in &cards {
                for &second in &cards {
                    if first != second {
                        choices.push(PlanningCommitment {
                            hero,
                            first: Some(first),
                            second: Some(second),
                        });
                    }
                }
            }
        }
        Ok(choices)
    }

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
            deadline: budget.deadline.map(|end| now + end.saturating_duration_since(now) / 4),
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
        let seed_time = budget.deadline.map(|deadline| deadline.saturating_duration_since(now));
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
        let team = self.world.hero(player).ok_or_else(|| EngineError::InvalidInput("unknown search player".into()))?.team;
        let mut reply = Vec::new();
        for hero in &self.world.heroes {
            if controlled.map_or(hero.team == team, |ids| ids.contains(&hero.id)) {
                continue;
            }
            let choice = self.sample_forecast_commitment(hero.id, forecasts, &[], seed)?;
            if let Some(choice) = choice {
                reply.push(choice);
            } else if let Some(choice) = self.greedy_opponent_commitment(
                hero.id, opponent_draw_seed(self, seed, hero.id),
            )? {
                reply.push(choice);
            }
        }
        Ok(JointPlans { plans: vec![reply], total: 1 })
    }

    fn sample_forecast_commitment(
        &self,
        hero: Symbol,
        forecasts: &[(Symbol, super::GreedySearch)],
        history: &[Vec<PlanningCommitment>],
        seed: u64,
    ) -> Result<Option<PlanningCommitment>> {
        let seed = opponent_draw_seed(self, seed, hero);
        if let Some((_, forecast)) = forecasts.iter().find(|(id, _)| *id == hero)
            && history.len() < 4
        {
            let legal = self.planning_choices(hero)?.into_iter().map(|option| match option {
                DecisionMove::Card { card, .. } => Some(card),
                _ => None,
            }).collect::<Vec<_>>();
            let mut prefix = [None; 3];
            for (slot, batch) in prefix.iter_mut().zip(history) {
                *slot = batch.iter().find(|entry| entry.hero == hero).copied();
            }
            if let Some(weights) = forecast.next_card_likelihoods(&prefix[..history.len()], &legal) {
                let mut draw = (seed >> 11) as f64 / (1u64 << 53) as f64;
                let mut first = weights.last().unwrap().0;
                for (card, weight) in weights {
                    draw -= weight;
                    if draw < 0.0 {
                        first = card;
                        break;
                    }
                }
                return Ok(Some(PlanningCommitment { hero, first, second: None }));
            }
        }
        Ok(None)
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
        let limits = SearchLimits { max_nodes: limits.max_nodes - greedy_nodes, ..limits };
        let fixed_opponent_seed = options.fixed_opponent_seed(seed);
        let solo = [player];
        let controlled = fixed_opponent_seed.map(|_| solo.as_slice());
        let friendly = self.joint_plans_with_reply_order(
            player, true, cap, controlled, JointPlanOrder::PlayerFirst, &greedy_scores,
        )?;
        let opposing = if let Some(seed) = fixed_opponent_seed {
            self.fixed_planning_reply(player, controlled, &forecasts, seed)?
        } else {
            self.joint_plans_with_reply_order(
                player, false, options.reply_cap.unwrap_or(cap), None, JointPlanOrder::OpponentReplies(seed), &greedy_scores,
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
        self.search_bot_team_planning_with_seed_pass(player, scope, cap, limits, options, budget, false)
    }

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
        self.search_bot_team_planning_inner(player, scope, cap, limits, options, budget, seed_pass, None)
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
        let budget = ExecutionBudget { search_nodes: Some(&meter), ..*budget };
        let run = |cap, max_depth, max_nodes, options, incumbent: Option<&[PlanningCommitment]>| {
            self.search_bot_team_planning_inner(
                player, scope, cap, SearchLimits { max_depth, max_nodes, ..limits },
                options, &budget, false, incumbent,
            )
        };
        let mut best = run(
            cap.min(4), 0, 512,
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
                cap.min(4), limits.max_depth.min(2), 16_384, shorter, None,
            ))? && planning_has_score(&result)
            {
                best = result;
            }
        }
        if meter.remaining() > 0 && best.matrix.score != Some(WIN)
            && let Some(deeper) = retain_arena_backup(run(
                cap, limits.max_depth, ARENA_SEARCH_CEILING, options,
                planning_has_score(&best).then_some(best.commitments.as_slice()),
            ))?
            && (deeper.matrix.score.is_some() || (!planning_has_score(&best) && planning_has_score(&deeper)))
        {
            best = deeper;
        }
        best.matrix.nodes = limits.max_nodes - meter.remaining();
        Ok(best)
    }

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
        self.search_bot_team_planning_inner(player, scope, cap, limits, options, budget, false, incumbent)
    }

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
            return Err(EngineError::InvalidInput("planning reply caps must be in 1..4096".into()));
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
                    deadline: seed_budget.deadline.zip(seed_slice).map(|(end, slice)| {
                        end.min(std::time::Instant::now() + slice)
                    }),
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
                world.fixed_planning_reply(player, Some(&controlled), &forecasts, seed)?.plans
            } else {
                world.joint_plans_with_reply_order(
                    player, false, probe_cap.max(reply_cap), Some(&controlled), JointPlanOrder::OpponentReplies(sample_seed), &greedy_scores,
                )?.plans
            };
            let probe_nodes = forecast_work / 4 / scope.worlds;
            // Each cell consumes at least one node. Screening cannot change
            // the shortlist unless it scores more than reply_cap full columns.
            let can_change_shortlist = probe_nodes / rows.plans.len() > reply_cap;
            let probe = if replies.len() > reply_cap && can_change_shortlist {
                world.screen_planning_replies(
                    player, &rows.plans, &mut replies, reply_cap,
                    SearchLimits { max_nodes: probe_nodes, ..limits },
                    partition, &probe_budget,
                )?
            } else {
                (0, 0)
            };
            replies.truncate(reply_cap);
            let weights = options.greedy_card_order
                .then(|| shared_row_likelihoods(&rows.plans, &greedy_scores));
            let card_scores = greedy_scores.iter().map(|(_, cards)| cards.len()).sum::<usize>();
            Ok((PlanningWorld { session: world, replies, forecasts, fixed_opponent_seed }, probe, greedy_nodes, card_scores, weights))
        };
        let mut samples = Vec::with_capacity(scope.worlds);
        let mut probe_nodes = 0;
        let mut screened_replies = 0;
        let mut greedy_nodes = 0;
        let mut greedy_tactical_probes = 0;
        let mut greedy_card_scores = 0;
        let mut row_weights = options.greedy_card_order.then(|| vec![0.0; rows.plans.len()]);
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
                    Ok::<_, EngineError>(jobs.into_iter()
                        .map(|job| {
                            job.join().unwrap_or_else(|_| {
                                Err(EngineError::Unsupported(
                                    "planning sample worker panicked".into(),
                                ))
                            })
                        })
                        .collect())
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
                        greedy_tactical_probes += sample.forecasts.iter()
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
        let live_incumbent = live_incumbent
            .and_then(|incumbent| rows.plans.iter().position(|row| row == incumbent));
        if let Some(index) = live_incumbent {
            rows.plans[..=index].rotate_right(1);
        }
        let remaining = limits.max_nodes - probe_nodes - greedy_nodes;
        let shallow_nodes = if options.rollout_turns.is_some() {
            (remaining / 16).min(512)
        } else {
            0
        };
        let seed_allowance = if seed_pass && limits.max_depth > 0 && options.rollout_turns.is_some() {
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
                SearchLimits { max_nodes: seed_allowance, max_depth: 0, ..limits },
                options.workers,
                PlanningPartition { future_branching: Some(false), ..partition },
                &seed_budget,
                None,
            )?;
            seed_used = seed.nodes;
            if seed.score.is_some() {
                rows.plans[..=seed.row].rotate_right(1);
                seed_score = seed.score;
            }
        }
        let deep_limits = SearchLimits { max_nodes: remaining - shallow_nodes - seed_used, ..limits };
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
                deep_limits,
                options.workers,
                partition,
                &deep_budget,
                live_incumbent.map(|_| 4),
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
                SearchLimits { max_nodes: shallow_nodes, max_depth: 0, ..limits },
                options.workers,
                PlanningPartition { rollout_turns: None, future_branching: Some(false), ..partition },
                budget,
                None,
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
                    player, std::slice::from_ref(reference), std::slice::from_ref(reply),
                    SearchLimits { max_depth: 1, max_nodes: limits.max_nodes - used, ..limits },
                    PlanningPartition { rollout_turns: Some(1), ..partition },
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
                            SearchLimits { max_nodes, ..limits },
                            partition,
                            budget,
                        )
                    })
                    .map_err(|error| EngineError::Unsupported(format!(
                        "could not spawn planning matrix worker: {error}",
                    )))?;
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
                    partition.fixed_opponent_seed.is_some() || !partition.greedy_forecasts.is_empty(),
                );
                if expand || !partition.greedy_forecasts.is_empty() || partition.fixed_opponent_seed.is_some() {
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
                player, limits, rollout_turns, planning_sample, budget, true,
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
            player, (sample, worlds, workers), limits, rollout_turns, planning_sample, None, budget,
        )
    }

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
        let budget = ExecutionBudget { search_nodes: Some(&meter), ..*budget };
        if self.planning.phase == Phase::LevelUp {
            return self.search_upgrade_choices(player, limits, &budget);
        }
        let mut ranking = Vec::new();
        let mut best = self.search_live_bot_resolution(
            player, sample, worlds, bots, None, &mut ranking, probabilistic_hand_responses,
            SearchLimits { max_depth: limits.max_depth.min(2), max_nodes: 8_192, ..limits },
            rollout_turns.min(2), &budget,
        )?;
        if meter.remaining() > 0 && best.score != Some(WIN)
            && let Some(deeper) = retain_arena_backup(self.search_live_bot_resolution(
                player, sample, worlds, bots,
                best.score.map(|_| &best.decision), &mut ranking, probabilistic_hand_responses,
                SearchLimits { max_nodes: ARENA_SEARCH_CEILING, ..limits },
                rollout_turns, &budget,
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
            player, (sample, worlds, 1), limits, Some(rollout_turns), None,
            Some(LiveResolution { bots, incumbent, ranking, probabilistic_hand_responses }), budget,
        )
    }

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
            hand_belief: live.as_ref().filter(|scope| scope.probabilistic_hand_responses).map(|scope| {
                HandBelief { known: scope.bots, upgrade_checkpoints: sample.upgrade_checkpoints,
                             turn: self.search_turn_index() }
            }),
            planning_sample,
            future_planning: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
        };
        let mut sampled = match self.sampled_resolution_workers(
            player, sample, worlds, live.as_ref().map_or(&[], |scope| scope.bots), budget,
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
        let incumbent = live.as_ref().and_then(|scope| scope.incumbent)
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
            Some(live_root_width(scope.ranking.iter().map(|(_, score)| *score), moves.len()))
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

        let short = if incumbent.is_none() && resolved_scores.iter().flatten().any(Option::is_none) {
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
            let ordered = std::iter::once(preferred)
                .chain(root_order.iter().copied().filter(|&index| index != preferred));
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
                                    let mut local = Search { nodes: 0, future_planning: None, ..search };
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
                        if live.is_some() && let Some((index, sum)) = best_sum {
                            // These roots completed in every sampled world at
                            // this depth. Ignore the interrupted root's score.
                            result.decision = moves[index].clone();
                            result.score = Some((sum / worlds as i128) as i64);
                            result.score_kind = Some(search.depth_score_kind());
                            if result.completed_depth == 0
                                && let Some((scope, scores)) = live.as_mut().zip(depth_scores.as_ref())
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
            player, limits, rollout_turns, planning_sample, budget, false,
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

    pub(super) fn order_movement_roots(&self, player: Symbol, moves: &mut [DecisionMove]) {
        if moves.len() < 3 {
            return;
        }
        let Some(actor) = self.movement_root_actor() else { return };
        let Some(hero) = self.world.hero(actor).filter(|hero| hero.id == player) else {
            return;
        };
        let mut enemy_heroes = Vec::new();
        let mut enemy_minions = Vec::new();
        for &(id, hex) in &self.world.locations {
            let Some(entity) = self.world.entity(id) else { continue };
            if entity.team.is_some_and(|team| team != hero.team) {
                match entity.kind {
                    EntityKind::Hero | EntityKind::Piece => enemy_heroes.push(hex),
                    EntityKind::Minion => enemy_minions.push(hex),
                    _ => {}
                }
            }
        }
        if enemy_heroes.is_empty() && enemy_minions.is_empty() {
            return;
        }
        let destination = |decision: &DecisionMove| match decision {
            DecisionMove::Input { selection: selection @ super::Datum::Object(_), .. } => {
                super::datum_hex(selection, &self.symbols, self.coordinates).ok()
            }
            _ => None,
        };
        let nearest = |hex: Hex, enemies: &[Hex]| {
            enemies.iter().map(|&enemy| hex.distance(enemy)).min()
        };
        let distances = moves.iter().filter_map(|decision| {
            let hex = destination(decision)?;
            Some((hex, nearest(hex, &enemy_minions), nearest(hex, &enemy_heroes)))
        }).collect::<Vec<_>>();
        let mut approach_minion = Vec::new();
        let mut approach_hero = Vec::new();
        let mut retreat = Vec::new();
        for (hex, minion, hero) in distances {
            if let Some(distance) = minion {
                approach_minion.push((hex, distance));
            }
            if let Some(distance) = hero {
                approach_hero.push((hex, distance));
            }
            if let Some(distance) = minion.into_iter().chain(hero).min() {
                retreat.push((hex, distance));
            }
        }
        approach_minion.sort_by_key(|&(_, distance)| distance);
        approach_hero.sort_by_key(|&(_, distance)| distance);
        retreat.sort_by_key(|&(_, distance)| std::cmp::Reverse(distance));
        let mut priority = std::collections::HashMap::with_capacity(retreat.len());
        for rank in 0..retreat.len() {
            for lane in [&approach_minion, &approach_hero, &retreat] {
                if let Some(&(hex, _)) = lane.get(rank) {
                    let next = priority.len();
                    priority.entry(hex).or_insert(next);
                }
            }
        }
        moves.sort_by_cached_key(|decision| {
            destination(decision).and_then(|hex| priority.get(&hex).copied()).unwrap_or(usize::MAX)
        });
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
                let eligible = || self.world.heroes.iter()
                    .filter(|hero| self.resolution_request_addresses(request, hero.id));
                controlled
                    .and_then(|ids| eligible().find(|hero| ids.contains(&hero.id)))
                    .or_else(|| eligible().next())
                    .map(|hero| hero.id)
            },
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

    /// The Python lab's lab-attack-range-v1 leaf evaluation.
    pub fn evaluate_lab_attack_range(&self, player: Symbol) -> Result<f64> {
        self.evaluate_attack_range(player, false)
    }

    fn evaluate_attack_range(&self, player: Symbol, accumulated_gold: bool) -> Result<f64> {
        let hero = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| EngineError::InvalidInput("evaluation player is not a hero".into()))?;
        if let Some(winner) = self.world.combat.outcome.and_then(|outcome| outcome.team) {
            return Ok(if winner == hero.team { 1.0 } else { -1.0 });
        }

        let sign = |team| if team == hero.team { 1 } else { -1 };
        let life = self
            .world
            .combat
            .team_lives()
            .map(|(team, life)| sign(team) * life)
            .sum::<i64>();
        let gold = self
            .world
            .heroes
            .iter()
            .map(|row| {
                let spent = if accumulated_gold {
                    // The upgrade from level L to L+1 costs L gold.
                    let level = row.level.clamp(1, 8);
                    level * (level - 1) / 2
                } else {
                    0
                };
                sign(row.team) * (row.gold + spent)
            })
            .sum::<i64>();
        let mut minions = 0;
        let mut near_minion = false;
        let mut near_hero = false;
        let mut cover = false;
        let mut exposure = false;
        let range = hero
            .hand
            .iter()
            .filter_map(|id| {
                let card = self.world.card(*id)?;
                let def = &self.world.definitions.cards[card.definition];
                let color = def.color.map(|color| self.symbols.name(color));
                (!card.played_this_round && matches!(color, Some("RED" | "GOLD")))
                    .then_some(def.range.filter(|range| *range != 0).unwrap_or(1))
            })
            .max();
        let positions = self.world.positions(player).collect::<Vec<_>>();
        for &(id, target) in &self.world.locations {
            let Some(entity) = self.world.entity(id) else {
                continue;
            };
            let Some(team) = entity.team else { continue };
            let distance = positions
                .iter()
                .map(|position| i64::from(position.distance(target)))
                .min();
            match entity.kind {
                EntityKind::Minion => {
                    let heavy = entity.minion_kind == Some(MinionKind::Heavy);
                    minions += sign(team) * if heavy { 4 } else { 2 };
                    if team == hero.team {
                        cover |= distance == Some(1)
                            && matches!(
                                entity.minion_kind,
                                Some(MinionKind::Melee | MinionKind::Heavy)
                            );
                    } else {
                        near_minion |= distance.is_some_and(|d| d <= range.unwrap_or(1));
                        exposure |= entity.minion_kind == Some(MinionKind::Ranged)
                            && distance.is_some_and(|d| d <= 2);
                    }
                }
                EntityKind::Hero | EntityKind::Piece if team != hero.team => {
                    near_hero |= distance.is_some_and(|d| d <= range.unwrap_or(1));
                }
                _ => {}
            }
        }
        let setup = if near_minion && range.is_some() {
            0.12
        } else if near_hero {
            0.08
        } else if near_minion {
            0.04
        } else {
            0.0
        };
        let bonus =
            setup + 0.005 * f64::from(u8::from(cover)) - 0.01 * f64::from(u8::from(exposure));
        let material = life as f64 + 0.1 * gold as f64 + 0.25 * minions as f64;
        let value = (material / 5.0).tanh().clamp(-1.0 + 1e-6, 1.0 - 1e-6);
        Ok((value.atanh() + bonus / 5.0).tanh())
    }

    pub(super) fn search_score(
        &self,
        evaluation: SearchEvaluation,
        team: Symbol,
        player: Symbol,
    ) -> Result<i64> {
        if matches!(evaluation, SearchEvaluation::MaterialPosition)
            || self.world.combat.outcome.is_some()
        {
            return Ok(self.evaluate_resolution(team, player));
        }
        let mut score = self.evaluate_attack_range(
            player, matches!(evaluation, SearchEvaluation::LanePressureV3AccumulatedGold),
        )?;
        if !matches!(evaluation, SearchEvaluation::LabAttackRangeV1) {
            score = (score.clamp(-1.0 + 1e-6, 1.0 - 1e-6).atanh()
                + self.evaluate_lane_pressure(team, evaluation) / 5.0).tanh();
        }
        Ok((score * (WIN - 1) as f64).round() as i64)
    }

    fn evaluate_lane_pressure(&self, team: Symbol, evaluation: SearchEvaluation) -> f64 {
        let graph = &self.world.definitions.zones;
        let direction = if self.symbols.name(team) == "RED" { 1.0 } else { -1.0 };
        let last_wave = self.world.waves.iter().any(|(_, remaining)| *remaining <= 1);
        let depth_weight = if last_wave { 0.65 } else { 0.35 };
        let timed_push = matches!(evaluation, SearchEvaluation::LanePressureV4 | SearchEvaluation::LanePressureV5);
        let lane_coverage = graph.lanes.len() > 1 && matches!(evaluation, SearchEvaluation::LanePressureV5);
        let any_lane_exhausted = timed_push
            && self.world.waves.iter().any(|(_, remaining)| *remaining <= 0);
        let mut score = 0.0;
        for (lane, active) in &self.world.battle_zones {
            let Some((_, ordered)) = graph.lanes.iter().find(|(id, _)| id == lane) else {
                continue;
            };
            let Some(index) = ordered.iter().position(|id| id == active) else {
                continue;
            };
            let Some(zone) = graph.zones.iter().find(|zone| zone.id == *active) else {
                continue;
            };
            let red_depth = index.saturating_sub(1);
            let blue_depth = ordered.len().saturating_sub(2 + index);
            // Each ordinary lane shift changes the depth differential by two.
            let depth = direction * (red_depth as f64 - blue_depth as f64) / 2.0;
            if !timed_push {
                score += depth_weight * direction * (red_depth as f64 - blue_depth as f64) / 2.0;
            }
            let mut closest_readiness = [0.0_f64; 2];
            let mut advancing_readiness = 0.0;
            let mut advancing_heroes = 0;

            let mut minions = [0.0, 0.0];
            for &(id, hex) in &self.world.locations {
                if zone.hexes.contains(&hex)
                    && let Some(entity) = self.world.entity(id)
                    && entity.kind == EntityKind::Minion
                    && let Some(owner) = entity.team
                {
                    minions[usize::from(owner != team)] += 1.0;
                }
            }
            let [friendly, enemy] = minions;
            score += if matches!(evaluation, SearchEvaluation::WaveBreakthrough) {
                // Clearing the last blocker matters more when a friendly wave survives.
                let chance = |own, opposing| (own / (own + 2.0)) / (1.0 + opposing);
                0.60 * (chance(friendly, enemy) - chance(enemy, friendly))
            } else {
                0.30 * (friendly - enemy) / (friendly + enemy + 1.0)
            };
            for hero in &self.world.heroes {
                let advances = timed_push && depth != 0.0 && ((hero.team == team) == (depth > 0.0));
                advancing_heroes += usize::from(advances);
                let positions = self.world.positions(hero.id);
                let distance = if matches!(evaluation, SearchEvaluation::LanePressureV1) {
                    positions.into_iter().any(|hex| zone.hexes.contains(&hex)).then_some(0)
                } else {
                    positions.filter_map(|hex| {
                        if (matches!(evaluation,
                            SearchEvaluation::LanePressureV3
                                | SearchEvaluation::LanePressureV3AccumulatedGold
                                | SearchEvaluation::WaveBreakthrough
                        ) || timed_push)
                            && let Some(distance) = zone.spawns.iter()
                                .filter(|spawn| spawn.team != hero.team)
                                .map(|spawn| hex.distance(spawn.hex))
                                .min()
                        {
                            return Some(distance);
                        }
                        if zone.hexes.contains(&hex) {
                            Some(0)
                        } else {
                            zone.hexes.iter().map(|target| hex.distance(*target)).min()
                        }
                    }).min()
                };
                if let Some(distance) = distance {
                    let readiness = 0.20 / (1.0 + f64::from(distance));
                    if lane_coverage {
                        let side = usize::from(hero.team != team);
                        closest_readiness[side] = closest_readiness[side].max(readiness);
                    } else {
                        score += if hero.team == team { readiness } else { -readiness };
                    }
                    if advances {
                        advancing_readiness += 1.0 / (1.0 + f64::from(distance.saturating_sub(1)));
                    }
                }
            }
            if lane_coverage {
                score += closest_readiness[0] - closest_readiness[1];
            }
            if timed_push {
                // Once any lane is exhausted, a push in either lane can
                // trigger the last-push comparison.
                let lane_last_wave = any_lane_exhausted || self.world.waves.iter()
                    .any(|(id, remaining)| id == lane && *remaining <= 1);
                let weight = if lane_last_wave { 0.65 } else { 0.35 };
                let readiness = advancing_readiness / advancing_heroes.max(1) as f64;
                score += weight * depth * (0.25 + 0.75 * readiness);
            }
        }
        score
    }

    fn evaluate_resolution(&self, root_team: Symbol, player: Symbol) -> i64 {
        if let Some(outcome) = self.world.combat.outcome {
            let winner = outcome.team.or_else(|| {
                outcome
                    .hero
                    .and_then(|hero| self.world.hero(hero).map(|row| row.team))
            });
            return match winner {
                Some(team) if team == root_team => WIN,
                Some(_) => -WIN,
                None => 0,
            };
        }
        let sign = |team| if team == root_team { 1i128 } else { -1 };
        let life = self
            .world
            .combat
            .team_lives()
            .map(|(team, life)| sign(team) * i128::from(life) * 100)
            .sum::<i128>();
        let heroes = self
            .world
            .heroes
            .iter()
            .map(|hero| {
                sign(hero.team)
                    * (i128::from(hero.level) * 15
                        + i128::from(hero.gold) * 3
                        + hero.hand.len() as i128)
            })
            .sum::<i128>();
        let position = i128::from(self.team_position_score(root_team, player)).clamp(-31, 31);
        ((life + heroes) * 64 + position).clamp(i128::from(-WIN + 1), i128::from(WIN - 1)) as i64
    }

    pub(super) fn team_position_score(&self, team: Symbol, player: Symbol) -> i64 {
        let red = self.symbols.lookup("RED");
        let gold = self.symbols.lookup("GOLD");
        self.world.heroes.iter().map(|hero| {
            let sign = if hero.team == team { 1 } else { -1 };
            sign * self.hero_position_score(hero, hero.id == player, red, gold)
        }).sum()
    }

    pub(super) fn hero_position_score(
        &self,
        hero: &HeroRecord,
        inspect_hand: bool,
        red: Option<Symbol>,
        gold: Option<Symbol>,
    ) -> i64 {
        let mut range = 1;
        let mut has_gold_or_red = false;
        for id in hero.hand.iter().filter(|_| inspect_hand) {
            let Some(card) = self.world.card(*id) else {
                continue;
            };
            let definition = &self.world.definitions.cards[card.definition];
            has_gold_or_red |=
                definition.color.is_some() && (definition.color == red || definition.color == gold);
            if definition.ranged
                && (definition.primary == Some(ActionKind::Attack)
                    || definition
                        .secondary
                        .iter()
                        .any(|(kind, _)| *kind == ActionKind::Attack))
            {
                range = range.max(definition.range.unwrap_or(1));
            }
        }
        self.world
            .positions(hero.id)
            .map(|position| {
                let mut pressure = 0;
                let mut support = false;
                let mut exposed = false;
                for &(id, target) in &self.world.locations {
                    let Some(entity) = self.world.entity(id) else {
                        continue;
                    };
                    let distance = i64::from(position.distance(target));
                    let Some(team) = entity.team else {
                        continue;
                    };
                    match entity.kind {
                        EntityKind::Minion if team == hero.team => {
                            support |= distance == 1
                                && matches!(
                                    entity.minion_kind,
                                    Some(MinionKind::Melee | MinionKind::Heavy)
                                );
                        }
                        EntityKind::Minion => {
                            exposed |=
                                distance <= 2 && entity.minion_kind == Some(MinionKind::Ranged);
                            if distance <= range {
                                pressure = pressure.max(if has_gold_or_red { 12 } else { 4 });
                            }
                        }
                        EntityKind::Hero | EntityKind::Piece
                            if team != hero.team && distance <= range =>
                        {
                            pressure = pressure.max(8);
                        }
                        _ => {}
                    }
                }
                pressure + i64::from(support) - i64::from(exposed)
            })
            .max()
            .unwrap_or(0)
    }
}

fn opponent_draw_seed(worker: &NativeSession, seed: u64, hero: Symbol) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hash = DefaultHasher::new();
    (seed, hero, worker.search_turn_index(), worker.last_request.as_ref().map(|r| r.prompt))
        .hash(&mut hash);
    hash.finish()
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

fn arena_search_meter(limits: SearchLimits, budget: &ExecutionBudget<'_>) -> Result<SearchNodeBudget> {
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
    let contenders = 1 + scores.take_while(|&score| best - score <= LIVE_CHALLENGER_MARGIN).count();
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
    ranking.extend(scores.iter().enumerate().filter_map(|(index, score)| {
        score.map(|sum| (index, (sum / worlds as i128) as i64))
    }));
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
            deadline: self.budget.deadline.map(|end| now + end.saturating_duration_since(now) / 8),
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
            let resolved = branch.planning.phase == Phase::GameOver
                || branch.search_turn_index() >= cutoff;
            Ok(Some((self.score(branch)?, resolved)))
        })
    }

    fn score_sampled_root(
        &mut self,
        worker: &mut NativeSession,
        cached: &mut Option<i64>,
        decision: &DecisionMove,
        depth: u8,
    ) -> Result<Option<ValueBound>> {
        self.budget.check(0)?;
        if let Some(score) = *cached {
            return Ok(Some(ValueBound {
                score,
                resolved: true,
            }));
        }
        // Root cutoffs in one world cannot bound the mean.
        let value = self.branch(worker, decision.clone(), depth, i64::MIN, i64::MAX)?;
        if let Some(value) = &value
            && value.resolved
        {
            *cached = Some(value.score);
        }
        Ok(value)
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

    fn choose(
        &mut self,
        worker: &mut NativeSession,
        moves: &[DecisionMove],
        depth: u8,
        preferred: usize,
        resolved_scores: &mut [Option<i64>],
    ) -> Result<Iteration> {
        let mut best = None;
        let mut alpha = i64::MIN;
        let mut resolved = true;
        let remaining_depth = depth - u8::from(moves.len() > 1);
        for index in root_move_order(moves.len(), preferred) {
            let earlier = best.is_some_and(|(best_index, _)| index < best_index);
            // A fail-low result at alpha cannot distinguish equality from a
            // worse move. Widen by one when an exact tie would change the winner.
            let lower = if earlier {
                alpha.saturating_sub(1)
            } else {
                alpha
            };
            let score = if let Some(score) = resolved_scores[index] {
                if let Err(error) = self.budget.check(0) {
                    let stop = match error {
                        EngineError::Deadline => ResolutionStop::Deadline,
                        EngineError::Cancelled => ResolutionStop::Cancelled,
                        other => return Err(other),
                    };
                    return Ok(Iteration {
                        best,
                        interrupted: Some(stop),
                        resolved: false,
                    });
                }
                score
            } else {
                let value = match self.branch(
                    worker,
                    moves[index].clone(),
                    remaining_depth,
                    lower,
                    i64::MAX,
                ) {
                    Ok(Some(value)) => Ok(value),
                    Ok(None) => Err(ResolutionStop::NodeLimit),
                    Err(EngineError::Deadline) => Err(ResolutionStop::Deadline),
                    Err(EngineError::Cancelled) => Err(ResolutionStop::Cancelled),
                    Err(error) => return Err(error),
                };
                let value = match value {
                    Ok(value) => value,
                    Err(stop) => {
                        return Ok(Iteration {
                            best,
                            interrupted: Some(stop),
                            resolved: false,
                        });
                    }
                };
                if value.resolved {
                    // No depth-limited leaves or skipped siblings contributed.
                    // This score is exact for any later depth at this root.
                    resolved_scores[index] = Some(value.score);
                }
                resolved &= value.resolved;
                value.score
            };
            if best.is_none() || score > alpha || (earlier && score == alpha) {
                alpha = score;
                best = Some((index, score));
            }
        }
        Ok(Iteration {
            best,
            interrupted: None,
            resolved,
        })
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
            let budget = ExecutionBudget { max_steps: remaining, ..*self.budget };
            let simulated_upgrade = self.allow_simulated_upgrades && worker.stack.last().is_some_and(|frame| {
                matches!(worker.steps[frame.instruction].instruction,
                    super::Instruction::Flow(crate::turn::FlowStep::Upgrades))
            });
            let advance = if simulated_upgrade {
                // This prompt is generated on a sampled future position. It has
                // no pending real upgrade choices to reveal.
                worker.advance_objects(&budget, Some(1), false)?
            } else if let Some(belief) = self.hand_belief.filter(|belief| belief.turn == worker.search_turn_index()) {
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

    fn pending_hand_branch(&self, worker: &NativeSession) -> Result<Option<Symbol>> {
        match self.hand_belief.filter(|belief| belief.turn == worker.search_turn_index()) {
            Some(belief) => worker.pending_hidden_hand_reader(self.player, belief.known),
            None => Ok(None),
        }
    }

    fn weighted_hand_response(
        &mut self,
        worker: &mut NativeSession,
        target: Symbol,
        mut continue_search: impl FnMut(&mut Self, &mut NativeSession) -> Result<Option<ValueBound>>,
    ) -> Result<Option<ValueBound>> {
        let belief = self.hand_belief.take().expect("a hand branch requires a belief");
        let result = (|| {
            let observation = worker.player_observation(self.player)?;
            let id = worker.symbols.name(target);
            let evidence = belief.upgrade_checkpoints.and_then(|rows| rows.get(id))
                .map(Vec::as_slice).unwrap_or_default();
            let hypotheses = observation.resolution_hand_hypotheses(id, evidence);
            let Some(hypotheses) = (match hypotheses {
                Ok(hands) => hands,
                Err(EngineError::Unsupported(_) | EngineError::InvalidInput(_)) => None,
                Err(error) => return Err(error),
            }) else {
                return worker.with_rollback(|branch| {
                    self.advance(branch)?;
                    continue_search(self, branch)
                });
            };
            let mut sum = 0i128;
            let mut weight_total = 0i128;
            let mut resolved = true;
            for (hand, weight) in hypotheses {
                if !self.reserve_node() {
                    return Ok(None);
                }
                let value = worker.with_hidden_hand_hypothesis(target, &hand, |branch| {
                    self.advance(branch)?;
                    continue_search(self, branch)
                })?;
                let Some(value) = value else { return Ok(None) };
                sum += i128::from(value.score) * i128::from(weight);
                weight_total += i128::from(weight);
                resolved &= value.resolved;
            }
            Ok(Some(ValueBound { score: (sum / weight_total) as i64, resolved }))
        })();
        self.hand_belief = Some(belief);
        result
    }

    fn rollout(
        &mut self,
        worker: &mut NativeSession,
        cutoff: i128,
        capture_state: bool,
        lab_evaluation: bool,
    ) -> Result<Option<GameTurnRollout>> {
        let history = self.future_planning.as_ref().map(|rollout| vec![rollout.root.clone()]);
        self.rollout_with_history(worker, cutoff, capture_state, lab_evaluation, history)
    }

    fn rollout_with_history(
        &mut self,
        worker: &mut NativeSession,
        cutoff: i128,
        capture_state: bool,
        lab_evaluation: bool,
        mut history: Option<Vec<Vec<PlanningCommitment>>>,
    ) -> Result<Option<GameTurnRollout>> {
        loop {
            self.budget.check(0)?;
            if worker.search_turn_index() < cutoff
                && let Some(target) = self.pending_hand_branch(worker)?
            {
                let mut witness = None;
                let expected = self.weighted_hand_response(worker, target, |search, branch| {
                    let Some(leaf) = search.rollout_with_history(
                        branch, cutoff, capture_state, lab_evaluation, history.clone(),
                    )? else { return Ok(None) };
                    let value = ValueBound { score: leaf.score, resolved: leaf.reached_cutoff(cutoff) };
                    // Preserve an early endpoint if any positive-mass branch
                    // could not reach the requested horizon.
                    if witness.is_none() || !value.resolved {
                        witness = Some(leaf);
                    }
                    Ok(Some(value))
                })?;
                return Ok(expected.map(|value| {
                    let mut leaf = witness.expect("scored hand branch has an endpoint");
                    leaf.score = value.score;
                    leaf.nodes = self.nodes;
                    leaf
                }));
            }
            if worker.planning.phase == Phase::GameOver
                || worker.search_turn_index() >= cutoff
                || (self.guard_hidden && worker.last_request.is_none()
                    && (worker.planning.phase == Phase::LevelUp
                        || (worker.planning.phase != Phase::Planning && !worker.stack.is_empty())))
            {
                return Ok(Some(GameTurnRollout {
                    score: self.score(worker)?,
                    nodes: self.nodes,
                    round: worker.round.round,
                    turn: worker.world.combat.turn,
                    phase: worker.planning.phase,
                    state: capture_state.then(|| worker.semantic_state()).transpose()?,
                    lab_value: lab_evaluation
                        .then(|| worker.evaluate_lab_attack_range(self.player))
                        .transpose()?,
                }));
            }
            if self.nodes == self.max_nodes {
                return Ok(None);
            }
            if worker.planning.phase == Phase::Planning {
                if self.future_planning.as_ref().is_some_and(|rollout| rollout.expand)
                    && let Some(history) = history.as_ref().filter(|history| history.len() == 1)
                {
                    return self.expand_rollout_planning(worker, cutoff, capture_state, lab_evaluation, history);
                }
                if !self.reserve_node() {
                    return Ok(None);
                }
                if let Some(sample) = self.planning_sample {
                    let epoch_seed = sample.seed.wrapping_add(worker.search_turn_index() as u64);
                    return worker.with_sampled_planning_checkpoints(
                        self.player,
                        epoch_seed,
                        sample.upgrade_checkpoints,
                        |sampled| {
                            self.rollout_planning(sampled, None)?;
                            self.rollout(sampled, cutoff, capture_state, lab_evaluation)
                        },
                    );
                }
                self.rollout_planning(worker, history.as_mut())?;
                continue;
            }
            let (owner, team) = worker.search_player()?.ok_or_else(|| {
                EngineError::Unsupported("rollout stalled before its turn cutoff".into())
            })?;
            let controlled = self.controlled.map_or(team == self.team, |ids| ids.contains(&owner));
            let decision = match self.future_planning.as_ref().and_then(|rollout| rollout.fixed_opponent_seed) {
                _ if self.allow_simulated_upgrades && worker.planning.phase == Phase::LevelUp => {
                    worker.greedy_opponent_input(owner, opponent_draw_seed(worker, 0, owner))?
                }
                Some(seed) if !controlled => {
                    worker.greedy_opponent_input(owner, opponent_draw_seed(worker, seed, owner))?
                }
                _ => worker.first_resolution_choice(owner)?,
            }.ok_or_else(|| {
                EngineError::Unsupported("rollout decision has no native choices".into())
            })?;
            if !self.apply_input(worker, decision)? {
                return Ok(None);
            }
        }
    }

    fn rollout_planning(
        &mut self,
        worker: &mut NativeSession,
        history: Option<&mut Vec<Vec<PlanningCommitment>>>,
    ) -> Result<()> {
        let commitments = self.rollout_commitments(worker, history.as_deref().map(Vec::as_slice))?;
        worker.apply_joint_planning_ordered(&commitments)?;
        if let Some(history) = history {
            history.push(commitments);
        }
        self.advance(worker)?;
        Ok(())
    }

    fn forecast_card(
        &self,
        hero: Symbol,
        history: &[Vec<PlanningCommitment>],
        legal: &[Option<crate::cards::CardId>],
    ) -> Option<Option<crate::cards::CardId>> {
        let (_, forecast) = self.future_planning.as_ref()?.forecasts.iter()
            .find(|(id, _)| *id == hero)?;
        if history.len() >= 4 {
            return None;
        }
        let mut prefix = [None; 3];
        for (slot, batch) in prefix.iter_mut().zip(history) {
            *slot = batch.iter().find(|entry| entry.hero == hero).copied();
        }
        forecast.preferred_next_card(&prefix[..history.len()], legal)
    }

    fn rollout_commitments(
        &mut self,
        worker: &NativeSession,
        history: Option<&[Vec<PlanningCommitment>]>,
    ) -> Result<Vec<PlanningCommitment>> {
        let mut commitments = Vec::new();
        for hero in &worker.world.heroes {
            let Some(fallback) = worker.first_planning_commitment(hero.id)? else {
                continue;
            };
            let controlled = self.controlled.map_or(hero.team == self.team, |ids| ids.contains(&hero.id));
            let mut choice = fallback;
            if !controlled
                && let Some(rollout) = &self.future_planning
                && let Some(seed) = rollout.fixed_opponent_seed
            {
                self.forecast_checks += 1;
                if let Some(predicted) = worker.sample_forecast_commitment(
                    hero.id, rollout.forecasts, history.unwrap_or_default(), seed,
                )? {
                    self.forecast_matches += 1;
                    choice = predicted;
                } else if let Some(predicted) = worker.greedy_opponent_commitment(
                    hero.id, opponent_draw_seed(worker, seed, hero.id),
                )? {
                    choice = predicted;
                }
            } else if !controlled
                && let Some(history) = history
                && let Some(rollout) = &self.future_planning
                && rollout.forecasts.iter().any(|(id, _)| *id == hero.id)
            {
                let legal: Vec<_> = worker.planning_choices(hero.id)?.into_iter()
                    .map(|option| match option {
                        DecisionMove::Card { card, .. } => Some(card),
                        _ => None,
                    })
                    .collect();
                self.forecast_checks += 1;
                if let Some(card) = self.forecast_card(hero.id, history, &legal) {
                    self.forecast_matches += 1;
                    choice.first = card;
                }
            }
            self.forecast_changes += usize::from(choice != fallback);
            commitments.push(choice);
        }
        Ok(commitments)
    }

    fn expand_rollout_planning(
        &mut self,
        worker: &mut NativeSession,
        cutoff: i128,
        capture_state: bool,
        lab_evaluation: bool,
        history: &[Vec<PlanningCommitment>],
    ) -> Result<Option<GameTurnRollout>> {
        let preferred = self.rollout_commitments(worker, Some(history))?;
        let fixed_opponents = self.future_planning.as_ref()
            .filter(|rollout| rollout.fixed_opponent_seed.is_some());
        let (rows, reply_alternative) = if fixed_opponents
            .is_some_and(|rollout| !rollout.legacy_fixed_followup)
        {
            // Spend the wider follow-up on legal controlled commitments.
            // Keep opponent choices and joint submission order fixed.
            let controlled = worker.joint_plans_with_reply_order(
                self.player, true, 4, self.controlled, JointPlanOrder::Balanced, &[],
            )?;
            (controlled.plans.into_iter().map(|row| {
                preferred.iter().map(|choice| {
                    row.iter().find(|candidate| candidate.hero == choice.hero)
                        .copied().unwrap_or(*choice)
                }).collect()
            }).collect::<Vec<Vec<PlanningCommitment>>>(), None)
        } else {
            self.narrow_rollout_rows(worker, history, preferred, fixed_opponents.is_some())?
        };

        let mut best = None::<GameTurnRollout>;
        'rows: for row in rows {
            let mut worst = None::<GameTurnRollout>;
            for reply in [None, reply_alternative].into_iter().take(1 + usize::from(reply_alternative.is_some())) {
                if !self.reserve_node() {
                    return Ok(None);
                }
                let mut joint = row.clone();
                if let Some((index, choice)) = reply {
                    joint[index] = choice;
                }
                let mut branch_history = history.to_vec();
                branch_history.push(joint.clone());
                let leaf = worker.with_joint_planning(&joint, |branch| {
                    self.advance(branch)?;
                    self.rollout_with_history(
                        branch, cutoff, capture_state, lab_evaluation, Some(branch_history),
                    )
                });
                let leaf = match leaf {
                    Ok(Some(leaf)) => leaf,
                    Ok(None) | Err(EngineError::Deadline | EngineError::Cancelled) => return Ok(None),
                    Err(error) => return Err(error),
                };
                // The row's worst reply can only lower its score further.
                if best.as_ref().is_some_and(|prior| leaf.score <= prior.score) {
                    continue 'rows;
                }
                if worst.as_ref().is_none_or(|prior| leaf.score < prior.score) {
                    worst = Some(leaf);
                }
            }
            if let Some(worst) = worst
                && best.as_ref().is_none_or(|prior| worst.score > prior.score)
            {
                best = Some(worst);
            }
        }
        Ok(best)
    }

    fn narrow_rollout_rows(
        &self,
        worker: &NativeSession,
        history: &[Vec<PlanningCommitment>],
        preferred: Vec<PlanningCommitment>,
        fixed_opponents: bool,
    ) -> Result<(Vec<Vec<PlanningCommitment>>, Option<(usize, PlanningCommitment)>)> {
        let mut row_alternative = None;
        let mut reply_alternative = None;
        for (index, choice) in preferred.iter().enumerate() {
            let team = worker.world.hero(choice.hero).unwrap().team;
            let controlled = self.controlled.map_or(team == self.team, |ids| ids.contains(&choice.hero));
            if !controlled && (fixed_opponents || matches!(self.opponent_policy, OpponentPolicy::FirstLegal)) {
                continue;
            }
            let slot = if controlled { &mut row_alternative } else { &mut reply_alternative };
            if slot.is_some() {
                continue;
            }
            let alternatives: Vec<_> = worker.hero_planning_commitments(choice.hero)?
                .into_iter()
                .filter(|candidate| candidate.first != choice.first && candidate.second.is_none())
                .collect();
            if let Some(fallback) = alternatives.first() {
                let cards: Vec<_> = alternatives.iter().map(|candidate| candidate.first).collect();
                let first = self.forecast_card(choice.hero, history, &cards).unwrap_or(fallback.first);
                *slot = Some((index, PlanningCommitment { first, ..*fallback }));
            }
        }

        let mut rows = vec![preferred];
        if let Some((index, choice)) = row_alternative {
            let mut alternative = rows[0].clone();
            alternative[index] = choice;
            rows.push(alternative);
        }
        Ok((rows, reply_alternative))
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
            matches!(choice, DecisionMove::Input { selection: super::Datum::Object(_), .. })
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
        Ok(Some(ValueBound { score: best, resolved: false }))
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
            let controlled = self.controlled.map_or(team == self.team, |ids| ids.contains(&player));
            if !controlled
                && let Some(seed) = self.future_planning.as_ref().and_then(|rollout| rollout.fixed_opponent_seed)
            {
                let decision = worker.greedy_opponent_input(player, opponent_draw_seed(worker, seed, player))?
                    .ok_or_else(|| EngineError::Unsupported("fixed opponent has no native choices".into()))?;
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
                && matches!(self.opponent_policy, OpponentPolicy::AdversarialMovementTargets)
                && let Some(cutoff) = self.turn_cutoff
                && worker.last_request.as_ref().is_some_and(|request| {
                    matches!(&request.choices, RequestChoices::Hexes(hexes)
                        if hexes.len() + usize::from(request.can_skip) > 1)
                })
                && worker.movement_root_actor()
                    .is_some_and(|actor| worker.world.hero_identity(actor) == player)
            {
                return self.movement_leaf(worker, player, cutoff, beta);
            }
            let extend_attack = depth == 0
                && !fixed_policy
                && matches!(self.opponent_policy, OpponentPolicy::AdversarialAttackTargets)
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
mod tests {
    use super::*;
    use crate::interpreter::{NativeRequest, RequestToken};

    #[test]
    fn sampled_reply_cap_prefers_greedy_commitments_and_full_search_keeps_every_reply() {
        use std::collections::HashSet;

        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let root = NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
        let player = root.world.heroes[0].id;
        let seed = 39;
        let sampled = root.sampled_planning_world(player, seed).unwrap();
        let first = sampled.joint_plans_with_reply_order(player, false, 1, None, JointPlanOrder::OpponentReplies(seed), &[])
            .unwrap();
        let reply = &first.plans[0];
        assert!(!reply.is_empty());
        for choice in reply {
            assert_eq!(Some(*choice), sampled.greedy_opponent_commitment(choice.hero, seed).unwrap());
        }
        let ordinary = sampled.joint_plans(player, false, 4096).unwrap();
        let ranked = sampled.joint_plans_with_reply_order(player, false, 4096, None, JointPlanOrder::OpponentReplies(seed), &[])
            .unwrap();
        assert_eq!(ranked.total, ordinary.total);
        assert_eq!(ranked.plans.len(), ranked.total);
        assert_eq!(ranked.plans.into_iter().collect::<HashSet<_>>(),
            ordinary.plans.into_iter().collect::<HashSet<_>>());
    }

    #[test]
    fn joint_card_order_uses_score_gaps_and_keeps_unscored_cards() {
        let choice = PlanningCommitment { hero: Symbol(0), first: None, second: None };
        let dimensions = [vec![choice; 2], vec![choice; 3]];
        let costs = [
            Some(vec![greedy_card_cost(0, Some(0)), greedy_card_cost(0, Some(-150))]),
            Some(vec![
                greedy_card_cost(0, Some(0)),
                greedy_card_cost(0, Some(-30)),
                greedy_card_cost(0, None),
            ]),
        ];
        assert_eq!(ranked_reply_indices(&dimensions, &[1, 2], &costs, 6), [0, 2, 1, 3, 4, 5]);
    }

    #[test]
    fn shared_rows_average_likelihoods_across_worlds() {
        use crate::cards::CardId;

        let hero = Symbol(0);
        let a = Some(CardId(1));
        let b = Some(CardId(2));
        let rows = [a, b].map(|first| vec![PlanningCommitment { hero, first, second: None }]);
        let first = shared_row_likelihoods(&rows, &[(hero, vec![(a, 0), (b, -300)])]);
        let second = shared_row_likelihoods(&rows, &[(hero, vec![(a, -100), (b, 0)])]);
        assert!(first[0] > first[1] && second[0] < second[1]);
        assert!(first[0] + second[0] > first[1] + second[1]);
        assert_eq!(shared_row_likelihoods(&rows, &[]), [0.5, 0.5]);
    }

    #[test]
    fn four_turn_greedy_scores_order_both_sides_of_sampled_planning() {
        std::thread::Builder::new()
            .stack_size(SEARCH_WORKER_STACK)
            .spawn(|| {
                use std::collections::HashSet;
                let fixture = super::super::object_execution::fixture_case(
                    "object-execution-card-planning-cycle",
                );
                let root = NativeSession::compile(
                    serde_json::from_value(fixture["envelope"].clone()).unwrap(),
                ).unwrap();
                let player = root.world.heroes[0].id;
                let seed = 39;
                let mut world = root.sampled_planning_world(player, seed).unwrap();
                let before = world.semantic_state().unwrap();
                let (scores, nodes, _) = world.greedy_planning_scores(
                    seed, 1024, false, &ExecutionBudget::default(),
                ).unwrap();
                assert!(nodes > 0 && nodes <= 1024);
                assert_eq!(scores.len(), world.world.heroes.len());
                assert_eq!(world.semantic_state().unwrap(), before);
                for friendly in [true, false] {
                    let ordinary = world.joint_plans(player, friendly, 4096).unwrap();
                    let ranked = world.joint_plans_with_reply_order(
                        player, friendly, 4096, None, JointPlanOrder::PlayerFirst, &scores,
                    ).unwrap();
                    for commitment in &ranked.plans[0] {
                        let (_, row) = scores.iter()
                            .find(|(hero, _)| *hero == commitment.hero).unwrap();
                        assert!(row.iter().any(|(card, score)| {
                            *card == commitment.first && Some(*score) == row.iter()
                                .map(|(_, score)| *score).max()
                        }));
                    }
                    assert_eq!(ranked.plans.into_iter().collect::<HashSet<_>>(),
                        ordinary.plans.into_iter().collect::<HashSet<_>>());
                }
                let mut outcomes = Vec::new();
                for workers in [1, 2] {
                    let result = world.search_plans_on_sample(
                        player, seed, 2,
                        SearchLimits {
                            max_depth: 1,
                            max_nodes: 4096,
                            opponent_policy: OpponentPolicy::Adversarial,
                            evaluation: SearchEvaluation::MaterialPosition,
                        },
                        PlanningSearchOptions {
                            rollout_turns: Some(2),
                            workers,
                            upgrade_checkpoints: None,
                            reply_cap: None,
                            reply_probe_cap: None,
                            greedy_card_order: true,
                            tactical_forecast: false,
                            forecast_node_share: ForecastNodeShare::default(),
                            legacy_fixed_followup: false,
                            fixed_future_planning: false,
                            branch_future_planning: false,
                        },
                        &ExecutionBudget::default(),
                    ).unwrap();
                    assert!(result.matrix.score.is_some());
                    assert!(result.matrix.forecast_checks > 0);
                    assert!(result.matrix.nodes <= 4096);
                    assert_eq!(world.semantic_state().unwrap(), before);
                    outcomes.push((result.commitment, result.matrix.score));
                }
                assert_eq!(outcomes[0], outcomes[1]);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn forecast_search_uses_one_repeatable_opponent_reply() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut world = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        let player = world.world.heroes[0].id;
        let budget = ExecutionBudget::default();
        let before = world.semantic_state().unwrap();
        let options = PlanningSearchOptions {
            rollout_turns: Some(2),
            workers: 1,
            upgrade_checkpoints: None,
            reply_cap: Some(4),
            reply_probe_cap: None,
            greedy_card_order: true,
            tactical_forecast: false,
            forecast_node_share: ForecastNodeShare::default(),
            legacy_fixed_followup: false,
            fixed_future_planning: false,
            branch_future_planning: false,
        };
        let limits = SearchLimits {
            max_depth: 1,
            max_nodes: 4096,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
        };
        let first = world.search_plans_on_sample(player, 17, 3, limits, options, &budget).unwrap();
        let second = world.search_plans_on_sample(player, 17, 3, limits, options, &budget).unwrap();
        assert_eq!(first.total_opposing_plans, 1);
        assert_eq!(first.matrix.total_cells, 3);
        assert!(first.matrix.score.is_some());
        assert!(first.matrix.forecast_checks > 0);
        assert_eq!(first.commitment, second.commitment);
        assert_eq!(first.matrix.score, second.matrix.score);
        let legacy = world.search_plans_on_sample(
            player, 17, 3, limits,
            PlanningSearchOptions { legacy_fixed_followup: true, ..options },
            &budget,
        ).unwrap();
        assert_eq!(legacy.total_opposing_plans, 1);
        assert!(legacy.matrix.score.is_some());
        assert!(legacy.matrix.forecast_checks > 0);
        assert_eq!(world.semantic_state().unwrap(), before);
    }

    #[test]
    fn later_planning_expansion_matches_fixed_rollout_matrix_and_requires_an_exact_value() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut world = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        let player = world.world.heroes[0].id;
        let team = world.world.heroes[0].team;
        let budget = ExecutionBudget::default();
        let before = world.semantic_state().unwrap();

        let preferred: Vec<_> = world.world.heroes.iter()
            .filter_map(|hero| world.first_planning_commitment(hero.id).unwrap())
            .collect();
        let alternative = |friendly| {
            let (index, choice) = preferred.iter().enumerate()
                .find(|(_, choice)| (world.world.hero(choice.hero).unwrap().team == team) == friendly)
                .unwrap();
            let next = world.hero_planning_commitments(choice.hero).unwrap().into_iter()
                .find(|next| next.first != choice.first).unwrap();
            (index, next)
        };
        let row = alternative(true);
        let reply = alternative(false);
        let mut scores = [[0; 2]; 2];
        let mut costs = [[0; 2]; 2];
        for (i, row) in [None, Some(row)].into_iter().enumerate() {
            for (j, reply) in [None, Some(reply)].into_iter().enumerate() {
                let mut joint = preferred.clone();
                for (index, choice) in row.into_iter().chain(reply) {
                    joint[index] = choice;
                }
                let leaf = world.with_joint_planning(&joint, |branch| {
                    branch.rollout_game_turns(player, 1, 4096, &budget)
                }).unwrap().unwrap();
                scores[i][j] = leaf.score;
                costs[i][j] = leaf.nodes + 1;
            }
        }
        let first_row = scores[0][0].min(scores[0][1]);
        let expected = first_row.max(scores[1][0].min(scores[1][1]));
        for (limit, score) in [
            (costs[0][0], None),
            (costs[0].iter().sum(), None),
            (costs[0].iter().sum::<usize>() + 1, None),
            (costs.iter().flatten().sum(), Some(expected)),
        ] {
            let mut search = Search {
                player,
                team,
                budget: &budget,
                nodes: 0,
                max_nodes: limit,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
                turn_cutoff: None,
                controlled: None,
                shared_nodes: None,
                planning_sample: None,
                guard_hidden: false,
                allow_simulated_upgrades: false,
                hand_belief: None,
                forecast_changes: 0,
                forecast_checks: 0,
                forecast_matches: 0,
                future_planning: None,
            };
            let cutoff = world.search_turn_index() + 1;
            let result = search.expand_rollout_planning(&mut world, cutoff, false, false, &[vec![]])
                .unwrap();
            assert_eq!(result.map(|leaf| leaf.score), score);
            assert!(search.nodes <= limit);
            if score.is_none() {
                assert_eq!(search.nodes, limit);
            }
            assert_eq!(world.semantic_state().unwrap(), before);
        }
        let (row, replies): (Vec<_>, Vec<_>) = preferred.into_iter()
            .partition(|choice| world.world.hero(choice.hero).unwrap().team == team);
        let mut costs = Vec::new();
        for expand in [false, true] {
            let result = world.search_planning_matrix_partition(
                player,
                std::slice::from_ref(&row),
                std::slice::from_ref(&replies),
                SearchLimits {
                    max_depth: 0,
                    max_nodes: 4096,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: SearchEvaluation::MaterialPosition,
                },
                PlanningPartition {
                    rollout_turns: Some(2),
                    controlled: None,
                    shared_nodes: None,
                    greedy_forecasts: &[],
                    future_branching: Some(expand),
                    legacy_fixed_followup: false,
                    fixed_opponent_seed: None,
                },
                &budget,
            ).unwrap();
            assert!(result.score.is_some());
            assert_eq!(result.forecast_checks, 0);
            assert_eq!(world.semantic_state().unwrap(), before);
            costs.push(result.nodes);
        }
        assert!(costs[1] > costs[0]);
    }

    #[test]
    fn resolution_probe_distinguishes_duplicate_prefixes_from_forced_inputs() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = worker.world.heroes[0].id;
        worker.last_request = Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(1),
            owner: player,
            prompt: player,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Numbers {
                values: vec![1, 1, 2],
                labels: Default::default(),
            },
            can_skip: false,
            discard: false,
        });
        let full = worker.resolution_choices(player).unwrap();
        assert_eq!(full.len(), 2);
        assert_eq!(worker.resolution_branch_probe(player).unwrap(), full);
        worker.last_request.as_mut().unwrap().choices = RequestChoices::ConfirmAction;
        assert_eq!(worker.resolution_branch_probe(player).unwrap().len(), 1);
    }

    #[test]
    fn lab_search_evaluation_matches_fixed_point_reference() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = worker.world.heroes[0].id;
        let team = worker.world.heroes[0].team;
        worker.world.heroes[0].gold += 7;
        let budget = ExecutionBudget::default();
        let search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 1,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::LabAttackRangeV1,
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
        let expected =
            (worker.evaluate_lab_attack_range(player).unwrap() * (WIN - 1) as f64).round() as i64;
        assert_eq!(search.score(&worker).unwrap(), expected);
        assert_ne!(expected, worker.evaluate_resolution(team, player));
    }

    #[test]
    fn alpha_beta_matches_exhaustive_live_attack_and_defense() {
        fn exhaustive(
            session: &mut NativeSession,
            player: Symbol,
            team: Symbol,
            depth: u8,
            branches: &mut [usize; 2],
            budget: &ExecutionBudget<'_>,
        ) -> Result<i64> {
            if matches!(session.planning.phase, Phase::Planning | Phase::GameOver) {
                return Ok(session.evaluate_resolution(team, player));
            }
            let Some((owner, owner_team)) = session.search_player()? else {
                return Ok(session.evaluate_resolution(team, player));
            };
            let moves = session.resolution_choices(owner)?;
            assert!(!moves.is_empty());
            let branching = moves.len() > 1;
            if branching && depth == 0 {
                return Ok(session.evaluate_resolution(team, player));
            }
            let maximizing = team == owner_team;
            if branching {
                branches[usize::from(!maximizing)] += 1;
            }
            let mut scores = Vec::new();
            for choice in moves {
                let DecisionMove::Input {
                    request: Some(token),
                    selection,
                    ..
                } = choice else {
                    panic!("expected a live resolution input");
                };
                scores.push(session.with_rollback(|child| {
                    child.submit(token, selection)?;
                    child.last_request = None;
                    child.advance_to_decision(budget)?;
                    exhaustive(child, player, team, depth - u8::from(branching), branches, budget)
                })?);
            }
            Ok(if maximizing {
                scores.into_iter().max().unwrap()
            } else {
                scores.into_iter().min().unwrap()
            })
        }

        let budget = ExecutionBudget::default();
        let mut branches = [0; 2];
        for name in [
            "object-execution-attack-macro-optional-first",
            "object-execution-attack-macro-existing",
        ] {
            let fixture = super::super::object_execution::fixture_case(name);
            let mut session =
                NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                    .unwrap();
            session.advance_to_decision(&budget).unwrap();
            let player = session.world.heroes[0].id;
            let team = session.world.heroes[0].team;
            let state = session.semantic_state().unwrap();
            let request = session.last_request.clone();
            for depth in 0..=3 {
                let expected = exhaustive(&mut session, player, team, depth, &mut branches, &budget)
                    .unwrap();
                let mut search = Search {
                    player,
                    team,
                    budget: &budget,
                    nodes: 0,
                    max_nodes: 2_048,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: SearchEvaluation::MaterialPosition,
                    turn_cutoff: None,
                    controlled: None,
                    shared_nodes: None,
                    guard_hidden: false,
                    allow_simulated_upgrades: false,
                    hand_belief: None,
                    planning_sample: None,
                    forecast_changes: 0,
                    forecast_checks: 0,
                    forecast_matches: 0,
                    future_planning: None,
                };
                let actual = session
                    .with_rollback(|branch| search.value(branch, depth, i64::MIN, i64::MAX))
                    .unwrap()
                    .expect("small fixture must fit the node budget");
                assert_eq!(actual.score, expected, "{name}, depth {depth}");
                assert_eq!(session.semantic_state().unwrap(), state);
                assert_eq!(session.last_request, request);
            }
        }
        assert!(branches.into_iter().all(|count| count > 0), "{branches:?}");
    }

    #[test]
    fn exact_terminal_root_score_can_be_reused_without_expanding_nodes() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-confirmation-repeated-step",
        );
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let budget = ExecutionBudget {
            cancelled: Some(&cancelled),
            ..ExecutionBudget::default()
        };
        let player = worker.advance(&budget).unwrap().request.unwrap().owner;
        let team = worker.world.hero(player).unwrap().team;
        let moves = worker.resolution_choices(player).unwrap();
        assert_eq!(moves.len(), 1);
        let mut search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 1,
            opponent_policy: OpponentPolicy::Adversarial,
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
        let mut cache = [None];
        let shallow = search
            .choose(&mut worker, &moves, 1, 0, &mut cache)
            .unwrap();
        assert!(shallow.interrupted == Some(ResolutionStop::NodeLimit) && !shallow.resolved);
        assert_eq!(cache, [None]);
        search.nodes = 0;
        search.max_nodes = 2;
        let solved = search
            .choose(&mut worker, &moves, 1, 0, &mut cache)
            .unwrap();
        assert!(solved.resolved);
        assert_eq!(search.nodes, 2);
        let reused = search
            .choose(&mut worker, &moves, 3, 0, &mut cache)
            .unwrap();
        assert!(reused.resolved);
        assert_eq!(reused.best, solved.best);
        assert_eq!(search.nodes, 2);

        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        let interrupted = search
            .choose(&mut worker, &moves, 3, 0, &mut cache)
            .unwrap();
        assert_eq!(interrupted.interrupted, Some(ResolutionStop::Cancelled));
        assert!(interrupted.best.is_none());
        assert!(!interrupted.resolved);
        assert_eq!(search.nodes, 2);

        let expired = ExecutionBudget {
            deadline: Some(std::time::Instant::now()),
            ..ExecutionBudget::default()
        };
        search.budget = &expired;
        let interrupted = search
            .choose(&mut worker, &moves, 3, 0, &mut cache)
            .unwrap();
        assert_eq!(interrupted.interrupted, Some(ResolutionStop::Deadline));
        assert!(interrupted.best.is_none());
        assert!(!interrupted.resolved);
        assert_eq!(search.nodes, 2);
    }

    #[test]
    fn fixed_rollout_crosses_planning_and_restores_the_root_on_budget_exhaustion() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = worker.world.heroes[0].id;
        let before = worker.semantic_state().unwrap();
        let budget = ExecutionBudget::default();
        assert_eq!(
            worker.rollout_game_turns(player, 2, 1, &budget).unwrap(),
            None
        );
        assert_eq!(worker.semantic_state().unwrap(), before);
        let one = worker
            .rollout_game_turns(player, 1, 512, &budget)
            .unwrap()
            .unwrap();
        let two = worker
            .rollout_game_turns(player, 2, 512, &budget)
            .unwrap()
            .unwrap();
        let origin = worker.search_turn_index();
        assert_eq!(i128::from(one.round) * 4 + i128::from(one.turn), origin + 1);
        assert_eq!(i128::from(two.round) * 4 + i128::from(two.turn), origin + 2);
        assert!(two.nodes > one.nodes);
        assert_eq!(worker.semantic_state().unwrap(), before);
    }

    #[test]
    fn movement_root_order_explores_approaches_and_retreats() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-live-movement-sequence-base-plain-normal",
        );
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        root.advance_prefix(1).unwrap();
        root.advance_prefix(1).unwrap();
        let (owner, team) = root.search_player().unwrap().unwrap();
        let original = root.resolution_choices(owner).unwrap();
        assert!(original.len() >= 3);
        let mut ordered = original.clone();
        root.order_movement_roots(owner, &mut ordered);
        assert_eq!(ordered.len(), original.len());
        assert!(original.iter().all(|choice| ordered.contains(choice)));
        let distance = |choice: &DecisionMove, kinds: &[EntityKind]| {
            let DecisionMove::Input { selection, .. } = choice else { return None };
            let hex = super::super::datum_hex(selection, &root.symbols, root.coordinates).ok()?;
            root.world.locations.iter().filter_map(|&(id, enemy)| {
                let entity = root.world.entity(id)?;
                (entity.team.is_some_and(|owner| owner != team) && kinds.contains(&entity.kind))
                    .then_some(hex.distance(enemy))
            }).min()
        };
        for kinds in [&[EntityKind::Minion][..], &[EntityKind::Hero, EntityKind::Piece][..]] {
            let closest = original.iter().filter_map(|choice| distance(choice, kinds)).min();
            assert!(closest.is_some());
            assert!(ordered[..3].iter().any(|choice| distance(choice, kinds) == closest));
        }
        let threats = [EntityKind::Minion, EntityKind::Hero, EntityKind::Piece];
        let farthest = original.iter().filter_map(|choice| distance(choice, &threats)).max();
        assert!(farthest.is_some());
        assert!(ordered[..3].iter().any(|choice| distance(choice, &threats) == farthest));
    }

    #[test]
    fn movement_extension_retains_first_legal_and_charges_each_probe() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        let plan: Vec<_> = root.world.heroes.iter().map(|hero| PlanningCommitment {
            hero: hero.id,
            first: Some(hero.hand[1]),
            second: None,
        }).collect();
        let budget = ExecutionBudget::default();
        root.with_joint_planning(&plan, |worker| {
            worker.advance_to_decision(&budget)?;
            let (owner, team) = worker.search_player()?.unwrap();
            let choices = worker.resolution_choices(owner)?;
            let (token, action) = choices.into_iter().find_map(|choice| match choice {
                DecisionMove::Input { request: Some(token), selection: super::super::Datum::Text(id), .. }
                    if worker.symbols.name(id) == "FAST_TRAVEL" => Some((token, id)),
                _ => None,
            }).expect("opening actor can Fast Travel");
            worker.submit(token, super::super::Datum::Text(action))?;
            worker.last_request = None;
            worker.advance_to_decision(&budget)?;
            assert!(worker.movement_root_actor().is_some());
            assert!(worker.resolution_choices(owner)?.len() > 5);
            let before = worker.semantic_state()?;
            let request = worker.last_request.clone();
            let mut search = Search {
                player: owner,
                team,
                budget: &budget,
                nodes: 0,
                max_nodes: 1024,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
                turn_cutoff: worker.search_turn_cutoff(Some(1))?,
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
            let reference = worker.with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))?
                .unwrap();
            let reference_nodes = search.nodes;
            search.opponent_policy = OpponentPolicy::AdversarialMovementTargets;
            search.nodes = 0;
            let extended = worker.with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))?
                .unwrap();
            assert!(extended.score >= reference.score);
            assert!(search.nodes > reference_nodes);
            assert!(!extended.resolved);

            search.nodes = 0;
            search.max_nodes = 1;
            assert!(worker.with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))?
                .is_none());
            assert_eq!(search.nodes, 1);
            assert_eq!(worker.semantic_state()?, before);
            assert_eq!(worker.last_request, request);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn mandatory_upgrade_search_retains_a_scored_legal_choice_at_the_node_limit() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-cleanup-live-level-gold-3",
        );
        let mut session = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        )
        .unwrap();
        session.advance_prefix(1).unwrap();
        session.advance_prefix(1).unwrap().request.unwrap();
        let (player, _) = session.search_player().unwrap().unwrap();
        let legal = session.resolution_choices(player).unwrap();
        assert!(legal.len() > 1);
        let before = session.semantic_state().unwrap();
        let result = session
            .search_resolution(
                player,
                SearchLimits {
                    max_depth: 1,
                    max_nodes: 1,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: SearchEvaluation::MaterialPosition,
                },
                &ExecutionBudget::default(),
            )
            .unwrap();
        assert_eq!(result.stop, ResolutionStop::NodeLimit);
        assert_eq!(result.nodes, 1);
        assert_eq!(result.score_kind, Some(ResolutionScoreKind::Immediate));
        assert!(legal.contains(&result.decision));
        assert_eq!(session.semantic_state().unwrap(), before);
    }

    #[test]
    fn resolution_search_scores_root_moves_at_the_game_turn_cutoff() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = worker.world.heroes[0].id;
        let mut plan = worker.joint_plans(player, true, 1).unwrap().plans.remove(0);
        plan.extend(
            worker
                .joint_plans(player, false, 1)
                .unwrap()
                .plans
                .remove(0),
        );
        let budget = ExecutionBudget::default();
        worker
            .with_joint_planning(&plan, |worker| {
                worker.advance_to_decision(&budget)?;
                let (owner, _) = worker.search_player()?.unwrap();
                let choices = worker.resolution_choices(owner)?;
                let before = worker.semantic_state()?;
                let limits = SearchLimits {
                    max_depth: 1,
                    max_nodes: 4096,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: SearchEvaluation::MaterialPosition,
                };
                let ordinary = worker.search_resolution(owner, limits, &budget)?;
                let sampled_root = worker.search_resolution_in_sampled_world(
                    owner,
                    PlanningRolloutSample {
                        seed: 39,
                        upgrade_checkpoints: None,
                    },
                    limits,
                    &budget,
                )?;
                assert!(choices.contains(&sampled_root.decision));
                assert!(sampled_root.score.is_some());
                assert_eq!(worker.semantic_state()?, before);
                let tiny = SearchLimits {
                    max_nodes: 1,
                    ..limits
                };
                let fallback =
                    worker.search_resolution_with_turn_rollout(owner, tiny, Some(2), &budget)?;
                assert!(fallback.score.is_some());
                assert_eq!(fallback.score_kind, Some(ResolutionScoreKind::Immediate));
                assert_eq!((fallback.completed_depth, fallback.nodes), (0, 1));
                assert_eq!(fallback.stop, ResolutionStop::NodeLimit);
                let cancelled = std::sync::atomic::AtomicBool::new(true);
                for (blocked, reason) in [
                    (
                        ExecutionBudget {
                            deadline: Some(std::time::Instant::now()),
                            ..budget
                        },
                        ResolutionStop::Deadline,
                    ),
                    (
                        ExecutionBudget {
                            cancelled: Some(&cancelled),
                            ..budget
                        },
                        ResolutionStop::Cancelled,
                    ),
                ] {
                    let result = worker.search_resolution(owner, limits, &blocked)?;
                    assert_eq!(result.stop, reason);
                    assert_eq!(result.score, None);
                    assert_eq!(result.nodes, 0);
                    assert!(choices.contains(&result.decision));
                }
                assert!(choices.contains(&fallback.decision));
                assert_eq!(worker.semantic_state()?, before);
                assert!(choices.len() > 1);
                let one_turn =
                    worker.search_resolution_with_turn_rollout(owner, limits, Some(1), &budget)?;
                let team = worker.world.hero(owner).unwrap().team;
                let request = worker.last_request.clone();
                let mut seed = Search {
                    player: owner,
                    team,
                    budget: &budget,
                    nodes: 0,
                    max_nodes: limits.max_nodes,
                    opponent_policy: limits.opponent_policy,
                    evaluation: limits.evaluation,
                    turn_cutoff: worker.search_turn_cutoff(Some(2))?,
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
                let mut ordered = choices.clone();
                worker.order_movement_roots(owner, &mut ordered);
                let (index, score) = seed
                    .seed_short_turn(std::slice::from_mut(worker), &ordered)?
                    .expect("one-turn pass should fit the seed budget");
                assert_eq!(ordered[index], one_turn.decision);
                assert_eq!(Some(score), one_turn.score);
                assert!(seed.nodes >= choices.len() && seed.nodes <= limits.max_nodes / 8);
                assert_eq!(worker.semantic_state()?, before);
                assert_eq!(worker.last_request, request);

                let rolled =
                    worker.search_resolution_with_turn_rollout(owner, limits, Some(2), &budget)?;
                assert!(rolled.score.is_some());
                assert_eq!(rolled.score_kind, Some(ResolutionScoreKind::TurnRollout));
                assert_eq!(rolled.completed_depth, 1);
                assert!(rolled.nodes > ordinary.nodes);
                assert!(choices.contains(&rolled.decision));
                assert_eq!(worker.semantic_state()?, before);
                for turns in [2, 3] {
                    let sampled = worker.search_resolution_with_sampled_turn_rollout(
                        owner,
                        limits,
                        Some(turns),
                        Some(PlanningRolloutSample {
                            seed: 39,
                            upgrade_checkpoints: None,
                        }),
                        &budget,
                    )?;
                    assert_eq!(sampled.score_kind, Some(ResolutionScoreKind::TurnRollout));
                    assert_eq!(sampled.completed_depth, 1);
                    assert!(choices.contains(&sampled.decision));
                    assert_eq!(worker.semantic_state()?, before);
                }
                let invalid_evidence =
                    std::collections::BTreeMap::from([("unknown".into(), vec![])]);
                let invalid = worker.search_resolution_with_sampled_turn_rollout(
                    owner,
                    limits,
                    Some(2),
                    Some(PlanningRolloutSample {
                        seed: 39,
                        upgrade_checkpoints: Some(&invalid_evidence),
                    }),
                    &budget,
                );
                assert!(matches!(invalid, Err(EngineError::InvalidInput(_))));
                let root_sample = PlanningRolloutSample {
                    seed: 39,
                    upgrade_checkpoints: None,
                };
                for workers in [1, 2] {
                    let fallback = worker.search_resolution_worlds(
                        owner,
                        (root_sample, 2, workers),
                        SearchLimits { max_nodes: 2, ..limits },
                        Some(2),
                        None,
                        &budget,
                    )?;
                    assert!(choices.contains(&fallback.decision));
                    assert!(fallback.score.is_some());
                    assert_eq!(fallback.score_kind, Some(ResolutionScoreKind::Immediate));
                    assert_eq!((fallback.completed_depth, fallback.nodes), (0, 2));
                    assert_eq!(fallback.stop, ResolutionStop::NodeLimit);
                    assert_eq!(worker.semantic_state()?, before);
                }
                let serial = worker.search_resolution_worlds(
                    owner, (root_sample, 2, 1), limits, Some(2), None, &budget,
                )?;
                let parallel = worker.search_resolution_worlds(
                    owner, (root_sample, 2, 2), limits, Some(2), None, &budget,
                )?;
                assert_eq!(serial.completed_depth, 1);
                assert_eq!(serial.score_kind, Some(ResolutionScoreKind::TurnRollout));
                assert_eq!(serial.completed_depth, parallel.completed_depth);
                assert_eq!(serial.decision, parallel.decision);
                assert_eq!(serial.score, parallel.score);
                assert_eq!(worker.semantic_state()?, before);
                let both = worker.search_sampled_resolution_with_turn_rollout(
                    owner,
                    root_sample,
                    limits,
                    Some(2),
                    Some(root_sample),
                    &budget,
                )?;
                assert_eq!(both.score_kind, Some(ResolutionScoreKind::TurnRollout));
                assert_eq!(both.completed_depth, 1);
                assert!(choices.contains(&both.decision));
                assert_eq!(worker.semantic_state()?, before);
                let lab = worker.search_resolution_with_turn_rollout(
                    owner,
                    SearchLimits {
                        evaluation: SearchEvaluation::LabAttackRangeV1,
                        ..limits
                    },
                    Some(2),
                    &budget,
                )?;
                assert!(choices.contains(&lab.decision));
                assert_eq!(lab.completed_depth, 1);
                assert_eq!(lab.score_kind, Some(ResolutionScoreKind::TurnRollout));
                assert!(lab.score.is_some());
                assert_eq!(worker.semantic_state()?, before);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn arena_v2_interrupts_both_policies_at_the_same_shared_cap() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let budget = ExecutionBudget::default();
        let scope = BotPlanningScope {
            seed: 39, bots: &[player], worlds: 2, mode: super::super::TeamKnowledgeMode::TrueRules,
        };
        let options = PlanningSearchOptions {
            rollout_turns: Some(4),
            workers: 1,
            upgrade_checkpoints: None,
            reply_cap: Some(4),
            reply_probe_cap: None,
            greedy_card_order: true,
            tactical_forecast: false,
            forecast_node_share: ForecastNodeShare::default(),
            legacy_fixed_followup: false,
            fixed_future_planning: false,
            branch_future_planning: false,
        };
        let limits = SearchLimits {
            max_depth: 3,
            max_nodes: 32,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::LanePressureV3,
        };
        let run = || root.search_node_limited_planning(player, scope, 8, limits, options, &budget);
        let planned = run().unwrap();
        assert_eq!(planned, run().unwrap());
        assert_eq!(planned.matrix.nodes, limits.max_nodes);
        assert!(planning_has_score(&planned));
        assert_eq!(planned.commitments.len(), 1);

        let mut joint = root.joint_plans(player, true, 1).unwrap().plans.remove(0);
        joint.extend(root.joint_plans(player, false, 1).unwrap().plans.remove(0));
        root.apply_joint_planning_ordered(&joint).unwrap();
        root.advance_to_decision(&budget).unwrap();
        let player = root.search_player().unwrap().unwrap().0;
        let legal = root.resolution_choices(player).unwrap();
        let before = root.semantic_state().unwrap();
        let result = root.search_node_limited_resolution(
            player, PlanningRolloutSample { seed: 39, upgrade_checkpoints: None },
            2, &[player], false, limits, 4, &budget,
        ).unwrap();
        assert_eq!(result.nodes, limits.max_nodes);
        assert_eq!(result.stop, ResolutionStop::NodeLimit);
        assert!(result.score.is_some());
        assert!(legal.contains(&result.decision));
        assert_eq!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn accumulated_team_gold_preserves_upgrade_spend_and_counts_the_opposing_team() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let team = root.world.heroes[0].team;
        let enemy = root.world.heroes.iter().find(|hero| hero.team != team).unwrap().id;
        let hero = root.world.hero_row_mut(player).unwrap();
        hero.level = 1;
        hero.gold = 9;
        let score = |session: &NativeSession, evaluation| {
            session.search_score(evaluation, team, player).unwrap()
        };
        let accumulated = SearchEvaluation::LanePressureV3AccumulatedGold;
        let standard = SearchEvaluation::LanePressureV3;
        let before = score(&root, accumulated);
        let cash_before = score(&root, standard);
        let hero = root.world.hero_row_mut(player).unwrap();
        hero.level = 3;
        hero.gold -= 3;
        assert_eq!(score(&root, accumulated), before);
        assert!(score(&root, standard) < cash_before);

        root.world.hero_row_mut(enemy).unwrap().gold += 1;
        assert!(score(&root, accumulated) < before);
    }

    #[test]
    fn live_resolution_widens_when_the_shallow_ranking_is_uncertain() {
        assert_eq!(live_root_width([100_000, 40_000, 10_000].into_iter(), 12), 3);
        assert_eq!(live_root_width([100_000; 10].into_iter(), 12), 8);
        assert!(!live_roots_uncertain(Some((0, 90_000)), Some(20_000), 0, 35_000));
        assert!(live_roots_uncertain(Some((0, 90_000)), Some(70_000), 0, 35_000));
        assert!(live_roots_uncertain(Some((1, 90_000)), Some(20_000), 0, 35_000));
    }

    #[test]
    fn live_sampled_resolution_can_roll_four_turns_across_a_round() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let mut plan = root.joint_plans(player, true, 1).unwrap().plans.remove(0);
        plan.extend(root.joint_plans(player, false, 1).unwrap().plans.remove(0));
        let budget = ExecutionBudget::default();
        root.apply_joint_planning_ordered(&plan).unwrap();
        root.advance_to_decision(&budget).unwrap();
        let player = root.search_player().unwrap().unwrap().0;
        let team = root.world.hero(player).unwrap().team;
        let sample = PlanningRolloutSample { seed: 39, upgrade_checkpoints: None };
        let mut sampled = root.sampled_resolution_workers(player, sample, 1, &[player], &budget)
            .unwrap().remove(0);
        let cutoff = sampled.search_turn_cutoff(Some(4)).unwrap().unwrap();
        let mut search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 4_096,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::LanePressureV3,
            turn_cutoff: Some(cutoff),
            controlled: None,
            shared_nodes: None,
            guard_hidden: true,
            allow_simulated_upgrades: true,
            hand_belief: None,
            planning_sample: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
            future_planning: None,
        };
        let leaf = search.rollout(&mut sampled, cutoff, false, false).unwrap().unwrap();
        assert_eq!(sampled.search_turn_index(), cutoff);
        assert!(leaf.round > root.round.round);

        let choices = root.resolution_choices(player).unwrap();
        assert!(choices.len() > 1);
        let incumbent = choices.last().unwrap();
        let mut worlds = root.sampled_resolution_workers(player, sample, 2, &[player], &budget)
            .unwrap();
        search.nodes = 0;
        let sum = worlds.iter_mut().map(|world| {
            i128::from(search.score_sampled_root(world, &mut None, incumbent, 0)
                .unwrap().unwrap().score)
        }).sum::<i128>();
        let mut ranking = Vec::new();
        let reference = root.search_live_bot_resolution(
            player, sample, 2, &[player], None, &mut ranking, false,
            SearchLimits {
                max_depth: 1,
                max_nodes: 4_096,
                opponent_policy: search.opponent_policy,
                evaluation: search.evaluation,
            },
            2, &budget,
        ).unwrap();
        assert!(reference.score.is_some());
        assert!(ranking.len() > 1);
        assert!(ranking.windows(2).all(|pair| pair[0].1 >= pair[1].1));
        let limited = root.search_live_bot_resolution(
            player, sample, 2, &[player], Some(incumbent), &mut ranking, false,
            SearchLimits {
                max_depth: 1,
                max_nodes: search.nodes,
                opponent_policy: search.opponent_policy,
                evaluation: search.evaluation,
            },
            4, &budget,
        ).unwrap();
        assert_eq!(limited.stop, ResolutionStop::NodeLimit);
        assert_eq!(limited.completed_depth, 0);
        assert_eq!(limited.decision, *incumbent);
        assert_eq!(limited.score, Some((sum / 2) as i64));
        assert_eq!(limited.score_kind, Some(ResolutionScoreKind::TurnRollout));
    }

    #[test]
    fn short_turn_cutoff_rejects_a_rollout_stopped_at_hidden_upgrades() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        worker.planning.phase = Phase::LevelUp;
        assert!(worker.last_request.is_none());
        let budget = ExecutionBudget::default();
        let cutoff = worker.search_turn_index() + 1;
        let mut search = Search {
            player: worker.world.heroes[0].id,
            team: worker.world.heroes[0].team,
            budget: &budget,
            nodes: 0,
            max_nodes: 16,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
            turn_cutoff: Some(cutoff),
            controlled: None,
            shared_nodes: None,
            guard_hidden: true,
            allow_simulated_upgrades: false,
            hand_belief: None,
            planning_sample: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
            future_planning: None,
        };
        let leaf = search.rollout(&mut worker, cutoff, false, false).unwrap().unwrap();
        assert_eq!(leaf.phase, Phase::LevelUp);
        assert_eq!(leaf.nodes, 0);
        assert!(!leaf.reached_cutoff(cutoff));
    }

    #[test]
    fn unsupported_future_planning_sample_cannot_supply_a_rollout_score() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        assert_eq!(worker.planning.phase, Phase::Planning);
        let player = worker.world.heroes[0].id;
        let team = worker.world.heroes[0].team;
        let opponent = worker.world.heroes.iter().find(|hero| hero.team != team).unwrap();
        let (opponent, card) = (opponent.id, opponent.hand[0]);
        // Simulate a publicly revealed opposing card at a future planning
        // boundary. The ordinary planning sampler cannot rebuild this root.
        worker.world.card_mut(card).unwrap().facedown = false;
        worker.world.hero_row_mut(opponent).unwrap().current = Some(card);
        let request = worker.last_request.clone();
        let cutoff = worker.search_turn_index() + 1;
        let budget = ExecutionBudget::default();
        let mut search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 16,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
            turn_cutoff: Some(cutoff),
            controlled: None,
            shared_nodes: None,
            guard_hidden: true,
            allow_simulated_upgrades: false,
            hand_belief: None,
            planning_sample: Some(PlanningRolloutSample {
                seed: 17,
                upgrade_checkpoints: None,
            }),
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
            future_planning: None,
        };

        assert!(matches!(
            search.rollout(&mut worker, cutoff, false, false),
            Err(EngineError::Unsupported(message)) if message.contains("publicly revealed current cards")
        ));
        assert_eq!(worker.planning.phase, Phase::Planning);
        assert_eq!(worker.last_request, request);
        assert_eq!(worker.world.hero(opponent).unwrap().current, Some(card));
    }

    #[test]
    fn forced_root_move_at_turn_cutoff_reuses_warmup_score() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let mut plan = root.joint_plans(player, true, 1).unwrap().plans.remove(0);
        plan.extend(root.joint_plans(player, false, 1).unwrap().plans.remove(0));
        root.apply_joint_planning_ordered(&plan).unwrap();
        let budget = ExecutionBudget::default();
        root.advance_to_decision(&budget).unwrap();
        let turn = root.search_turn_index();
        for _ in 0..64 {
            let (owner, team) = root.search_player().unwrap().unwrap();
            let moves = root.resolution_choices(owner).unwrap();
            let decision = moves
                .iter()
                .find(|decision| {
                    matches!(decision, DecisionMove::Input { selection: crate::data::Datum::Text(id), .. } if root.symbols.name(*id) == "HOLD")
                })
                .unwrap_or(&moves[0]);
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = decision
            else {
                panic!("expected a live resolution input");
            };
            let mut branch = root.clone();
            branch.submit(*token, selection.clone()).unwrap();
            branch.last_request = None;
            branch.advance_to_decision(&budget).unwrap();
            if branch.search_turn_index() > turn {
                assert_eq!(moves.len(), 1);
                let limits = SearchLimits {
                    max_depth: 1,
                    max_nodes: 1,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: SearchEvaluation::MaterialPosition,
                };
                let result = root
                    .search_resolution_with_turn_rollout(owner, limits, Some(1), &budget)
                    .unwrap();
                assert_eq!(result.score, Some(branch.evaluate_resolution(team, owner)));
                assert_eq!(
                    (result.completed_depth, result.nodes, result.stop),
                    (1, 1, ResolutionStop::Resolved)
                );
                let before = root.semantic_state().unwrap();
                let sampled = root
                    .search_sampled_resolution_with_turn_rollout(
                        owner,
                        PlanningRolloutSample {
                            seed: 39,
                            upgrade_checkpoints: None,
                        },
                        limits,
                        Some(1),
                        None,
                        &budget,
                    )
                    .unwrap();
                assert_eq!(sampled.decision, result.decision);
                assert_eq!(sampled.score, result.score);
                assert_eq!((sampled.completed_depth, sampled.nodes), (1, 1));
                assert_eq!(root.semantic_state().unwrap(), before);
                return;
            }
            root = branch;
        }
        panic!("fixture did not reach a turn boundary");
    }

    #[test]
    fn joint_plans_close_an_open_second_card_slot_and_restore_it_after_scoring() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-planning-command-finish-dual-last",
        );
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let hero = root.world.heroes[0].id;
        let first = root.observed_planning_cards(hero).unwrap().0.unwrap();
        let before = root.semantic_state().unwrap();
        let rows = root.joint_plans(hero, true, 32).unwrap();
        assert!(rows.plans.iter().any(|row| row[0].first.is_none()));
        assert!(rows.plans.iter().any(|row| row[0].first.is_some()));
        for row in rows.plans {
            assert_eq!(row.len(), 1);
            assert_eq!(row[0].second, None);
            root.with_joint_planning(&row, |worker| {
                assert_ne!(worker.planning.phase, Phase::Planning);
                let revealed = worker.world.hero(hero).unwrap();
                assert_eq!(
                    (revealed.current, revealed.extra),
                    (Some(first), row[0].first)
                );
                Ok(())
            })
            .unwrap();
            assert_eq!(root.semantic_state().unwrap(), before);
        }
        // First-legal continuation must cross this window too. With only one
        // node it can apply planning but cannot finish the subsequent turn.
        let budget = ExecutionBudget::default();
        assert!(
            root.rollout_game_turns(hero, 1, 1, &budget)
                .unwrap()
                .is_none()
        );
        assert_eq!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn lane_pressure_timing_rewards_readiness_in_the_advanced_zone() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-live-battle-multi-same-direction-first",
        );
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let red = root.symbols.lookup("RED").unwrap();
        let player = root.world.heroes.iter().find(|hero| hero.team == red).unwrap().id;
        let graph = &root.world.definitions.zones;
        let (lane, ordered) = &graph.lanes[0];
        let zone = ordered.iter().enumerate()
            .filter(|(index, _)| index.saturating_sub(1) > ordered.len().saturating_sub(2 + index))
            .filter_map(|(_, id)| graph.zones.iter().find(|zone| zone.id == *id))
            .find(|zone| zone.spawns.iter().any(|spawn| spawn.team != red))
            .unwrap();
        let (lane, other_lane) = (*lane, graph.lanes[1].0);
        root.world.battle_zones = vec![(lane, zone.id)];
        root.world.waves = vec![(lane, 2), (other_lane, 1)];
        let enemy_spawns: Vec<_> = zone.spawns.iter()
            .filter(|spawn| spawn.team != red).map(|spawn| spawn.hex).collect();
        let distance = |hex: Hex| {
            enemy_spawns.iter().map(|&spawn| hex.distance(spawn)).min().unwrap()
        };
        let positions = root.world.tile_hexes().iter().copied().filter(|&hex| {
            !root.world.terrain(hex)
                && root.world.occupant(hex).is_none_or(|id| id == player)
        });
        let near = positions.clone().min_by_key(|&hex| distance(hex)).unwrap();
        let far = positions.max_by_key(|&hex| distance(hex)).unwrap();
        assert!(distance(far) > distance(near).max(1));

        let timing_delta = |session: &NativeSession| {
            session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4)
                - session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV3)
        };
        root.world.place_entity(player, far).unwrap();
        let unready = timing_delta(&root);
        root.world.move_entity(player, near).unwrap();
        assert!(timing_delta(&root) > unready);

        let ally = root.world.heroes.iter()
            .find(|hero| hero.team == red && hero.id != player).unwrap().id;
        let coverage_delta = |session: &NativeSession| {
            session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV5)
                - session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4)
        };
        if root.world.position(ally).is_some() {
            root.world.move_entity(ally, far).unwrap();
        } else {
            root.world.place_entity(ally, far).unwrap();
        }
        let spread = coverage_delta(&root);
        let second_near = root.world.tile_hexes().iter().copied()
            .filter(|&hex| !root.world.terrain(hex) && root.world.occupant(hex).is_none())
            .min_by_key(|&hex| distance(hex)).unwrap();
        assert!(distance(second_near) < distance(far));
        root.world.move_entity(ally, second_near).unwrap();
        assert!(coverage_delta(&root) < spread);

        let ordinary = root.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4);
        root.world.waves[1].1 = 0;
        let last_push = root.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4);
        assert!(last_push > ordinary);

        root.world.combat.outcome = Some(crate::combat::GameOutcome {
            team: Some(red),
            hero: None,
            condition: red,
        });
        assert_eq!(root.search_score(SearchEvaluation::LanePressureV4, red, player).unwrap(), WIN);
    }

    #[test]
    fn multi_world_planning_prunes_rows_that_can_only_tie_a_win() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let hero = &root.world.heroes[0];
        let (player, team) = (hero.id, hero.team);
        root.world.combat.outcome = Some(crate::combat::GameOutcome {
            team: Some(team),
            hero: None,
            condition: team,
        });
        let rows = root.joint_plans(player, true, 2).unwrap().plans;
        let replies = root.joint_plans(player, false, 1).unwrap().plans;
        let mut samples = [
            PlanningWorld { session: root.clone(), replies: replies.clone(), forecasts: vec![], fixed_opponent_seed: None },
            PlanningWorld { session: root, replies, forecasts: vec![], fixed_opponent_seed: None },
        ];
        let result = search_planning_worlds(
            player,
            &rows,
            &mut samples,
            SearchLimits {
                max_depth: 0,
                max_nodes: 2,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
            },
            1,
            PlanningPartition {
                rollout_turns: None,
                controlled: None,
                shared_nodes: None,
                greedy_forecasts: &[],
                future_branching: None,
                legacy_fixed_followup: false,
                fixed_opponent_seed: None,
            },
            &ExecutionBudget::default(),
            None,
        )
        .unwrap();
        assert_eq!(result.row, 0);
        assert_eq!(result.score, Some(WIN));
        assert_eq!((result.completed_rows, result.pruned_rows), (1, 1));
        assert_eq!((result.evaluated_cells, result.nodes), (2, 2));
    }

    #[test]
    fn balanced_joint_plans_vary_the_ally_before_more_player_alternatives() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let root = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        let player = root.world.heroes[0].id;
        let team = root.world.heroes[0].team;
        let ally = root.world.heroes.iter()
            .find(|hero| hero.id != player && hero.team == team).unwrap().id;
        let controlled = [player, ally];
        let plans = root.joint_plans_with_reply_order(
            player, true, 3, Some(&controlled), JointPlanOrder::Balanced, &[],
        ).unwrap().plans;
        assert_eq!(plans.len(), 3);
        assert_eq!(plans[0][0].hero, player);
        assert_eq!(plans[0][1].hero, ally);
        assert_ne!(plans[1][0], plans[0][0]);
        assert_eq!(plans[1][1], plans[0][1]);
        assert_eq!(plans[2][0], plans[0][0]);
        assert_ne!(plans[2][1], plans[0][1]);
    }

    #[test]
    fn scoped_planning_returns_only_bot_commitments_and_ignores_hidden_human_choices() {
        use super::super::TeamKnowledgeMode;
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let team = root.world.heroes[0].team;
        let ally = root
            .world
            .heroes
            .iter()
            .find(|hero| hero.id != player && hero.team == team)
            .unwrap();
        let ally_id = ally.id;
        let hand = ally.hand.clone();
        let limits = SearchLimits {
            max_depth: 0,
            max_nodes: 16,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
        };
        let options = PlanningSearchOptions {
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
        };
        let budget = ExecutionBudget::default();
        let run = |root: &NativeSession, bots: &[Symbol], mode| {
            root.search_bot_team_planning(
                player,
                BotPlanningScope {
                    bots,
                    mode,
                    worlds: 2,
                    seed: 39,
                },
                2,
                limits,
                options,
                &budget,
            )
            .unwrap()
        };
        let both = [player, ally_id];
        let coordinated = run(&root, &both, TeamKnowledgeMode::Coordinated);
        assert_eq!(
            coordinated
                .commitments
                .iter()
                .map(|row| row.hero)
                .collect::<Vec<_>>(),
            both
        );
        assert_eq!(
            run(&root, &both, TeamKnowledgeMode::TrueRules)
                .commitments
                .len(),
            1
        );
        let only = [player];
        let before = run(&root, &only, TeamKnowledgeMode::CoordinatedShow);
        assert_eq!(before.matrix.completed_rows, 2);
        assert_eq!(before.matrix.evaluated_cells, 8);
        assert_eq!(before.matrix.total_cells, 8);
        let parallel = root
            .search_bot_team_planning(
                player,
                BotPlanningScope {
                    bots: &only,
                    mode: TeamKnowledgeMode::CoordinatedShow,
                    worlds: 2,
                    seed: 39,
                },
                2,
                limits,
                PlanningSearchOptions {
                    workers: 2,
                    reply_probe_cap: Some(32),
                    ..options
                },
                &budget,
            )
            .unwrap();
        assert_eq!(parallel, before);
        let state_before_retry = root.semantic_state().unwrap();
        let retry = root.search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::TrueRules,
                worlds: 2,
                seed: 39,
            },
            2,
            SearchLimits { max_depth: 2, max_nodes: 64, ..limits },
            PlanningSearchOptions { rollout_turns: Some(2), reply_cap: Some(1), ..options },
            &budget,
        ).unwrap();
        assert_eq!(retry.matrix.completed_rows, 0);
        assert!(retry.matrix.score.is_none());
        assert!(retry.shallow_score.is_some());
        assert!((1..=4).contains(&retry.shallow_nodes));
        assert_eq!(retry.greedy_nodes + retry.probe_nodes, 0);
        assert!(retry.matrix.nodes > retry.shallow_nodes);
        assert!(retry.matrix.nodes <= 64);
        assert_eq!(retry.commitments[0].hero, player);
        assert_eq!(root.semantic_state().unwrap(), state_before_retry);
        let seed_pass = root.search_bot_team_planning_with_seed_pass(
            player,
            BotPlanningScope { bots: &only, mode: TeamKnowledgeMode::TrueRules, worlds: 2, seed: 39 },
            2,
            SearchLimits { max_depth: 2, max_nodes: 8192, ..limits },
            PlanningSearchOptions { rollout_turns: Some(2), reply_cap: Some(1), ..options },
            &budget,
            true,
        ).unwrap();
        assert!((1..=512).contains(&seed_pass.seed_nodes));
        assert!(seed_pass.seed_score.is_some());
        assert!(seed_pass.matrix.score.is_some());
        assert!(seed_pass.matrix.nodes <= 8192);
        assert_eq!(root.semantic_state().unwrap(), state_before_retry);
        let run_fixed = |root: &NativeSession, forecast_node_share| root.search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::CoordinatedShow,
                worlds: 2,
                seed: 39,
            },
            2,
            SearchLimits { max_nodes: 8192, ..limits },
            PlanningSearchOptions {
                workers: 2,
                greedy_card_order: true,
                rollout_turns: Some(2),
                forecast_node_share,
                ..options
            },
            &budget,
        ).unwrap();
        let seeded = run_fixed(&root, ForecastNodeShare::Quarter);
        let unseeded = run_fixed(&root, ForecastNodeShare::Zero);
        assert_eq!(unseeded.greedy_nodes, 0);
        assert_eq!(unseeded.greedy_card_scores, 0);
        assert_eq!(unseeded.matrix.forecast_matches, 0);
        assert!(unseeded.matrix.forecast_checks > 0);
        assert_eq!(unseeded.matrix.total_cells, unseeded.total_rows * 2);
        assert!(unseeded.matrix.score.is_some());
        assert_eq!(root.semantic_state().unwrap(), state_before_retry);
        assert_eq!(seeded.prepared_worlds, 2);
        assert!(seeded.greedy_card_scores > 0);
        assert_eq!(before.greedy_card_scores, 0);
        assert_eq!(seeded.commitments[0].hero, player);
        assert!(seeded.matrix.score.is_some());
        assert!(seeded.greedy_nodes > 0);
        assert_eq!(seeded.shallow_nodes, 0);
        assert!(seeded.matrix.nodes >= seeded.greedy_nodes + seeded.probe_nodes
            + seeded.matrix.evaluated_cells);
        assert!(seeded.matrix.nodes <= 8192);

        let partial_parallel = root
            .search_bot_team_planning(
                player,
                BotPlanningScope {
                    bots: &only,
                    mode: TeamKnowledgeMode::CoordinatedShow,
                    worlds: 2,
                    seed: 39,
                },
                2,
                SearchLimits {
                    max_nodes: 3,
                    ..limits
                },
                PlanningSearchOptions {
                    workers: 2,
                    ..options
                },
                &budget,
            )
            .unwrap();
        assert_eq!(partial_parallel.matrix.completed_rows, 0);
        assert_eq!(partial_parallel.matrix.score, None);
        assert_eq!(partial_parallel.matrix.nodes, 3);

        let limited = root
            .search_bot_team_planning(
                player,
                BotPlanningScope {
                    bots: &only,
                    mode: TeamKnowledgeMode::CoordinatedShow,
                    worlds: 2,
                    seed: 39,
                },
                2,
                SearchLimits {
                    max_nodes: 3,
                    ..limits
                },
                options,
                &budget,
            )
            .unwrap();
        assert_eq!(limited.matrix.completed_rows, 0);
        assert_eq!(limited.matrix.score, None);
        assert_eq!(limited.matrix.nodes, 3);
        assert_eq!(limited.prepared_worlds, 2);
        let expired = ExecutionBudget {
            deadline: Some(std::time::Instant::now()),
            ..budget
        };
        let fallback = root
            .search_bot_team_planning(
                player,
                BotPlanningScope {
                    bots: &only,
                    mode: TeamKnowledgeMode::CoordinatedShow,
                    worlds: 2,
                    seed: 39,
                },
                2,
                limits,
                options,
                &expired,
            )
            .unwrap();
        assert_eq!(fallback.prepared_worlds, 0);
        assert_eq!(fallback.requested_worlds, 2);
        assert_eq!(fallback.matrix.score, None);
        assert_eq!(fallback.commitments, limited.commitments);
        root.commit_card(ally_id, hand[0]).unwrap();
        assert_eq!(run_fixed(&root, ForecastNodeShare::Quarter), seeded);
        assert_eq!(
            run(&root, &only, TeamKnowledgeMode::CoordinatedShow),
            before
        );
        let know = run(
            &root,
            &only,
            TeamKnowledgeMode::CoordinatedKnowSameTeamHumans,
        );
        assert_eq!(know.commitments.len(), 1);
        assert_eq!(know.commitments[0].hero, player);
    }

    #[test]
    fn sampled_midround_planning_keeps_public_cards_in_their_live_slots() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = worker.world.heroes[0].id;
        let team = worker.world.heroes[0].team;
        let budget = ExecutionBudget::default();
        let mut search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 512,
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
        worker
            .with_rollback(|worker| {
                let cutoff = worker.search_turn_index() + 1;
                assert!(search.rollout(worker, cutoff, false, false)?.is_some());
                assert_eq!(worker.planning.phase, Phase::Planning);
                let opponent = worker
                    .world
                    .heroes
                    .iter()
                    .find(|hero| hero.team != team && hero.played.iter().any(Option::is_some))
                    .unwrap();
                let sampled = worker.sampled_planning_world(player, 39)?;
                let sampled_hero = sampled.world.hero(opponent.id).unwrap();
                assert_eq!(sampled_hero.played, opponent.played);
                assert_eq!(sampled_hero.discard, opponent.discard);
                for card in opponent.played.iter().flatten().chain(&opponent.discard) {
                    assert_eq!(
                        sampled.world.card(*card).unwrap().lifecycle,
                        worker.world.card(*card).unwrap().lifecycle
                    );
                    assert!(!sampled_hero.hand.contains(card));
                }
                assert!(!sampled.planning_choices(opponent.id)?.is_empty());
                let expected = sampled.semantic_state()?;
                let before = worker.semantic_state()?;
                let actual =
                    worker.with_sampled_planning(player, 39, |branch| branch.semantic_state())?;
                assert_eq!(actual, expected);
                assert_eq!(worker.semantic_state()?, before);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn sampled_planning_generates_bounded_joint_choices_and_returns_a_live_card() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let live = root.planning_choices(player).unwrap();
        let cap = 2;
        let seed = 39;
        let mut worker = root.sampled_planning_world(player, seed).unwrap();
        let friendly = worker.joint_plans(player, true, cap).unwrap();
        let opposing = worker.joint_plans(player, false, cap).unwrap();
        assert!(friendly.total > cap && opposing.total > cap);
        assert_eq!(friendly.plans.len(), cap);
        assert_eq!(opposing.plans.len(), cap);
        assert_eq!(friendly.plans[0][0].hero, player);
        assert_ne!(friendly.plans[0][0].first, friendly.plans[1][0].first);
        assert_eq!(friendly.plans[0][1..], friendly.plans[1][1..]);
        let limits = SearchLimits {
            max_depth: 0,
            max_nodes: 4,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
        };
        let budget = ExecutionBudget::default();
        let expected = worker
            .search_planning_matrix(player, &friendly.plans, &opposing.plans, limits, &budget)
            .unwrap();
        let result = root
            .search_sampled_planning(player, seed, cap, limits, &budget)
            .unwrap();
        assert_eq!(result.matrix, expected);
        assert_eq!(result.matrix.evaluated_cells, 4);
        assert_eq!(result.total_friendly_plans, friendly.total);
        assert_eq!(result.total_opposing_plans, opposing.total);
        let card = result.commitment.first.unwrap();
        assert!(live.contains(&DecisionMove::Card { hero: player, card }));
    }

    #[test]
    fn planning_matrix_matches_exhaustive_payoffs_and_excludes_incomplete_rows() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let team = root.world.heroes[0].team;
        let mut worker = root.sampled_planning_world(player, 71).unwrap();
        let side = |friendly| {
            (0..2)
                .map(|index| {
                    worker
                        .world
                        .heroes
                        .iter()
                        .filter(|hero| (hero.team == team) == friendly)
                        .map(|hero| {
                            let moves = worker.planning_choices(hero.id).unwrap();
                            let DecisionMove::Card { card, .. } = moves[index] else {
                                panic!("expected ordinary card");
                            };
                            PlanningCommitment {
                                hero: hero.id,
                                first: Some(card),
                                second: None,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let rows = side(true);
        let columns = side(false);
        let before = serde_json::to_value(worker.export_envelope().unwrap()).unwrap();
        let budget = ExecutionBudget::default();
        let mut payoffs = Vec::new();
        for row in &rows {
            let mut worst = i64::MAX;
            for column in &columns {
                let choices = [row.as_slice(), column.as_slice()].concat();
                let value = worker
                    .with_joint_planning(&choices, |branch| {
                        branch.advance_to_decision(&budget)?;
                        Ok(branch.evaluate_resolution(team, player))
                    })
                    .unwrap();
                worst = worst.min(value);
            }
            payoffs.push(worst);
        }
        let limits = SearchLimits {
            max_depth: 0,
            max_nodes: 4,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
        };
        let result = worker
            .search_planning_matrix(player, &rows, &columns, limits, &budget)
            .unwrap();
        assert_eq!(result.completed_rows, 2);
        assert_eq!(result.evaluated_cells, result.total_cells);
        assert_eq!(result.score, payoffs.iter().copied().max());
        assert_eq!(result.row, usize::from(payoffs[1] > payoffs[0]));
        let parallel = worker
            .search_planning_matrix_parallel(player, &rows, &columns, limits, 2, &budget)
            .unwrap();
        assert_eq!(parallel, result);
        let partial = worker
            .search_planning_matrix(
                player,
                &rows,
                &columns,
                SearchLimits {
                    max_nodes: 3,
                    ..limits
                },
                &budget,
            )
            .unwrap();
        assert_eq!(partial.completed_rows, 1);
        assert_eq!(partial.evaluated_cells, 3);
        assert_eq!(partial.score, Some(payoffs[0]));
        assert_eq!(partial.row, 0);
        let rollout_limits = SearchLimits {
            max_nodes: 512,
            ..limits
        };
        let mut ordered_columns = columns
            .iter()
            .map(|column| {
                let cell = worker
                    .search_planning_matrix_with_rollout(
                        player,
                        &rows[..1],
                        std::slice::from_ref(column),
                        rollout_limits,
                        Some(2),
                        &budget,
                    )
                    .unwrap();
                (cell.score.unwrap(), column.clone())
            })
            .collect::<Vec<_>>();
        ordered_columns.sort_by_key(|(score, _)| *score);
        let expected = ordered_columns[0].0;
        let ordered_columns = ordered_columns
            .into_iter()
            .map(|(_, column)| column)
            .collect::<Vec<_>>();
        let tied_rows = vec![rows[0].clone(), rows[0].clone()];
        let pruned = worker
            .search_planning_matrix_with_rollout(
                player,
                &tied_rows,
                &ordered_columns,
                rollout_limits,
                Some(2),
                &budget,
            )
            .unwrap();
        assert_eq!(pruned.score, Some(expected));
        assert_eq!(pruned.row, 0);
        assert_eq!(
            (
                pruned.completed_rows,
                pruned.pruned_rows,
                pruned.evaluated_cells
            ),
            (1, 1, 3)
        );
        assert_eq!(
            serde_json::to_value(worker.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn planning_pair_replay_checks_round_rollover_and_detects_world_divergence() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let world = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        assert!(matches!(
            world.clone().compare_planning_continuation(world.clone()).unwrap(),
            PlanningPair::NextRound,
        ));
        let mut changed = world.clone();
        changed.world.heroes[0].gold += 1;
        assert!(matches!(
            world.clone().compare_planning_continuation(changed).unwrap(),
            PlanningPair::Diverged,
        ));
    }

    #[test]
    fn resolution_reuse_census_respects_caps_and_restores_the_prompt() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-attack-macro-optional-first",
        );
        let mut worker = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        worker.advance_to_decision(&ExecutionBudget::default()).unwrap();
        let before = worker.semantic_state().unwrap();
        let request = worker.last_request.clone();
        let census = worker.resolution_reuse_census(4, 2).unwrap();
        assert_eq!(census.visited, 2);
        assert!(census.internal > 0);
        assert_eq!(worker.semantic_state().unwrap(), before);
        assert_eq!(worker.last_request, request);
    }

    #[test]
    fn attack_target_extension_searches_beyond_depth_zero_within_the_node_cap() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-attack-macro-optional-first",
        );
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        worker.advance_prefix(1).unwrap();
        let owner = worker.advance_prefix(1).unwrap().request.unwrap().owner;
        let team = worker.world.hero(owner).unwrap().team;
        assert!(worker.pending_attack_target());
        let choices = worker.resolution_choices(owner).unwrap().len();
        assert!(choices > 1);
        let before = worker.semantic_state().unwrap();
        let budget = ExecutionBudget::default();
        let mut search = Search {
            player: owner,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 100,
            opponent_policy: OpponentPolicy::Adversarial,
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
        worker.with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))
            .unwrap().unwrap();
        assert_eq!(search.nodes, 0);

        search.opponent_policy = OpponentPolicy::AdversarialAttackTargets;
        worker.with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))
            .unwrap().unwrap();
        assert!(search.nodes >= choices);

        search.nodes = 0;
        search.max_nodes = 1;
        assert!(worker.with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))
            .unwrap().is_none());
        assert_eq!(search.nodes, 1);
        assert_eq!(worker.semantic_state().unwrap(), before);
    }

    #[test]
    fn scoped_search_prefers_an_eligible_bot_for_a_team_prompt() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-hero-respawn-live-ordinary-first",
        );
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let budget = ExecutionBudget::default();
        let owner = worker.advance(&budget).unwrap().request.unwrap().owner;
        let team = worker.world.hero(owner).unwrap().team;
        let mut allies = worker.world.heroes.iter().filter(|hero| hero.team == team);
        let first = allies.next().unwrap().id;
        let bot = allies.next().unwrap().id;
        let enemy = worker.world.heroes.iter().find(|hero| hero.team != team).unwrap().id;

        assert_eq!(
            worker.search_player_preferring(Some(&[enemy, bot])).unwrap(),
            Some((owner, team)),
        );
        worker.last_request.as_mut().unwrap().choices = RequestChoices::Actors {
            team,
            values: vec![first, bot],
        };
        assert_eq!(worker.search_player().unwrap(), Some((first, team)));
        assert_eq!(
            worker.search_player_preferring(Some(&[enemy, bot])).unwrap(),
            Some((bot, team)),
        );
        assert_eq!(
            worker.search_player_preferring(Some(&[enemy])).unwrap(),
            Some((first, team)),
        );
    }

    #[test]
    fn fixed_policy_expands_only_the_first_opposing_move() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-hero-respawn-live-ordinary-first",
        );
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let budget = ExecutionBudget::default();
        let owner = worker.advance(&budget).unwrap().request.unwrap().owner;
        let owner_team = worker.world.hero(owner).unwrap().team;
        let root = worker
            .world
            .heroes
            .iter()
            .find(|hero| hero.team != owner_team)
            .unwrap();
        let (player, team) = (root.id, root.team);
        let moves = worker.resolution_choices(owner).unwrap();
        assert!(moves.len() > 1);
        let mut search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 100,
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
        let before = worker.semantic_state().unwrap();
        let fixed = worker
            .with_rollback(|branch| search.value(branch, 1, i64::MIN, i64::MAX))
            .unwrap()
            .unwrap();
        assert_eq!(search.nodes, 1);
        assert_eq!(worker.semantic_state().unwrap(), before);
        let first = search
            .branch(&mut worker, moves[0].clone(), 0, i64::MIN, i64::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(fixed.score, first.score);
        search.nodes = 0;
        search.opponent_policy = OpponentPolicy::Adversarial;
        let adversarial = worker
            .with_rollback(|branch| search.value(branch, 1, i64::MIN, i64::MAX))
            .unwrap()
            .unwrap();
        assert_eq!(search.nodes, moves.len());
        assert!(adversarial.score <= fixed.score);
    }

    #[test]
    fn preferred_root_move_is_searched_first_without_changing_complete_result() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-hero-respawn-live-ordinary-first",
        );
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let budget = ExecutionBudget::default();
        let request = worker.advance(&budget).unwrap().request.unwrap();
        let player = request.owner;
        let team = worker.world.hero(player).unwrap().team;
        let moves = worker.resolution_choices(player).unwrap();
        assert!(moves.len() > 1);
        let preferred = moves.len() - 1;
        let mut search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 1,
            opponent_policy: OpponentPolicy::Adversarial,
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
        let limited = search
            .choose(
                &mut worker,
                &moves,
                1,
                preferred,
                &mut vec![None; moves.len()],
            )
            .unwrap();
        assert_eq!(limited.interrupted, Some(ResolutionStop::NodeLimit));
        assert_eq!(limited.best.unwrap().0, preferred);
        search.max_nodes = 1000;
        let ordered = search
            .choose(
                &mut worker,
                &moves,
                2,
                preferred,
                &mut vec![None; moves.len()],
            )
            .unwrap();
        let baseline = search
            .choose(&mut worker, &moves, 2, 0, &mut vec![None; moves.len()])
            .unwrap();
        assert!(ordered.interrupted.is_none() && baseline.interrupted.is_none());
        assert_eq!(ordered.best, baseline.best);
    }
}
