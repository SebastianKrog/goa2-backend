# Second ten locally actionable cleanup tasks

Completed on `bots-upstream` after `a4023b72`, 10 November 2026. The
[structured evidence](second-ten.json) records current checks separately from
prior batches. The checklist now has **20 completed and 64 open tasks**.

| Task | Result / maintained proof |
| --- | --- |
| C04-07 | [CI selector and candidate matrix](ci-gates.md): engine/worker/codec/arena/fixture changes require full and restore. Version tags/manual candidates require all nine expensive recipes plus native/tooling/service prerequisites regardless of narrow diffs. Missing inputs remain failures. |
| C05-03 | [Serialized-boundary audit](compatibility-decisions.md#serialized-boundary-policy-c05-03), native/service regressions for optional defaults, unknown fields, future schema/rules/source and full-width values. Existing unit-variant extra-field tolerance is explicitly preserved. |
| C05-04 | [Recipient coverage](compatibility-decisions.md#recipient-and-freshness-coverage-c05-04): added teammate/opponent event privacy/order, team requests, simultaneous upgrade scoping, generic numeric labels and stale-token nonmutation; retained process checks cover auth/restart/rollback/retries/clocks. Finished synthetic public shares are checked independently of private archives. |
| C05-05 | [Nine shim categories](compatibility-decisions.md#shim-classification-and-removal-gates-c05-0506) record required/temporary status, concrete consumers, tests, owner roles and removal conditions. No item is declared obsolete because its full corpus is missing. |
| C05-06 | Same retain/migration decision: no breaking cleanup is proposed. All supported readers/operations/defaults remain; future breaking changes require consumer migration and version/deprecation/backup evidence. Rules corrections remain separate. |
| C06-02 | [Documentation inventory](documentation.json): retained links/heading targets checked against the actual extraction selection; 36 stale/excluded archive links repaired or annotated. Original dates/commits/hashes retained; 62 original-repository documents inventoried separately from current onboarding. |
| C06-03 | `goa2-core/src/api.rs` rustdoc now details trusted inputs, rejection/error categories, atomic candidate publication, determinism, explicit clocks, cooperative cancellation/step budgets, private state and recipient filtering. Strict rustdoc/Clippy and compiled embedding example pass. |
| C07-11 | [Retained crates.io policy](../platform-and-packaging.md#cratesio-retained-policy-c07-11): all three crates inherit disabled publication, now enforced by tooling. Source hosting remains the only packaging target under preparation, subject to rights/payload decisions. |
| C08-02 | [Online WAL backup rehearsal](server-harness.md#online-backup-and-restoration-c08-02): live WAL, schema/table bytes, credentials/links, revisions, retry receipts, replay reconstruction, independent share groups and continued play after restore. Destination overwrite and post-snapshot leakage reject. |
| C08-06 | [Harness evaluation](server-harness.md): retain private server modules, cheap adjacent unit tests and split real-process suites; defer a public server library until a concrete second consumer or measured testing limitation justifies it. Preserve process acceptance either way. |

## Verification and limits

The pinned Linux/Rust 1.99.1 iterate suite passed **389 native tests**, with 64
explicit ignores and no failures. The seven added native tests cover concrete
serialization/privacy/recipient gaps. Quick data verification passed **572/572**.
Strict Clippy (all targets/features), formatting, rustdoc and the embedding example
passed. Tooling passed **35 tests**, including selector behavior/CLI, extraction
link membership/anchors and the crates.io guard. The final extracted tree also
passes those tooling/data/gate/link checks and the standalone backup scenario.
The full service run covers **15 disposable scenarios** across six selectable
suites; backup is the new fifteenth scenario. Reports retain explicit counts.

The backup share uses an explicitly synthetic terminal checkpoint to exercise
finished-state storage/projection; it does not impersonate a missing complete-match
corpus. Existing serialization behavior, private source data and gameplay are
unchanged: production Rust changes are documentation only; other Rust additions
are tests. No original report, fixture family, content version or oracle pin is
removed/replaced. Full preflight still fails on **23,955 missing indexed cases
and 12 missing monolithic payloads**, with zero admitted mismatches.

These ten use available source, bundled inputs and disposable native processes.
Earlier acquisition/full/historical gates still need their admitted artifacts;
final layout/public-payload walks and release/rights/destination/maintainer decisions
remain open. C07's maintained advisory scan still needs its tool/database and
finding dispositions; Cargo license metadata is not such a scan. Broader
concurrency/load auditing, historical binary rollback and extended hosted completed-
match verification remain distinct C08 tasks. This batch's local CI selection and
workflow definition checks do not establish remote job success or cold acquisition.
No publication, deployment, real-game mutation or original-history rewrite occurs.
