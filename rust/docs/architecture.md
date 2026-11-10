# Architecture and intended API

`goa2_core::api` is the intended pre-1.0 embedding facade. The compiled
[local_game example](../crates/goa2-core/examples/local_game.rs) loads a trusted
content pack, creates a game, submits a revision-checked command and emits an
actor-scoped view. Authentication belongs to the transport, not the engine.

`LiveGame` owns authoritative progression, revision checks, setup, clocks,
rollback and player projections. `NativeSession` is the lower-level compiled
rules/session interface used by the worker and search. Its representation and
arena slots are experimental. A successfully decoded `StateEnvelope` establishes
codec compatibility, not that every requested operation has native coverage.
`require_native_execution()` intentionally rejects the old codec-only diagnostic
operation; it must not be relaxed to bypass actual compilation/support checks.

The interpreter compiles instructions and frozen templates into a bounded
execution stack. Effects and continuations can suspend at input requests.
Object-aware execution preserves aliases and physical instances through clone,
restore and rollback. Printed-card equality does not identify a physical card.
Immutable catalog/template data is shared process-wide; mutable game state is
private to each worker. Dense execution still serves canonical regressions and
reference tools and has not been retired.

`content/catalogs/` supplies production defaults, templates, primary, action,
defense and spell builders and loadout provenance. `content/manifest.json`
records byte hashes. These are distinct from parity indexes and downloaded
case payloads. `oracle-manifest.json` admits exact source and runtime identities.

Search code under `interpreter/search.rs` and `greedy.rs` operates on private
sessions and public observations. Cancellation, deadlines, node limits and
unsupported-rule errors must remain distinguishable. Private fingerprints are
search-local keys, not portable save formats. See [search](search.md).

The server separates auth/transport (`http.rs`), orchestration (`game.rs`),
persistence (`store.rs` and numbered migrations), replay, shares, clocks and
bots. Accepted commands, state and retry receipts commit transactionally.
[Compatibility](compatibility.md) defines external promises; broad public modules
remain available until their consumers have migrated.
