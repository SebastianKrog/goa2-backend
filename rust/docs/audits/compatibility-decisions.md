# Serialization, audiences and retained compatibility decisions

C05-03–06 audit the current supported boundaries, 10 November 2026. These are
local contracts, not a completed historical corpus matrix (C05-01/02). Historical
checkpoints/binaries are not invented. No wire, rules, oracle, content or save
version changes accompany these tests and decisions.

## Serialized boundary policy (C05-03)

| Boundary / implementation | Absent optional fields | Unknown/future behavior and regression |
| --- | --- | --- |
| `GameSetup`, `live_game/setup.rs` | Long mode, cheats off, no explicit tie breaker | Unknown fields reject; full-width seed preserved. `serialized_boundaries_keep_defaults_and_reject_future_identity_and_unknown_fields` |
| `GameContent`, same file | Required identity/roster fields remain required | Top-level unknown fields, content version, oracle/source disagreement reject in that contract test. Internal trusted hero/template model JSON stays extensible. |
| `GameCheckpoint`, `live_game.rs` | v1 upgrade history/rollback absent or null default | Unknown fields and future checkpoint reject; restore requires native resume/decision boundary. `checkpoint_v1_defaults_restore_and_future_versions_reject` plus serialization contract |
| `StateEnvelope`, `codec.rs` | Empty object bindings; optional board keys/native resume | Unknown typed envelope/reference/resume fields reject; schema/rules/oracle/source disagreement and future resume reject. Codec `rejects_bad_references_conflicts_content_and_future_rules`, `native_resume_roundtrips_without_altering_semantic_state`, contract restore checks |
| `PlayerCommand`, `live_game.rs` | Only declared variant fields accepted | Unknown structured fields/variants reject; unit variants retain Serde's extra-field tolerance. JSON `selection` remains typed arbitrary rules input. `command_unknown_fields_and_stale_revisions_do_not_mutate_game` |
| Starting positions/table slots | Variant-specific fields | Nested enums currently permit extra fields. Retain this additive tolerance; unknown operations/zone variants reject. Frontend deployment suite covers consent/restart; no new strictness imposed. |
| Time control, `live_game/clock.rs` | Initiative bonus 0, automatic limit 2, timeout enforcement true; internal clock defaults explicitly annotated | Extra configuration fields remain tolerated. Serialization contract tests defaults; clock service suite tests restart, ready/deadline behavior. Negative or out-of-range u64 values reject. |
| REST creation, `http.rs::CreateRequest` | Default map/long mode, empty names, no bots/time control; seed may be generated | Extra frontend fields remain tolerated. Backup suite creates with `future_optional` and maximum u64 seed. Wire REST/WS use their own field parsing; do not assume the strict engine enum applies to frontend packets. |
| REST/WS command parsing, `http.rs` | Optional revision/action receipt IDs retain existing legacy behavior | Unknown message types reject; known packets retain existing additive field handling. Auth suite covers stale/reused receipt, malformed/unauthorized commands and fresh tokens. Tightening extras would require consumer migration. |
| Private replay `Archive`/`Entry`/`Decision`/`Operation`, `replay.rs` | Only optional rebase time defaults | Unknown archive/entry/decision and structured operation fields/types and future archive schema reject. Unit operation variants tolerate extra fields under Serde; the new regression preserves that distinction. Replay serde unit test preserves maximum-width time; backup suite rejects future schema and unknown archive fields; auth verifies tampering/noncontiguous journals. |
| Stored credentials, `store.rs::Credentials` | Bots empty; no fixed time/provision hash | Extra stored fields tolerated on read, not retained on rewrite; no promise of forward write compatibility. New unit test preserves tokens and optional defaults. |
| SQLite, numbered migrations | Legacy columns migrate transactionally | Future database schema rejects before mutation. Existing migration tests cover versions 0–3, partial migration failure and idempotence. |
| JSON-lines worker, `goa2-search-cli/src/main.rs` | Declared defaults only; request correlation unchanged | Unknown fields and future schema reject; `protocol_rejects_unknown_fields_and_preserves_prior_board_on_error` protects prior board. |
| Share metadata/groups, `shares.rs`/store | Output-only baked groups and seek defaults | Not a private archive import boundary. Only finished-game projection is shared. Backup suite checks public frames contain no credentials/native continuation and survive independent restore. |

Unknown-field handling is intentionally heterogeneous. Strict typed private saves
prevent silent data loss; additive frontend/configuration tolerance is retained.
Future-schema rejection is never weakened to make a fixture load. A deserialized
`Value` is not proof of native support or authorization.

## Recipient and freshness coverage (C05-04)

| Audience/contract | Executed coverage |
| --- | --- |
| Actor, teammate, opponent, spectator | Native hidden-hand contract and new `ordered_events_hide_facedown_identity_for_teammate_and_spectator`; two marker events retain order while private card metadata is filtered. Actor/private projection retains the identity. |
| Team decision | New `team_decision_is_available_to_eligible_teammates_without_exposing_other_seats` uses an admitted bounded tie case; only eligible red teammates receive the same actor choices. |
| Simultaneous decisions | New synthetic `simultaneous_upgrade_prompts_are_recipient_scoped_and_cannot_submit_for_another_seat` checks per-seat option metadata, spectator absence and wrong-seat rejection without mutation. Votes service suite still exercises actual upgrades. |
| Generic labels and submission types | New `generic_number_labels_preserve_typed_choices_and_request_freshness` uses the admitted select-step recipe: numeric values stay numeric while labels remain strings; stale-token rejection leaves checkpoint unchanged. |
| Request freshness, rollback, stale revisions | Auth suite covers REST/WS, rollback/restart, distinct reissued IDs, consumed-token rejection, stale revisions and idempotent/conflicting retries. Native identity-history regressions cover restored and exhausted request/step generations. |
| Private replay and public share | Auth `verify_replay` checks credentials, access, ordered contiguous revisions, hashes and reconstruction. Backup suite tests finished synthetic share frames and stable links/groups after online backup; unfinished sharing rejects. This does not replace the full completed-match/hosted suite C08-04. |
| Wide seed/time and clocks | Setup contract and backup use maximum u64 seed; replay decision unit test roundtrips maximum u64 timestamp and rejects negative time. Clock service contracts check running/pause/restart behavior. JavaScript consumers must preserve integers, not pass them through lossy numeric conversions. |

The small contracts do not claim complete frozen view parity: that remains the
explicit ignored gate with its missing view corpus under C04-05.

## Shim classification and removal gates (C05-05/06)

Owner roles remain engine/service/search/release maintainers until named owners
are assigned under C07. The following decisions retain behavior; **no supported
consumer is removed and no breaking cleanup is proposed in this batch**.

| Shim / status | Concrete consumer and regression | Removal condition / migration note |
| --- | --- | --- |
| Dense session execution — required | Canonical stack traces; interpreter restore/identity tests and worker `stack_trace` callers in `worker-operations.json` | Migrate all canonical/reference/restore callers to object-aware execution; preserve unique cases and full differential/restore evidence before removing dense storage. |
| Codec-only execution guard — required rejection | `state_codec.py`, worker `validate_execution`; codec alias/native-resume tests deliberately reject old transition path | Migrate diagnostics to actual native compilation with explicit capability/versioned operation. Never change guard rejection into apparent rules execution. |
| `legacy_fixed_followup` — required reproducibility | `rollout_parity.py::verify_strength_baseline` control baseline; search fixed-opponent planning tests | Record independent old/new baseline evidence and migrate every arena/evaluation consumer; do not change policy semantics under the same comparison label. |
| Plain JSON plus gzip/cache readers — required | `fixture_cache.py`, native test-support reader, all corpus consumers; compression/corruption/identity tooling tests | Announce a format/acquisition migration, preserve authoring and old admitted hashes, and migrate all retained readers first. |
| Worker diagnostics/broad core modules — temporary experimental exposure, retained | Worker operation inventory and reference/search Rust callers; worker protocol regression | Complete C09 caller/API migration and preserve unique diagnostics before restricting exports; pre-1.0 is not a no-consumer assumption. |
| `verify_rust_game_server` compatibility exports — required | Hosted-draft/replay-share verifier imports `serve`, `request`, `backup_database`, `packet`, `hero`, `sockets` | Migrate those exact imports together; keep process/SQLite/transport contracts. Extraction includes both entry point and modules. |
| Legacy REST `advance` — required nonadvancing adapter | Reference frontend/REST callers; auth suite checks current boundary result, no journal/revision advancement and recipient scoping | Version/deprecate only after all frontend consumers migrate; never reinterpret as an extra rules step. |
| `cargo-wsl.sh` — required compatibility entry point | Historical contributor/verification commands; delegates to portable `cargo-dev.sh` | Migrate documented/caller commands with deprecation notice. Retention does not claim native Windows support. |
| Optional save/credential defaults — required | Retained v1 saves and legacy SQLite; serialization/migration/restart tests | A future version needs an explicit migration and supported-reader window; do not silently remove defaults or overwrite unknown future state. |

No item was classified obsolete merely because its consumer is optional or its
full input is presently unavailable. Proposed future breaking changes need the
named consumer's replacement path, producer/consumer versions, migration/backup
instructions, deprecation window and independent old/new verification. Rules
corrections require a separate failing scenario and rules/content-version decision,
not a mechanical cleanup commit. This retain decision satisfies the conditional
breaking-cleanup review without inventing a version migration that did not occur.
