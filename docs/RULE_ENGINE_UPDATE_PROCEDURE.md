# Rules, oracle and Python update procedure

Status: operational procedure for future changes; implementation prerequisites
are listed in the [versioning and audit plan](RULE_VERSION_AUDIT_IMPLEMENTATION_PLAN.md).
Written 11 November 2026. This is plan 5's procedure. No engine change, audit,
fixture regeneration, model update or production rollout is claimed by writing it.

Use the repository [rule-engine-update skill](../.agents/skills/rule-engine-update/SKILL.md)
to carry out a change. Use the [NN update procedure](NN_RULE_CHANGE_UPDATE_PROCEDURE.md)
when an NN exists and a change can affect its inputs, targets or decisions.

## 1. Authority and scope

Rust is the authoritative live engine and the working presumption is that its
implementation is correct. That presumption is not a reason to ignore a
disagreement. For now, the exact versioned Python rules engine is the gold
standard for rule behavior, except for explicitly documented known Python errors.
Official rule changes/adjudications and the applicable version's intended contract
resolve those exceptions; a newly discovered discrepancy is not automatically
a Python bug or a Rust bug.

Keep three questions separate:

| Audit mode | Question | Basis |
| --- | --- | --- |
| As played | Does the recorded implementation reproduce its recorded decisions/state/result? | Original Rust artifact and recorded evidence |
| Original contract | Did play satisfy its original intended rules? | Original Python rule version plus its scoped known-error decisions and Rust variant contract |
| Candidate contract | Is the recorded path compatible with changed rules? | Proposed Python/Rust target contract and target engine |

A game legal under its original rules may differ under a new rule without
having been played incorrectly. Conversely, a Rust bug may reproduce perfectly
under its original binary and still violate the original intended contract.
Use the appropriate mode in every report. These are separate requested checks,
not a requirement to run three replays for every update. An existing original
trace/verdict is reused when its bound identities remain valid.

Updating Python, changing a Python pin, changing Rust behavior, changing static
content, changing a codec and changing a model are distinct changes with separate
identities/evidence. Never update all version numbers just to make a failing
comparison pass.

## 2. Follow Python's rule versions, with a Rust subrevision

The current source [rules_version.py](../src/goa2/domain/rules_version.py)
defines numbered rule changes and states that a game retains the version it was
created with for its whole life. Follow that convention.

Represent the rule contract as structured fields:

- `python_rules_version`: the upstream Python rule-version integer.
- `rust_rules_revision`: a nonnegative integer within that Python version.
- `variant_manifest_digest`: the declared interpretation/difference registry.
- Separate exact Python/Rust source, build/runtime, content/catalog and codec
  identities. These do not substitute for a semantic rules version.

A display such as `9.r2` is a label, not a float or a replacement for the fields.
Do not order different contracts by decimal arithmetic.

| Change | Version treatment |
| --- | --- |
| Follow a new Python rule version | Adopt that integer; start the Rust revision at 0 unless documented target differences require another explicit revision |
| Semantic Rust fix or intentional difference within the same Python version | Increment Rust revision; record what changed, why, and the exact target reference/exception |
| Known Python defect corrected in Rust | Retain upstream integer, increment Rust revision for the new behavior and bind a narrow known-error entry |
| Python implementation fix without upstream integer change | Pin a new Python source/runtime identity; if it changes Rust-observable behavior, give the target a new Rust revision and interpretation record |
| Proven behavior-preserving refactor | No semantic revision bump; update build provenance and run equivalence checks |
| Codec/feature/model-only change | Its own version; assess semantic impact rather than assuming it changes game rules |

A Rust revision identifies a released behavior/interpretation change, not a
claim that every previous bug was intentional. Retain original-contract defect
findings separately from as-played reproduction.

New games select an admitted full contract. Existing games remain pinned to
their original contract/runtime/content for their lifetime. Do not silently
upgrade active games on server restart, rollback, client update or model update.
An explicit current-game transition must be lossless and separately evidenced;
if rules genuinely change, an authorized fork/new game or declared transition
policy is required. Preserve original history and mark the transition boundary.
If a necessary old runtime cannot safely run, stop that affected path with an
actionable status rather than reinterpret it under newer rules.

Retaining artifacts/adapters for auditing recorded contracts is not an obligation
to support every obsolete public API or historical model in the live service.

## 3. Intake and change record

Before changing code:

1. Read repository guidance, current foundation/client/data/neural contracts and
   the actual rule-version definitions in the relevant pinned Python source.
2. Create a dedicated change branch and an immutable baseline reference. Identify
   the intended rule, source/adjudication, current and proposed full contracts,
   affected cards/heroes/maps/phases and possible indirect dependencies.
3. Freeze baseline Python source/runtime/lock, Rust source/build, content/catalog,
   fixtures/indexes and any model/dataset identities. The original repository/
   artifact history remains intact.
4. Classify the work: new rule, implementation defect, Python-known-error case,
   representation-only change or unresolved mismatch. Record intended versus
   unwanted differences and an unaffected negative-control cohort.
5. Specify current-game handling, regression tests, historical-audit scope,
   fixture regeneration scope and NN impact before implementing the change.

Maintain an update report with change ID/status, baseline/target identities,
rule reference, affected-domain hypothesis, tests and commands, audit coverage,
findings/exceptions, fixture mapping, data/statistics/rating/NN decisions and
release/rollback evidence. Unavailable artifacts are explicit missing inputs,
not permission to replace the original baseline.

### Known Python error registry

Each exception has an ID, precise rule/scope/version/source predicate, minimal
reproducer, expected behavior, evidence/rationale, affected tests, resolution
status and the Python fix/version that ends its applicability.
Until evidence resolves a discrepancy, classify it unresolved.

Only matching cases receive a known-reference-error classification. Still
report their actual differences; do not turn them into ordinary equality passes.
Test adjacent cases and the reverse/negative predicate so the exception cannot
hide unrelated Rust errors. A patched Python reference is a separately identified
artifact, not an invisible edit to the gold-standard checkout.

## 4. Tests before and after implementation

Create a minimal regression that demonstrates the old defect or old/new rule
difference. For a new rule, exercise both sides of the version gate with their
own expected behavior. Include normal and edge cases, failure/skip paths,
recipient views/events, physical-card bindings and complete affected transitions.

Test Rust against the exact target Python contract. If Python has a known error,
show original Python behavior, the narrow exception and independently evidenced
expected behavior. Do not generate an expected answer solely from the Rust code
being changed and call that reference parity.

Implement the engine/content/exporter changes as separately reviewable steps.
Preserve live validation, transactional acceptance, clocks, replay and old
contract dispatch for active games. Hashing/recording must not choose actions,
consume RNG, reorder events or add synthetic prompts.

Run the completed-cleanup documented test tiers. Standard Rust checks from
`rust/` include:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
```

Also run the documented content/fixture integrity, affected service, differential,
restore and tooling suites with the pinned compiler/locked dependencies.
Use the actual wrappers/CLI present in the checkout; do not invent a historical
audit command before its implementation. Report expected/executed/missing/failed
and explicit ignored cases. A quick pass is not full parity.

Tests must prove the target change, original-version preservation, unaffected
behavior, privacy/nonmutation, deterministic replay and candidate client/content
compatibility. New tests pass and any expected changed old tests have an explicit
version/rule explanation; do not broadly delete or rebaseline failures.

## 5. Historical checking: one replay against recorded decision hashes

**User requirement:** replaying a game is fine. Replaying the original engine
again for every comparison is unnecessary. During original play, record a
reproducible decision-space hash. A target engine replays the recorded game once
and compares its decision spaces with that original evidence.

The implementation contract is in the [audit plan](RULE_VERSION_AUDIT_IMPLEMENTATION_PLAN.md).
It defines canonical complete decision spaces, stable identities and normalized
opaque handles, plus separate state/event/result checks.

For each audit:

1. Select immutable original records/trace and an exact target engine/contract.
   Validate hashes, schemas, versions and reference exceptions before execution.
2. Validate an admitted target-start recipe and audit range as specified in the
   audit plan. Preserve the source checkpoint/identity; never just rewrite its
   header to bypass compatibility. Restore once and replay recorded operations
   once in the target. At each archived decision point, compute the canonical full
   issued decision-space hash and compare it to the recorded hash.
3. Check the recorded selected action against the target's real legal space,
   then execute it with recorded time/chance semantics. Compare supported
   state, event and terminal-result evidence as well.
4. Continue after a discrepancy only while the recorded path remains executable
   and alignment is meaningful. If it is not, report the earliest divergence and
   an unreachable/unverified suffix. Do not invent replacement actions or force
   illegal input just to reach the original terminal.
5. Store findings/coverage keyed to original trace, target, adapter, target-start
   recipe, audited range and comparison profile. Reusing a completed identical audit is permitted; changing a contract,
   adapter, hash schema or exception registry invalidates its cache.

Each additional target engine runs its own single replay; comparison to original
recorded hashes does not need a paired original replay. Different engines may
have different internal states but must implement the declared semantic
canonicalization. Same seed alone is not cross-engine RNG equivalence.

Decision-space hashes identify choice availability, not all rule behavior.
A damage/result error can leave the same choices. Separate state/event/result
digests and invariants detect those cases during the same replay, without a
second pass. A hash match is strong reproducibility evidence for the defined
canonical bytes, not proof that the original engine implemented every rule.

For older games without trustworthy recorded hashes, mark original-hash coverage
missing. An optional one-time original-version backfill may create a cached
baseline, explicitly marked reconstructed rather than recorded at play.
That is separate initial recovery work, not a second original replay on every
audit. Candidate legality/state checks can still run with honest coverage limits.

### Scope and finding labels

Default to all retained games in the affected contract/content cohort when
feasible. If sampling is needed, record the exact population, seed/selection
recipe, strata, targeted rule exposures and executed fraction. Include negative
controls and general cases to detect unintended effects. “Many checked” never
means “all verified.”

| Finding | Meaning |
| --- | --- |
| Verified within profile | Comparable checked points matched the declared original/target contract |
| Observed original-contract violation | Evidence shows a selected action/transition broke the intended rules applicable then |
| Decision-space divergence/exposure | Available choices differ; not by itself proof that the player chose an illegal move |
| Expected candidate-rule change | A documented change relative to old rules; not an accusation of wrong original play |
| Known Python reference error | Exact scoped exception matched; discrepancy remains visible |
| Unresolved engine/adapter mismatch | Needs adjudication; do not auto-blame the target or original Rust |
| Missing/uncheckable | Missing trace, unsupported schema/rule/operation/RNG mapping or unreachable suffix; never a pass |

Flag games in a versioned audit-findings registry with the first witness, affected
revision/rule, comparison mode, reference/exception, severity and review status.
Keep original games/replays/results unchanged. Player-facing flags/notices use
scoped safe summaries; they must not reveal hidden original decisions or private
archive data. Findings do not automatically ban a player, revoke a result or
rewrite Elo.

## 6. Fixtures, catalogs and oracle artifacts

Classify outputs before regenerating:

- Runtime rules/content catalogs need the exact new contract/provenance.
- Minimal regressions prove the intended rule and exception boundaries.
- Differential/restore corpora retain their original admitted identities.
- Historical traces/findings are evidence, not expected outputs to overwrite.
- Neural features/targets/datasets follow their separate impact/admission path.

Generate into a new versioned destination using the exact target source/runtime,
generator/config and deterministic compression. Compare old/new catalogs,
indexes and cases, preserving physical identities, object aliases, order,
null/unknown semantics and bounded streaming.

Record old-to-new artifact/case mapping and explained changed counts. Unaffected
cases should remain equivalent under the stated comparison. Investigate
unexpected drift. Never overwrite old frozen hashes, relax oracle validation or
relabel changed bytes under an old identity to make tests pass.

A Python/oracle source change may require a new source digest even if the
upstream rule integer stayed the same. Update code and manifests deliberately
and verify their declared relationship. Rebuild native/client artifacts that
embed catalogs. Archive the admitted source/runtime and generators so later
verification does not depend on a developer's original Git database.

Re-run integrity plus relevant parity/restore against newly admitted target
artifacts. Keep old artifacts needed by active games/audit retention. Content
and fixture redistribution/retention follow existing policy; do not copy real
match secrets into public regressions.

## 7. Data, statistics, Elo and NN impact

Feed audit findings through the database plan's versioned eligibility/result
workflow. Keep a separate annotation for “old path changes under new rules.”
Only an explicitly adjudicated original-contract defect/result correction
enters rating certification/rebuild decisions. A new rule alone does not
retroactively make an old ranked game invalid.

Quarantine affected training sources/targets pending review. A decision-hash
match alone does not prove that original outcomes/value labels remain equivalent
under changed rules. Reusing those labels requires the relevant complete
comparison evidence and feature/target re-extraction. Separately, a fully
executed target replay can supply a new scripted replay-derived trajectory and
its own validated outcome, even when original hashes are missing or states differ.
Admit it under its own lineage/recipe; do not call it original equivalence or new
human/ranked play, and do not borrow old search-policy telemetry. Incomplete
or unreachable target paths have no complete terminal value labels.

If no NN exists, record NN work not applicable and preserve the impact record.
When one exists, invoke the [NN rule-change procedure](NN_RULE_CHANGE_UPDATE_PROCEDURE.md).
Its choices include no weight change after evidenced revalidation, fresh training,
warm-starting the current model or starting from an earlier compatible ancestor.
Do not silently use an old-model compatibility label as evidence of retraining.

## 8. Release and completion

A release record must bind the target rule/oracle/source/runtime/content,
fixtures, adapters/hash schema, client artifacts and any admitted model/data
versions, with baseline/target tests and historical-scan coverage/findings.

New games use the admitted target. Existing games keep their pinned contract or
the explicit separately tested transition policy. Model incompatibility may
leave new games on the admitted heuristic evaluator while NN work continues;
it must not silently change game rules or strand old games.

Rehearse current-game restore, deployment routing, rollback and post-change
accepted-command preservation on disposable data. Rollback selects a matching
runtime/content/model/database state; it cannot lose accepted moves or silently
reinterpret the rule identity. Actual publication/deployment and player
communications occur only within the user's authorized scope.

Complete a change only when:

- [ ] Rule/authority/version/known-error decisions and exact identities are recorded.
- [ ] Regression/version-gate/negative-control and required full correctness
      checks pass with truthful counts.
- [ ] Historical coverage/findings, missing cases and expected changes are recorded.
- [ ] New fixtures/catalogs/pins are independently admitted; old evidence is retained.
- [ ] Current-game, client, data/statistics/Elo and NN dispositions are explicit.
- [ ] Release/rollback evidence and unresolved limitations are reviewable.

If audit/version tools are not implemented, complete authorized code/tests/
fixture analysis independently and record the blocked gates. Do not claim an
all-games audit or safe deployment that has not been demonstrated.
