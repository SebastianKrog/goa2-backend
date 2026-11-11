//! Turn-horizon rollout and future planning expansion.

use super::{
    DecisionMove, EngineError, GameTurnRollout, JointPlanOrder, NarrowedPlanningRows,
    NativeSession, OpponentPolicy, Phase, PlanningCommitment, Result, Search, Symbol, ValueBound,
};

use super::opponent_draw_seed;

impl Search<'_, '_> {
    pub(super) fn rollout(
        &mut self,
        worker: &mut NativeSession,
        cutoff: i128,
        capture_state: bool,
        lab_evaluation: bool,
    ) -> Result<Option<GameTurnRollout>> {
        let history = self
            .future_planning
            .as_ref()
            .map(|rollout| vec![rollout.root.clone()]);
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
                        branch,
                        cutoff,
                        capture_state,
                        lab_evaluation,
                        history.clone(),
                    )?
                    else {
                        return Ok(None);
                    };
                    let value = ValueBound {
                        score: leaf.score,
                        resolved: leaf.reached_cutoff(cutoff),
                    };
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
                || (self.guard_hidden
                    && worker.last_request.is_none()
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
                if self
                    .future_planning
                    .as_ref()
                    .is_some_and(|rollout| rollout.expand)
                    && let Some(history) = history.as_ref().filter(|history| history.len() == 1)
                {
                    return self.expand_rollout_planning(
                        worker,
                        cutoff,
                        capture_state,
                        lab_evaluation,
                        history,
                    );
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
            let controlled = self
                .controlled
                .map_or(team == self.team, |ids| ids.contains(&owner));
            let decision = match self
                .future_planning
                .as_ref()
                .and_then(|rollout| rollout.fixed_opponent_seed)
            {
                _ if self.allow_simulated_upgrades && worker.planning.phase == Phase::LevelUp => {
                    worker.greedy_opponent_input(owner, opponent_draw_seed(worker, 0, owner))?
                }
                Some(seed) if !controlled => {
                    worker.greedy_opponent_input(owner, opponent_draw_seed(worker, seed, owner))?
                }
                _ => worker.first_resolution_choice(owner)?,
            }
            .ok_or_else(|| {
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
        let commitments =
            self.rollout_commitments(worker, history.as_deref().map(Vec::as_slice))?;
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
        let (_, forecast) = self
            .future_planning
            .as_ref()?
            .forecasts
            .iter()
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
            let controlled = self
                .controlled
                .map_or(hero.team == self.team, |ids| ids.contains(&hero.id));
            let mut choice = fallback;
            if !controlled
                && let Some(rollout) = &self.future_planning
                && let Some(seed) = rollout.fixed_opponent_seed
            {
                self.forecast_checks += 1;
                if let Some(predicted) = worker.sample_forecast_commitment(
                    hero.id,
                    rollout.forecasts,
                    history.unwrap_or_default(),
                    seed,
                )? {
                    self.forecast_matches += 1;
                    choice = predicted;
                } else if let Some(predicted) = worker.greedy_opponent_commitment(
                    hero.id,
                    opponent_draw_seed(worker, seed, hero.id),
                )? {
                    choice = predicted;
                }
            } else if !controlled
                && let Some(history) = history
                && let Some(rollout) = &self.future_planning
                && rollout.forecasts.iter().any(|(id, _)| *id == hero.id)
            {
                let legal: Vec<_> = worker
                    .planning_choices(hero.id)?
                    .into_iter()
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

    pub(super) fn expand_rollout_planning(
        &mut self,
        worker: &mut NativeSession,
        cutoff: i128,
        capture_state: bool,
        lab_evaluation: bool,
        history: &[Vec<PlanningCommitment>],
    ) -> Result<Option<GameTurnRollout>> {
        let preferred = self.rollout_commitments(worker, Some(history))?;
        let fixed_opponents = self
            .future_planning
            .as_ref()
            .filter(|rollout| rollout.fixed_opponent_seed.is_some());
        let (rows, reply_alternative) =
            if fixed_opponents.is_some_and(|rollout| !rollout.legacy_fixed_followup) {
                // Spend the wider follow-up on legal controlled commitments.
                // Keep opponent choices and joint submission order fixed.
                let controlled = worker.joint_plans_with_reply_order(
                    self.player,
                    true,
                    4,
                    self.controlled,
                    JointPlanOrder::Balanced,
                    &[],
                )?;
                (
                    controlled
                        .plans
                        .into_iter()
                        .map(|row| {
                            preferred
                                .iter()
                                .map(|choice| {
                                    row.iter()
                                        .find(|candidate| candidate.hero == choice.hero)
                                        .copied()
                                        .unwrap_or(*choice)
                                })
                                .collect()
                        })
                        .collect::<Vec<Vec<PlanningCommitment>>>(),
                    None,
                )
            } else {
                self.narrow_rollout_rows(worker, history, preferred, fixed_opponents.is_some())?
            };

        let mut best = None::<GameTurnRollout>;
        'rows: for row in rows {
            let mut worst = None::<GameTurnRollout>;
            for reply in [None, reply_alternative]
                .into_iter()
                .take(1 + usize::from(reply_alternative.is_some()))
            {
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
                        branch,
                        cutoff,
                        capture_state,
                        lab_evaluation,
                        Some(branch_history),
                    )
                });
                let leaf = match leaf {
                    Ok(Some(leaf)) => leaf,
                    Ok(None) | Err(EngineError::Deadline | EngineError::Cancelled) => {
                        return Ok(None);
                    }
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
    ) -> Result<NarrowedPlanningRows> {
        let mut row_alternative = None;
        let mut reply_alternative = None;
        for (index, choice) in preferred.iter().enumerate() {
            let team = worker.world.hero(choice.hero).unwrap().team;
            let controlled = self
                .controlled
                .map_or(team == self.team, |ids| ids.contains(&choice.hero));
            if !controlled
                && (fixed_opponents || matches!(self.opponent_policy, OpponentPolicy::FirstLegal))
            {
                continue;
            }
            let slot = if controlled {
                &mut row_alternative
            } else {
                &mut reply_alternative
            };
            if slot.is_some() {
                continue;
            }
            let alternatives: Vec<_> = worker
                .hero_planning_commitments(choice.hero)?
                .into_iter()
                .filter(|candidate| candidate.first != choice.first && candidate.second.is_none())
                .collect();
            if let Some(fallback) = alternatives.first() {
                let cards: Vec<_> = alternatives
                    .iter()
                    .map(|candidate| candidate.first)
                    .collect();
                let first = self
                    .forecast_card(choice.hero, history, &cards)
                    .unwrap_or(fallback.first);
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
}
