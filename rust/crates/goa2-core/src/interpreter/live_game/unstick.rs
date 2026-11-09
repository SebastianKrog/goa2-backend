//! Consensus recovery commands for a live game, shared by live play and replay.
use super::{ExecutionBudget, GameTransition, LiveGame, Result, invalid};
use crate::codec::Phase;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnstickCommand {
    SkipInput,
    AbortAction,
    EndTurn,
    ForceActor { hero_id: String },
}

impl UnstickCommand {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SkipInput => "skip_input",
            Self::AbortAction => "abort_action",
            Self::EndTurn => "end_turn",
            Self::ForceActor { .. } => "force_actor",
        }
    }
}

impl LiveGame {
    pub(super) fn table_edit_candidate(&self, now: u64) -> Result<Self> {
        let mut next = Self {
            session: self.session.clone(),
            revision: self
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("game revision exhausted"))?,
            upgrade_history: self.upgrade_history.clone(),
            rollback: None,
            lane_progress: self.lane_progress.clone(),
            clock: self.clock.clone(),
        };
        next.begin_table_edit_clock(now);
        Ok(next)
    }

    pub fn prepare_unstick(
        &self,
        command: &UnstickCommand,
        now: u64,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(Self, GameTransition)> {
        budget.check(0)?;
        let phase = self.session.planning.phase;
        if self.clock_paused() {
            return Err(invalid("The match is paused"));
        }
        if !matches!(phase, Phase::Resolution | Phase::Cleanup | Phase::LevelUp) {
            return Err(invalid("No action is in progress"));
        }
        let mut next = self.table_edit_candidate(now)?;
        let mut previous_request = None;
        match command {
            UnstickCommand::SkipInput => {
                let request = next
                    .session
                    .last_request
                    .as_ref()
                    .ok_or_else(|| invalid("No pending input request to skip"))?;
                if !request.can_skip {
                    return Err(invalid("The pending input does not accept SKIP"));
                }
                let id = request.id;
                let selection = next.session.import_selection(&serde_json::json!("SKIP"))?;
                next.session.submit(id, selection)?;
                next.session.last_request = None;
            }
            UnstickCommand::AbortAction if phase == Phase::Resolution => {
                next.session.unstick_abort()?
            }
            UnstickCommand::EndTurn if phase == Phase::Resolution => {
                next.session.unstick_end_turn()?
            }
            UnstickCommand::ForceActor { hero_id } if phase == Phase::Resolution => {
                let actor = next.hero(hero_id)?;
                previous_request = next
                    .session
                    .last_request
                    .as_ref()
                    .map(|request| next.session.request_token(request.id));
                next.session.unstick_force_actor(actor)?;
            }
            _ => return Err(invalid("This recovery requires the resolution phase")),
        }
        let transition = next.advance(budget)?;
        if let (Some(previous), Some(request)) =
            (previous_request, next.session.last_request.as_ref())
        {
            next.preserve_response_credit(&previous, next.session.request_token(request.id));
        }
        next.update_rollback()?;
        next.reconcile_clock(now)?;
        Ok((next, transition))
    }
}
