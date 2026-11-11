//! Reuse exact resolution scores between depth iterations and sampled worlds.

use super::{
    DecisionMove, EngineError, Iteration, NativeSession, ResolutionStop, Result, Search,
    ValueBound, root_move_order,
};

impl Search<'_, '_> {
    pub(super) fn score_sampled_root(
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

    pub(super) fn choose(
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
}
