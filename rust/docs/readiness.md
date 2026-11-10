# Cleanup implementation and release gates

Plan baseline: backend `f0fb93e117f358266ef522e6d5f23d3452e83e97`;
plan `fc0ff702`; reference frontend `a3c5e0a`. Implementation remains on
`bots-upstream`. No repository has been published and original history is intact.

The [remaining-task checklist](cleanup-checklist.md) expands every open C01–C12
item, including implementation gaps, decisions and acceptance gates. The
[first](audits/next-ten.md) and [second](audits/second-ten.md) ten-task follow-ups
record **20 completed and 64 open items**, with reproducible commands and limits.

| Package | Implemented work | Remaining acceptance / owner |
| --- | --- | --- |
| C01 | Versioned extraction groups and dependency/consumer inventory; source-map exporter; synthetic service and compatibility baselines; audited embed/import graph and four portable baseline traces | Final public payload approval: release maintainers |
| C02 | Locked compiler policy, portable wrapper, bundled test content, atomic SHA-verified content install, verified oracle source archives without original history; retained Linux/compiler policy | Published full-content/artifact distribution and rights: release maintainers |
| C03 | Eight production catalogs relocated byte-for-byte, content hashes, fixture registry and bounded smoke inputs, retained full indexes and plain/gzip readers; benchmark/error/report ownership and cache-admission audit | Full external artifact acquisition and licensing; no families retired: engine/release maintainers |
| C04 | Separate rustfmt baseline; strict all-target lint, quick/full tiers, explicit ignores and case counts, CI and independent Python tool lock; nine expensive-gate recipes, tested conservative full/restore selection and complete candidate matrix | Full differential/restore run needs all admitted payloads: engine maintainers |
| C05 | Boundary/default/field policy, actor/team/simultaneous/event privacy regressions, receipt/restart contracts and explicit shim retention/migration decisions | Full historical corpus and hosted compatibility gates: engine/service maintainers |
| C06 | Task-based README/guides, preserved historical logs, native agent instructions, facade rustdoc and compiled example, accessible frontend facts; retained-link/history audit and detailed API failure/determinism/privacy/budget rustdoc | Final public-tree link/metadata review: release maintainers |
| C07 | Contribution/changelog/security/conduct drafts, issue/PR templates, dependency license inventory, clean extraction/source mapping; disabled crates.io policy enforced | License grant/holders/notices, advisory scan, named maintainers/security destination, public-tree/history review: repository owner |
| C08 | Shared process/API/WebSocket helpers, five independently runnable service suites, atomic schema v1–v3 migrations; WAL backup/restore/independent shares and retained process-harness decision | Extended release/hosted replay-share and migration archive matrix: service maintainers |
| C09 | Intended `api` facade, worker operation/caller inventory, explicit experimental/codec guard policy | Migration of broad internal consumers before visibility reduction: engine/search maintainers |
| C10 | No semantic interpreter/search rewrite or dense-path retirement attempted | Blocked on dependable full parity/restore artifacts; stage options types and module moves separately: engine/search maintainers |
| C11 | Acquisition corruption/path/membership/atomicity tests, privacy/stale-command contracts, performance evidence policy | Targeted fuzzing/trust-boundary review and measured hot-path changes after full gates: engine/search maintainers |
| C12 | Disposable manifest-based clean snapshot tooling and native/2D extraction rehearsal | Full artifacts, rights, hosted integration and final release gates remain required: release maintainers |

## Explicit blockers

- Full preflight selects **6,592 canonical**, **17,347 live**, **12 object-arena**,
  and **2 + 2 complete-game** cases. Missing payloads are reported as missing and
  fail the selected tier; bundled smoke inputs do not satisfy this check.
- Targeted regeneration of a newer missing payload changed its frozen case hash.
  That output was rejected and not bundled, and no oracle pin or index was changed.
  Obtain the original admitted payload or review regeneration/provenance separately.
- Public redistribution rights, attribution and the full license text need the
  repository owner's decision. Cargo's existing MIT metadata is not that decision.
- Destination repository, private security reporting contact and named ownership
  are not established. No fabricated URL/contact is added.
- Hosted draft/Worker verification needs an explicitly compatible frontend and
  deployment. The reference frontend only supports `BACKEND_ORIGIN`.

Native/tooling/service results and extraction evidence are recorded in
`cleanup-evidence.json` after checks. The earlier clean-snapshot native suite passed with 382 executed tests and 64
explicit ignores. The second batch passes 389 native tests (64 explicit ignores)
in the pinned optimized iterate profile, 35 tooling tests and 15 process scenarios.
A quick pass is never reported as full
parity, release readiness or a remote CI success. Remaining P1/P2 work is retained
above with owners and gates; no unique regressions are deleted to complete it.
