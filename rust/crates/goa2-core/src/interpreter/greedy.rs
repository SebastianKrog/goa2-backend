//! Fixed, full-information opponent policy for offline greedy search.
use super::{DecisionMove, EngineError, NativeSession, PlanningCommitment, RequestChoices, Result};
use crate::{
    cards::CardId,
    data::{Datum, Symbol},
    stats::Stat,
    world::ActionKind,
};

impl NativeSession {
    /// Choose one legal commitment, preserving primary defense cards whenever possible.
    pub fn greedy_opponent_commitment(
        &self,
        hero: Symbol,
        seed: u64,
    ) -> Result<Option<PlanningCommitment>> {
        let mut cards = Vec::new();
        let mut can_pass = false;
        let mut can_finish = false;
        for choice in self.planning_choices(hero)? {
            match choice {
                DecisionMove::Card { card, .. } => cards.push(card),
                DecisionMove::Pass { .. } => can_pass = true,
                DecisionMove::FinishPlanning { .. } => can_finish = true,
                _ => unreachable!("planning choices are commitments"),
            }
        }
        if can_finish {
            return Ok(Some(PlanningCommitment {
                hero,
                first: None,
                second: None,
            }));
        }
        let first = self.greedy_lowest_defense(hero, cards, seed)?;
        Ok((first.is_some() || can_pass).then_some(PlanningCommitment {
            hero,
            first,
            second: None,
        }))
    }

    /// Choose from the offered legal inputs. The seed is supplied by the rollout
    /// so competing plans can reuse the same random choices.
    pub fn greedy_opponent_input(
        &mut self,
        hero: Symbol,
        seed: u64,
    ) -> Result<Option<DecisionMove>> {
        let moves = self.resolution_choices(hero)?;
        if moves.is_empty() {
            return Ok(None);
        }
        let request = self
            .last_request
            .as_ref()
            .expect("choices require a request");
        let name = |decision: &DecisionMove| match decision {
            DecisionMove::Input {
                selection: Datum::Text(id),
                ..
            } => self.symbols.name(*id),
            _ => "",
        };
        if matches!(request.choices, RequestChoices::Action(_)) {
            return moves
                .iter()
                .find(|decision| name(decision) == "HOLD")
                .cloned()
                .map(Some)
                .ok_or_else(|| {
                    EngineError::Unsupported(
                        "greedy policy expected Hold in the action menu".into(),
                    )
                });
        }
        if let RequestChoices::Upgrades(players) = &request.choices
            && let Some(player) = players.iter().find(|row| row.hero == hero)
        {
            let card_key = self.symbols.lookup("card_id");
            let preferred: Vec<_> = moves
                .iter()
                .filter(|decision| {
                    let DecisionMove::Input {
                        selection: Datum::Object(fields),
                        ..
                    } = decision
                    else {
                        return false;
                    };
                    let chosen = fields.iter().find_map(|(key, value)| match value {
                        Datum::Text(id) if Some(*key) == card_key => Some(*id),
                        _ => None,
                    });
                    player.options.iter().any(|option| {
                        let mut definitions = option.pair.iter().filter_map(|id| {
                            self.world
                                .card(*id)
                                .map(|card| &self.world.definitions.cards[card.definition])
                        });
                        let (Some(a), Some(b)) = (definitions.next(), definitions.next()) else {
                            return false;
                        };
                        (chosen == Some(a.printed_id) && b.item == Some(Stat::Defense))
                            || (chosen == Some(b.printed_id) && a.item == Some(Stat::Defense))
                    })
                })
                .collect();
            if !preferred.is_empty() {
                return Ok(Some(
                    preferred[super::loadout::sample_index(seed, preferred.len())].clone(),
                ));
            }
        }
        if request.discard && !request.can_skip {
            let victim = match &request.choices {
                RequestChoices::CardIds {
                    forced_victim,
                    defeat_victim,
                    ..
                } => forced_victim.or(*defeat_victim),
                _ => None,
            }
            .or(request.controlled)
            .unwrap_or(hero);
            if let Some(row) = self.world.hero(victim) {
                let eligible =
                    row.hand.iter().copied().filter(|id| {
                        let printed = self.world.definitions.cards
                            [self.world.card(*id).unwrap().definition]
                            .printed_id;
                        moves.iter().any(|decision| matches!(
                        decision,
                        DecisionMove::Input { selection: Datum::Text(id), .. } if *id == printed
                    ))
                    });
                if let Some(card) = self.greedy_lowest_defense(victim, eligible, seed)? {
                    let printed = self.world.definitions.cards
                        [self.world.card(card).unwrap().definition]
                        .printed_id;
                    return Ok(moves.into_iter().find(|decision| {
                        matches!(
                            decision,
                            DecisionMove::Input { selection: Datum::Text(id), .. } if *id == printed
                        )
                    }));
                }
            }
        }
        let skipped = moves.iter().find(|decision| {
            let choice = name(decision);
            choice == "SKIP"
                || choice == "PASS"
                || choice == "DONE"
                || (choice == "NO"
                    && matches!(
                        request.choices,
                        RequestChoices::YesNo
                            | RequestChoices::Repeat { .. }
                            | RequestChoices::Passive { .. }
                            | RequestChoices::Laugh
                    ))
        });
        Ok(Some(
            skipped
                .unwrap_or_else(|| &moves[super::loadout::sample_index(seed, moves.len())])
                .clone(),
        ))
    }

    fn greedy_lowest_defense(
        &self,
        hero: Symbol,
        cards: impl IntoIterator<Item = CardId>,
        seed: u64,
    ) -> Result<Option<CardId>> {
        let mut best = None;
        let mut ties = Vec::new();
        for id in cards {
            let rank = self.greedy_defense_rank(hero, id)?;
            if best.is_none_or(|best| rank < best) {
                best = Some(rank);
                ties.clear();
            }
            if best == Some(rank) {
                ties.push(id);
            }
        }
        Ok((!ties.is_empty()).then(|| ties[super::loadout::sample_index(seed, ties.len())]))
    }

    pub(super) fn greedy_defense_rank(&self, hero: Symbol, id: CardId) -> Result<(bool, i64)> {
        let card = self
            .world
            .card(id)
            .ok_or_else(|| EngineError::Unsupported("greedy policy card is not bound".into()))?;
        let def = &self.world.definitions.cards[card.definition];
        let primary = !card.facedown
            && matches!(
                def.primary,
                Some(ActionKind::Defense | ActionKind::DefenseSkill)
            );
        let base = if card.facedown {
            0
        } else if primary {
            def.primary_value.unwrap_or(0)
        } else {
            def.secondary
                .iter()
                .find_map(|(action, value)| (*action == ActionKind::Defense).then_some(*value))
                .unwrap_or(0)
        };
        let defense = self
            .world
            .computed_stat_for_card(hero, Stat::Defense, base, Some(id))
            .map_err(EngineError::Unsupported)?;
        Ok((primary, defense))
    }
}

/// A replayable planning batch or input in a full-information greedy trace.
#[derive(Clone, Debug, PartialEq)]
pub enum GreedyStep {
    Planning(Vec<PlanningCommitment>),
    Input(DecisionMove),
}

#[derive(Clone, Copy, Debug)]
pub struct GreedySearchOptions<'a> {
    /// Include the searching hero. Additional heroes must be on their team.
    pub controlled: &'a [Symbol],
    pub turns: u8,
    /// Benchmark baseline using the first legal controlled commitment batch.
    pub fixed_card_order: bool,
    pub defer_upgrades: bool,
    pub positional_ordering: bool,
    pub position_first: bool,
    pub tactical_replies: bool,
    pub reuse_movement: bool,
    pub profile: bool,
    pub max_nodes: usize,
    pub seed: u64,
    pub evaluation: super::SearchEvaluation,
}

#[derive(Clone, Debug)]
pub struct GreedyPlan {
    pub steps: Vec<GreedyStep>,
    pub score: i64,
}

type CardSequence = [Option<PlanningCommitment>; 4];

#[derive(Debug, Default)]
pub struct GreedySearch {
    pub best: Option<GreedyPlan>,
    card_sequence_scores: Option<std::collections::HashMap<CardSequence, i64>>,
    pub nodes: usize,
    pub completed_sequences: usize,
    pub positional_probes: usize,
    pub position_routes: usize,
    pub position_groups: usize,
    pub upgraded_position_groups: usize,
    pub upgrade_discovery_steps: usize,
    pub shared_upgrade_decisions: usize,
    pub resumed_position_routes: usize,
    pub shared_upgrade_commitments: usize,
    pub shared_card_moves: usize,
    pub movement_cache_hits: usize,
    pub movement_cache_misses: usize,
    pub travel_cache_hits: usize,
    pub travel_cache_misses: usize,
    pub profile: Option<GreedyProfile>,
    pub tactical_probes: usize,
    pub tactical_target_reuses: usize,
}

impl GreedySearch {
    pub fn turn_card_scores(&self, offset: usize) -> Vec<(Option<CardId>, i64)> {
        self.card_scores_at(offset, &[])
    }

    fn card_scores_at(
        &self,
        offset: usize,
        history: &[Option<PlanningCommitment>],
    ) -> Vec<(Option<CardId>, i64)> {
        let mut scores: Vec<(Option<CardId>, i64)> = Vec::new();
        if let Some(sequences) = &self.card_sequence_scores {
            for (sequence, &score) in sequences {
                if !sequence.starts_with(history) {
                    continue;
                }
                let Some(Some(choice)) = sequence.get(offset) else {
                    continue;
                };
                if let Some((_, best)) = scores.iter_mut().find(|(card, _)| *card == choice.first) {
                    *best = (*best).max(score);
                } else {
                    scores.push((choice.first, score));
                }
            }
        }
        scores.sort_by_key(|(card, _)| card.map(|id| id.0));
        scores
    }

    /// Normalize cards seen in completed plans at one offset.
    pub fn turn_card_likelihoods(&self, offset: usize) -> Option<Vec<(Option<CardId>, f64)>> {
        let scores = self.turn_card_scores(offset);
        let cards = scores.iter().map(|(card, _)| *card);
        Self::normalize_card_scores(&scores, cards)
    }

    /// Predict the next card given the previous planning commitments and
    /// the distinct legal first choices on the actual branch. No matching
    /// legal forecast lets the caller fall back to its ordinary policy.
    pub fn next_card_likelihoods(
        &self,
        history: &[Option<PlanningCommitment>],
        legal: &[Option<CardId>],
    ) -> Option<Vec<(Option<CardId>, f64)>> {
        let mut scores = self.card_scores_at(history.len(), history);
        scores.retain(|(card, _)| legal.contains(card));
        if scores.is_empty() {
            return None;
        }
        Self::normalize_card_scores(&scores, legal.iter().copied())
    }

    pub(super) fn preferred_next_card(
        &self,
        history: &[Option<PlanningCommitment>],
        legal: &[Option<CardId>],
    ) -> Option<Option<CardId>> {
        let scores = self.card_scores_at(history.len(), history);
        legal
            .iter()
            .filter_map(|&card| {
                let score = scores
                    .iter()
                    .find_map(|&(id, score)| (id == card).then_some(score))?;
                Some((card, score))
            })
            .rev()
            .max_by_key(|(_, score)| *score)
            .map(|(card, _)| card)
    }

    fn normalize_card_scores(
        scores: &[(Option<CardId>, i64)],
        legal: impl Iterator<Item = Option<CardId>>,
    ) -> Option<Vec<(Option<CardId>, f64)>> {
        use super::search::{GREEDY_CARD_TEMPERATURE, greedy_card_cost};

        let best = scores.iter().map(|(_, score)| *score).max();
        let mut weights: Vec<_> = legal
            .map(|card| {
                let cost = best.map_or(0, |best| {
                    let score = scores
                        .iter()
                        .find_map(|&(id, score)| (id == card).then_some(score));
                    greedy_card_cost(best, score)
                });
                (
                    card,
                    (-(cost as f64) / GREEDY_CARD_TEMPERATURE as f64).exp(),
                )
            })
            .collect();
        if weights.is_empty() {
            return None;
        }
        let total: f64 = weights.iter().map(|(_, weight)| *weight).sum();
        for (_, weight) in &mut weights {
            *weight /= total;
        }
        Some(weights)
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct GreedyProfile {
    pub search_ns: u64,
    pub first_plan_ns: Option<u64>,
    pub incumbent_nodes: usize,
    pub advance_ns: u64,
    pub advance_calls: usize,
    pub clone_ns: u64,
    pub clones: usize,
    pub grouping_ns: u64,
    pub replay_ns: u64,
    pub replayed_steps: usize,
    pub largest_upgrade_batch: usize,
    card_sync: super::object_execution::CardSyncProfile,
    /// Inclusive dispatch time by step type, including guards and prompt attempts.
    rule_steps: super::object_execution::RuleStepProfiles,
}

fn elapsed_ns(start: std::time::Instant) -> u64 {
    start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

impl NativeSession {
    /// Replay before encoding because a choice can reference symbols created
    /// after the root. Return the final state for offline oracle comparison.
    pub fn greedy_trace_json(&mut self, plan: &GreedyPlan) -> Result<serde_json::Value> {
        use serde_json::json;

        let budget = super::ExecutionBudget::default();
        self.with_rollback(|worker| {
            let mut trace = Vec::with_capacity(plan.steps.len());
            for step in &plan.steps {
                worker.advance_to_decision(&budget)?;
                match step {
                    GreedyStep::Planning(commitments) => {
                        let mut ordered = commitments.clone();
                        ordered.sort_by_key(|choice| {
                            worker
                                .world
                                .heroes
                                .iter()
                                .position(|hero| hero.id == choice.hero)
                        });
                        trace.push(worker.planning_trace_json(&ordered)?);
                        worker.apply_joint_planning_ordered(&ordered)?;
                    }
                    GreedyStep::Input(
                        decision @ DecisionMove::Input {
                            player,
                            request: Some(token),
                            selection,
                        },
                    ) => {
                        if !worker.resolution_choices(*player)?.contains(decision) {
                            return Err(EngineError::InvalidInput(
                                "invalid greedy trace input".into(),
                            ));
                        }
                        trace.push(json!({
                            "kind": "resolution", "hero": worker.symbols.name(*player),
                            "selection": worker.selection_json(selection),
                        }));
                        worker.submit(*token, selection.clone())?;
                        worker.last_request = None;
                    }
                    GreedyStep::Input(_) => {
                        return Err(EngineError::InvalidInput(
                            "invalid greedy trace input".into(),
                        ));
                    }
                }
            }
            worker.advance_to_decision(&budget)?;
            Ok(json!({ "trace": trace, "state": worker.semantic_state()? }))
        })
    }

    /// Search controlled card orders and actions against the fixed opponent
    /// policy. The returned trace is fully scored at the turn cutoff.
    pub fn search_greedy(
        &mut self,
        player: Symbol,
        options: GreedySearchOptions<'_>,
        budget: &super::ExecutionBudget<'_>,
    ) -> Result<GreedySearch> {
        self.search_greedy_inner(player, options, budget, false)
    }

    /// Collect the best completed score for each card at each planning offset.
    /// A missing score means no completed rollout used that card at that offset.
    pub fn search_greedy_card_scores(
        &mut self,
        player: Symbol,
        options: GreedySearchOptions<'_>,
        budget: &super::ExecutionBudget<'_>,
    ) -> Result<GreedySearch> {
        self.search_greedy_inner(player, options, budget, true)
    }

    fn search_greedy_inner(
        &mut self,
        player: Symbol,
        options: GreedySearchOptions<'_>,
        budget: &super::ExecutionBudget<'_>,
        collect_card_scores: bool,
    ) -> Result<GreedySearch> {
        let team = self
            .world
            .hero(player)
            .filter(|row| row.id == player)
            .ok_or_else(|| EngineError::InvalidInput("greedy player must be a hero".into()))?
            .team;
        if !(1..=4).contains(&options.turns)
            || options.max_nodes == 0
            || !options.controlled.contains(&player)
            || options.controlled.iter().any(|id| {
                self.world
                    .hero(*id)
                    .is_none_or(|row| row.id != *id || row.team != team)
            })
        {
            return Err(EngineError::InvalidInput(
                "invalid greedy search scope or limits".into(),
            ));
        }
        let mut search = GreedyTraversal {
            options,
            budget,
            player,
            team,
            cutoff: self.search_turn_index() + i128::from(options.turns),
            result: GreedySearch {
                card_sequence_scores: collect_card_scores.then(Default::default),
                profile: options.profile.then(GreedyProfile::default),
                ..GreedySearch::default()
            },
            started: options.profile.then(std::time::Instant::now),
            path: Vec::new(),
            incumbent_only: false,
        };
        let mut visit = || {
            if options.reuse_movement {
                let definitions = self.world.definitions.clone();
                let (outcome, reuse) = crate::terrain::with_search_traversals(definitions, || {
                    self.with_rollback(|worker| search.run(worker))
                });
                search.result.movement_cache_hits = reuse.movement_hits;
                search.result.movement_cache_misses = reuse.movement_misses;
                search.result.travel_cache_hits = reuse.travel_hits;
                search.result.travel_cache_misses = reuse.travel_misses;
                outcome
            } else {
                self.with_rollback(|worker| search.run(worker))
            }
        };
        let outcome = if options.profile {
            let ((outcome, sync), rule_steps) =
                super::object_execution::with_rule_step_profile(|| {
                    super::object_execution::with_card_sync_profile(visit)
                });
            let profile = search.result.profile.as_mut().unwrap();
            profile.card_sync = sync;
            profile.rule_steps = rule_steps;
            outcome
        } else {
            visit()
        };
        if let (Some(start), Some(profile)) = (search.started, &mut search.result.profile) {
            profile.search_ns = elapsed_ns(start);
        }
        match outcome {
            Ok(()) | Err(EngineError::Deadline | EngineError::Cancelled) => Ok(search.result),
            Err(error) => Err(error),
        }
    }
}

struct UpgradeCandidateChanges<'a> {
    chosen: CardId,
    replaced: Option<CardId>,
    item: Option<CardId>,
    final_hand: Option<&'a [CardId]>,
}

impl UpgradeCandidateChanges<'_> {
    fn preserves(&self, card: CardId) -> bool {
        Some(card) != self.replaced
            && Some(card) != self.item
            && self
                .final_hand
                .is_none_or(|hand| card == self.chosen || hand.contains(&card))
    }
}

struct GreedyTraversal<'a, 'b, 'c> {
    options: GreedySearchOptions<'a>,
    budget: &'b super::ExecutionBudget<'c>,
    player: Symbol,
    team: Symbol,
    cutoff: i128,
    result: GreedySearch,
    path: Vec<GreedyStep>,
    incumbent_only: bool,
    started: Option<std::time::Instant>,
}

struct TacticalReply {
    input: DecisionMove,
    target: Option<DecisionMove>,
}

#[derive(Hash, PartialEq)]
struct UpgradeBoundary {
    menu: super::NativeRequest,
    locations: Vec<(Symbol, crate::geometry::Hex)>,
    score: i64,
}

struct UpgradeWorld {
    worker: NativeSession,
    steps: Vec<GreedyStep>,
    target: Option<DecisionMove>,
}

struct PositionRoute {
    resume: Option<Box<NativeSession>>,
    terminal: bool,
    destination: Vec<Vec<crate::geometry::Hex>>,
    score: i64,
    steps: Vec<GreedyStep>,
}

struct PositionRouteFrame {
    moves: std::vec::IntoIter<DecisionMove>,
    next_target: Option<DecisionMove>,
    child: Option<super::object_execution::RollbackCheckpoint>,
}

impl GreedyTraversal<'_, '_, '_> {
    fn profile_clock(&self) -> Option<std::time::Instant> {
        self.started.map(|_| std::time::Instant::now())
    }

    fn advance_profiled(&mut self, worker: &mut NativeSession) -> Result<()> {
        let start = self.profile_clock();
        let result = worker.advance_to_decision(self.budget);
        if let (Some(start), Some(profile)) = (start, &mut self.result.profile) {
            profile.advance_ns += elapsed_ns(start);
            profile.advance_calls += 1;
        }
        result
    }

    fn fork_profiled(&mut self, worker: &NativeSession) -> NativeSession {
        let start = self.profile_clock();
        let fork = worker.clone();
        if let (Some(start), Some(profile)) = (start, &mut self.result.profile) {
            profile.clone_ns += elapsed_ns(start);
            profile.clones += 1;
        }
        fork
    }

    fn run(&mut self, worker: &mut NativeSession) -> Result<()> {
        let limit = self.options.max_nodes;
        let seed_divisor = if self.options.turns == 4 { 2 } else { 8 };
        let seed_limit = (limit / seed_divisor).min(128);
        if self.result.card_sequence_scores.is_some()
            && self.options.controlled == [self.player]
            && !self.options.fixed_card_order
            && worker.planning.phase == crate::codec::Phase::Planning
            && self.seed_first_cards(worker, limit)?
        {
            return self.visit(worker, limit);
        }
        if self.options.position_first && !self.options.fixed_card_order && seed_limit >= 32 {
            // Complete one legal line before spending the remaining budget
            // collecting routes. Charge both passes to the same node limit.
            self.incumbent_only = true;
            let result = worker.with_rollback(|worker| self.visit(worker, seed_limit));
            self.incumbent_only = false;
            if let Some(profile) = &mut self.result.profile {
                profile.incumbent_nodes = self.result.nodes;
            }
            result?;
        }
        self.visit(worker, limit)
    }

    fn seed_first_cards(&mut self, worker: &mut NativeSession, limit: usize) -> Result<bool> {
        let mut seen = Vec::new();
        let plans: Vec<_> = worker
            .hero_planning_commitments(self.player)?
            .into_iter()
            .filter(|plan| {
                if seen.contains(&plan.first) {
                    return false;
                }
                seen.push(plan.first);
                true
            })
            .collect();
        let quota = (limit / 2 / plans.len().max(1)).min(128);
        if plans.is_empty() || quota < 32 {
            return Ok(false);
        }
        let opponents = self.opponent_commitments(worker)?;
        self.incumbent_only = true;
        for plan in plans {
            let mut commitments = vec![plan];
            commitments.extend_from_slice(&opponents);
            let branch_limit = self.result.nodes + quota;
            self.result.nodes += 1;
            self.path.push(GreedyStep::Planning(commitments.clone()));
            let result =
                worker.with_joint_planning(&commitments, |branch| self.visit(branch, branch_limit));
            self.path.pop();
            if let Err(error) = result {
                self.incumbent_only = false;
                return Err(error);
            }
        }
        self.incumbent_only = false;
        if let Some(profile) = &mut self.result.profile {
            profile.incumbent_nodes = self.result.nodes;
        }
        Ok(true)
    }

    fn visit(&mut self, worker: &mut NativeSession, limit: usize) -> Result<()> {
        self.visit_with_target(worker, limit, None)
    }

    fn visit_with_target(
        &mut self,
        worker: &mut NativeSession,
        limit: usize,
        target: Option<DecisionMove>,
    ) -> Result<()> {
        let base = self.path.len();
        let result = self.visit_fixed_prefix(worker, limit, target);
        self.path.truncate(base);
        result
    }

    fn visit_fixed_prefix(
        &mut self,
        worker: &mut NativeSession,
        limit: usize,
        target: Option<DecisionMove>,
    ) -> Result<()> {
        use crate::codec::Phase;

        loop {
            self.budget.check(0)?;
            self.advance_profiled(worker)?;
            if self.options.tactical_replies
                || target.is_some()
                || matches!(worker.planning.phase, Phase::Planning | Phase::GameOver)
                || worker.search_turn_index() >= self.cutoff
                || self.result.nodes >= limit
                || matches!(
                    worker.last_request.as_ref().map(|request| &request.choices),
                    Some(RequestChoices::Upgrades(_))
                )
            {
                break;
            }
            let Some((owner, _)) = worker.search_player()? else {
                break;
            };
            if self.options.controlled.contains(&owner) {
                break;
            }
            let decision = worker
                .greedy_opponent_input(owner, self.policy_seed(worker, owner))?
                .ok_or_else(|| {
                    EngineError::Unsupported("greedy search found no legal continuation".into())
                })?;
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = &decision
            else {
                unreachable!("fixed choices are issued inputs");
            };
            let (token, selection) = (*token, selection.clone());
            self.result.nodes += 1;
            self.path.push(GreedyStep::Input(decision));
            // The caller already owns a rollback scope or a private fork.
            worker.last_request = None;
            worker.submit(token, selection)?;
        }
        self.visit_ready(worker, limit, target)
    }

    fn visit_ready(
        &mut self,
        worker: &mut NativeSession,
        limit: usize,
        target: Option<DecisionMove>,
    ) -> Result<()> {
        use crate::codec::Phase;

        if worker.planning.phase == Phase::GameOver || worker.search_turn_index() >= self.cutoff {
            let score = worker.search_score(self.options.evaluation, self.team, self.player)?;
            self.record_leaf(score);
            return Ok(());
        }
        if self.result.nodes >= limit {
            return Ok(());
        }
        if !self.incumbent_only
            && self.options.defer_upgrades
            && matches!(
                worker.last_request.as_ref().map(|request| &request.choices),
                Some(RequestChoices::Upgrades(_))
            )
            && self.visit_deferred_upgrades(worker, limit)?
        {
            return Ok(());
        }
        if !self.incumbent_only
            && self.options.position_first
            && !self.options.fixed_card_order
            && self.path.is_empty()
            && worker.planning.phase == Phase::Planning
        {
            self.explore_position_routes(worker, limit)?;
        }
        let planning = worker.planning.phase == Phase::Planning;
        let mut next_target = None;
        let candidates = if planning {
            let fixed = self.opponent_commitments(worker)?;
            self.controlled_plans(worker, limit - self.result.nodes)?
                .into_iter()
                .map(|mut plan| {
                    plan.extend_from_slice(&fixed);
                    GreedyStep::Planning(plan)
                })
                .collect::<Vec<_>>()
        } else {
            let (owner, _) = worker.search_player()?.ok_or_else(|| {
                EngineError::Unsupported("greedy search stalled before its turn cutoff".into())
            })?;
            let (mut moves, followup) = self.search_input_choices(worker, owner, limit, target)?;
            next_target = followup;
            if self.options.positional_ordering {
                if self.incumbent_only {
                    worker.order_movement_roots(owner, &mut moves);
                } else {
                    self.order_position_choices(worker, owner, &mut moves, limit)?;
                }
            }
            moves.into_iter().map(GreedyStep::Input).collect()
        };
        if candidates.is_empty() {
            return Err(EngineError::Unsupported(
                "greedy search found no legal continuation".into(),
            ));
        }

        let completed = self.result.completed_sequences;
        let count = candidates.len();
        // Preserve room for another card after a later planning boundary.
        // Small branches still prioritize finishing one complete rollout.
        let balance_cards = planning
            && self.result.card_sequence_scores.is_some()
            && (limit - self.result.nodes) / count >= 32;
        for (index, step) in candidates.into_iter().enumerate() {
            if self.result.nodes >= limit {
                break;
            }
            // Reserve work for competing root choices instead of exhausting
            // the budget under the first card order.
            let branch_limit = self.branch_limit(limit, count - index, balance_cards);
            self.result.nodes += 1;
            self.path.push(step.clone());
            let result = match step {
                GreedyStep::Planning(commitments) => worker
                    .with_joint_planning(&commitments, |branch| self.visit(branch, branch_limit)),
                GreedyStep::Input(DecisionMove::Input {
                    request: Some(token),
                    selection,
                    ..
                }) => worker.with_consumed_request(|branch| {
                    branch.submit(token, selection)?;
                    self.visit_with_target(branch, branch_limit, next_target.take())
                }),
                GreedyStep::Input(_) => unreachable!("resolution choices are issued inputs"),
            };
            self.path.pop();
            result?;
            if self.incumbent_only && self.result.completed_sequences > completed {
                break;
            }
        }
        Ok(())
    }

    fn record_leaf(&mut self, score: i64) {
        if let (Some(start), Some(profile)) = (self.started, &mut self.result.profile) {
            profile
                .first_plan_ns
                .get_or_insert_with(|| elapsed_ns(start));
        }
        self.result.completed_sequences += 1;
        if let Some(scores) = &mut self.result.card_sequence_scores {
            let mut sequence = [None; 4];
            let planning = self.path.iter().filter_map(|step| match step {
                GreedyStep::Planning(commitments) => Some(commitments),
                GreedyStep::Input(_) => None,
            });
            for (slot, commitments) in sequence.iter_mut().zip(planning) {
                *slot = commitments
                    .iter()
                    .find(|row| row.hero == self.player)
                    .copied();
            }
            scores
                .entry(sequence)
                .and_modify(|best| *best = (*best).max(score))
                .or_insert(score);
        }
        if self
            .result
            .best
            .as_ref()
            .is_none_or(|best| score > best.score)
        {
            self.result.best = Some(GreedyPlan {
                steps: self.path.clone(),
                score,
            });
        }
    }

    fn short_discovery_deadline(&self) -> Option<std::time::Instant> {
        let now = std::time::Instant::now();
        let time_left = self.budget.deadline?.saturating_duration_since(now);
        (time_left <= std::time::Duration::from_millis(200)).then_some(now + time_left / 4)
    }

    fn explore_position_routes(&mut self, worker: &mut NativeSession, limit: usize) -> Result<()> {
        let remaining = limit - self.result.nodes;
        if remaining < 128 {
            return Ok(());
        }
        let discovery_limit = self.result.nodes + remaining / 4;
        let follow_limit = self.result.nodes + remaining / 2;
        let discovery_deadline = self.short_discovery_deadline();
        let controlled = self.controlled_plans(worker, (remaining / 64).min(32))?;
        let fixed = self.opponent_commitments(worker)?;
        let turn = worker.search_turn_index();
        let mut routes = Vec::new();
        let count = controlled.len();
        for (index, mut plan) in controlled.into_iter().enumerate() {
            if self.result.nodes == discovery_limit
                || (!routes.is_empty()
                    && discovery_deadline
                        .is_some_and(|deadline| std::time::Instant::now() >= deadline))
            {
                break;
            }
            let quota = ((discovery_limit - self.result.nodes) / (count - index)).max(1);
            // Recheck the wall-time split between bounded collection chunks.
            let quota = if discovery_deadline.is_some() {
                quota.min(128)
            } else {
                quota
            };
            let plan_limit = self.result.nodes + quota;
            plan.extend_from_slice(&fixed);
            self.result.nodes += 1;
            self.path.push(GreedyStep::Planning(plan.clone()));
            let result = worker.with_joint_planning(&plan, |branch| {
                self.collect_position_routes(branch, turn, plan_limit, 0, &mut routes, None)
            });
            self.path.pop();
            result?;
        }

        self.follow_position_routes(worker, follow_limit, routes, false)
    }

    fn follow_position_routes(
        &mut self,
        worker: &mut NativeSession,
        follow_limit: usize,
        routes: Vec<PositionRoute>,
        upgraded: bool,
    ) -> Result<()> {
        let mut indices = std::collections::HashMap::new();
        let mut groups: Vec<Vec<PositionRoute>> = Vec::new();
        for route in routes {
            let next = groups.len();
            let index = *indices.entry(route.destination.clone()).or_insert(next);
            if index == next {
                groups.push(Vec::new());
            }
            groups[index].push(route);
        }
        self.result.position_groups += groups.len();
        if upgraded {
            self.result.upgraded_position_groups += groups.len();
        }
        for group in &mut groups {
            group.sort_by_key(|route| std::cmp::Reverse(route.score));
            if upgraded {
                // Give distinct upgrade histories a chance to share the
                // next commitment before repeating routes from one history.
                let mut seen = Vec::new();
                group.sort_by_cached_key(|route| {
                    let prefix: Vec<_> = route
                        .steps
                        .iter()
                        .take_while(|step| !matches!(step, GreedyStep::Planning(_)))
                        .cloned()
                        .collect();
                    let repeated = seen.contains(&prefix);
                    if !repeated && !route.terminal {
                        seen.push(prefix);
                    }
                    (route.terminal, repeated)
                });
            }
        }
        groups.sort_by_key(|group| std::cmp::Reverse(group[0].score));

        // Try the strongest route to each destination before considering
        // another route to the same place. Each still has its own state.
        let mut groups: Vec<_> = groups.into_iter().map(Vec::into_iter).collect();
        let mut first_pass = true;
        loop {
            let mut advanced = false;
            let count = groups.len();
            for (index, group) in groups.iter_mut().enumerate() {
                let Some(mut route) = group.next() else {
                    continue;
                };
                advanced = true;
                if route.terminal {
                    continue;
                }
                let replay_cost = if route.resume.is_some() {
                    0
                } else {
                    route.steps.len()
                };
                let remaining = follow_limit - self.result.nodes;
                if replay_cost >= remaining {
                    continue;
                }
                let minimum = if upgraded {
                    group
                        .as_slice()
                        .first()
                        .filter(|next| !next.terminal)
                        .map_or(replay_cost + 8, |next| {
                            replay_cost
                                + if next.resume.is_some() {
                                    0
                                } else {
                                    next.steps.len()
                                }
                                + 32
                        })
                } else {
                    replay_cost + 8
                };
                let quota = if first_pass {
                    (remaining / (count - index)).max(minimum).min(remaining)
                } else {
                    remaining
                };
                let branch_limit = self.result.nodes + quota;
                if upgraded && group.as_slice().first().is_some_and(|next| !next.terminal) {
                    let mut batch = vec![route];
                    let mut cost = replay_cost;
                    while batch.len() < 4 && branch_limit - self.result.nodes > cost + 8 {
                        let Some(next) = group.as_slice().first() else {
                            break;
                        };
                        let next_cost = if next.resume.is_some() {
                            0
                        } else {
                            next.steps.len()
                        };
                        if next.terminal || branch_limit - self.result.nodes <= cost + next_cost + 8
                        {
                            break;
                        }
                        cost += next_cost;
                        batch.push(group.next().unwrap());
                    }
                    self.follow_upgrade_planning(worker, batch, branch_limit)?;
                    continue;
                }
                let base = self.path.len();
                if let Some(mut state) = route.resume.take() {
                    self.result.resumed_position_routes += 1;
                    self.path.extend(route.steps);
                    let result = self.visit(&mut state, branch_limit);
                    self.path.truncate(base);
                    result?;
                    continue;
                }
                self.result.nodes += replay_cost;
                let result = worker.with_rollback(|branch| {
                    self.replay_position_steps(branch, &route.steps)?;
                    self.path.extend(route.steps);
                    self.visit(branch, branch_limit)
                });
                self.path.truncate(base);
                result?;
            }
            first_pass = false;
            if !advanced || self.result.nodes >= follow_limit {
                break;
            }
        }
        Ok(())
    }

    fn follow_upgrade_planning(
        &mut self,
        root: &mut NativeSession,
        routes: Vec<PositionRoute>,
        limit: usize,
    ) -> Result<()> {
        use crate::codec::Phase;
        use std::collections::HashMap;

        let mut worlds = Vec::new();
        for route in routes {
            let mut worker = if let Some(state) = route.resume {
                self.result.resumed_position_routes += 1;
                *state
            } else {
                self.result.nodes += route.steps.len();
                root.with_rollback(|branch| {
                    self.replay_position_steps(branch, &route.steps)?;
                    Ok(self.fork_profiled(branch))
                })?
            };
            self.advance_profiled(&mut worker)?;
            worlds.push(UpgradeWorld {
                worker,
                steps: route.steps,
                target: None,
            });
        }

        let base = self.path.len();
        let mut planning = Vec::new();
        for mut world in worlds {
            if world.worker.planning.phase == Phase::Planning
                && world.worker.search_turn_index() < self.cutoff
            {
                planning.push(world);
            } else {
                self.path.extend(world.steps);
                let result = self.visit(&mut world.worker, limit);
                self.path.truncate(base);
                result?;
            }
        }
        let mut indices = HashMap::new();
        let mut choices: Vec<Vec<(usize, Vec<PlanningCommitment>)>> = Vec::new();
        for (index, world) in planning.iter().enumerate() {
            let fixed = self.opponent_commitments(&world.worker)?;
            let cap = (limit - self.result.nodes).min(256);
            if cap == 0 {
                return Ok(());
            }
            for mut plan in self.controlled_plans(&world.worker, cap)? {
                let next = choices.len();
                let group = *indices
                    .entry((world.worker.search_turn_index(), plan.clone()))
                    .or_insert(next);
                if group == next {
                    choices.push(Vec::new());
                }
                plan.extend_from_slice(&fixed);
                choices[group].push((index, plan));
            }
        }
        let count = choices.len();
        for (index, group) in choices.into_iter().enumerate() {
            if self.result.nodes >= limit {
                break;
            }
            let available = limit - self.result.nodes;
            let quota = (available / (count - index))
                .max(group.len() * 12 + 4)
                .min(available);
            let branch_limit = self.result.nodes + quota;
            let mut children = Vec::new();
            for (world_index, plan) in group {
                if self.result.nodes >= branch_limit {
                    break;
                }
                self.result.nodes += 1;
                let world = &mut planning[world_index];
                let worker = world
                    .worker
                    .with_joint_planning(&plan, |branch| Ok(self.fork_profiled(branch)))?;
                let mut steps = world.steps.clone();
                steps.push(GreedyStep::Planning(plan));
                children.push(UpgradeWorld {
                    worker,
                    steps,
                    target: None,
                });
            }
            let upgrade_prefix = |world: &UpgradeWorld| {
                world
                    .steps
                    .iter()
                    .position(|step| matches!(step, GreedyStep::Planning(_)))
                    .unwrap_or(world.steps.len())
            };
            if let Some(first) = children.first()
                && children.iter().skip(1).any(|other| {
                    first.steps[..upgrade_prefix(first)] != other.steps[..upgrade_prefix(other)]
                })
            {
                self.result.shared_upgrade_commitments += 1;
            }
            if let Some(child) = children.first() {
                let turn = child.worker.search_turn_index();
                let mut continuations = Vec::new();
                self.collect_upgrade_world_routes(
                    children,
                    turn,
                    branch_limit,
                    base,
                    &mut continuations,
                )?;
                self.follow_position_routes(root, branch_limit, continuations, false)?;
            }
        }
        Ok(())
    }

    fn replay_position_steps(
        &mut self,
        branch: &mut NativeSession,
        steps: &[GreedyStep],
    ) -> Result<()> {
        let start = self.profile_clock();
        for step in steps {
            self.advance_profiled(branch)?;
            match step {
                GreedyStep::Planning(plan) => {
                    let mut ordered = plan.clone();
                    ordered.sort_by_key(|choice| {
                        branch
                            .world
                            .heroes
                            .iter()
                            .position(|hero| hero.id == choice.hero)
                    });
                    branch.apply_joint_planning_ordered(&ordered)?;
                }
                GreedyStep::Input(
                    decision @ DecisionMove::Input {
                        player,
                        request: Some(token),
                        selection,
                    },
                ) => {
                    if !branch.resolution_choices(*player)?.contains(decision) {
                        return Err(EngineError::InvalidInput("invalid position route".into()));
                    }
                    branch.submit(*token, selection.clone())?;
                    branch.last_request = None;
                }
                GreedyStep::Input(_) => {
                    unreachable!("position routes contain issued inputs")
                }
            }
        }
        if let (Some(start), Some(profile)) = (start, &mut self.result.profile) {
            profile.replay_ns += elapsed_ns(start);
            profile.replayed_steps += steps.len();
        }
        Ok(())
    }

    fn collect_position_routes(
        &mut self,
        worker: &mut NativeSession,
        turn: i128,
        limit: usize,
        base: usize,
        routes: &mut Vec<PositionRoute>,
        target: Option<DecisionMove>,
    ) -> Result<()> {
        let prefix = self.path.len();
        let mut frames = Vec::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(frame) =
                self.position_route_frame(worker, turn, limit, base, routes, target)?
            {
                frames.push(frame);
            }
            while let Some(frame) = frames.last_mut() {
                if let Some(checkpoint) = frame.child.take() {
                    worker.restore_rollback_checkpoint(checkpoint);
                    self.path.pop();
                }
                let Some(decision) = frame.moves.next().filter(|_| self.result.nodes < limit)
                else {
                    frames.pop();
                    continue;
                };
                let DecisionMove::Input {
                    request: Some(token),
                    selection,
                    ..
                } = &decision
                else {
                    unreachable!("position choices are issued inputs");
                };
                let (token, selection) = (*token, selection.clone());
                self.result.nodes += 1;
                self.path.push(GreedyStep::Input(decision));
                frame.child = Some(worker.begin_rollback_checkpoint(true)?);
                worker.submit(token, selection)?;
                let target = frame.next_target.take();
                if let Some(child) =
                    self.position_route_frame(worker, turn, limit, base, routes, target)?
                {
                    frames.push(child);
                }
            }
            Ok(())
        }));
        // The explicit traversal owns these checkpoints even when submission,
        // deadline checks or route evaluation interrupt the current branch.
        for frame in frames.into_iter().rev() {
            if let Some(checkpoint) = frame.child {
                worker.restore_rollback_checkpoint(checkpoint);
            }
        }
        self.path.truncate(prefix);
        match result {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    fn position_route_frame(
        &mut self,
        worker: &mut NativeSession,
        turn: i128,
        limit: usize,
        base: usize,
        routes: &mut Vec<PositionRoute>,
        target: Option<DecisionMove>,
    ) -> Result<Option<PositionRouteFrame>> {
        use crate::codec::Phase;

        self.budget.check(0)?;
        self.advance_profiled(worker)?;
        let terminal =
            worker.planning.phase == Phase::GameOver || worker.search_turn_index() >= self.cutoff;
        if terminal || worker.search_turn_index() > turn || worker.planning.phase == Phase::LevelUp
        {
            let score = worker.search_score(self.options.evaluation, self.team, self.player)?;
            self.result.position_routes += 1;
            if terminal {
                self.record_leaf(score);
            }
            let destination = self
                .options
                .controlled
                .iter()
                .map(|hero| {
                    let mut hexes: Vec<_> = worker.world.positions(*hero).collect();
                    hexes.sort_by_key(|hex| hex.coordinates());
                    hexes
                })
                .collect();
            // Bound retained arenas per frontier. Prefer distinct endpoints
            // so a single attractive destination cannot take every slot.
            let resume = (!terminal
                && routes.iter().filter(|route| route.resume.is_some()).count() < 8
                && !routes
                    .iter()
                    .any(|route| route.resume.is_some() && route.destination == destination))
            .then(|| Box::new(self.fork_profiled(worker)));
            routes.push(PositionRoute {
                resume,
                terminal,
                destination,
                score,
                steps: self.path[base..].to_vec(),
            });
            return Ok(None);
        }
        let (owner, _) = worker.search_player()?.ok_or_else(|| {
            EngineError::Unsupported("position route stalled before the next turn".into())
        })?;
        let (mut moves, next_target) = self.search_input_choices(worker, owner, limit, target)?;
        if self.options.positional_ordering {
            worker.order_movement_roots(owner, &mut moves);
        }
        Ok(Some(PositionRouteFrame {
            moves: moves.into_iter(),
            next_target,
            child: None,
        }))
    }

    fn order_position_choices(
        &mut self,
        worker: &mut NativeSession,
        owner: Symbol,
        moves: &mut [DecisionMove],
        limit: usize,
    ) -> Result<()> {
        if moves.len() < 2
            || !worker.last_request.as_ref().is_some_and(|request| {
                matches!(
                    request.choices,
                    RequestChoices::Action(_)
                        | RequestChoices::Hexes(_)
                        | RequestChoices::ZoneHexes { .. }
                        | RequestChoices::Units { .. }
                        | RequestChoices::Mines(_)
                        | RequestChoices::Numbers { .. }
                        | RequestChoices::MirrorDistances(_)
                )
            })
        {
            return Ok(());
        }

        worker.order_movement_roots(owner, moves);
        let count = moves
            .len()
            .min(12)
            .min((limit - self.result.nodes) / 8)
            .min((self.options.max_nodes / 8).saturating_sub(self.result.positional_probes));
        if count < 2 {
            return Ok(());
        }

        let mut ranked = Vec::with_capacity(count);
        for (index, decision) in moves[..count].iter().enumerate() {
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = decision
            else {
                unreachable!("position choices are issued inputs");
            };
            self.budget.check(0)?;
            self.result.nodes += 1;
            self.result.positional_probes += 1;
            let score = worker.with_consumed_request(|branch| {
                branch.submit(*token, selection.clone())?;
                self.advance_profiled(branch)?;
                let position = branch.team_position_score(self.team, self.player);
                let spare = self
                    .positional_budget(limit)
                    .saturating_sub(count - index - 1);
                let option = self.token_swap_position(branch, position, spare.min(3))?;
                Ok((
                    branch.search_score(self.options.evaluation, self.team, self.player)?,
                    position + (option - position) / 2,
                ))
            })?;
            ranked.push((score, decision.clone()));
        }
        ranked.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        for (slot, (_, decision)) in moves.iter_mut().zip(ranked) {
            *slot = decision;
        }
        Ok(())
    }

    fn positional_budget(&self, limit: usize) -> usize {
        (limit - self.result.nodes)
            .min((self.options.max_nodes / 8).saturating_sub(self.result.positional_probes))
    }

    fn token_swap_position(
        &mut self,
        worker: &mut NativeSession,
        current: i64,
        cap: usize,
    ) -> Result<i64> {
        if cap == 0 || !worker.pending_token_swap() {
            return Ok(current);
        }
        let (owner, _) = worker
            .search_player()?
            .ok_or_else(|| EngineError::Unsupported("token swap has no decision maker".into()))?;
        let mut best = None;
        for decision in self.input_choices(worker, owner)?.into_iter().take(cap) {
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = decision
            else {
                unreachable!("token swaps are issued inputs");
            };
            self.budget.check(0)?;
            self.result.nodes += 1;
            self.result.positional_probes += 1;
            let position = worker.with_consumed_request(|branch| {
                branch.submit(token, selection)?;
                self.advance_profiled(branch)?;
                Ok(branch.team_position_score(self.team, self.player))
            })?;
            best = Some(best.map_or(position, |best: i64| best.max(position)));
        }
        Ok(best.unwrap_or(current))
    }

    fn controlled_plans(
        &self,
        worker: &NativeSession,
        cap: usize,
    ) -> Result<Vec<Vec<PlanningCommitment>>> {
        Ok(worker
            .joint_plans_partition(
                self.player,
                true,
                if self.options.fixed_card_order {
                    1
                } else {
                    cap.min(4096)
                },
                Some(self.options.controlled),
            )?
            .plans)
    }

    fn opponent_commitments(&self, worker: &NativeSession) -> Result<Vec<PlanningCommitment>> {
        let mut commitments = Vec::new();
        for hero in &worker.world.heroes {
            if !self.options.controlled.contains(&hero.id)
                && let Some(choice) =
                    worker.greedy_opponent_commitment(hero.id, self.policy_seed(worker, hero.id))?
            {
                commitments.push(choice);
            }
        }
        Ok(commitments)
    }

    fn walk_upgrade_paths(
        &mut self,
        worker: &mut NativeSession,
        limit: usize,
        first: Option<&[CardId]>,
        visit_planning: &mut impl FnMut(&mut Self, &mut NativeSession) -> Result<()>,
    ) -> Result<()> {
        use crate::codec::Phase;

        self.budget.check(0)?;
        self.advance_profiled(worker)?;
        if worker.search_turn_index() >= self.cutoff || worker.planning.phase == Phase::GameOver {
            let score = worker.search_score(self.options.evaluation, self.team, self.player)?;
            self.record_leaf(score);
            return Ok(());
        }
        if self.result.nodes >= limit {
            return Ok(());
        }
        if let Some(cards) = first
            && !cards
                .iter()
                .any(|&card| self.upgrade_can_keep_first(worker, card))
        {
            return Ok(());
        }
        if worker.planning.phase == Phase::Planning {
            return visit_planning(self, worker);
        }
        if !matches!(
            worker.last_request.as_ref().map(|request| &request.choices),
            Some(RequestChoices::Upgrades(_))
        ) {
            return Ok(());
        }
        let (owner, _) = worker.search_player()?.ok_or_else(|| {
            EngineError::Unsupported("upgrade request has no decision maker".into())
        })?;
        for choice in self.input_choices(worker, owner)? {
            if self.result.nodes >= limit {
                break;
            }
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = &choice
            else {
                unreachable!("upgrade choices are issued inputs");
            };
            let compatible = first.map(|cards| {
                let changes = (owner == self.player)
                    .then(|| self.upgrade_candidate_changes(worker, selection))
                    .flatten();
                cards
                    .iter()
                    .copied()
                    .filter(|&card| {
                        self.upgrade_can_keep_first(worker, card)
                            && changes.as_ref().is_none_or(|change| change.preserves(card))
                    })
                    .collect::<Vec<_>>()
            });
            if compatible.as_ref().is_some_and(Vec::is_empty) {
                continue;
            }
            let (token, selection) = (*token, selection.clone());
            self.result.nodes += 1;
            self.result.upgrade_discovery_steps += 1;
            self.path.push(GreedyStep::Input(choice));
            let result = worker.with_consumed_request(|branch| {
                branch.submit(token, selection)?;
                self.walk_upgrade_paths(branch, limit, compatible.as_deref(), visit_planning)
            });
            self.path.pop();
            result?;
        }
        Ok(())
    }

    fn upgrade_can_keep_first(&self, worker: &NativeSession, card: CardId) -> bool {
        use crate::world::CardLifecycle;

        let Some(hero) = worker.world.hero(self.player) else {
            return false;
        };
        if hero.hand.contains(&card) {
            return true;
        }
        worker
            .world
            .card(card)
            .is_some_and(|row| row.lifecycle == CardLifecycle::Deck)
            && worker
                .round
                .pending
                .iter()
                .any(|(hero, _)| *hero == self.player)
    }

    fn upgrade_candidate_changes<'a>(
        &self,
        worker: &'a NativeSession,
        selection: &Datum,
    ) -> Option<UpgradeCandidateChanges<'a>> {
        let Datum::Object(fields) = selection else {
            return None;
        };
        let printed = fields.iter().find_map(|(key, value)| match value {
            Datum::Text(id) if worker.symbols.name(*key) == "card_id" => Some(*id),
            _ => None,
        })?;
        let hero = worker.world.hero(self.player)?;
        let RequestChoices::Upgrades(players) = &worker.last_request.as_ref()?.choices else {
            return None;
        };
        let (option, chosen) = players
            .iter()
            .filter(|row| row.hero == self.player)
            .flat_map(|row| &row.options)
            .find_map(|option| {
                let chosen = option.pair.iter().copied().find(|&id| {
                    worker.world.card(id).is_some_and(|row| {
                        worker.world.definitions.cards[row.definition].printed_id == printed
                    })
                })?;
                Some((option, chosen))
            })?;
        let ordinary = worker.symbols.name(option.tier) != "IV";
        let replaced = ordinary
            .then(|| {
                hero.hand.iter().copied().find(|&id| {
                    worker.world.card(id).is_some_and(|row| {
                        worker.world.definitions.cards[row.definition].color == Some(option.color)
                    })
                })
            })
            .flatten();
        let item = ordinary
            .then(|| option.pair.iter().copied().find(|&id| id != chosen))
            .flatten();
        let final_hand = worker
            .round
            .pending
            .iter()
            .any(|(id, levels)| *id == self.player && *levels == 1)
            .then_some(hero.hand.as_slice());
        Some(UpgradeCandidateChanges {
            chosen,
            replaced,
            item,
            final_hand,
        })
    }

    fn deferred_first_plays(&self, worker: &NativeSession) -> Vec<CardId> {
        let hero = worker
            .world
            .hero(self.player)
            .expect("validated search player");
        let remaining_levels = worker
            .round
            .pending
            .iter()
            .find_map(|(id, remaining)| (*id == self.player).then_some(*remaining))
            .unwrap_or(0);
        let mut first_plays = hero.hand.clone();
        if remaining_levels == 1
            && let Some(RequestChoices::Upgrades(players)) =
                worker.last_request.as_ref().map(|request| &request.choices)
        {
            first_plays.extend(
                players
                    .iter()
                    .filter(|row| row.hero == self.player)
                    .flat_map(|row| &row.options)
                    .flat_map(|option| option.pair.iter().copied()),
            );
        } else if remaining_levels > 1 {
            first_plays.extend(hero.deck.iter().copied().filter(|id| {
                let Some(row) = worker.world.card(*id) else {
                    return false;
                };
                let def = &worker.world.definitions.cards[row.definition];
                row.lifecycle == crate::world::CardLifecycle::Deck
                    && matches!(worker.symbols.name(def.tier), "II" | "III")
                    && matches!(
                        def.color.map(|color| worker.symbols.name(color)),
                        Some("RED" | "BLUE" | "GREEN")
                    )
            }));
        }
        first_plays.sort_unstable_by_key(|card| card.0);
        first_plays.dedup();
        first_plays
    }

    fn visit_deferred_upgrades(
        &mut self,
        worker: &mut NativeSession,
        limit: usize,
    ) -> Result<bool> {
        let remaining = limit - self.result.nodes;
        if remaining < 8 {
            return Ok(false);
        }
        let base = self.path.len();
        let completed = self.result.completed_sequences;
        let position_first = self.options.position_first && !self.options.fixed_card_order;
        let walk_limit = limit - remaining / 4;
        let mut routes = Vec::new();
        let first_plays = self.deferred_first_plays(worker);
        if position_first {
            // Longer continuations need more time after upgraded route
            // discovery. Keep the wider two-turn discovery allocation.
            let discovery_deadline = (self.options.turns > 2)
                .then(|| self.short_discovery_deadline())
                .flatten();
            let mut groups: Vec<_> = (0..first_plays.len()).map(|_| Vec::new()).collect();
            let indices: std::collections::HashMap<_, _> = first_plays
                .iter()
                .copied()
                .enumerate()
                .map(|(i, card)| (card, i))
                .collect();
            let discovery_limit = self.result.nodes + remaining / 5;
            self.walk_upgrade_paths(
                worker,
                discovery_limit,
                Some(&first_plays),
                &mut |search, planning| {
                    let fixed = search.opponent_commitments(planning)?;
                    let turn = planning.search_turn_index();
                    for mut plan in search.controlled_plans(planning, 256)? {
                        let Some(index) = plan
                            .iter()
                            .find(|choice| choice.hero == search.player)
                            .and_then(|choice| choice.first)
                            .and_then(|card| indices.get(&card))
                            .copied()
                        else {
                            continue;
                        };
                        if groups[index].len() >= 4 || search.result.nodes >= discovery_limit {
                            continue;
                        }
                        plan.extend_from_slice(&fixed);
                        search.result.nodes += 1;
                        search.path.push(GreedyStep::Planning(plan.clone()));
                        let result = planning.with_joint_planning(&plan, |branch| {
                            groups[index].push((
                                turn,
                                UpgradeWorld {
                                    worker: search.fork_profiled(branch),
                                    steps: search.path[base..].to_vec(),
                                    target: None,
                                },
                            ));
                            Ok(())
                        });
                        search.path.pop();
                        result?;
                    }
                    Ok(())
                },
            )?;
            // Interleave witnesses from different first cards. Destination
            // choices can now retain both the card and the upgrade alternatives.
            let mut groups: Vec<_> = groups.into_iter().map(Vec::into_iter).collect();
            let mut witnesses = Vec::new();
            loop {
                let before = witnesses.len();
                witnesses.extend(groups.iter_mut().filter_map(Iterator::next));
                if witnesses.len() == before {
                    break;
                }
            }
            let mut witnesses = witnesses.into_iter();
            while witnesses.len() != 0 && self.result.nodes < walk_limit {
                if !routes.is_empty()
                    && discovery_deadline
                        .is_some_and(|deadline| std::time::Instant::now() >= deadline)
                {
                    break;
                }
                let batches = witnesses.len().div_ceil(8);
                let available = walk_limit - self.result.nodes;
                let quota = (available / batches).max(1);
                let quota = if discovery_deadline.is_some() {
                    quota.min(256)
                } else {
                    quota
                };
                let batch: Vec<_> = witnesses.by_ref().take(8).collect();
                let turn = batch[0].0;
                self.collect_upgrade_world_routes(
                    batch.into_iter().map(|(_, world)| world).collect(),
                    turn,
                    self.result.nodes + quota,
                    base,
                    &mut routes,
                )?;
            }
            self.follow_position_routes(worker, limit, routes, true)?;
            return Ok(self.result.completed_sequences > completed);
        }
        let count = first_plays.len();

        for (index, first) in first_plays.into_iter().enumerate() {
            if self.result.nodes >= walk_limit {
                break;
            }
            let available = walk_limit - self.result.nodes;
            let card_limit = self.result.nodes + (available / (count - index)).max(1);
            self.walk_upgrade_paths(
                worker,
                card_limit,
                Some(&[first]),
                &mut |search, planning| {
                    let mut commitments = search.controlled_plans(planning, 4096)?;
                    commitments.retain(|plan| {
                        plan.iter().any(|choice| {
                            choice.hero == search.player && choice.first == Some(first)
                        })
                    });
                    let fixed = search.opponent_commitments(planning)?;
                    // Leave room to examine another compatible upgrade path.
                    let available = card_limit - search.result.nodes;
                    let hand_limit = search.result.nodes + available.min((available / 2).max(16));
                    let count = commitments.len();
                    for (index, mut plan) in commitments.into_iter().enumerate() {
                        if search.result.nodes >= hand_limit {
                            break;
                        }
                        let available = hand_limit - search.result.nodes;
                        let plan_limit = search.result.nodes + (available / (count - index)).max(1);
                        plan.extend_from_slice(&fixed);
                        search.result.nodes += 1;
                        search.path.push(GreedyStep::Planning(plan.clone()));
                        let result = planning
                            .with_joint_planning(&plan, |branch| search.visit(branch, plan_limit));
                        search.path.pop();
                        result?;
                    }
                    Ok(())
                },
            )?;
        }
        // The caller can still search the ordinary upgrade tree with the
        // reserved budget when discovery failed to reach a scored plan.
        Ok(self.result.completed_sequences > completed)
    }

    fn collect_upgrade_world_routes(
        &mut self,
        worlds: Vec<UpgradeWorld>,
        turn: i128,
        limit: usize,
        base: usize,
        routes: &mut Vec<PositionRoute>,
    ) -> Result<()> {
        use crate::codec::Phase;

        if let Some(profile) = &mut self.result.profile {
            profile.largest_upgrade_batch = profile.largest_upgrade_batch.max(worlds.len());
        }
        if worlds.len() == 1 {
            let mut world = worlds.into_iter().next().unwrap();
            let prefix = self.path.len();
            self.path.extend(world.steps);
            let result = self.collect_position_routes(
                &mut world.worker,
                turn,
                limit,
                base,
                routes,
                world.target,
            );
            self.path.truncate(prefix);
            return result;
        }
        // Physical worlds are kept as witnesses, so a shared decision
        // never borrows another alternative's hand or item bonuses.
        use std::{
            collections::HashMap,
            hash::{DefaultHasher, Hash, Hasher},
        };
        let mut replies = Vec::<(
            usize,
            Datum,
            Vec<(usize, DecisionMove, Option<DecisionMove>)>,
        )>::new();
        let mut buckets = HashMap::<u64, Vec<usize>>::new();
        let mut boundaries = Vec::<UpgradeBoundary>::new();
        let mut live: Vec<(Option<UpgradeWorld>, usize)> = Vec::new();
        for mut world in worlds {
            self.budget.check(0)?;
            self.advance_profiled(&mut world.worker)?;
            if world.worker.search_turn_index() > turn
                || world.worker.search_turn_index() >= self.cutoff
                || matches!(
                    world.worker.planning.phase,
                    Phase::LevelUp | Phase::GameOver
                )
            {
                let prefix = self.path.len();
                self.path.extend(world.steps);
                let result = self.collect_position_routes(
                    &mut world.worker,
                    turn,
                    limit,
                    base,
                    routes,
                    world.target,
                );
                self.path.truncate(prefix);
                result?;
                continue;
            }
            let (owner, _) = world.worker.search_player()?.ok_or_else(|| {
                EngineError::Unsupported("upgrade route stalled before the next turn".into())
            })?;
            let (mut moves, target) =
                self.search_input_choices(&mut world.worker, owner, limit, world.target.take())?;
            if self.options.positional_ordering {
                world.worker.order_movement_roots(owner, &mut moves);
            }
            let grouping = self.profile_clock();
            let mut menu = world.worker.last_request.as_ref().unwrap().clone();
            menu.id = super::RequestToken::Generated(0);
            let locations = world.worker.world.locations.clone();
            let score =
                world
                    .worker
                    .search_score(self.options.evaluation, self.team, self.player)?;
            let moving = world.worker.movement_root_actor().is_some();
            let index = live.len();
            let uses = moves.len();
            for decision in moves {
                let DecisionMove::Input { selection, .. } = &decision else {
                    unreachable!("route choices are issued inputs");
                };
                let mut selected_menu = menu.clone();
                if let RequestChoices::Action(action) = &mut selected_menu.choices
                    && let Datum::Text(name) = selection
                    && let Some(option) = action
                        .options
                        .iter()
                        .find(|option| {
                            option.kind.name() == world.worker.symbols.name(*name)
                                && !option.primary_effect
                                && matches!(
                                    option.kind,
                                    ActionKind::Movement
                                        | ActionKind::Hold
                                        | ActionKind::FastTravel
                                )
                        })
                        .copied()
                {
                    action.card = action.hero;
                    action.name = action.hero;
                    selected_menu.prompt = action.hero;
                    action.options = vec![option];
                } else if moving
                    && matches!(selected_menu.choices, RequestChoices::Hexes(_))
                    && matches!(selection, Datum::Object(_))
                {
                    // All witnesses independently offered this destination.
                    // Other available hexes do not constrain the chosen move.
                    selected_menu.choices = RequestChoices::Hexes(Vec::new());
                }
                let boundary = UpgradeBoundary {
                    menu: selected_menu,
                    locations: locations.clone(),
                    score,
                };
                let mut hash = DefaultHasher::new();
                boundary.hash(&mut hash);
                world.worker.symbols.hash_local(&mut hash);
                selection.hash(&mut hash);
                let bucket = buckets.entry(hash.finish()).or_default();
                let existing = bucket.iter().copied().find(|&group| {
                    let (known, value, entries) = &replies[group];
                    let prior = entries[0].0;
                    *value == *selection
                        && boundaries[*known] == boundary
                        && (index == prior
                            || live[prior]
                                .0
                                .as_ref()
                                .unwrap()
                                .worker
                                .symbols
                                .same_local_names(&world.worker.symbols))
                });
                if let Some(group) = existing {
                    replies[group].2.push((index, decision, target.clone()));
                } else {
                    bucket.push(replies.len());
                    replies.push((
                        boundaries.len(),
                        selection.clone(),
                        vec![(index, decision, target.clone())],
                    ));
                    boundaries.push(boundary);
                }
            }
            if let (Some(start), Some(profile)) = (grouping, &mut self.result.profile) {
                profile.grouping_ns += elapsed_ns(start);
            }
            live.push((Some(world), uses));
        }
        for (boundary, _, entries) in replies {
            if self.result.nodes >= limit {
                break;
            }
            let mut children = Vec::new();
            for (index, decision, target) in entries {
                if self.result.nodes >= limit {
                    break;
                }
                let DecisionMove::Input {
                    request: Some(token),
                    selection,
                    ..
                } = &decision
                else {
                    unreachable!("route choices are issued inputs");
                };
                let (source, uses) = &mut live[index];
                *uses -= 1;
                self.result.nodes += 1;
                let (token, selection) = (*token, selection.clone());
                let mut child = if *uses == 0 {
                    let mut child = source.take().expect("last use owns its world");
                    child.worker.submit(token, selection)?;
                    child.worker.last_request = None;
                    child
                } else {
                    let source = source.as_mut().expect("earlier use keeps its world");
                    let worker = source.worker.with_consumed_request(|worker| {
                        worker.submit(token, selection)?;
                        Ok(self.fork_profiled(worker))
                    })?;
                    UpgradeWorld {
                        worker,
                        steps: source.steps.clone(),
                        target: None,
                    }
                };
                child.steps.push(GreedyStep::Input(decision));
                child.target = target;
                children.push(child);
            }
            let shared_movement = match &boundaries[boundary].menu.choices {
                RequestChoices::Hexes(hexes) => hexes.is_empty(),
                RequestChoices::Action(action) => {
                    action.card == action.hero
                        && action.options.iter().any(|option| {
                            matches!(option.kind, ActionKind::Movement | ActionKind::FastTravel)
                        })
                }
                _ => false,
            };
            let first_card = |world: &UpgradeWorld| {
                world.steps.iter().find_map(|step| match step {
                    GreedyStep::Planning(plan) => plan
                        .iter()
                        .find(|choice| choice.hero == self.player)
                        .and_then(|choice| choice.first),
                    _ => None,
                })
            };
            if shared_movement
                && let Some(first) = children.first()
                && children
                    .iter()
                    .skip(1)
                    .any(|world| first_card(world) != first_card(first))
            {
                self.result.shared_card_moves += 1;
            }
            if children.len() > 1 {
                self.result.shared_upgrade_decisions += 1;
            }
            self.collect_upgrade_world_routes(children, turn, limit, base, routes)?;
        }
        Ok(())
    }

    fn search_input_choices(
        &mut self,
        worker: &mut NativeSession,
        owner: Symbol,
        limit: usize,
        target: Option<DecisionMove>,
    ) -> Result<(Vec<DecisionMove>, Option<DecisionMove>)> {
        // The target is carried only along the exact attack branch that
        // produced it. Regenerate the menu to validate the issued input and
        // recreate any symbols interned during the probe.
        if let Some(target) = target
            && worker.pending_attack_target()
            && worker.resolution_choices(owner)?.contains(&target)
        {
            self.result.tactical_target_reuses += 1;
            return Ok((vec![target], None));
        }
        if self.options.tactical_replies
            && worker
                .world
                .hero(owner)
                .is_some_and(|hero| hero.team != self.team)
            && let Some(reply) = self.tactical_reply(worker, owner, limit)?
        {
            return Ok((vec![reply.input], reply.target));
        }
        Ok((self.input_choices(worker, owner)?, None))
    }

    fn tactical_budget(&self, limit: usize) -> usize {
        (limit - self.result.nodes)
            .min((self.options.max_nodes / 16).saturating_sub(self.result.tactical_probes))
    }

    fn tactical_reply(
        &mut self,
        worker: &mut NativeSession,
        owner: Symbol,
        limit: usize,
    ) -> Result<Option<TacticalReply>> {
        if self.tactical_budget(limit) < 2 {
            return Ok(None);
        }
        if worker.pending_attack_target() {
            return Ok(self
                .tactical_target(worker, owner, limit)?
                .map(|input| TacticalReply {
                    input,
                    target: None,
                }));
        }
        // One action probe and up to four target probes.
        if self.tactical_budget(limit) < 5 {
            return Ok(None);
        }
        if !worker.last_request.as_ref().is_some_and(|request| {
            matches!(&request.choices, RequestChoices::Action(menu)
                if menu.options.iter().any(|option| option.kind == ActionKind::Attack))
        }) {
            return Ok(None);
        }
        let Some(attack) = worker
            .resolution_choices(owner)?
            .into_iter()
            .find(|decision| {
                matches!(decision, DecisionMove::Input { selection: Datum::Text(id), .. }
                if worker.symbols.name(*id) == "ATTACK")
            })
        else {
            return Ok(None);
        };
        let DecisionMove::Input {
            request: Some(token),
            selection,
            ..
        } = &attack
        else {
            unreachable!("attack choices are issued inputs");
        };
        self.budget.check(0)?;
        self.result.nodes += 1;
        self.result.tactical_probes += 1;
        let target = worker.with_consumed_request(|branch| {
            branch.submit(*token, selection.clone())?;
            self.advance_profiled(branch)?;
            if !branch.pending_attack_target() {
                return Ok(None);
            }
            let Some((target_owner, _)) = branch.search_player()? else {
                return Ok(None);
            };
            if target_owner != owner {
                return Ok(None);
            }
            self.tactical_target(branch, owner, limit)
        })?;
        Ok(target.map(|target| TacticalReply {
            input: attack,
            target: Some(target),
        }))
    }

    fn tactical_target(
        &mut self,
        worker: &mut NativeSession,
        owner: Symbol,
        limit: usize,
    ) -> Result<Option<DecisionMove>> {
        use crate::world::EntityKind;

        let choices = worker.resolution_choices(owner)?;
        let mut adjacent = Vec::new();
        let mut minions = Vec::new();
        for choice in choices {
            let DecisionMove::Input {
                selection: Datum::Text(id),
                ..
            } = &choice
            else {
                continue;
            };
            let target = *id;
            let Some(entity) = worker
                .world
                .entity(target)
                .filter(|e| e.team == Some(self.team))
            else {
                continue;
            };
            match entity.kind {
                EntityKind::Minion => minions.push((target, choice)),
                EntityKind::Hero | EntityKind::Piece
                    if self
                        .options
                        .controlled
                        .contains(&worker.world.hero_identity(target))
                        && worker.world.position(target).is_some_and(|hex| {
                            worker
                                .world
                                .positions(owner)
                                .any(|from| from.distance(hex) == 1)
                        }) =>
                {
                    adjacent.push((target, choice))
                }
                _ => {}
            }
        }
        let candidates = adjacent
            .into_iter()
            .take(2)
            .map(|row| (row, false))
            .chain(minions.into_iter().take(2).map(|row| (row, true)));
        let mut best = None;
        for ((target, choice), minion) in candidates {
            if self.tactical_budget(limit) == 0 {
                break;
            }
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = &choice
            else {
                unreachable!("attack targets are issued inputs");
            };
            self.budget.check(0)?;
            self.result.nodes += 1;
            self.result.tactical_probes += 1;
            let score = worker.with_consumed_request(|branch| {
                branch.submit(*token, selection.clone())?;
                self.advance_profiled(branch)?;
                if minion && branch.world.position(target).is_some() {
                    return Ok(None);
                }
                Ok(Some(branch.search_score(
                    self.options.evaluation,
                    self.team,
                    self.player,
                )?))
            })?;
            if let Some(score) = score {
                let rank = (score, !minion);
                if best.as_ref().is_none_or(|(best, _)| rank < *best) {
                    best = Some((rank, choice));
                }
            }
        }
        Ok(best.map(|(_, choice)| choice))
    }

    fn input_choices(
        &self,
        worker: &mut NativeSession,
        owner: Symbol,
    ) -> Result<Vec<DecisionMove>> {
        if self.options.controlled.contains(&owner) {
            worker.resolution_choices(owner)
        } else {
            Ok(worker
                .greedy_opponent_input(owner, self.policy_seed(worker, owner))?
                .into_iter()
                .collect())
        }
    }

    fn branch_limit(&self, limit: usize, remaining_choices: usize, balance_cards: bool) -> usize {
        if !self.incumbent_only && (self.path.is_empty() || balance_cards) {
            self.result.nodes + ((limit - self.result.nodes) / remaining_choices).max(1)
        } else {
            limit
        }
    }

    fn policy_seed(&self, worker: &NativeSession, hero: Symbol) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let mut hash = DefaultHasher::new();
        self.options.seed.hash(&mut hash);
        worker.search_turn_index().hash(&mut hash);
        hero.hash(&mut hash);
        worker
            .last_request
            .as_ref()
            .map(|request| request.prompt)
            .hash(&mut hash);
        hash.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::{NativeRequest, RequestToken, object_execution::fixture_case};

    fn session(name: &str) -> NativeSession {
        let fixture = fixture_case(name);
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap()
    }

    #[test]
    fn tactical_reply_confirms_a_minion_defeat_through_an_attack_macro() {
        let mut session = session("object-execution-attack-macro-optional-first");
        session.advance_prefix(1).unwrap();
        let request = session.advance_prefix(1).unwrap().request.unwrap();
        let target = session.symbols.lookup("minion_37").unwrap();
        let team = session.world.entity(target).unwrap().team.unwrap();
        let player = session
            .world
            .heroes
            .iter()
            .find(|hero| hero.team == team)
            .unwrap()
            .id;
        let budget = super::super::ExecutionBudget::default();
        let mut traversal = GreedyTraversal {
            options: GreedySearchOptions {
                controlled: &[player],
                turns: 1,
                max_nodes: 256,
                seed: 17,
                evaluation: super::super::SearchEvaluation::MaterialPosition,
                fixed_card_order: false,
                defer_upgrades: true,
                positional_ordering: true,
                position_first: false,
                tactical_replies: true,
                reuse_movement: true,
                profile: false,
            },
            budget: &budget,
            player,
            team,
            cutoff: session.search_turn_index() + 1,
            result: GreedySearch::default(),
            started: None,
            path: Vec::new(),
            incumbent_only: false,
        };
        assert!(session.pending_attack_target());
        let before = session.semantic_state().unwrap();
        let choice = traversal
            .tactical_reply(&mut session, request.owner, 256)
            .unwrap()
            .unwrap()
            .input;
        let DecisionMove::Input {
            request: Some(token),
            selection: Datum::Text(id),
            ..
        } = choice
        else {
            panic!("expected a legal target selection");
        };
        let probes = traversal.result.tactical_probes;
        let input = DecisionMove::Input {
            player: request.owner,
            request: Some(token),
            selection: Datum::Text(id),
        };
        let (choices, followup) = traversal
            .search_input_choices(&mut session, request.owner, 256, Some(input.clone()))
            .unwrap();
        assert_eq!(choices, vec![input]);
        assert!(followup.is_none());
        assert_eq!(traversal.result.tactical_target_reuses, 1);
        assert_eq!(traversal.result.tactical_probes, probes);
        assert_eq!(id, target);
        assert_eq!(session.semantic_state().unwrap(), before);
        session.submit(token, Datum::Text(id)).unwrap();
        session.last_request = None;
        session.advance_to_decision(&budget).unwrap();
        assert!(session.world.position(target).is_none());
    }

    #[test]
    fn lane_push_changes_the_position_value_at_the_same_hero_hex() {
        let mut pushed = session("object-execution-live-battle-red-push-first");
        pushed.advance_prefix(1).unwrap();
        let mut initial = pushed.clone();
        pushed.advance_prefix(1).unwrap();
        let player = pushed.symbols.lookup("hero_arien").unwrap();
        let team = pushed.world.hero(player).unwrap().team;
        let hex = crate::geometry::Hex::new(5, 0, -5).unwrap();
        for state in [&mut initial, &mut pushed] {
            state.world.place_entity(player, hex).unwrap();
        }
        let before = initial.team_position_score(team, player);
        let after = pushed.team_position_score(team, player);
        assert!(
            after > before,
            "lane push should create attack or support opportunities"
        );
        let opposing = pushed
            .world
            .heroes
            .iter()
            .find(|hero| hero.team != team)
            .unwrap()
            .team;
        assert_eq!(after, -pushed.team_position_score(opposing, player));
    }

    #[test]
    fn tactical_reply_selects_a_legal_adjacent_hero_on_an_attack_prompt() {
        let mut session = session("object-execution-arena-push-card-rogue_wave-first");
        session.advance_prefix(1).unwrap();
        session.advance_prefix(1).unwrap();
        let request = session.advance_prefix(1).unwrap().request.unwrap();
        let RequestChoices::Units { values, .. } = request.choices else {
            panic!("fixture must offer attack targets");
        };
        let player = values
            .iter()
            .map(|(id, _)| *id)
            .find(|id| {
                session.world.position(*id).is_some_and(|hex| {
                    session
                        .world
                        .positions(request.owner)
                        .any(|from| from.distance(hex) == 1)
                })
            })
            .expect("fixture should contain an adjacent target");
        let team = session.world.hero(player).unwrap().team;
        let budget = super::super::ExecutionBudget::default();
        let mut traversal = GreedyTraversal {
            options: GreedySearchOptions {
                controlled: &[player],
                turns: 1,
                max_nodes: 256,
                seed: 17,
                evaluation: super::super::SearchEvaluation::MaterialPosition,
                fixed_card_order: false,
                defer_upgrades: true,
                positional_ordering: true,
                position_first: false,
                tactical_replies: true,
                reuse_movement: true,
                profile: false,
            },
            budget: &budget,
            player,
            team,
            cutoff: session.search_turn_index() + 1,
            result: GreedySearch::default(),
            started: None,
            path: Vec::new(),
            incumbent_only: false,
        };
        assert!(session.pending_attack_target());
        let before = session.semantic_state().unwrap();
        let choice = traversal
            .tactical_reply(&mut session, request.owner, 256)
            .unwrap()
            .unwrap()
            .input;
        assert!(
            matches!(choice, DecisionMove::Input { selection: Datum::Text(id), .. }
            if id == player)
        );
        assert_eq!(session.semantic_state().unwrap(), before);
    }

    #[test]
    fn positional_probes_rank_real_push_outcomes_and_restore_the_root() {
        let name = "object-execution-arena-push-card-rogue_wave-first";
        let fixture = fixture_case(name);
        let mut session = session(name);
        for operation in fixture["operations"].as_array().unwrap().iter().take(24) {
            if operation["kind"] == "input" {
                let token = session.last_request.as_ref().unwrap().id;
                let selection = session.import_selection(&operation["selection"]).unwrap();
                session.submit(token, selection).unwrap();
            } else {
                session.advance_prefix(1).unwrap();
            }
        }
        let request = session.last_request.as_ref().unwrap();
        assert_eq!(session.symbols.name(request.prompt), "Choose push distance");
        let player = request.owner;
        let team = session.world.hero(player).unwrap().team;
        let budget = super::super::ExecutionBudget::default();
        let mut traversal = GreedyTraversal {
            options: GreedySearchOptions {
                controlled: &[player],
                turns: 1,
                max_nodes: 128,
                seed: 17,
                evaluation: super::super::SearchEvaluation::MaterialPosition,
                fixed_card_order: false,
                defer_upgrades: true,
                positional_ordering: true,
                position_first: true,
                tactical_replies: false,
                reuse_movement: true,
                profile: false,
            },
            budget: &budget,
            player,
            team,
            cutoff: session.search_turn_index() + 1,
            result: GreedySearch::default(),
            started: None,
            path: Vec::new(),
            incumbent_only: false,
        };
        let mut moves = session.resolution_choices(player).unwrap();
        let original = moves.clone();
        let before = session.semantic_state().unwrap();
        traversal
            .order_position_choices(&mut session, player, &mut moves, 128)
            .unwrap();
        assert_eq!(traversal.result.positional_probes, original.len());
        assert_eq!(session.semantic_state().unwrap(), before);
        assert!(original.iter().all(|decision| moves.contains(decision)));

        let budget = super::super::ExecutionBudget {
            max_steps: 1,
            ..budget
        };
        traversal.budget = &budget;
        traversal.result.nodes = 0;
        let turn = session.search_turn_index();
        let request = session.last_request.clone();
        assert!(matches!(
            traversal.collect_position_routes(&mut session, turn, 128, 0, &mut Vec::new(), None),
            Err(EngineError::StepLimit)
        ));
        assert!(traversal.result.nodes > 0);
        assert!(traversal.path.is_empty());
        assert_eq!(session.last_request, request);
        assert_eq!(session.semantic_state().unwrap(), before);
    }

    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn deferred_first_plays_keep_only_compatible_upgrade_paths() {
        use std::collections::{HashMap, HashSet};

        for (fixture, levels, path_count) in [
            (
                "object-execution-phase-closure-live-round-upgrade-one",
                1,
                6,
            ),
            ("object-execution-cleanup-live-level-gold-3", 2, 24),
            ("object-execution-cleanup-live-level-gold-6", 3, 48),
            ("object-execution-cleanup-live-level-gold-28", 6, 2304),
        ] {
            let mut session = session(fixture);
            let player = session.symbols.lookup("hero_arien").unwrap();
            let team = session.world.hero(player).unwrap().team;
            let budget = super::super::ExecutionBudget::default();
            session.advance_to_decision(&budget).unwrap();
            let before = session.semantic_state().unwrap();
            let RequestChoices::Upgrades(players) = &session.last_request.as_ref().unwrap().choices
            else {
                panic!("fixture must enter Level Up");
            };
            assert_eq!(
                players
                    .iter()
                    .find(|row| row.hero == player)
                    .unwrap()
                    .remaining,
                levels
            );
            if levels == 6 {
                assert_eq!(session.world.hero(player).unwrap().level, 8);
            }
            let node_limit = 4096;
            let initial_hand = session.world.hero(player).unwrap().hand.clone();
            let mut traversal = GreedyTraversal {
                options: GreedySearchOptions {
                    controlled: &[player],
                    turns: 2,
                    max_nodes: node_limit,
                    seed: 17,
                    evaluation: super::super::SearchEvaluation::MaterialPosition,
                    fixed_card_order: false,
                    defer_upgrades: true,
                    positional_ordering: true,
                    position_first: true,
                    tactical_replies: false,
                    reuse_movement: true,
                    profile: false,
                },
                budget: &budget,
                player,
                team,
                cutoff: session.search_turn_index() + 2,
                result: GreedySearch::default(),
                started: None,
                path: Vec::new(),
                incumbent_only: false,
            };
            let candidates: HashSet<_> = initial_hand
                .iter()
                .chain(&session.world.hero(player).unwrap().deck)
                .copied()
                .collect();
            let mut expected_paths = HashMap::<_, HashSet<_>>::new();
            let mut distinct_paths = HashSet::new();
            let mut offered_existing = false;
            let mut offered_upgrade = false;
            traversal
                .walk_upgrade_paths(&mut session, node_limit, None, &mut |search, worker| {
                    assert_eq!(search.path.len(), levels as usize);
                    if distinct_paths.is_empty() {
                        assert_eq!(search.result.upgrade_discovery_steps, levels as usize);
                    }
                    let path = format!("{:?}", search.path);
                    distinct_paths.insert(path.clone());
                    for &card in &worker.world.hero(player).unwrap().hand {
                        expected_paths.entry(card).or_default().insert(path.clone());
                    }
                    let choices = worker.planning_choices(player)?;
                    for commitments in search.controlled_plans(worker, 256)? {
                        let first = commitments
                            .iter()
                            .find(|choice| choice.hero == player)
                            .unwrap()
                            .first
                            .unwrap();
                        offered_existing |= initial_hand.contains(&first);
                        offered_upgrade |= !initial_hand.contains(&first);
                        assert!(choices.contains(&DecisionMove::Card {
                            hero: player,
                            card: first
                        }));
                    }
                    Ok(())
                })
                .unwrap();
            let discovered = traversal.deferred_first_plays(&session);
            assert!(expected_paths.keys().all(|card| discovered.contains(card)));
            assert_eq!(session.semantic_state().unwrap(), before);
            for card in candidates {
                traversal.result.nodes = 0;
                let mut actual_paths = HashSet::new();
                traversal
                    .walk_upgrade_paths(
                        &mut session,
                        node_limit,
                        Some(&[card]),
                        &mut |search, _| {
                            actual_paths.insert(format!("{:?}", search.path));
                            Ok(())
                        },
                    )
                    .unwrap();
                assert_eq!(
                    actual_paths,
                    expected_paths.remove(&card).unwrap_or_default(),
                    "{fixture}, card {card:?}"
                );
            }
            assert!(expected_paths.is_empty());
            assert_eq!(session.semantic_state().unwrap(), before);
            assert_eq!(distinct_paths.len(), path_count);
            assert!(offered_existing && offered_upgrade);
        }
    }

    #[test]
    fn future_card_likelihoods_follow_the_commitment_prefix_and_live_legal_cards() {
        let hero = Symbol(0);
        let [a, b, c, d] = [1, 2, 3, 4].map(|id| Some(CardId(id)));
        let play = |first| {
            Some(PlanningCommitment {
                hero,
                first,
                second: None,
            })
        };
        let result = GreedySearch {
            card_sequence_scores: Some(std::collections::HashMap::from([
                ([play(a), play(b), None, None], 100),
                ([play(a), play(c), None, None], 0),
                ([play(b), play(c), None, None], 200),
            ])),
            ..GreedySearch::default()
        };
        assert_eq!(result.preferred_next_card(&[play(a)], &[c, b]), Some(b));
        assert_eq!(result.preferred_next_card(&[play(b)], &[b, c]), Some(c));
        assert_eq!(result.preferred_next_card(&[play(d)], &[b, c]), None);
        let after_a = result
            .next_card_likelihoods(&[play(a)], &[b, c, d])
            .unwrap();
        assert!(after_a[0].1 > after_a[1].1 && after_a[1].1 > after_a[2].1);
        assert!(after_a[2].1 > 0.0);
        let after_b = result
            .next_card_likelihoods(&[play(b)], &[b, c, d])
            .unwrap();
        assert!(after_b[1].1 > after_b[0].1);
        assert_eq!(
            result.next_card_likelihoods(&[play(a)], &[c]),
            Some(vec![(c, 1.0)])
        );
        assert_eq!(result.next_card_likelihoods(&[play(d)], &[b, c]), None);
    }

    #[test]
    fn greedy_search_returns_replayable_two_and_four_turn_plans() {
        assert_replayable_greedy_plan(2);
        // Unoptimized rollback frames exceed the default test thread stack
        // when the search keeps an entire round of decisions open.
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(|| assert_replayable_greedy_plan(4))
            .unwrap()
            .join()
            .unwrap();
    }

    fn assert_replayable_greedy_plan(turns: u8) {
        let mut session = session("object-execution-card-planning-cycle");
        let player = session.world.heroes[0].id;
        let team = session.world.heroes[0].team;
        let before = session.semantic_state().unwrap();
        let start = session.search_turn_index();
        let budget = super::super::ExecutionBudget::default();
        let options = GreedySearchOptions {
            controlled: &[player],
            turns,
            fixed_card_order: false,
            defer_upgrades: true,
            positional_ordering: true,
            position_first: true,
            tactical_replies: false,
            reuse_movement: true,
            profile: true,
            max_nodes: if turns == 4 { 4096 } else { 1024 },
            seed: 17,
            evaluation: super::super::SearchEvaluation::MaterialPosition,
        };
        let result = session
            .search_greedy_card_scores(player, options, &budget)
            .unwrap();
        assert_eq!(session.semantic_state().unwrap(), before);
        let profile = result.profile.as_ref().unwrap();
        assert!(profile.rule_steps.len() > 1);
        assert!(!profile.rule_steps.contains_key("unknown"));
        if turns == 2 {
            let unprofiled = session
                .search_greedy_card_scores(
                    player,
                    GreedySearchOptions {
                        profile: false,
                        ..options
                    },
                    &budget,
                )
                .unwrap();
            assert!(unprofiled.profile.is_none());
            assert_eq!(unprofiled.nodes, result.nodes);
            assert_eq!(unprofiled.completed_sequences, result.completed_sequences);
            let profiled_plan = result.best.as_ref().unwrap();
            let unprofiled_plan = unprofiled.best.as_ref().unwrap();
            assert_eq!(profiled_plan.score, unprofiled_plan.score);
            assert_eq!(profiled_plan.steps, unprofiled_plan.steps);
            assert_eq!(session.semantic_state().unwrap(), before);
        }
        assert!(result.nodes <= options.max_nodes);
        assert!(result.completed_sequences > 1);
        assert!(result.position_groups > 0);
        assert!(result.position_routes > result.position_groups);
        assert!((1..=options.max_nodes / 2).contains(&profile.incumbent_nodes));
        if turns == 4 {
            let sequences = result.card_sequence_scores.as_ref().unwrap();
            assert!(
                sequences
                    .keys()
                    .any(|sequence| { result.card_scores_at(1, &sequence[..1]).len() > 1 })
            );
        }
        let by_turn: Vec<_> = (0..usize::from(turns))
            .map(|offset| result.turn_card_scores(offset))
            .collect();
        for offset in 0..usize::from(turns) {
            let likelihoods = result.turn_card_likelihoods(offset).unwrap();
            let total = likelihoods.iter().map(|(_, weight)| weight).sum::<f64>();
            assert!((total - 1.0).abs() < 1e-9);
        }
        let scores = &by_turn[0];
        assert!(scores.len() > 1);
        assert_eq!(
            scores.iter().map(|(_, score)| *score).max(),
            result.best.as_ref().map(|plan| plan.score),
        );
        let legal = session.planning_choices(player).unwrap();
        assert_eq!(scores.len(), legal.len());
        assert!(scores.iter().all(|(card, _)| {
            card.is_some_and(|card| legal.contains(&DecisionMove::Card { hero: player, card }))
        }));
        let plan = result.best.unwrap();

        let mut scored_turns = by_turn.iter();
        for step in plan.steps {
            session.advance_to_decision(&budget).unwrap();
            match step {
                GreedyStep::Planning(commitments) => {
                    let own = commitments.iter().find(|row| row.hero == player).unwrap();
                    assert!(
                        scored_turns
                            .next()
                            .unwrap()
                            .contains(&(own.first, plan.score))
                    );
                    session.apply_joint_planning_ordered(&commitments).unwrap();
                }
                GreedyStep::Input(DecisionMove::Input {
                    player,
                    request: Some(token),
                    selection,
                }) => {
                    assert!(session.resolution_choices(player).unwrap().contains(
                        &DecisionMove::Input {
                            player,
                            request: Some(token),
                            selection: selection.clone(),
                        }
                    ));
                    session.submit(token, selection).unwrap();
                    session.last_request = None;
                }
                _ => panic!("plan must contain issued inputs"),
            }
        }
        session.advance_to_decision(&budget).unwrap();
        assert_eq!(session.search_turn_index(), start + i128::from(turns));
        assert_eq!(
            session
                .search_score(options.evaluation, team, player)
                .unwrap(),
            plan.score
        );
    }

    #[test]
    fn commitment_and_forced_discard_use_the_same_defense_order() {
        let mut session = session("object-execution-card-planning-cycle");
        let hero = session.world.heroes[0].id;
        let chosen = session
            .greedy_opponent_commitment(hero, 7)
            .unwrap()
            .unwrap();
        let hand = session.world.hero(hero).unwrap().hand.clone();
        let printed = |id| {
            let row = session.world.card(id).unwrap();
            session.world.definitions.cards[row.definition].printed_id
        };
        let expected = Datum::Text(printed(chosen.first.unwrap()));
        session.last_request = Some(NativeRequest {
            id: RequestToken::Generated(1),
            owner: hero,
            controlled: None,
            prompt: hero,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::CardIds {
                values: hand.into_iter().map(printed).collect(),
                forced_victim: Some(hero),
                defeat_victim: None,
            },
            can_skip: false,
            discard: true,
        });
        let decision = session.greedy_opponent_input(hero, 7).unwrap().unwrap();
        assert!(matches!(decision, DecisionMove::Input { selection, .. } if selection == expected));

        session.last_request.as_mut().unwrap().can_skip = true;
        let decision = session.greedy_opponent_input(hero, 7).unwrap().unwrap();
        assert!(
            matches!(decision, DecisionMove::Input { selection: Datum::Text(id), .. }
            if session.symbols.name(id) == "SKIP")
        );
    }

    #[test]
    fn primary_defense_is_preserved_even_with_a_lower_value() {
        let mut session = session("object-execution-card-planning-cycle");
        let hero = session.world.heroes[0].id;
        let hand = &session.world.hero(hero).unwrap().hand;
        let pair = [hand[0], hand[1]];
        let first = session.world.card(pair[0]).unwrap().definition;
        let second = session.world.card(pair[1]).unwrap().definition;
        let defs = &mut std::sync::Arc::make_mut(&mut session.world.definitions).cards;
        defs[first].primary = Some(ActionKind::DefenseSkill);
        defs[first].primary_value = Some(0);
        defs[second].primary = Some(ActionKind::Attack);
        defs[second].secondary = vec![(ActionKind::Defense, 100)];
        assert_eq!(
            session.greedy_lowest_defense(hero, pair, 3).unwrap(),
            Some(pair[1])
        );
    }

    #[test]
    fn greedy_opponent_holds_in_the_live_action_menu() {
        use crate::codec::ObjectPathPart::{Index, Key};

        let mut session = session("object-execution-card-action-menu-arien-liquid_leap");
        session
            .queue_steps(&[
                Key("execution_context".into()),
                Key("menu_groups".into()),
                Index(0),
            ])
            .unwrap();
        let request = session.advance_prefix(1).unwrap().request.unwrap();
        let decision = session
            .greedy_opponent_input(request.owner, 11)
            .unwrap()
            .unwrap();
        assert!(
            matches!(decision, DecisionMove::Input { selection: Datum::Text(id), .. }
            if session.symbols.name(id) == "HOLD")
        );
    }

    #[test]
    fn mandatory_random_choices_are_repeatable_and_legal() {
        let mut session = session("object-execution-card-planning-cycle");
        let hero = session.world.heroes[0].id;
        session.last_request = Some(NativeRequest {
            id: RequestToken::Generated(1),
            owner: hero,
            controlled: None,
            prompt: hero,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Numbers {
                values: vec![1, 2, 3],
                labels: vec![],
            },
            can_skip: false,
            discard: false,
        });
        let legal = session.resolution_choices(hero).unwrap();
        let a = session.greedy_opponent_input(hero, 13).unwrap().unwrap();
        let b = session.greedy_opponent_input(hero, 13).unwrap().unwrap();
        assert_eq!(a, b);
        assert!(legal.contains(&a));
    }
}
