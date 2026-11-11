//! Hidden hand response branches and seeded opponent forecasts.

use super::{
    DecisionMove, EngineError, NativeSession, PlanningCommitment, Result, Search, Symbol,
    ValueBound,
};
use crate::interpreter::GreedySearch;

pub(super) fn opponent_draw_seed(worker: &NativeSession, seed: u64, hero: Symbol) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hash = DefaultHasher::new();
    (
        seed,
        hero,
        worker.search_turn_index(),
        worker.last_request.as_ref().map(|r| r.prompt),
    )
        .hash(&mut hash);
    hash.finish()
}

impl NativeSession {
    pub(super) fn sample_forecast_commitment(
        &self,
        hero: Symbol,
        forecasts: &[(Symbol, GreedySearch)],
        history: &[Vec<PlanningCommitment>],
        seed: u64,
    ) -> Result<Option<PlanningCommitment>> {
        let seed = opponent_draw_seed(self, seed, hero);
        if let Some((_, forecast)) = forecasts.iter().find(|(id, _)| *id == hero)
            && history.len() < 4
        {
            let legal = self
                .planning_choices(hero)?
                .into_iter()
                .map(|option| match option {
                    DecisionMove::Card { card, .. } => Some(card),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut prefix = [None; 3];
            for (slot, batch) in prefix.iter_mut().zip(history) {
                *slot = batch.iter().find(|entry| entry.hero == hero).copied();
            }
            if let Some(weights) = forecast.next_card_likelihoods(&prefix[..history.len()], &legal)
            {
                let mut draw = (seed >> 11) as f64 / (1u64 << 53) as f64;
                let mut first = weights.last().unwrap().0;
                for (card, weight) in weights {
                    draw -= weight;
                    if draw < 0.0 {
                        first = card;
                        break;
                    }
                }
                return Ok(Some(PlanningCommitment {
                    hero,
                    first,
                    second: None,
                }));
            }
        }
        Ok(None)
    }
}

impl Search<'_, '_> {
    pub(super) fn pending_hand_branch(&self, worker: &NativeSession) -> Result<Option<Symbol>> {
        match self
            .hand_belief
            .filter(|belief| belief.turn == worker.search_turn_index())
        {
            Some(belief) => worker.pending_hidden_hand_reader(self.player, belief.known),
            None => Ok(None),
        }
    }

    pub(super) fn weighted_hand_response(
        &mut self,
        worker: &mut NativeSession,
        target: Symbol,
        mut continue_search: impl FnMut(&mut Self, &mut NativeSession) -> Result<Option<ValueBound>>,
    ) -> Result<Option<ValueBound>> {
        let belief = self
            .hand_belief
            .take()
            .expect("a hand branch requires a belief");
        let result = (|| {
            let observation = worker.player_observation(self.player)?;
            let id = worker.symbols.name(target);
            let evidence = belief
                .upgrade_checkpoints
                .and_then(|rows| rows.get(id))
                .map(Vec::as_slice)
                .unwrap_or_default();
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
            Ok(Some(ValueBound {
                score: (sum / weight_total) as i64,
                resolved,
            }))
        })();
        self.hand_belief = Some(belief);
        result
    }
}
