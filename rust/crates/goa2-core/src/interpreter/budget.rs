use super::{EngineError, Result};
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkBudget {
    NodesV1,
    NodesV2,
}

pub struct SearchNodeBudget {
    remaining: AtomicUsize,
}

impl SearchNodeBudget {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            remaining: AtomicUsize::new(limit),
        }
    }

    pub(super) fn remaining(&self) -> usize {
        self.remaining.load(Ordering::Relaxed)
    }

    fn reserve(&self, nodes: usize) -> bool {
        self.remaining
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(nodes)
            })
            .is_ok()
    }
}

pub struct ExecutionBudget<'a> {
    pub deadline: Option<Instant>,
    pub cancelled: Option<&'a AtomicBool>,
    pub max_steps: usize,
    pub search_nodes: Option<&'a SearchNodeBudget>,
}
impl Default for ExecutionBudget<'_> {
    fn default() -> Self {
        Self {
            deadline: None,
            cancelled: None,
            max_steps: 1000,
            search_nodes: None,
        }
    }
}
impl ExecutionBudget<'_> {
    pub fn for_search(
        started: Instant,
        time_ms: Option<u64>,
        work_budget: Option<WorkBudget>,
        workers: usize,
    ) -> Result<ExecutionBudget<'static>> {
        let deadline = match (time_ms, work_budget) {
            (Some(ms), None) if (1..=60_000).contains(&ms) => {
                Some(started + std::time::Duration::from_millis(ms))
            }
            (None, Some(WorkBudget::NodesV1 | WorkBudget::NodesV2)) if workers == 1 => None,
            (None, Some(WorkBudget::NodesV1 | WorkBudget::NodesV2)) => {
                return Err(EngineError::InvalidInput(
                    "fixed-node search requires one search worker".into(),
                ));
            }
            _ => {
                return Err(EngineError::InvalidInput(
                    "specify either time_ms in 1..60000 or a fixed-node work_budget".into(),
                ));
            }
        };
        Ok(ExecutionBudget {
            deadline,
            ..ExecutionBudget::default()
        })
    }

    pub(super) fn reserve_search_nodes(&self, nodes: usize) -> bool {
        self.search_nodes.is_none_or(|limit| limit.reserve(nodes))
    }

    pub(super) fn check(&self, steps: usize) -> Result<()> {
        if self.cancelled.is_some_and(|v| v.load(Ordering::Relaxed)) {
            return Err(EngineError::Cancelled);
        }
        if self.deadline.is_some_and(|v| Instant::now() >= v) {
            return Err(EngineError::Deadline);
        }
        if steps >= self.max_steps.min(1000) {
            return Err(EngineError::StepLimit);
        }
        Ok(())
    }
}
