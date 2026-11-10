//! Intended pre-1.0 embedding surface for authoritative games.
//!
//! Load a trusted, identity-checked [`GameContent`], create a [`LiveGame`],
//! authenticate callers outside the engine, and submit commands with the current
//! revision and an [`ExecutionBudget`]. Rejected commands must not be committed
//! by adapters. Use `client_view(Some(hero_id))` or `client_view(None)` for player
//! and spectator responses; checkpoints and private replay projections are secrets.
//!
//! Content, rules and save-version disagreement is a deliberate error. Cloned
//! sessions keep mutable physical-card and continuation state private. Search and
//! arena implementation modules remain experimental; see the compatibility guide.
#![warn(missing_docs)]

pub use crate::interpreter::live_game::{
    GameCheckpoint, GameContent, GameMode, GamePosition, GameResult, GameSetup, GameTransition,
    LiveGame, PlayerCommand, SetupTeam, TimeControlConfig,
};
pub use crate::interpreter::{EngineError, ExecutionBudget};
