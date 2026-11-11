//! Search configuration and results shared by planning and resolution.

use super::{DecisionMove, PlanningCommitment};
use crate::{
    codec::Phase,
    data::Symbol,
    interpreter::{TeamKnowledgeMode, UpgradeCheckpoint},
};

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
    StallLaneAdvance,
    StallWaveClear,
    StallMinionApproach,
    StallHeroThreat,
    StallCrossfire,
    StallNextWave,
    StallLanePresence,
    StallLifePressure,
    StallAttackReady,
    StallGoldDiscount,
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
    pub(super) fn reached_cutoff(&self, cutoff: i128) -> bool {
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

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastNodeShare {
    #[default]
    Quarter,
    Eighth,
    Zero,
}

impl ForecastNodeShare {
    pub(super) fn allowance(self, nodes: usize) -> usize {
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
    pub upgrade_checkpoints: Option<&'a std::collections::BTreeMap<String, Vec<UpgradeCheckpoint>>>,
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
    pub(super) fn fixed_opponent_seed(self, seed: u64) -> Option<u64> {
        (self.greedy_card_order && !self.fixed_future_planning && !self.branch_future_planning)
            .then_some(seed)
    }

    pub(super) fn future_branching(self) -> Option<bool> {
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
    pub upgrade_checkpoints: Option<&'a std::collections::BTreeMap<String, Vec<UpgradeCheckpoint>>>,
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
    pub mode: TeamKnowledgeMode,
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
