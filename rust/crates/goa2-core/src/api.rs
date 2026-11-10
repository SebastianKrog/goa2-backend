//! Intended pre-1.0 embedding surface for authoritative games.
//!
//! # Trust and supported boundaries
//!
//! Load an identity-checked [`GameContent`] once and reuse it. [`GameContent::read`]
//! rejects malformed JSON, unsupported content versions and differing oracle/source
//! identities. A checksum establishes admitted bytes, not publisher authenticity.
//! Content and imported snapshots are trusted rules data: adapters must bound
//! untrusted input sizes and authenticate callers before invoking the engine.
//! [`LiveGame::create`] validates map, roster and setup; `import` is a trusted
//! migration/fixture boundary, not a player command. `restore` requires a supported
//! checkpoint at a decision boundary, matching rules/source and native resume version.
//! Missing optional v1 history/rollback metadata defaults; future versions reject.
//!
//! # Transaction and failure contract
//!
//! Submit the authenticated hero, current revision and [`PlayerCommand`].
//! [`LiveGame::prepare_submission`] produces an independent candidate and ordered
//! transition without publishing it. Persist the candidate, replay decision and
//! retry receipt atomically before replacing the authoritative game and broadcasting.
//! [`LiveGame::submit`] commits only a successful preparation in memory; it supplies
//! no database transaction, authentication, idempotency receipt or concurrency lock.
//! Invalid/stale revisions, wrong seats, stale request IDs and illegal choices reject.
//! Do not retry an uncertain durable write as a different action ID.
//!
//! [`EngineError::InvalidInput`] describes a rejected input/boundary;
//! [`EngineError::Unsupported`] describes unavailable native coverage/representation;
//! [`EngineError::Rule`] is a rules execution error. These are distinct from
//! [`EngineError::Cancelled`], [`EngineError::Deadline`] and [`EngineError::StepLimit`].
//! Error text is diagnostic, not a stable category or wire schema. Do not convert
//! unsupported execution into a successful result or silently relax version checks.
//!
//! # Determinism, time and budgets
//!
//! Identical admitted content, full-width seed, setup, ordered commands and explicit
//! clock instants produce the same rules progression. Preserve physical-card aliases,
//! list/map order, tie breaks and event order; printed IDs alone are not instances.
//! Use `*_at` methods to supply a coherent millisecond timestamp for timed
//! games; convenience methods use the game's recorded clock timestamp. A checkpoint
//! captures remaining time but does not advance a turn. Restart/rebase policy belongs
//! to the service. Wall-clock deadlines and parallel search completion are not a
//! deterministic replay workload; replay accepted commands and recorded times.
//!
//! [`ExecutionBudget`] checks cooperative cancellation/deadlines between engine
//! instructions, with a maximum of `min(max_steps, 1000)` instructions per advance.
//! Its default is 1000 steps with no deadline or cancellation flag. It does not
//! impose an input-size, allocation or service-wide CPU quota and cannot preempt an
//! individual instruction. On cancellation/deadline/step-limit failure, discard a
//! prepared candidate; retain the authoritative session. Bot recommendation can
//! deliberately return a legal fallback at a deadline; inspect its fallback flag.
//!
//! # Recipient privacy and persistence
//!
//! `client_view(Some(hero_id))` and `client_result` produce scoped player responses;
//! `None` means spectator. Adapters must authorize the chosen identity. Teammates
//! still cannot see another seat's hidden hand/commitment. Team and simultaneous
//! requests are scoped to eligible responders; generic/typed selection values must
//! be forwarded unchanged. `client_result` requires the exact transition revision;
//! events must be filtered through it and retain their order.
//!
//! [`GameCheckpoint`], raw transitions and `private_replay_projection` are private.
//! Never broadcast checkpoints, mutable continuation state, credentials or private
//! search fingerprints. Only a finished, separately authorized share projection may
//! reveal all cards; it is not a checkpoint/replay-archive export. Cloned sessions
//! keep mutable state independent while immutable catalogs may be shared.
//!
//! The compiled `local_game` example demonstrates creation, revision-checked submit
//! and scoped output. Broader arena/search modules remain experimental. This facade
//! is pre-1.0; source-breaking changes require a caller/migration review. Current
//! wire/storage contracts do not change merely because a Rust item is public.
#![warn(missing_docs)]

pub use crate::interpreter::live_game::{
    GameCheckpoint, GameContent, GameMode, GamePosition, GameResult, GameSetup, GameTransition,
    LiveGame, PlayerCommand, SetupTeam, TimeControlConfig,
};
pub use crate::interpreter::{EngineError, ExecutionBudget};
