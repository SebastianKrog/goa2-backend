# Rust engine cleanup plan

> Implementation: native safeguards, tooling and extraction work are tracked in
> [Rust readiness](../rust/docs/readiness.md). That record distinguishes verified
> work from remaining artifact, licensing and later-refactor gates.
> Track all outstanding work in the [remaining-task checklist](../rust/docs/cleanup-checklist.md).

Prepare the existing GoA2 Rust engine for a standalone public repository where contributors can build, test, understand, and change it without access to the original developer’s machine or private development history.

The first priority is reproducibility and clear contracts. Preserve the working engine and 2D frontend integration while separating runtime content from test fixtures, establishing a dependable test workflow, and documenting the supported interfaces. Larger interpreter and search refactors should follow those safeguards. A complete architectural rewrite is not a prerequisite for opening the repository.

## Scope and baseline

Reviewed on 10 November 2026 against backend commit `f0fb93e117f358266ef522e6d5f23d3452e83e97` and frontend commit `a3c5e0a46b35a42cd6e054512ce3dbc6a61a579c`. Paths below are relative to `goa2-backend/` unless prefixed with `goa2-frontend/`. Proposed paths describe the future repository.

The scope includes all three Rust crates, their runtime catalogs, fixtures, Python export and verification tooling, deployment packaging, documentation, and the frontend boundary. Python gameplay and frontend redesign are outside the cleanup scope; both remain relevant as reference implementations and consumers.

### Observed state

| Area | Finding | Implication |
| --- | --- | --- |
| Workspace | Three crates: `goa2-core`, `goa2-search-cli`, and `goa2-game-server`; edition 2024; Rust pinned to 1.99.1; workspace version 0.1.0; MIT declared; publishing disabled | A sensible starting boundary already exists. Public source hosting does not require crates.io publication. |
| Size | 176 Rust files and approximately 99,623 lines, including tests | Cleanup needs staged changes with meaningful regression checks. |
| Concentration | `interpreter.rs` is 12,594 lines; `search.rs` 5,559; `object_execution.rs` 4,476; its integration test file 4,060 | The main contributor navigation problem is concentrated in a few files. |
| Documentation | Rust README is 5,295 lines; fixture README is 3,247 | Operational instructions, current architecture, historical coverage, and development logs need separate homes. |
| CI | `.github/workflows/ci.yml` installs Python and runs Python lint/type/test checks; there is no Rust job | Rust build, formatting, lint, tests, and docs need explicit CI coverage. |
| Fixtures | Canonical index has 6,592 cases; live object-execution index has 17,347. Every referenced case payload in those two indexes is absent locally | A checkout cannot run the major parity suites without an additional acquisition or regeneration step. |
| Runtime data | Rule templates, builders, defaults, primary effects, and loadout provenance are compiled from `rust/tests/fixtures/` | Files called fixtures include production dependencies. They must be classified before removal or relocation. |
| Oracle tooling | 151 tracked Python files under `scripts/rust_oracle/` and 84 under `tests/rust_oracle/` | The Rust project has a substantial development dependency outside `rust/`. Copying that directory alone is insufficient. |
| Server tests | No Rust test attributes in the server source; a 1,906-line process verifier and other Python verification scripts exist | Server coverage exists outside Cargo and needs a visible, maintained entry point. |
| Public project files | No root license text, contribution guide, security policy, code of conduct, or changelog is tracked | License metadata and repository collaboration materials need deliberate completion. |

### Checks performed

| Check | Result |
| --- | --- |
| Bootstrap oracle integration tests, pinned toolchain, locked dependencies, iterate profile | 2 passed: movement parity and state/identity roundtrip |
| Explicit native setup parity test using the prepared content and setup cases | Passed; matched 28 opening states and identity graphs |
| `cargo fmt --all -- --check` through the repository wrapper | Failed; differences in 26 distinct files |
| Strict Clippy, locked dependencies, iterate profile, workspace and all targets | Failed while checking the core library, with 11 diagnostics; later targets are not established as clean |

Clippy findings include collapsible control flow, `bool::then` in `filter_map`, four functions with excessive argument counts, a complex return type, and a manual multiple-of check. Full corpus tests, process-level service verification, hosted deployment verification, benchmarks, and the full frontend suite were not run for this planning review. Full corpus payloads are missing; existing milestone reports are historical evidence rather than a current full-suite result.

No engine, frontend, fixture, or dependency source was changed during this review.

## Cleanup principles

1. Establish behavior and compatibility baselines before restructuring code.
2. Keep mechanical moves, behavior fixes, oracle repins, and format migrations in separate changes.
3. Give each retained compatibility path a named consumer and test. Age alone does not make a path obsolete.
4. Give every fixture family a purpose, generator, provenance, and retention decision. Preserve unique coverage.
5. Make ordinary Rust development possible without the Python engine, full Git history, private assets, or Layout Lab.
6. Keep expensive reference verification available and enforce it for changes that need it. A reduced developer suite must never masquerade as full parity.
7. Preserve existing protections: unsafe code is forbidden, views are scoped to recipients, content identities are checked, saves and receipts are transactional, and private replay state is separated from public views.

## Prioritized work packages

P0 items block extraction or public readiness. P1 items materially improve collaboration and should land before or soon after opening. P2 items are staged follow-up refactors once regression coverage is dependable. Effort estimates are engineering days for one maintainer familiar with the code, including review and verification; they are planning ranges, not commitments.

| ID | Priority | Work package | Effort | Depends on |
| --- | --- | --- | --- | --- |
| C01 | P0 | Record extraction inventory and supported contracts | 1–2 | None |
| C02 | P0 | Make build and content acquisition reproducible | 2–4 | C01 |
| C03 | P0 | Classify runtime catalogs and fixture corpora | 3–5 | C01 |
| C04 | P0 | Establish test tiers and Rust CI | 3–5 | C02, C03 |
| C05 | P0 | Define and test compatibility policy | 2–4 | C01; C04 for automated gates |
| C06 | P0 | Replace historical onboarding with current documentation | 2–3 | C01–C05 |
| C07 | P0 | Complete public repository and release hygiene | 1–3 | C01–C06 |
| C08 | P1 | Organize server verification and persistence migrations | 3–5 | C04, C05 |
| C09 | P1 | Define the Rust library and worker API boundaries | 2–4 | C04, C05 |
| C10 | P2 | Reduce interpreter and search coupling | 5–10 | C03, C04, C09 |
| C11 | P2 | Improve validation, diagnostics, and performance maintenance | 3–6 | C04; coordinate with C08–C10 |
| C12 | P0 | Rehearse standalone extraction and frontend integration | 1–2 | C01–C07; identified blockers from C08–C09 |

C01–C07 and C12 total roughly 15–28 engineering days before overlap. Missing redistribution rights or inaccessible oracle artifacts could extend that range. C08–C11 need not all finish before public source hosting: land the compatibility-critical parts first and publish the remaining work as issues.

### C01 Record the repository boundary and baseline

Create an extraction manifest that assigns every required path to runtime, development, reference verification, deployment, or historical material. Include embedded JSON and manifest paths, the Rust test-support modules, Python generators and their dependencies, schema/catalog exporters, packaging scripts, and service verifiers. Record the original commit and the destination for each retained group.

Trace dependencies in both directions. Examples already found include `include_str!` paths escaping a crate into the workspace fixtures directory; Python tools resolving the backend root from `__file__`; `run_pinned.py` extracting `src/goa2` and `src/automata` with `git archive`; packaging loading `rust/deploy/goa2-rust.service`; and hosted-draft verification requiring Python service code.

Create a small set of synthetic, redistributable baseline games covering manual 2D play, bots, timed play, rollback, upgrades, restart, and replay. Record expected commands, views, events, revisions, and content identity. Never copy real player credentials or private match state into public fixtures.

**Complete when:** the extraction manifest explains every external dependency, supported consumers are listed, baseline artifacts have provenance, and a clean source/build status is recorded.

### C02 Make build and content acquisition reproducible

Keep `Cargo.lock` and the pinned toolchain. Decide and document whether 1.99.1 is the required compiler or whether an older minimum supported Rust version will be tested. Add `rust-version` once that policy is established. Verify the compiler policy against file-lock APIs used by `Store`, edition 2024, and the dependency graph; do not infer a minimum from the edition alone.

Use ordinary Cargo commands as the primary instructions. Rename or supplement `cargo-wsl.sh` with a platform-neutral development entry point while retaining its useful target-directory and bounded-concurrency defaults. Document Linux first; test macOS and Windows if claiming support. Hosted service packaging can remain explicitly Linux-only. Document the C compiler requirement for bundled SQLite.

Provide a versioned, checksum-verified route to `live-game-content.json`. The service runs without Python once it has a valid pack, but a new contributor currently needs Python and historical backend sources to create one. Recommended split:

- Ordinary builds and quick tests use bundled runtime catalogs and a small public test pack.
- Running a full local game uses a verified published content pack, subject to content licensing.
- Reference maintainers can regenerate the exact pack using a separately pinned oracle source artifact and locked Python environment.

The oracle artifact must contain the exact admitted source and runtime identity. `run_pinned.py` currently requires a historical commit available in the original Git object database. Replace that hidden requirement with an explicit artifact or pinned external checkout, checksum verification, and actionable acquisition errors. Do not move the oracle pin to current Python code merely to simplify extraction.

**Complete when:** documented build, quick-test, content acquisition, and local server startup succeed from a clean directory with no sibling repositories, warm caches, or original Git history.

### C03 Separate runtime content from fixtures and retire legacy material deliberately

Introduce distinct locations for production catalogs, small public regression examples, full parity indexes, ignored/downloaded corpus payloads, and benchmark inputs. Suggested destinations are `content/catalogs/`, `tests/fixtures/smoke/`, `tests/fixtures/parity/`, and a configurable cache outside the source tree.

Move production catalogs mechanically first, preserving bytes and identity checks. These include instruction templates, effect defaults, primary effects/builders, action and defense builders, spell builders, and loadout provenance. Audit all `include_str!` consumers, not just their filenames. If crates.io distribution is later enabled, check that packaged crates include their required data; current paths outside the crate would need redesign.

Create a fixture registry with family, purpose, schema, rules version, oracle identity, generator command, expected counts/hashes, consumers, download location, licensing status, and owner. Reconcile `fixture_cache.COMMITTED`, `rust/.gitignore`, and that registry automatically.

| Material | Initial treatment | Requirement before deletion or change |
| --- | --- | --- |
| Production catalogs under fixtures | Keep and relocate | Same content identities and native behavior after relocation |
| `movement.json` and `states.json` | Keep as bootstrap regressions | Replace only with equivalent or better documented coverage |
| Canonical `stack-traces.json` corpus | Keep as reference coverage initially | Prove any replacement preserves its unique dense-path and protocol coverage |
| Live `object-execution.json` corpus | Keep as authoritative differential coverage | Preserve state, request, event order, aliases, and restoration assertions |
| Complete-game indexes and snapshot chunks | Keep as end-to-end coverage | Verified acquisition and replay of every retained game and chunk |
| Error fixtures and guard cases | Keep until classified | Distinguish intentional rejection contracts from historical porting limitations |
| `.json` and `.json.gz` readers | Keep initially | Inventory real consumers before removing plain-file support; it is also useful for authoring |
| Milestone validation JSON | Archive with commit/toolchain/scope metadata | Check references and extract any unique regression input first |
| Large generated payloads and build outputs | Keep outside Git | Preserve deterministic regeneration or verified artifact access |
| Unreferenced scratch reports and duplicated outputs | Delete only after inventory | Establish they contain no unique scenario, source, or provenance |

The current compression workflow already has valuable properties: deterministic gzip, atomic publication, logical paths unchanged, and case-by-case decompression. Preserve these. Do not turn cleanup into full-corpus extraction, whole-corpus memory caching, or a mass fixture regeneration.

Audit cache identity and invalidation across schema, oracle, rules, generator version, and runtime dependencies. Preserve physical-instance identity, dictionary/list order, null semantics, and semantic hashes. Human-readable formatting changes must not accidentally redefine semantic identity.

**Complete when:** every retained file has a category; runtime builds no longer depend on a misleading test directory; full corpora can be acquired and verified; and every deleted family has a documented coverage replacement or retirement decision.

### C04 Establish test tiers and Rust CI

The normal contributor command should pass in a fresh checkout and exercise useful behavior. Several current unit tests also load corpus fixtures, so splitting only by Cargo integration-test target is insufficient. Introduce explicit suite membership and move the small scenarios required by fast tests into the public smoke set.

| Tier | Content | When it runs |
| --- | --- | --- |
| Quick native | Unit tests, small native scenarios, bootstrap codec/movement tests, small setup/view/privacy checks | Every PR; no Python reference source or large downloads |
| Service contract | REST and WebSocket behavior, auth, retries, stale writes, restart, persistence, replay and shares | Server changes and required release checks; use disposable processes/databases |
| Full differential | Canonical/live corpus, complete games, request/event/identity comparisons | Engine changes and release candidates; scheduled runs as a backstop |
| Restore exhaustive | All required save/restore boundaries and worker isolation | Restore/codec/arena changes and release candidates |
| Performance and strength | Fixed-work throughput, decision latency, node counts, search quality and matched arena evaluation | Search/hot-path changes and releases; separate from ordinary correctness jobs |
| Hosted integration | Draft provisioning, Worker routing, ingress restrictions, backup/restore | Relevant integration changes and deployment promotion |

Add Rust CI with the pinned compiler, locked build, rustfmt, strict Clippy, quick native tests, and rustdoc/doctests. Keep the Python tooling tests in their own job with their actual dependency lock; the backend’s current Ruff/Black commands target `src tests`, so scripts need explicit coverage too. Use artifact caches as accelerators and verify fixture identities after retrieval. Test the cold path periodically.

Fix formatting in a separate mechanical commit. Resolve the current 11 Clippy diagnostics, then rerun to discover later-target findings. For argument-heavy search methods, prefer a meaningful options/context type; allow a lint locally with rationale when a forced rewrite adds no clarity. Do not blanket-disable warnings or combine automatic lint fixes with rules changes.

Make expensive tests explicit, named, and counted. Existing ignored setup/view/restore/benchmark gates need owners and scheduled commands. Missing fixtures must fail a selected parity job, not silently reduce its scope. Reports should distinguish expected, executed, skipped, failed, and missing cases and detect zero-match filters.

**Complete when:** a fresh contributor can run the documented quick suite; CI invokes the correct tiers; expensive checks cannot disappear behind successful reduced runs; and the current format/lint failures are cleared.

### C05 Define backwards compatibility by boundary

Separate code API compatibility from wire compatibility, persistent data compatibility, rules semantics, and experimental tooling. A public Git repository does not imply every existing internal `pub` item or diagnostic operation must be supported indefinitely.

| Boundary | Existing evidence | Proposed policy |
| --- | --- | --- |
| Frontend REST/WebSocket protocol | `goa2-game-server/src/http.rs`, core `live_game/view/`, frontend `src/types/game.ts`, `src/api/`, `useGameSocket.ts` | Preserve existing message names, request/submission values, scope rules, revisions, retry acknowledgements, and error behavior during cleanup. Prefer additive changes. |
| Saved games and checkpoints | `GameCheckpoint` version 1, defaulted history/rollback fields, `NativeResume` | Maintain loadability for explicitly supported saved versions; test historical examples and restore semantics before changes. |
| SQLite | Inline creation and conditional alterations in `store.rs` | Introduce numbered, tested schema migrations and backup/rollback guidance; do not rely only on current-schema creation. |
| Replay archives and public shares | `replay.rs` schema 1; per-revision digests; separate share projection | Preserve reconstruction and digest behavior; private archives remain private and public shares retain their filtering. |
| Content packs and envelopes | Exact schema, rules-v9, oracle revision, and source-hash checks | Keep deliberate incompatibility rejection. An unsupported oracle/rules version must not be accepted merely by relaxing validation. |
| JSON-lines worker | Versioned request/response packets and Python callers | Document retained operations and consumers; deprecate obsolete diagnostic operations explicitly. |
| Rust API | Broad public modules and session/search methods | Define a small intended API, mark internals/experimental areas, and record pre-1.0 change policy. Restrict visibility only after caller inventory. |
| Search policies and private keys | `legacy_fixed_followup` is still used by rollout parity and arena configuration | Keep reproducibility controls while they have baseline consumers; private search hashes are not save-format promises. |

Add a compatibility matrix with producer version, consumer version, data schema, expected result, and migration path. Include current frontend against the cleaned server; older supported saved data against the new server; unsupported future versions rejected safely; and retry/rollback behavior through restart. Explicitly test absent optional fields and strict `deny_unknown_fields` boundaries before changing serialized structs.

Keep recipient filtering central: player, teammate/team decisions, spectator, private replay, and shared replay are different audiences. Preserve hidden choices, generic prompt labels, request-token freshness, 64-bit seed/time fidelity, event order, and stale-command protection. Preserve aliases for physically distinct cards and shared models through cloning and restore.

Do not preserve accidental internal behavior forever. Classify each compatibility shim as required, temporary, or obsolete; attach a consumer, test, removal condition, and migration note. Remove truly unused paths before public release where possible. Rules corrections must be separate changes with regression examples and a deliberate rules/content-version decision.

**Complete when:** supported boundaries and versions are written down, contract regressions run automatically, and every proposed breaking cleanup has either no supported consumer or a concrete migration/deprecation path.

### C06 Rewrite documentation around contributor tasks

Replace the current README with a short entry point covering purpose, current capabilities, crate responsibilities, build requirements, quick tests, running a game, and links to focused guides. Move milestone narratives and benchmark history to an archive with dates and commit identities. Preserve useful provenance without making readers reconstruct current behavior from chronological logs.

Create focused guides for architecture, contribution workflow, testing and fixtures, content generation, frontend protocol, persistence and compatibility, search and benchmarking, and operations. Supply a small Rust example that loads content, creates a game, submits a command, and produces a scoped view. Explain `NativeSession` versus `LiveGame`, object identity, compiled instructions, effects and continuations, rules catalogs, and bot observations.

Reconcile misleading status statements. The CLI module still says native search is not advertised; `StateEnvelope::require_native_execution()` always rejects and is still used by a diagnostic operation and tests. Document whether that operation is intentionally a codec-only guard or should be deprecated. Do not blindly change it to accept execution. The introductory README describes a backend milestone that later sections already implement.

Replace the Python-focused agent onboarding with concise Rust guidance for the extracted repository. Reconcile `docs/MAINTAINABILIY_ROADMAP.md`, `AI_STACK_CLEANUP.md`, and older plans by marking their Python/historical scope rather than carrying them forward as active Rust instructions. Fix broken sibling links, including the shared frontend decisions plan absent from the supplied frontend checkout. Move the actual shared contract into a versioned, accessible document.

Add rustdoc to the intended public surface, including failure conditions, determinism, trusted input boundaries, private state, cancellation, and examples. Use a documentation build gate, then introduce missing-doc checks for the supported facade rather than requiring comments on every internal item at once.

**Complete when:** someone new can build, test, start a game, locate a rule implementation, and understand compatibility without reading milestone logs or private notes; links and examples are verified.

### C07 Complete public repository and release hygiene

Confirm the intended license and add its full text with attribution. `license = "MIT"` alone does not establish rights for upstream code, generated rule definitions, hero/card text, names, map data, or third-party assets. Record which material can ship in source, content packs, and public fixture artifacts; separate restricted material if needed. Preserve required notices and source attribution in the extraction.

Add `CONTRIBUTING`, a changelog, issue and PR templates, a maintainer/reviewer ownership map, security reporting instructions, and a conduct policy suited to the intended community. Contribution guidance should include how to add a rule/card, update generated data, choose test tiers, report a reproducible bug, and propose a protocol or persistence change.

Inspect the proposed public tree and, if retaining history, the history for credentials, private replay state, databases, environment files, personal paths, large generated blobs, and nonredistributable material. Do not equate `.gitignore` with removal: 29 validation artifacts remain tracked despite `/artifacts/` being ignored. Choose a history-preserving filtered extraction or a clean initial snapshot deliberately; retain an original-commit mapping and oracle source provenance either way. Do not rewrite the original repository as part of cleanup.

Keep crates.io publication disabled until crate naming, API stability, packaged data paths, metadata, and packaging verification are ready. Add repository/description/readme metadata after the destination is known. Review dependencies with a license/advisory tool and document exceptions. No vulnerability finding is asserted by this review.

For server releases, retain the existing deterministic archive, binary/content hashes, validation command, and hardened systemd unit. Add release notes, supported architecture, provenance, content identity, database migration requirements, backup procedure, and a verified rollback process. Distinguish checksums from authenticity if distributing binaries publicly.

**Complete when:** the exact public payload is identified, licensing and attribution are resolved, contribution instructions and checks work, and releases can be traced to source and content versions.

### C08 Organize server verification and persistence

Split the large service verifier into reusable process/API/WebSocket helpers and suites for auth/views, commands, retries/concurrency, clocks/votes, persistence/backup, replay/shares, and bots. Retain the cross-process restart checks: replacing them entirely with mocks would lose valuable coverage. Make the suites discoverable through one documented command and CI.

Extract numbered SQLite migrations with an explicit schema version and transaction behavior. Test databases created by each retained schema, repeated startup, failure during migration, existing replay journals, credentials, accepted-action receipts, and share metadata. Verify that backup captures a consistent WAL snapshot and that restoring it preserves links, revisions, and receipts. Define when binary rollback requires restoring a database backup.

Consider a server library/test harness if it meaningfully improves route/store testing; the current server is a binary with private modules. Keep transport/auth, orchestration, persistence, replay, and engine rules as distinct responsibilities. Audit blocking work, lock scope, bot cancellation, deadline recovery, and shutdown around these boundaries.

**Complete when:** service tests are independently runnable and diagnosable, every supported schema has an upgrade test, and failure/restart behavior is documented and exercised.

### C09 Define library and worker APIs

Inventory callers before changing the broad module exports in `goa2-core/src/lib.rs`. Present `LiveGame`, setup/content, commands, scoped responses, checkpoints, and deliberate search entry points as the main API. Keep arena slot layouts, compiler state, temporary profiling structures, and import machinery internal where possible. Start with module boundaries; add another crate only if it materially clarifies dependencies or build costs.

Document or isolate worker diagnostics from supported automation. `goa2-search-cli` contains a large operation dispatcher serving many Python tools. Track each operation’s caller and status; remove unused porting operations only after migrating their callers. Add protocol examples, schema/error tests, and a clear distinction between unsupported rules, invalid input, budget exhaustion, cancellation, and internal errors.

Improve structured diagnostics at external boundaries. `EngineError` currently displays its debug representation, and the server maps it broadly to an invalid-request response. Introduce stable internal categories and useful context while preserving the supported wire behavior or versioning any intentional change.

**Complete when:** contributors can identify the supported API, internal implementation changes do not unnecessarily become public promises, and retained worker operations have documented callers and tests.

### C10 Reduce interpreter and search coupling

Start by moving existing code with no semantic change. Split `interpreter.rs` into instruction definitions/compilation, execution/session state, requests/events, import/export, budgets, and tests. Continue the existing `object_execution/` and `live_game/` modularization. Replace broad `use super::*` coupling with explicit dependencies where it helps reveal ownership.

Separate search orchestration, evaluation, hidden-world sampling, move generation/order, rollout, reuse, diagnostics, and result types. Extract meaningful options/context objects for repeated argument groups. Keep deterministic tie breaking, random draws, cancellation, deadline and node accounting, and fallback behavior unchanged.

Inventory dense and object-aware execution paths. Source still explicitly describes transitional dense execution, and canonical tests cover it. Determine which path each live, worker, search, and restore caller uses; migrate callers and retain unique regression scenarios before deleting an implementation. Similar method names are not proof of duplicated behavior.

Replace magic recipe operand sentinels, such as `1234568103` and `1234568104`, with a documented typed representation through a versioned generator/catalog change. Keep generator and consumer changes synchronized, and compare old and new semantics independently. Centralize version constants and identity metadata while retaining disagreement checks.

**Complete when:** responsibilities can be followed without reading giant files; retained regression suites produce equivalent outcomes and identities; retired paths have no callers; and representative fixed-work performance has not materially regressed.

### C11 Improve robustness and performance maintenance

Audit panic assumptions at trust boundaries. Many `unwrap()` calls are in tests; a raw count is not a defect list. Prioritize externally supplied content, archives, state envelopes, worker JSON, WebSocket commands, and persisted data. Preserve explicit limits on steps, generated entities, archive size, and execution budgets; document which imports are trusted.

Add targeted property/fuzz tests for malformed envelopes, reference tables, hex coordinates, catalog deserialization, replay inputs, and command sequences where they cover meaningful risk. Useful invariants include no mutation after rejected input, isolation between cloned workers, consistent physical-card references, save/restore equivalence, and no unauthorized hidden information in responses.

Adopt reproducible benchmark fixtures with content identity, compiler/profile, hardware, seed, policy, workload, and measured units. Track cold startup, memory, clone/restore cost, throughput, decision latency, and cancellation responsiveness. Use fixed work for deterministic comparisons; report deadline-limited policy quality separately. Historical speed targets and milestone results must retain their original scope.

Review runtime catalog parsing/initialization and binary footprint after the data move. Preserve process-wide immutable caches and private mutable continuation state. Optimize only after measurement, with correctness and identity checks around any representation change.

**Complete when:** key malformed-input paths fail predictably, high-value invariants are exercised, and hot-path refactors have a reproducible comparison rather than an anecdotal speed claim.

## Proposed standalone layout

```text
Cargo.toml
Cargo.lock
rust-toolchain.toml
README.md
LICENSE
CONTRIBUTING.md
CHANGELOG.md
SECURITY.md
crates/
  goa2-core/
  goa2-game-server/
  goa2-search-cli/
content/
  catalogs/
  manifest.json
tests/
  fixtures/smoke/
  fixtures/parity/          # indexes and small metadata
  support/
tools/
  oracle/                  # pinned source acquisition and export tools
  verification/
  packaging/
docs/
  architecture.md
  testing.md
  compatibility.md
  frontend-protocol.md
  content.md
  search.md
  operations.md
  archive/
deploy/
.github/workflows/
```

Choose final catalog placement after deciding how crates will be packaged. Keep Python as optional reference tooling with its own locked environment. Full corpora and game-content packs should have an explicit artifact/cache location rather than appear to be missing source files.

## Delivery sequence and review boundaries

1. **Baseline and contract inventory:** C01 and the policy portion of C05. Record the supported frontend and data versions before any moves.
2. **Mechanical quality baseline:** formatting-only change, then focused lint fixes. Establish a minimal Rust CI job immediately and expand it with C04.
3. **Data and fixture ownership:** C03, followed by the content/oracle acquisition work in C02. Preserve catalog bytes and corpus identities across moves.
4. **Reliable verification:** finish C04 and compatibility regressions. Integrate the existing service checks; prioritize migration and privacy/retry tests from C08.
5. **Contributor entry point:** C06 and C07. Prepare the license, source notices, guides, examples, and release instructions against the actual standalone layout.
6. **Extraction rehearsal:** C12. Fix all fresh-checkout or frontend blockers before the repository is opened.
7. **Staged architecture improvements:** finish C08/C09, then C10/C11 as separately reviewable changes. Publish the remaining backlog with owners and acceptance criteria.

Each change should identify whether it alters source layout, runtime behavior, generated content, a public protocol, or persistent formats. A mechanical change should have mechanical evidence: unchanged catalog identities, equivalent regression outputs, and no intentional contract delta. A behavior fix should have a concrete failing scenario and its own review.

## C12 Standalone extraction acceptance

Create a disposable extraction from the manifest, without changing or publishing the original repository. Test it in a clean environment with empty project caches and no sibling backend/frontend paths. The old Git object database must not be required unless the documented reference-source acquisition explicitly provides it.

The public readiness gate is complete when:

- Locked native build, formatting, strict lint, quick tests, rustdoc, and examples pass.
- The documented content-pack path starts a local game service without Python gameplay installed.
- Full parity artifacts can be acquired and verified, and the declared canonical/live/full-game suites pass with explicit counts.
- Restore-sensitive suites pass; supported checkpoint, replay, and SQLite versions survive load, commands, restart, and migration.
- The working frontend commit completes manual 2D creation, scoped REST view, WebSocket update, input submission, reconnect, and continued play against the extracted server.
- Hosted draft/Worker verification runs against an explicitly selected compatible frontend version. The supplied frontend Worker only proxies `BACKEND_ORIGIN`; it does not contain the `RUST_BACKEND_ORIGIN` routing described by the backend README. That documentation cannot be assumed to describe this checkout.
- Player/spectator scope, hidden decisions, action retries, stale revisions, bots, clocks, rollback, replay, and share behavior pass their relevant contract suites.
- Linux packaging validates the binary/content identity, and backup/restore and rollback procedures are rehearsed on disposable data.
- Public tree/history review, licensing, attribution, links, contribution instructions, and artifact provenance are complete.
- Remaining P1/P2 work is tracked with owners, scope, and acceptance criteria; unsupported workflows are identified accurately.

## Evidence map

These are the main starting points for implementation; reopen the current code before editing.

| Topic | Source locations |
| --- | --- |
| Workspace, compiler, build defaults | `rust/Cargo.toml`, `rust/rust-toolchain.toml`, `rust/cargo-wsl.sh`, each crate’s `Cargo.toml` |
| CI gap | `.github/workflows/ci.yml` |
| Public module surface | `rust/crates/goa2-core/src/lib.rs` |
| Interpreter and transitional execution | `rust/crates/goa2-core/src/interpreter.rs`, `interpreter/object_execution.rs`, `interpreter/object_execution/` |
| Search complexity and legacy baseline controls | `interpreter/search.rs`, `interpreter/greedy.rs`, `scripts/rust_oracle/rollout_parity.py`, `arena_policy.py` |
| Envelope versions and codec-only execution guard | `rust/crates/goa2-core/src/codec.rs` |
| Live API, checkpoints and content identity | `interpreter/live_game.rs`, `interpreter/live_game/setup.rs`, `interpreter/live_game/view/` |
| Runtime catalog consumers | `src/templates.rs`, `effect_steps.rs`, `primary.rs`, `interpreter/loadout.rs`, `object_execution/{card_actions,defense_bodies,primary_bodies,spell_bodies}.rs` within `goa2-core` |
| Recipe sentinels | `object_execution/primary_bodies.rs`, `scripts/rust_oracle/snorri_builders.py` |
| Fixture rules and missing payload behavior | `scripts/rust_oracle/fixture_cache.py`, `rust/.gitignore`, `rust/test-support/fixture_cache.rs`, `rust/tests/fixtures/` |
| Pinned reference dependency | `rust/oracle-manifest.json`, `scripts/rust_oracle/oracle.py`, `run_pinned.py` |
| Restore and fixture-driven tests | `rust/crates/goa2-core/tests/object_execution.rs`, `rust/test-support/`, core inline unit tests |
| Worker protocol | `rust/crates/goa2-search-cli/src/main.rs` and sibling worker modules |
| Server boundary and persistence | `rust/crates/goa2-game-server/src/{http,game,store,replay,shares,bots,presence}.rs` |
| Process and deployment checks | `scripts/verify_rust_game_server.py`, `verify_rust_hosted_draft.py`, `verify_rust_staging.py`, `verify_rust_replay_share.py` |
| Generated server metadata | `scripts/export_rust_hero_catalogue.py`, `export_override_schema.py`, server `hero_catalogue.json`, `override_schema.json` |
| Release package | `scripts/package_rust_game_server.py`, `rust/deploy/goa2-rust.service` |
| Documentation to reconcile | `rust/README.md`, `rust/tests/fixtures/README.md`, `CLAUDE.md`, `docs/CLIENT_INTEGRATION_GUIDE.md`, `SHARED_DECISION_FACTS_COMPATIBILITY.md`, `MAINTAINABILIY_ROADMAP.md`, `AI_STACK_CLEANUP.md` |
| Actual frontend consumer | `goa2-frontend/src/types/game.ts`, `src/api/`, `src/hooks/useGameSocket.ts`, `worker/index.ts` |
