# Compatibility policy

Cleanup preserves runtime rules, message names, scopes, submission values,
revision/retry behavior and content identity. Rules corrections require their
own failing scenario and deliberate rules/content-version decision.

| Producer/data | Consumer | Accepted behavior | Gate / migration |
| --- | --- | --- | --- |
| Frontend `a3c5e0a` | Cleaned REST/WS server | Existing manual 2D and single backend proxy | Service contracts and actual 2D rehearsal |
| Game checkpoint v1 | `LiveGame::restore` | Load; absent optional history/rollback fields default | `tests/contracts.rs`; no version relaxation |
| Future checkpoint | Current engine | Reject without committing state | Contract test |
| Envelope schema 1, rules 9 | Native codec/compiler | Exact admitted oracle and source identity | Bootstrap codec tests; full restore tier |
| Different oracle/rules/source hash | Current engine | Deliberate incompatibility rejection | Codec regressions; catalog manifest gate |
| SQLite legacy unversioned, v1–v3 | Current server | Transactional migration to v3; preserve state, receipts, replay/share metadata | Migration tests and service restart/backup suites |
| Future SQLite version | Current server | Reject before migration | Migration test; restore appropriate binary/backup |
| Private replay schema 1 | Private replay reader | Per-revision digests and reconstruction preserved | Auth/service and replay/share verifier |
| Public share projection | Share consumer | Keep recipient filtering; never export private archive as share | Replay/share verifier |
| JSON-lines schema 1 | Worker/Python automation | Preserve id correlation, version and error packet shape | Worker tests and operation inventory |
| Rust `api` facade | Embedders | Pre-1.0 source API; document breaking changes | Compiled example, rustdoc |

The broad existing Rust modules and worker diagnostics remain experimental;
public visibility alone does not make an internal slot layout a promise.
[Worker operations](worker-operations.json) name consumers and retention status.
Remove an operation only after migrating its callers and tests. The codec-only
`validate_execution` diagnostic intentionally rejects; it is not the native
session compilation path. Dense transitions still have canonical consumers.
`legacy_fixed_followup` is retained for arena/rollout baselines. `.json` authoring
and `.json.gz` caches both have active readers and remain supported.

Recipients differ: actor, teammate/team decision, spectator, private replay and
public share. Preserve hidden commitments, generic labels, request-token
freshness, ordered events, physical-card aliases and full-width seeds/times.
Never persist private search hashes as portable identities. Retry receipt IDs
must survive process restart; a reused ID with different content is a conflict.

Database v3 makes previously implicit alterations explicit. Binary rollback to
a pre-versioning build requires the matching backup policy in [operations](operations.md),
even though the retained table/column layout is unchanged.

The [serialization/audience/shim audit](audits/compatibility-decisions.md) records
per-boundary defaults, strict versus additive field handling, recipient regressions
and explicit retention/migration conditions. No breaking cleanup or rules change
was approved by retaining those policies.

[Current boundary ownership](audits/api-boundaries.md) preserves old module paths,
worker error strings/correlation and all 39 operations while migrating maintained
adapter imports. Typed diagnostic categories remain internal. [Version constants](audits/versions-and-robustness.md)
centralize active readers/writers without rewriting frozen/historical literals or
relaxing any supported-boundary disagreement checks.
