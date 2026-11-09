//! Track quiet lane turns at authoritative live decision boundaries.
use super::{NativeSession, Result, invalid};
use crate::data::Symbol;
use serde::{Deserialize, Serialize};

const QUIET_TURNS: i128 = 4;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LaneProgressCheckpoint {
    pub(super) quiet_since: i128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) rollback_quiet_since: Option<i128>,
}

#[derive(Clone)]
pub(super) struct LaneProgress {
    turn: i128,
    pub(super) quiet_since: i128,
    zones: Vec<(Symbol, Symbol)>,
    waves: Vec<(Symbol, i64)>,
}

impl LaneProgress {
    pub(super) fn new(session: &NativeSession, quiet_since: Option<i128>) -> Result<Self> {
        let turn = session.search_turn_index();
        let quiet_since = quiet_since.unwrap_or(turn);
        if quiet_since > turn + 1 {
            return Err(invalid("Lane progress checkpoint is ahead of the game"));
        }
        Ok(Self {
            turn,
            quiet_since,
            zones: session.world.battle_zones.clone(),
            waves: session.world.waves.clone(),
        })
    }

    pub(super) fn observe(&mut self, session: &NativeSession) {
        let turn = session.search_turn_index();
        if self.zones != session.world.battle_zones || self.waves != session.world.waves {
            // The turn in which a lane moved or a wave changed is not quiet.
            self.quiet_since = turn.max(self.turn + 1);
            self.zones.clone_from(&session.world.battle_zones);
            self.waves.clone_from(&session.world.waves);
        }
        self.turn = turn;
    }

    pub(super) fn needs_lane_advance(&self) -> bool {
        self.turn.saturating_sub(self.quiet_since) >= QUIET_TURNS
    }
}
