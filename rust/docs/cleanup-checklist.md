# Rust engine cleanup — task checklist

See the [readiness record](readiness.md) and [verification evidence](cleanup-evidence.json).
This tracks work across **all C01–C12 packages**, including release gates
and later P1/P2 work. An unchecked item is open; a documented policy or local
smoke pass does not establish its wider acceptance criterion.

The [ten-task follow-up](audits/next-ten.md) closes the ten checked items below.
The [second ten-task follow-up](audits/second-ten.md) closes ten further items.
The [third ten-task follow-up](audits/third-ten.md) closes ten further items.
The [fourth ten-task follow-up](audits/fourth-ten.md) closes ten further items.
**58 completed; 29 remain open.** The [corpus recovery and Arena v2 merge verification](audits/corpus-recovery.md)
records the additional local gate.

The implemented baseline already includes catalog relocation, bundled smoke
inputs/content, native CI definitions, SQLite migrations, service suites,
contributor guides, extraction tooling and an embedding facade. Recorded checks
include 382 native tests, 9 tooling tests, 14 service scenarios, clean-snapshot
build/quality checks, deterministic Linux packaging and manual 2D reconnect/play.
Those completed changes are not repeated below as new implementation tasks.

Close an item only with its decision or evidence: source/artifact identity,
command, expected/executed/missing counts and result, as applicable. Update this
checklist and the readiness/evidence records together. Owners below are roles;
assign named maintainers under C07. Conditional changes require a recorded
retain/defer decision when no change is justified.

## C01 — Extraction inventory and baseline contracts (P0)

Owner: engine and release maintainers. No prerequisite package.

- [x] **C01-01** Finish the bidirectional dependency audit of the extraction manifest: embedded files, generated server metadata, Python imports/root resolution, test support, packaging/service files and hosted-draft dependencies. Record required external inputs and consumers for every retained group.
- [x] **C01-02** Complete the portable synthetic baseline set for manual 2D play, bots, timed play, rollback, upgrades, restart and replay. Add any scenarios not already covered, and record expected commands, scoped views, ordered events, revisions, content identity and provenance.
- [ ] **C01-03** Confirm the baseline inputs are redistributable and contain no real credentials/private matches; record the rights decision with C07 before calling them public fixtures.
- [ ] **C01-04** Approve the final extraction manifest and exact retained payload after the remaining data/rights decisions. Retain original-to-extracted source mapping and record clean source/build status for that candidate.

## C02 — Reproducible build and content acquisition (P0)

Owner: engine and release maintainers. Depends on C01; public distribution also
requires C07 rights decisions. The compiler pin, portable wrapper, checksum
installer, small test pack and verified source-archive support already exist.

- [ ] **C02-01** Establish a versioned distribution location for the full `live-game-content.json` pack, with trusted release provenance and independently supplied checksums. Document its compatibility identity and distinguish authenticity from checksum verification.
- [ ] **C02-02** Make the exact admitted oracle source artifact available independently of the original Git database, subject to rights clearance. Publish or document its pinned acquisition location, archive checksum and locked Python/runtime requirements.
- [ ] **C02-03** Update acquisition instructions to use the actual distribution locations, then exercise download, verification, installation and actionable failure paths from a fresh checkout without original history or sibling repositories.
- [ ] **C02-04** Verify full-content server startup through that documented acquisition route with empty project caches and without Python gameplay installed. Record pack/binary identities; the bundled seven-hero test pack does not close this gate. The private package acquisition rehearsal verified the installed content and native binary in a fresh environment with no Python gameplay on the server path. See [cleanup evidence](cleanup-evidence.json). The final distribution route remains open.
- [x] **C02-05** Keep Linux as the supported baseline. If macOS, Windows or a lower compiler minimum is proposed, test and document it before making that support claim; otherwise explicitly retain the present policy.

## C03 — Runtime, fixture and legacy-material ownership (P0)

Owner: engine and release maintainers. Depends on C01. Production catalog moves
are already byte-verified; no fixture family has been retired.

- [x] **C03-01** Recover/acquire every admitted indexed payload: 6,592 canonical stack traces, 17,347 live object-execution cases, 12 object-arena cases and both two-game complete-game families, including their required snapshot/chunk data.
- [x] **C03-02** Acquire the 12 external monolithic reference payloads admitted by `tests/fixtures/registry.json`: action-validation, actions, card-equality, displacement-validation, effect-stats, effects, hex-filters, mine-paths, models, planning, targeting and terrain-validation.
- [x] **C03-03** Resolve the newer missing case whose targeted regeneration changed its frozen hash. Obtain the original admitted payload or separately review generator/provenance differences; do not substitute a changed case or silently repin the oracle/index.
- [x] **C03-04** Provide verified acquisition for all retained corpora and document where ignored cache payloads live. Preserve deterministic gzip, atomic publication, logical paths and one-case-at-a-time reads; do not extract/cache the full corpus in memory.
- [ ] **C03-05** Finish registry detail for each family: specific generator command, schema/rules version, oracle/runtime identity, expected counts/hashes, actual consumers, acquisition location, rights and owner. Replace generic generator/consumer descriptions where they leave reproduction ambiguous. All 22 rows now name their consumer, schema, identity and local acquisition route. All 22 families have pinned generator commands and verified fresh regeneration against the admitted hashes. The object-execution audit matched all 17,347 payload hashes and the rebuilt index. The public acquisition route, rights and named ownership still need a decision.
- [x] **C03-06** Classify benchmark inputs separately from runtime catalogs, smoke examples and full parity data; record their location, identity, generator and consumers.
- [x] **C03-07** Classify error fixtures and guards as intentional rejection contracts or historical porting limitations, and identify the tests/consumers that justify retaining each category.
- [x] **C03-08** Audit tracked milestone validation JSON (29 artifacts at the plan baseline), references and unique regression inputs. Record archive/exclusion decisions with original commit, compiler and scope metadata before retiring anything.
- [x] **C03-09** Inventory duplicated/scratch reports before any deletion. Preserve unique source, scenarios and provenance, and document coverage replacement or a justified retention decision for each proposed removal.
- [x] **C03-10** Audit cache identity/invalidation across schema, rules, oracle, generator and runtime dependencies. Verify physical-instance aliases, dictionary/list order, null semantics and semantic hashes survive acquisition or representation changes.
- [x] **C03-11** Decide final catalog/package placement for the standalone layout. If crates.io publication is later enabled, redesign external-crate data paths as needed and verify packaged-crate contents; keep publication disabled meanwhile.

- [x] **C03-12** Build and verify a private index-ordered seekable zstd pack with direct Rust and Python reads, bounded caching and frozen logical identities. Both pack builds passed the Python registry check for all 24,197 payloads. The native restore smoke read the deduplicated pack directly and matched all 136 shard cases against the pinned pack index. Full native parity and comparative performance remain under C04-10. See [parity corpus consolidation](fixture-archives.md#planned-parity-corpus-consolidation).
- [x] **C03-13** Consolidate name-only duplicates and share equivalent starting checkpoints or operation prefixes while preserving named cases, provenance and logical hashes. The pack consolidates 1,818 name-only duplicates, and the differential and restore runs verified named coverage with exact-trace reuse. The compiled-start cache passed its isolation check but was slower on a shard with 76 hits. The opt-in compiled-start cache matched all 17,347 cases and 150,142 restore boundaries, with 2,092 reuses including at least 543 across distinct traces.

## C04 — Test tiers and Rust CI (P0)

Owner: engine, service and search maintainers. Depends on C02/C03 for full-data
acceptance. Native/Python CI definitions and local quick/quality checks exist.

- [ ] **C04-01** Connect the full and restore CI jobs to the verified artifact acquisition route, checking identities after retrieval. Keep selected jobs failing on missing data; never satisfy them with the smoke fallback.
- [x] **C04-02** Run `python3 tools/fixtures.py verify --full` from `rust/` with all admitted payloads. Record separate case and monolithic-payload counts, with no missing or failed inputs.
- [x] **C04-03** Run `python3 tools/check.py full` and retain canonical/live/full-game state, request, event-order and identity comparison results with explicit counts.
- [x] **C04-04** Run `python3 tools/check.py restore` with the required complete data and retain exhaustive boundary/restoration and worker-isolation evidence. The local run passed the same fixture preflight and exhaustive test in eight disjoint shards after WSL interrupted the unsharded attempt.
- [x] **C04-05** Acquire the admitted setup/view inputs and explicitly run their ignored gates. Record variables, selected names and nonzero expected/executed counts; an ignored gate is not a pass.
- [x] **C04-06** Replace remaining generic expensive-tier command descriptions with executable, pinned workload commands and required inputs. Cover setup/view, restore, performance/strength and hosted integration, with accountable owners and event/schedule triggers.
- [x] **C04-07** Verify CI change selection enforces the appropriate reference/restore gates for engine, worker, codec/arena and fixture changes; releases must run the full required matrix regardless of a narrow file diff.
- [x] **C04-08** Establish periodic cold-acquisition/build verification. If CI caches are introduced, use them only as accelerators and test a path that does not depend on warm caches.
- [ ] **C04-09** Run the workflow remotely and record actual job results, including separate Python tooling/script checks. Local successes alone do not establish a successful remote CI run.

- [x] **C04-10** Make full parity and exhaustive restore reuse work safely, including contiguous restore shard ranges. Prove coverage equivalence and compare end-to-end time, memory and decode work with the frozen baseline before switching the default. The three modes matched all 17,347 restore cases and 150,142 boundaries, and matched full canonical and live differential coverage. Replay sharing remains opt-in because the measured improvement fell short of the parity target.

## C05 — Compatibility policy and regressions (P0)

Owner: engine and service maintainers. Depends on C01 and C04 automated gates.
The compatibility matrix and small checkpoint/privacy/rejection tests exist.

- [x] **C05-01** Assemble admitted historical checkpoint, replay/share and SQLite examples for every version claimed as supported; record producer, consumer, schema, expected behavior and migration path.
- [x] **C05-02** Complete and run that historical compatibility matrix through load, commands, restart, retry and rollback. Retain ordered-event, digest and physical-object/alias evidence from the full suites.
- [x] **C05-03** Audit absent optional fields, unknown-field rejection and unsupported future versions across supported serialized boundaries. Add missing regressions without duplicating already passing contracts or weakening deliberate oracle/rules rejection.
- [x] **C05-04** Finish audience/contract coverage for actor, teammate/team decisions, spectator, private replay and public shares. Fill gaps for simultaneous metadata, hidden decisions, generic prompts, request freshness, full-width seeds/times and stale-command protection.
- [x] **C05-05** Classify retained compatibility shims as required, temporary or obsolete, each with named consumer, test, removal condition and migration note. Keep dense execution, codec-only diagnostics, baseline policy controls and plain/gzip readers while their consumers remain.
- [x] **C05-06** For any proposed breaking cleanup, prove there is no supported consumer or supply a concrete migration/deprecation path. Keep rules corrections and rules/content-version decisions separate from mechanical cleanup.

## C06 — Contributor documentation (P0)

Owner: engine and release maintainers. Final acceptance depends on C01–C05.
The task-based guides, archive, example and native agent guidance exist.

- [ ] **C06-01** Reconcile guides with the final standalone layout, artifact URLs, supported versions, complete test commands and actual destination metadata after the preceding decisions land.
- [x] **C06-02** Audit carried-forward historical/status references and links, including any remaining Python-era plans. Keep archived records identifiable by original date/commit and prevent them from becoming active Rust onboarding instructions.
- [x] **C06-03** Verify the intended facade's rustdoc covers failure conditions, determinism, trusted inputs, private state, budgets and cancellation; fill API-specific gaps while retaining the existing documentation/missing-doc gates.
- [ ] **C06-04** Recheck current links and compiled examples against the final payload and perform a fresh contributor walkthrough: build, quick test, full-content startup, locating a rule and understanding compatibility without private history. The local audit checked 195 retained Markdown links and heading targets in both the working tree and a history-free extraction, with zero failures. The private candidate walkthrough passed the locked build, quick suite, compiled example and localhost full-content startup, and located rule entry points and compatibility guidance from the extracted README. Repeat link and walkthrough checks if release approval changes the retained payload.

## C07 — Public repository and release hygiene (P0)

Owner: repository owner and release maintainers. Depends on C01–C06. Existing
contribution/security/conduct materials are preparation drafts, not rights approval.

- [ ] **C07-01** Confirm the intended source-code license grant, holders and upstream obligations; add the full license text and accurate attribution/notices.
- [ ] **C07-02** Resolve redistribution rights separately for generated rules, hero/card text, names, maps, third-party assets, smoke fixtures, complete corpora and content packs. Record what may ship in source versus artifacts, and separate restricted material if needed.
- [ ] **C07-03** Select the destination repository and named maintainers/reviewers; replace role-only ownership with an accountable ownership map.
- [ ] **C07-04** Establish a private security reporting destination, supported-release window and conduct reporting/appeal route, and finalize the draft policies without invented contacts or promises.
- [ ] **C07-05** Review the exact public tree for credentials, private replay/game state, databases, environment files, personal paths, generated blobs and nonredistributable material. The current private extraction passed the automated 1,213-file payload scan with zero findings. Review the final selected public payload once its rights and history policy are settled. Include historical documents and exported artifacts; `.gitignore` is not proof of absence.
- [ ] **C07-06** Confirm clean initial snapshot versus filtered-history release strategy. If retaining history, audit the history as well; preserve source/oracle provenance and original-commit mappings without rewriting the original repository.
- [x] **C07-07** Run maintained dependency license/advisory tooling against the locked graph, resolve findings and document exceptions. The committed license metadata inventory does not establish an advisory scan or license approval.
- [ ] **C07-08** Add accurate repository/description/readme metadata once the destination is known; finalize contribution, changelog and community materials against the actual public payload.
- [x] **C07-09** Prepare release notes/provenance naming source commit, compiler/profile, supported architecture, content hash, schemas/migration requirements, verification counts and backup/rollback instructions.
- [ ] **C07-10** Decide the authenticity mechanism for public binary/content distribution and document how consumers verify it; checksums alone do not identify the publisher.
- [x] **C07-11** Keep crates.io disabled unless naming, API stability, data packaging, metadata and package verification are deliberately completed. Record this as a retained policy if public source hosting is the only intended release.

## C08 — Server verification and persistence (P1)

Owner: service maintainers. Depends on C04/C05. Split process suites and atomic,
idempotent schema v1–v3 migration tests already pass locally.

- [x] **C08-01** Extend the migration archive matrix to retained historical databases and realistic combinations of replay journals, credentials, accepted-action receipts and share metadata. Preserve actual process restart/failure behavior alongside unit tests.
- [x] **C08-02** Rehearse consistent WAL backup and restoration on disposable data, then verify links, revisions, receipts, replay reconstruction and share metadata after reopening the restored database.
- [x] **C08-03** Rehearse binary rollback with the matching database backup across supported migration boundaries; record when a newer schema prevents direct rollback.
- [x] **C08-04** Run the extended replay/share service verifier for the final candidate and close any coverage gaps across auth/views, retries/concurrency, clocks/votes, persistence and bots. The extracted release binary passed both service suites at `33e7e8bd`. See [verification evidence](cleanup-evidence.json).
- [x] **C08-05** Audit blocking work, lock scope, bot cancellation, deadline recovery and shutdown across transport, orchestration, persistence and replay boundaries; address concrete problems with their own regressions.
- [x] **C08-06** Evaluate whether a server library/test harness would materially improve testing and boundaries. Record retain/defer or implement the justified change; preserve real cross-process checks either way.

## C09 — Library and worker API boundaries (P1)

Owner: engine and search maintainers. Depends on C04/C05. The embedding facade
and initial worker operation/caller inventory exist.

- [x] **C09-01** Complete the caller inventory for broad core exports, including dynamic Python/worker uses and restore/search consumers, before restricting visibility.
- [x] **C09-02** Refine supported embedding/search entry points and migrate callers before making arena slots, compiler/import state and profiling structures internal. Add a crate only if it materially clarifies ownership/build cost.
- [x] **C09-03** Validate every retained worker operation's caller/status and link it to maintained protocol examples and schema/error tests. Isolate diagnostics and deprecate/remove a porting operation only after migrating all callers and preserving its unique coverage.
- [x] **C09-04** Introduce stable internal diagnostic categories and useful boundary context for unsupported rules, invalid input, budget exhaustion, cancellation and internal errors. Preserve current wire behavior or version an intentional change; test that distinction.

## C10 — Interpreter and search modularization (P2)

Owner: engine and search maintainers. Depends on C03/C04/C09; obtain dependable
full differential/restore coverage before semantic or representation changes.

- [ ] **C10-01** Establish the full regression and representative fixed-work performance baseline for each planned refactor before changing structure. The frozen baseline and interpreter comparison are recorded. The eight-shard restore report records revision `06bebd21`, whose committed restore inputs match the frozen baseline. The original runner fingerprint was reproduced from all three Git revisions, and all eight shard log checksums matched. Some subsequent changes share a grouped baseline and need individual attribution.
- [x] **C10-02** Mechanically split interpreter responsibilities into instruction definitions/compilation, execution/session state, requests/events, import/export, budgets and tests, with no intentional semantic change. The interpreter root owns session execution. Instructions, compilation, requests/events, checkpoints, budgets and tests have separate modules. The split passed full differential verification and all 17,347 exhaustive restores at `3356c5da`.
- [x] **C10-03** Continue object-execution/live-game modularization and replace broad `use super::*` coupling with explicit dependencies where it clarifies ownership. Object execution and live-game checkpoints have dedicated modules. The production object-execution boundary and child modules use explicit imports. Strict Clippy and 161 focused tests passed.
- [x] **C10-04** Separate search orchestration, evaluation, hidden-world sampling, generation/order, rollout, reuse, diagnostics and result types; introduce meaningful options/context types for repeated argument groups. The search root owns orchestration. Evaluation, hidden-world sampling, candidate generation, rollout, reuse, diagnostics and result types have separate modules. Planning uses options and context structures. Fixed-work verification is recorded in the [cleanup evidence](cleanup-evidence.json).
- [x] **C10-05** Inventory dense versus object-aware execution for every live, worker, search and restore caller. Migrate consumers and preserve unique dense/protocol regressions before retiring any implementation; record justified retention where needed.
- [x] **C10-06** Replace recipe operand sentinels such as `1234568103`/`1234568104` with a documented typed representation through a coordinated, versioned generator/catalog change and independent old/new semantic comparison. Catalog schema versions are 3/4/2. The [evidence](cleanup-evidence.json) records 1,943 old/new bindings and 24,197 admitted fixtures. Commit `48828dd9` pins the migrated catalog schemas and checksums.
- [x] **C10-07** Centralize version constants and identity metadata while retaining disagreement checks at the supported boundaries.
- [ ] **C10-08** After each separately reviewable change, verify equivalent state, requests, ordered events, aliases/restoration and fixed-work performance, including tie breaks, random draws, cancellation, deadlines, node accounting and fallback semantics. The merged runtime at `ed24857c` passed 24,197 differential fixture checks and all 17,347 restores. Four fixed-work comparisons preserved decisions, scores and node counts. Those runs cover the combined result. The remaining gap is per-change coverage. The fallback operand checkpoint `d284805c` passed all 24,197 payload checks, 638 native tests and all five differential gates in its isolated checkout. Its exhaustive restore passed all 17,347 cases across eight shards on the same source fingerprint. The four fixed-node comparisons against typed-operand checkpoint `67a852ed` matched decisions, scores and node counts. The typed-operand checkpoint also passed all 17,347 restores using one checksummed executable across all eight shards. All 15 structural comparisons matched all four fixed-node workloads per pair. Across those 60 comparisons, changes in median wall time ranged from −7.6% to +7.2%. The slowest comparison was one workload at `f52b9889`. Each median uses three runs, so the observed range is descriptive. All 17 tested revisions passed the same 136-case, 1,274-boundary restore sample. The full native suite at `a5e313a3` reported a spell restore failure after a successful fixture preflight. The replay attempted to submit recorded input with no pending request. The same test passed on the merged revision. The spell restore test passed on the verified inputs of the initial migration parent `832ea43e`, failed at the initial typed primary migration `7e87efd7`, and passed at the typed spell migration `b3ca2ffc`. A fresh build of its immediate parent `5f81b93a` reproduced the missing-request failure in the same focused test. A diagnostic run isolated zero-based operation 31 in both Shocking Grasp fixtures, `defend-first` and `defend-skip`. Neither replay had a pending request for the recorded input. The first post-fix import refactor `4ae20a58` passed 638 native tests, all five differential checks and all 17,347 restores. The second import refactor `40bad5ea` also passed 638 native tests, all five differential checks and all 17,347 restores across eight shards. The final import refactor `5996fe32` passed the same 638 native tests, five differential checks and all 17,347 restores. The historical spell-fix commit `b3ca2ffc` passed native spell restore but failed Python parity for primary-text movement and attack fallback at operation 0. Exhaustive restore did not start. Controlled comparisons now apply both engine fixes to each side, preserving all 11 original refactor diffs. The corrected baseline based on `7e87efd7` passed 638 native tests, four differential checks and all 17,347 restores across eight shards. The first corrected refactor based on `dea9cc39` passed the same 638 native tests, four differential checks and all 17,347 restores. The second corrected refactor based on `360f4263` passed the same 638 native tests, four differential checks and all 17,347 restores. The remaining corrected refactor revisions still require runtime verification. The corrected historical checkouts share tracked fixture inputs and pinned Cargo dependencies. They can reuse compiled dependencies across serial runs. Each run still builds and verifies its own test executable, and reuse of a fixture preflight requires verifying that the external cache stayed fixed. Full per-change differential and restore coverage beyond that sample remains open.

## C11 — Robustness and performance maintenance (P2)

Owner: engine, service and search maintainers. Depends on C04 and coordinates
with C08–C10. Acquisition/atomicity and small privacy/stale-input tests exist.

- [x] **C11-01** Audit panic assumptions at external content, archive, envelope, worker JSON, WebSocket and persisted-data boundaries. Distinguish trusted imports and test-only unwraps from actual reachable failures; preserve explicit size/entity/step/execution limits.
- [x] **C11-02** Add targeted property/fuzz cases for malformed envelopes, reference tables, hex coordinates, catalog deserialization, replay inputs and command sequences where current tests leave meaningful risk.
- [x] **C11-03** Extend invariant coverage for rejected-input nonmutation, worker isolation, physical-card identity, save/restore equivalence and recipient privacy, using concrete gaps rather than duplicating existing checks.
- [x] **C11-04** Establish reproducible benchmark inputs/results with content/oracle identity, compiler/profile, hardware, seed, policy, workload and units; record fixed-work comparisons separately from deadline-limited strength/quality.
- [x] **C11-05** Measure cold startup, memory, clone/restore cost, throughput, decision latency and cancellation responsiveness for the retained baseline and relevant future changes.
- [x] **C11-06** Review catalog parsing/initialization and binary footprint after relocation. Optimize only where measurement justifies it, preserving immutable process-wide caches and private continuation state with correctness/identity checks.

## C12 — Final standalone extraction and acceptance (P0)

Owner: release maintainers, with engine/service/search reviewers. Depends on
C01–C07 and compatibility-critical C08/C09 work. The earlier clean extraction,
small-pack startup, 2D flow and deterministic package checks are already recorded.

- [ ] **C12-01** Extract the final approved candidate from the finalized manifest into a disposable directory with no sibling checkouts/original Git database and empty project caches. Run locked build, formatting, strict Clippy, quick tests, rustdoc and examples; record candidate/source-map identities. The current private candidate passed all these local checks from a 1,213-file checksum-verified extraction with no Git database. Final payload approval is pending.
- [ ] **C12-02** Run the documented full-content acquisition/startup workflow without Python gameplay installed and verify the final binary/content identities (C02). The private package acquisition rehearsal verified the installed content and native binary in a fresh environment with no Python gameplay on the server path. See [cleanup evidence](cleanup-evidence.json). The final distribution route remains open.
- [ ] **C12-03** Acquire/verify every retained parity family, then pass full differential, complete-game and exhaustive restore suites with explicit counts and no missing cases (C03/C04). The private corpus passed on `ed24857c`, including four complete games. The current private extraction also passed all five differential gates, including four complete games, after verifying all 24,197 payloads from the pinned pack. Exhaustive restore for that exact private export is queued on Hetzner after the historical and fixed-work runs. The approved distribution route and final payload approval remain pending.
- [x] **C12-04** Pass the retained checkpoint/replay/SQLite load, command, restart and migration matrix, including recipient scope, hidden decisions, retries, stale revisions, bots, clocks, rollback and shares (C05/C08). The extracted release passed the local service, replay, and SQLite migration matrix. See [verification evidence](cleanup-evidence.json).
- [x] **C12-05** Revalidate the agreed frontend contract suite and manual 2D sequence against the final extracted server: creation, scoped REST view, WebSocket update, input, reconnect and continued play. Record the exact frontend commit and any unsupported workflows. The local 2D walkthrough passed against the extracted release binary. See [verification evidence](cleanup-evidence.json).
- [ ] **C12-06** Select a compatible hosted frontend/Worker and deployment, then explicitly run draft provisioning, routing, ingress restrictions, staging and backup/restore checks. Do not assume `RUST_BACKEND_ORIGIN` exists in reference frontend `a3c5e0a`; it only proxies `BACKEND_ORIGIN`.
- [ ] **C12-07** Validate final release-profile Linux packaging, binary/content identity and provenance, and rehearse backup restoration/binary rollback on disposable data. Existing deterministic dev-binary packaging evidence does not establish release-candidate promotion (C07/C08). The extracted Linux release passed deterministic packaging, full-content startup and a matching-backup rollback with a source-verified historical rebuild. The originally admitted producer executable was unavailable. Artifact provenance approval and deployment promotion remain open.
- [ ] **C12-08** Close public payload/history, licensing, attribution, dependency review, links, contribution instructions and artifact provenance gates (C06/C07).
- [ ] **C12-09** Assign named owners and acceptance criteria to any deferred P1/P2 work and transfer it to the chosen project's issue tracker when that destination is established. C08–C11 need not all finish before source hosting if the compatibility-critical gates are satisfied and deferrals are explicit.
- [ ] **C12-10** Update readiness/evidence with final local and remote results, identify remaining unsupported workflows, and obtain the release owner's final payload decision. This checklist does not authorize repository publication or deployment.
