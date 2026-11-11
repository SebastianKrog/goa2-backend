//! Read-only measurements of repeated resolution and planning states.

use super::{
    DecisionMove, EngineError, ExecutionBudget, NativeSession, Phase, ResolutionReuseCensus, Result,
};

pub(super) enum PlanningPair {
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
        self.census_resolution_node(depth, max_nodes, &mut Default::default(), &mut census)?;
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
                        match previous
                            .clone()
                            .compare_planning_continuation(self.clone())?
                        {
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
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = choice
            else {
                return Err(EngineError::Unsupported(
                    "census requires a live input".into(),
                ));
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

    pub(super) fn compare_planning_continuation(
        &mut self,
        mut other: Self,
    ) -> Result<PlanningPair> {
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
                    DecisionMove::Input {
                        player, selection, ..
                    } => Some((*player, selection.clone())),
                    _ => None,
                };
                if !left.iter().map(comparable).eq(right.iter().map(comparable)) {
                    return Ok(PlanningPair::Diverged);
                }
                for (worker, choices) in [(self as &mut Self, left), (&mut other, right)] {
                    let Some(DecisionMove::Input {
                        request: Some(token),
                        selection,
                        ..
                    }) = choices.into_iter().next()
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
}
