//! Private live-game persistence, import and decision-boundary restore.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{
    ExecutionBudget, GameTransition, LiveGame, NativeSession, PublicUpgradeHistory, Result, clock,
    invalid, lane_progress,
};
use crate::{codec::StateEnvelope, versions::CHECKPOINT as CHECKPOINT_VERSION};

/// Private persistence payload. Never send this to a player or spectator.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameCheckpoint {
    version: u32,
    revision: u64,
    envelope: StateEnvelope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upgrade_history: Option<PublicUpgradeHistory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rollback: Option<StateEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) lane_progress: Option<lane_progress::LaneProgressCheckpoint>,
}

impl GameCheckpoint {
    /// Reproduce the checkpoint wire format used by journals before lane progress was tracked.
    pub fn omit_lane_progress(&mut self) {
        self.lane_progress = None;
    }
}

impl LiveGame {
    /// Import a trusted rules snapshot, for example a migrated Lab Situation.
    pub fn import(
        envelope: StateEnvelope,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(Self, GameTransition)> {
        Self::import_with_rollback(envelope, None, budget)
    }

    /// Import a position and its existing action anchor from a trusted fixture.
    pub fn import_with_rollback(
        envelope: StateEnvelope,
        rollback: Option<StateEnvelope>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(Self, GameTransition)> {
        let (mut game, transition) = Self::open(envelope, 0, None, None, budget)?;
        game.restore_rollback(rollback, None, budget)?;
        Ok((game, transition))
    }

    pub fn restore(checkpoint: GameCheckpoint, budget: &ExecutionBudget<'_>) -> Result<Self> {
        if checkpoint.version != CHECKPOINT_VERSION || checkpoint.envelope.native_resume.is_none() {
            return Err(invalid("unsupported live game checkpoint"));
        }
        let (mut game, transition) = Self::open(
            checkpoint.envelope,
            checkpoint.revision,
            checkpoint.upgrade_history,
            checkpoint
                .lane_progress
                .as_ref()
                .map(|lane| lane.quiet_since),
            budget,
        )?;
        if !transition.events.is_empty() {
            return Err(invalid("live game checkpoint is between decisions"));
        }
        game.restore_rollback(
            checkpoint.rollback,
            checkpoint
                .lane_progress
                .and_then(|lane| lane.rollback_quiet_since),
            budget,
        )?;
        Ok(game)
    }

    pub(super) fn open(
        mut envelope: StateEnvelope,
        revision: u64,
        upgrade_history: Option<PublicUpgradeHistory>,
        lane_quiet_since: Option<i128>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(Self, GameTransition)> {
        let clock = clock::TimeControl::take_from(&mut envelope)?;
        let mut session = NativeSession::compile(envelope)?;
        session.track_attack_source = true;
        let upgrade_history = match upgrade_history {
            Some(history) => history,
            None => PublicUpgradeHistory::new(&session)?,
        };
        let lane_progress = lane_progress::LaneProgress::new(&session, lane_quiet_since)?;
        let mut game = Self {
            session,
            revision,
            upgrade_history,
            rollback: None,
            lane_progress,
            clock,
        };
        game.validate_clock_roster()?;
        let transition = game.advance(budget)?;
        Ok((game, transition))
    }

    pub fn checkpoint(&self) -> Result<GameCheckpoint> {
        self.checkpoint_at(self.clock_timestamp())
    }

    /// Capture the remaining balance at an explicit instant without advancing the game.
    pub fn checkpoint_at(&self, now_ms: u64) -> Result<GameCheckpoint> {
        let mut envelope = self.session.export_envelope()?;
        if let Some(clock) = &self.clock {
            clock.put_into(&mut envelope, now_ms);
        }
        Ok(GameCheckpoint {
            version: CHECKPOINT_VERSION,
            revision: self.revision,
            envelope,
            upgrade_history: Some(self.upgrade_history.clone()),
            rollback: self
                .rollback
                .as_deref()
                .map(|anchor| anchor.session.export_envelope())
                .transpose()?,
            lane_progress: Some(lane_progress::LaneProgressCheckpoint {
                quiet_since: self.lane_progress.quiet_since,
                rollback_quiet_since: self
                    .rollback
                    .as_ref()
                    .map(|anchor| anchor.lane_progress.quiet_since),
            }),
        })
    }

    /// Match the clock initialization when a legacy checkpoint is opened.
    pub fn reset_legacy_lane_progress(&mut self) -> Result<()> {
        self.lane_progress = lane_progress::LaneProgress::new(&self.session, None)?;
        if let Some(anchor) = &mut self.rollback {
            let anchor = Arc::make_mut(anchor);
            anchor.lane_progress = lane_progress::LaneProgress::new(&anchor.session, None)?;
        }
        Ok(())
    }
}
