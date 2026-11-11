# Cleanup implementation and release gates

Plan baseline: backend `f0fb93e117f358266ef522e6d5f23d3452e83e97`;
plan `fc0ff702`; reference frontend `a3c5e0a`. Implementation remains on
`bots-upstream`. No repository has been published and original history is intact.

The [remaining-task checklist](cleanup-checklist.md) expands every open C01–C12
item, including implementation gaps, decisions and acceptance gates. The
[first](audits/next-ten.md), [second](audits/second-ten.md) and
[third](audits/third-ten.md) and [fourth](audits/fourth-ten.md) ten-task follow-ups and the [corpus recovery and Arena v2 merge verification](audits/corpus-recovery.md)
record the implementation history and verification commands. The checklist has
**52 completed and 32 open items**.

| Package | Implemented work | Remaining acceptance / owner |
| --- | --- | --- |
| C01 | Versioned extraction groups and dependency/consumer inventory; source-map exporter; synthetic service and compatibility baselines; audited embed/import graph and four portable baseline traces | Final public payload approval: release maintainers |
| C02 | Locked compiler policy, portable wrapper, bundled test content, atomic SHA-verified content install, verified oracle source archives without original history; retained Linux/compiler policy | Published full-content/artifact distribution and rights: release maintainers |
| C03 | Eight production catalogs relocated byte-for-byte, content hashes, fixture registry and bounded smoke inputs, retained full indexes and plain/gzip readers; benchmark/error/report ownership and cache-admission audit | Local archive installation verified for the full corpus and chunks; remote distribution and licensing remain: engine/release maintainers |
| C04 | Separate rustfmt baseline; strict all-target lint, quick/full tiers, explicit ignores and case counts, CI and independent Python tool lock; nine expensive-gate recipes, tested conservative full/restore selection and complete candidate matrix; scheduled cold native path locally verified with empty Cargo/target directories | Local full differential and exhaustive restore passed for the combined candidate. Remote artifact retrieval and CI remain open: engine maintainers |
| C05 | Boundary/default/field policy, actor/team/simultaneous/event privacy regressions, receipt/restart contracts and explicit shim retention/migration decisions; actual producer archive and four schema load/command/restart/retry/rollback rows | Historical reader matrix and disposable binary rollback passed locally. Hosted compatibility remains open: engine/service maintainers |
| C06 | Task-based README/guides, preserved historical logs, native agent instructions, facade rustdoc and compiled example, accessible frontend facts; retained-link/history audit and detailed API failure/determinism/privacy/budget rustdoc | Final public-tree link/metadata review: release maintainers |
| C07 | Contribution/changelog/security/conduct drafts, issue/PR templates, dependency license inventory, clean extraction/source mapping; disabled crates.io policy enforced; maintained advisory/license scan, SQLite advisory remediated, internal release notes/provenance; preliminary payload scan and neutral historical machine paths | License grant/holders/notices, named maintainers/security destination, public-tree/history review: repository owner |
| C08 | Shared process/API/WebSocket helpers, six independently runnable service suites, atomic schema v1–v3 migrations; WAL backup/restore/independent shares and retained process-harness decision; executor/lock audit and contended-mutex fix; historical migration archive and actual matching-backup binary rollback | Extended replay-share and native service suites passed on the extracted release binary. Hosted checks remain open: service maintainers |
| C09 | Intended `api`/bot facade; maintained adapter migration, explicit experimental search imports, complete worker schema/caller/status inventory and internal diagnostic categories | Full data/caller migration before restricting raw internal visibility: engine/search maintainers |
| C10 | Interpreter, object execution, live checkpoint and search responsibilities split into modules. Typed primary operands verified against the old bindings. Full differential, exhaustive restore and representative fixed-work comparisons passed on the combined candidate | Audit the remaining module boundaries and per-change equivalence evidence: engine/search maintainers |
| C11 | Panic/trust audit, bounded reference/catalog/replay/worker mutation tests and root/sibling/ordered-restore/privacy invariants; reproducible release-profile startup/clone/restore/search/cancellation/memory baseline and catalog/footprint retention decision | Representative full regression and performance gates before hot-path changes: engine/search maintainers |
| C12 | Disposable manifest-based clean snapshot tooling and native/2D extraction rehearsal | Full artifacts, rights, hosted integration and final release gates remain required: release maintainers |

## Local release candidate verification

- The combined engine candidate `ebd52035` passed full differential verification
  over 24,197 admitted fixtures, including 6,592 canonical cases, 17,347 live
  cases and four complete games. All 17,347 live cases passed exhaustive
  restore across eight shards.
- The merged candidate `33e7e8bd` passed a clean, locked WSL build from a
  history-free 1,202-file export. Strict Clippy, rustdoc, formatting, 413 native
  tests and the local game example passed.
- The extracted Linux release binary passed 15 native service scenarios and
  replay-share verification across 193 commands and 194 frames. Two packages
  produced the same archive hash. The extracted package started with a fresh
  home and database and loaded all 33 heroes and six maps.
- The original historical fixture passed reader checks for SQLite versions 0
  through 3. A source-verified rebuild of the producer restored its matching
  pre-upgrade backup and continued play with a separate synthetic fixture.
  The original producer executable remains unavailable and its binary hash
  differs from the rebuild.
- Frontend `a3c5e0a` passed the local 2D browser sequence against the release
  binary, including scoped REST, WebSocket updates, input and reconnect.

The exact source, binary, content, archive and test-log identities are recorded
in [cleanup evidence](cleanup-evidence.json).

## Release decisions and external verification

- The original private WSL cache now passes all **6,592 canonical**,
  **17,347 live**, **12 object-arena**, **2 + 2 complete-game** cases and
  **12 monolithic payloads**. All **230 snapshot chunks** pass the additional
  complete-game preflight. This includes the previously unavailable frozen case;
  no regenerated substitute or changed oracle/index was installed.
- Local full differential verification at source `616f52d4` passed 636 native
  tests (eight ignored in that pass) and all four trace tests, covering 6,592
  canonical cases, 17,347 live cases and four complete games. See
  `local_full_differential_verification` in `cleanup-evidence.json`.
- Local exhaustive restoration passed for all 17,347 live cases across eight
  disjoint shards. See `local_exhaustive_restore_verification` in
  `cleanup-evidence.json`.
- The private fixture archive packed and installed all 24,197 files in a fresh
  cache from standalone source. The approved remote acquisition route and CI
  results are still pending.
- Public redistribution rights, attribution and the full license text need the
  repository owner's decision. Cargo's existing MIT metadata is not that decision.
- Destination repository, private security reporting contact and named ownership
  are not established. No fabricated URL/contact is added.
- Hosted draft/Worker verification needs an explicitly compatible frontend and
  deployment. The reference frontend only supports `BACKEND_ORIGIN`.

Native/tooling/service results and extraction evidence are recorded in
`cleanup-evidence.json` after checks. The earlier clean-snapshot native suite passed with 382 executed tests and 64
explicit ignores. The third batch passes 400 native tests (64 explicit ignores)
in the pinned optimized iterate profile, 38 tooling tests and 15 process scenarios.
The fourth batch passes 402 native tests, 44 tooling tests, 15 process scenarios,
four historical rows and one matching-backup binary rollback. Its cold native
run independently passes build, quality, 402 tests and the compiled example;
performance samples cover two packs with three fresh processes each.
A quick pass is never reported as full
parity, release readiness or a remote CI success. Remaining P1/P2 work is retained
above with owners and gates; no unique regressions are deleted to complete it.
