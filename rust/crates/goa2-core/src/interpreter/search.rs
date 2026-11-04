//! Bounded alpha-beta over live resolution decisions.
use super::{
    DecisionMove, EngineError, ExecutionBudget, NativeSession, PlanningCommitment, RequestChoices,
    Result,
};
use crate::{
    cards::HeroRecord,
    codec::Phase,
    data::Symbol,
    world::{ActionKind, EntityKind, MinionKind},
};

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpponentPolicy {
    #[default]
    Adversarial,
    FirstLegal,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub max_depth: u8,
    pub max_nodes: usize,
    pub opponent_policy: OpponentPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionScoreKind {
    Immediate,
    DecisionDepth,
    TurnRollout,
}

#[derive(Debug)]
pub struct ResolutionSearch {
    pub decision: DecisionMove,
    /// A scored root move may be available even before depth one is complete.
    pub score: Option<i64>,
    pub score_kind: Option<ResolutionScoreKind>,
    pub completed_depth: u8,
    pub resolved: bool,
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

#[derive(Debug, PartialEq, Eq)]
pub struct PlanningSearch {
    pub row: usize,
    pub score: Option<i64>,
    pub completed_rows: usize,
    pub pruned_rows: usize,
    pub evaluated_cells: usize,
    pub total_cells: usize,
    pub nodes: usize,
}

#[derive(Debug)]
pub struct JointPlans {
    pub plans: Vec<Vec<PlanningCommitment>>,
    pub total: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct PlanningSearchOptions {
    pub rollout_turns: Option<u8>,
    pub workers: usize,
}

#[derive(Debug)]
pub struct SampledPlanningSearch {
    pub commitment: PlanningCommitment,
    pub matrix: PlanningSearch,
    pub total_friendly_plans: usize,
    pub total_opposing_plans: usize,
}

const WIN: i64 = 1_000_000;

impl NativeSession {
    /// Generate up to cap joint plans. The requesting hero's choices vary
    /// first so a bounded friendly set covers their options before teammates'.
    pub fn joint_plans(&self, player: Symbol, friendly: bool, cap: usize) -> Result<JointPlans> {
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
        let mut total = 1usize;
        let mut heroes = self
            .world
            .heroes
            .iter()
            .filter(|hero| (hero.team == team) == friendly)
            .collect::<Vec<_>>();
        heroes.sort_by_key(|hero| hero.id != player);
        for hero in heroes {
            let moves = self.planning_choices(hero.id)?;
            if moves.is_empty() {
                continue;
            }
            let cards = moves
                .iter()
                .filter_map(|decision| match decision {
                    DecisionMove::Card { card, .. } => Some(*card),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut choices = Vec::new();
            for decision in moves {
                let first = match decision {
                    DecisionMove::Card { card, .. } => Some(card),
                    DecisionMove::Pass { .. } => None,
                    _ => {
                        return Err(invalid(
                            "joint plans require fresh or closed planning slots",
                        ));
                    }
                };
                choices.push(PlanningCommitment {
                    hero: hero.id,
                    first,
                    second: None,
                });
            }
            if self.planning.two_cards(&self.world, hero.id) {
                // Put single-card plans first so capped sets still see every
                // possible first card before considering second-card variants.
                for &first in &cards {
                    for &second in &cards {
                        if first != second {
                            choices.push(PlanningCommitment {
                                hero: hero.id,
                                first: Some(first),
                                second: Some(second),
                            });
                        }
                    }
                }
            }
            total = total
                .checked_mul(choices.len())
                .ok_or_else(|| invalid("joint plan count overflow"))?;
            dimensions.push(choices);
        }
        let plans = (0..total.min(cap))
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
            cap,
            limits,
            PlanningSearchOptions {
                rollout_turns: None,
                workers: 1,
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
        options: PlanningSearchOptions,
        budget: &ExecutionBudget<'_>,
    ) -> Result<SampledPlanningSearch> {
        if options.workers == 1 {
            self.with_sampled_planning(player, seed, |worker| {
                worker.search_plans_on_sample(player, cap, limits, options, budget)
            })
        } else {
            let mut worker = self.sampled_planning_world(player, seed)?;
            worker.search_plans_on_sample(player, cap, limits, options, budget)
        }
    }

    fn search_plans_on_sample(
        &mut self,
        player: Symbol,
        cap: usize,
        limits: SearchLimits,
        options: PlanningSearchOptions,
        budget: &ExecutionBudget<'_>,
    ) -> Result<SampledPlanningSearch> {
        let friendly = self.joint_plans(player, true, cap)?;
        let opposing = self.joint_plans(player, false, cap)?;
        let matrix = if options.workers == 1 {
            self.search_planning_matrix_with_rollout(
                player,
                &friendly.plans,
                &opposing.plans,
                limits,
                options.rollout_turns,
                budget,
            )?
        } else {
            self.search_planning_matrix_parallel_with_options(
                player,
                &friendly.plans,
                &opposing.plans,
                limits,
                options,
                budget,
            )?
        };
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
        self.search_planning_matrix_parallel_with_options(
            player,
            rows,
            columns,
            limits,
            PlanningSearchOptions {
                rollout_turns: None,
                workers,
            },
            budget,
        )
    }

    fn search_planning_matrix_parallel_with_options(
        &self,
        player: Symbol,
        rows: &[Vec<PlanningCommitment>],
        columns: &[Vec<PlanningCommitment>],
        limits: SearchLimits,
        options: PlanningSearchOptions,
        budget: &ExecutionBudget<'_>,
    ) -> Result<PlanningSearch> {
        let workers = options.workers;
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
                jobs.push(scope.spawn(move || {
                    session.search_planning_matrix_with_rollout(
                        player,
                        rows,
                        columns,
                        SearchLimits {
                            max_nodes,
                            ..limits
                        },
                        options.rollout_turns,
                        budget,
                    )
                }));
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
                !self
                    .world
                    .hero(choice.hero)
                    .is_some_and(|hero| hero.id == choice.hero && (hero.team == team) == friendly)
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
            turn_cutoff,
        };
        let mut joint = Vec::new();
        'rows: for (index, row) in rows.iter().enumerate() {
            let mut worst = i64::MAX;
            for (column_index, column) in columns.iter().enumerate() {
                if search.nodes == search.max_nodes {
                    break 'rows;
                }
                match budget.check(0) {
                    Ok(()) => {}
                    Err(EngineError::Deadline | EngineError::Cancelled) => break 'rows,
                    Err(error) => return Err(error),
                }
                joint.clear();
                joint.extend_from_slice(row);
                joint.extend_from_slice(column);
                search.nodes += 1;
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

    /// Search resolution choices with an optional fixed-policy leaf continuation.
    pub fn search_resolution_with_turn_rollout(
        &mut self,
        player: Symbol,
        limits: SearchLimits,
        rollout_turns: Option<u8>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<ResolutionSearch> {
        let turn_cutoff = self.search_turn_cutoff(rollout_turns)?;
        if limits.max_depth == 0 || limits.max_depth > 64 || limits.max_nodes == 0 {
            return Err(EngineError::InvalidInput(
                "search requires depth 1..64 and a positive node limit".into(),
            ));
        }
        let team = self
            .world
            .hero(player)
            .ok_or_else(|| EngineError::InvalidInput("search player is not a hero".into()))?
            .team;
        let moves = self.resolution_choices(player)?;
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
                nodes: 0,
            };
            let mut search = Search {
                player,
                team,
                budget,
                nodes: 0,
                max_nodes: limits.max_nodes,
                opponent_policy: limits.opponent_policy,
                turn_cutoff,
            };
            let mut preferred = 0;
            if turn_cutoff.is_some() {
                // Reserve most nodes for the requested horizon. Score quick
                // alternatives first so even an interrupted rollout has a fallback.
                let warmup_nodes = (limits.max_nodes / 8).max(1);
                for (index, decision) in moves.iter().take(warmup_nodes).enumerate() {
                    let score = worker.with_rollback(|branch| {
                        Ok(search
                            .apply_input(branch, decision)?
                            .then(|| branch.evaluate_resolution(team, player)))
                    });
                    match score {
                        Ok(Some(score)) if result.score.is_none_or(|best| score > best) => {
                            result.decision = decision.clone();
                            result.score = Some(score);
                            result.score_kind = Some(ResolutionScoreKind::Immediate);
                            preferred = index;
                        }
                        Ok(Some(_)) => {}
                        Ok(None) | Err(EngineError::Deadline | EngineError::Cancelled) => break,
                        Err(error) => return Err(error),
                    }
                }
            }
            let mut resolved_scores = vec![None; moves.len()];
            for depth in 1..=limits.max_depth {
                let iteration =
                    search.choose(worker, &moves, depth, preferred, &mut resolved_scores)?;
                if let Some((index, score)) = iteration.best
                    && (iteration.complete || result.completed_depth == 0)
                {
                    preferred = index;
                    result.decision = moves[index].clone();
                    result.score = Some(score);
                    result.score_kind = Some(if turn_cutoff.is_some() {
                        ResolutionScoreKind::TurnRollout
                    } else {
                        ResolutionScoreKind::DecisionDepth
                    });
                }
                if !iteration.complete {
                    break;
                }
                result.completed_depth = depth;
                if iteration.resolved {
                    result.resolved = true;
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
            turn_cutoff: None,
        };
        self.with_rollback(|worker| {
            worker.advance_to_decision(budget)?;
            search.rollout(worker, cutoff, capture_state, lab_evaluation)
        })
    }

    fn search_turn_cutoff(&self, turns: Option<u8>) -> Result<Option<i128>> {
        if turns.is_some_and(|turns| !(1..=2).contains(&turns)) {
            return Err(EngineError::InvalidInput(
                "rollout_turns must be one or two".into(),
            ));
        }
        Ok(turns.map(|turns| self.search_turn_index() + i128::from(turns)))
    }

    fn search_turn_index(&self) -> i128 {
        i128::from(self.round.round) * 4 + i128::from(self.world.combat.turn)
    }

    fn search_player(&self) -> Result<Option<(Symbol, Symbol)>> {
        let Some(request) = &self.last_request else {
            return Ok(None);
        };
        let player = match &request.choices {
            RequestChoices::Actors { team, .. } => self
                .world
                .heroes
                .iter()
                .find(|hero| hero.team == *team)
                .map(|hero| hero.id),
            RequestChoices::Upgrades(players) => players
                .iter()
                .find(|player| !player.options.is_empty())
                .map(|player| player.hero),
            _ => Some(request.owner),
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
            .map(|row| sign(row.team) * row.gold)
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
        let red = self.symbols.lookup("RED");
        let gold = self.symbols.lookup("GOLD");
        let position = self
            .world
            .heroes
            .iter()
            .map(|hero| {
                sign(hero.team)
                    * i128::from(self.hero_position_score(hero, hero.id == player, red, gold))
            })
            .sum::<i128>()
            .clamp(-31, 31);
        ((life + heroes) * 64 + position).clamp(i128::from(-WIN + 1), i128::from(WIN - 1)) as i64
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

struct Search<'a, 'b> {
    player: Symbol,
    team: Symbol,
    budget: &'a ExecutionBudget<'b>,
    nodes: usize,
    max_nodes: usize,
    opponent_policy: OpponentPolicy,
    turn_cutoff: Option<i128>,
}

struct Iteration {
    best: Option<(usize, i64)>,
    complete: bool,
    resolved: bool,
}

struct ValueBound {
    score: i64,
    resolved: bool,
}

impl Search<'_, '_> {
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
        let order =
            std::iter::once(preferred).chain((0..moves.len()).filter(|index| *index != preferred));
        for index in order {
            let earlier = best.is_some_and(|(best_index, _)| index < best_index);
            // A fail-low result at alpha cannot distinguish equality from a
            // worse move. Widen by one when an exact tie would change the winner.
            let lower = if earlier {
                alpha.saturating_sub(1)
            } else {
                alpha
            };
            let score = if let Some(score) = resolved_scores[index] {
                score
            } else {
                let value =
                    match self.branch(worker, &moves[index], remaining_depth, lower, i64::MAX) {
                        Ok(Some(value)) => value,
                        Ok(None) | Err(EngineError::Deadline | EngineError::Cancelled) => {
                            return Ok(Iteration {
                                best,
                                complete: false,
                                resolved: false,
                            });
                        }
                        Err(error) => return Err(error),
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
            complete: true,
            resolved,
        })
    }

    fn branch(
        &mut self,
        worker: &mut NativeSession,
        decision: &DecisionMove,
        depth: u8,
        alpha: i64,
        beta: i64,
    ) -> Result<Option<ValueBound>> {
        worker.with_rollback(|branch| {
            if !self.apply_input(branch, decision)? {
                return Ok(None);
            }
            self.value(branch, depth, alpha, beta)
        })
    }

    fn apply_input(&mut self, worker: &mut NativeSession, decision: &DecisionMove) -> Result<bool> {
        self.budget.check(0)?;
        if self.nodes == self.max_nodes {
            return Ok(false);
        }
        self.nodes += 1;
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
        worker.submit(*token, selection.clone())?;
        worker.last_request = None;
        worker.advance_to_decision(self.budget)?;
        Ok(true)
    }

    fn rollout(
        &mut self,
        worker: &mut NativeSession,
        cutoff: i128,
        capture_state: bool,
        lab_evaluation: bool,
    ) -> Result<Option<GameTurnRollout>> {
        loop {
            self.budget.check(0)?;
            if worker.planning.phase == Phase::GameOver || worker.search_turn_index() >= cutoff {
                return Ok(Some(GameTurnRollout {
                    score: worker.evaluate_resolution(self.team, self.player),
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
                let mut commitments = Vec::new();
                for hero in &worker.world.heroes {
                    let choices = worker.planning_choices(hero.id)?;
                    if choices.contains(&DecisionMove::FinishPlanning { hero: hero.id }) {
                        return Err(EngineError::Unsupported(
                            "rollout requires fresh or closed planning slots".into(),
                        ));
                    }
                    let first = match choices.first() {
                        Some(DecisionMove::Card { card, .. }) => Some(*card),
                        Some(DecisionMove::Pass { .. }) => None,
                        None => continue,
                        _ => {
                            return Err(EngineError::Unsupported(
                                "rollout requires fresh or closed planning slots".into(),
                            ));
                        }
                    };
                    commitments.push(PlanningCommitment {
                        hero: hero.id,
                        first,
                        second: None,
                    });
                }
                self.nodes += 1;
                worker.apply_joint_planning_ordered(&commitments)?;
                worker.advance_to_decision(self.budget)?;
                continue;
            }
            let (owner, _) = worker.search_player()?.ok_or_else(|| {
                EngineError::Unsupported("rollout stalled before its turn cutoff".into())
            })?;
            let decision = worker.first_resolution_choice(owner)?.ok_or_else(|| {
                EngineError::Unsupported("rollout decision has no native choices".into())
            })?;
            if !self.apply_input(worker, &decision)? {
                return Ok(None);
            }
        }
    }

    fn value(
        &mut self,
        worker: &mut NativeSession,
        depth: u8,
        mut alpha: i64,
        mut beta: i64,
    ) -> Result<Option<ValueBound>> {
        let (team, moves) = loop {
            if let Some(cutoff) = self.turn_cutoff {
                if worker.search_turn_index() >= cutoff {
                    return Ok(Some(ValueBound {
                        score: worker.evaluate_resolution(self.team, self.player),
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
            let terminal = worker.last_request.is_none()
                || matches!(worker.planning.phase, Phase::Planning | Phase::GameOver);
            if terminal {
                return Ok(Some(ValueBound {
                    score: worker.evaluate_resolution(self.team, self.player),
                    resolved: true,
                }));
            }
            let (player, team) = worker
                .search_player()?
                .expect("live request has a search owner");
            let moves = worker.resolution_choices(player)?;
            if moves.is_empty() {
                return Err(EngineError::Unsupported(
                    "search decision has no native choices".into(),
                ));
            }
            if moves.len() > 1 {
                break (team, moves);
            }
            if !self.apply_input(worker, &moves[0])? {
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
                score: worker.evaluate_resolution(self.team, self.player),
                resolved: false,
            }));
        }
        let maximizing = team == self.team;
        if !maximizing && matches!(self.opponent_policy, OpponentPolicy::FirstLegal) {
            return self.branch(worker, &moves[0], depth - 1, alpha, beta);
        }
        let mut best = if maximizing { i64::MIN } else { i64::MAX };
        let mut resolved = true;
        for (index, decision) in moves.iter().enumerate() {
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
            if alpha >= beta && index + 1 < moves.len() {
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

    #[test]
    fn exact_terminal_root_score_can_be_reused_without_expanding_nodes() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-confirmation-repeated-step",
        );
        let mut worker =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let budget = ExecutionBudget::default();
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
            turn_cutoff: None,
        };
        let mut cache = [None];
        let shallow = search
            .choose(&mut worker, &moves, 1, 0, &mut cache)
            .unwrap();
        assert!(!shallow.complete && !shallow.resolved);
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
                };
                let ordinary = worker.search_resolution(owner, limits, &budget)?;
                let tiny = SearchLimits {
                    max_nodes: 1,
                    ..limits
                };
                let fallback =
                    worker.search_resolution_with_turn_rollout(owner, tiny, Some(2), &budget)?;
                assert!(fallback.score.is_some());
                assert_eq!(fallback.score_kind, Some(ResolutionScoreKind::Immediate));
                assert_eq!((fallback.completed_depth, fallback.nodes), (0, 1));
                assert!(choices.contains(&fallback.decision));
                assert_eq!(worker.semantic_state()?, before);
                let rolled =
                    worker.search_resolution_with_turn_rollout(owner, limits, Some(2), &budget)?;
                assert!(rolled.score.is_some());
                assert_eq!(rolled.score_kind, Some(ResolutionScoreKind::TurnRollout));
                assert_eq!(rolled.completed_depth, 1);
                assert!(rolled.nodes > ordinary.nodes);
                assert!(choices.contains(&rolled.decision));
                assert_eq!(worker.semantic_state()?, before);
                Ok(())
            })
            .unwrap();
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
            turn_cutoff: None,
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
            turn_cutoff: None,
        };
        let fixed = search
            .value(&mut worker, 1, i64::MIN, i64::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(search.nodes, 1);
        let first = search
            .branch(&mut worker, &moves[0], 0, i64::MIN, i64::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(fixed.score, first.score);
        search.nodes = 0;
        search.opponent_policy = OpponentPolicy::Adversarial;
        let adversarial = search
            .value(&mut worker, 1, i64::MIN, i64::MAX)
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
            turn_cutoff: None,
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
        assert!(!limited.complete);
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
        assert!(ordered.complete && baseline.complete);
        assert_eq!(ordered.best, baseline.best);
    }
}
