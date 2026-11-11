---
name: rule-engine-update
description: >-
  Update GoA2 Rust rules, Python rule-version alignment, oracle/content pins or
  rule fixtures. Use for rule corrections, adopting Python rules, known Python
  errors and historical game rule audits. Requires evidence-based versioning,
  regression tests, one target replay against recorded decision-space hashes,
  explicit findings and NN/data impact.
---

# Update the rule engine

Use this skill for implementation work under an authorized rule/oracle update.
Writing a plan alone does not authorize an unrelated deployment or training run.
State that you are applying this skill when first using it.

## Read first

Read repository and path-specific AGENTS instructions, then:

- [Rule-update procedure](../../../docs/RULE_ENGINE_UPDATE_PROCEDURE.md).
- [Versioning/single-replay implementation plan](../../../docs/RULE_VERSION_AUDIT_IMPLEMENTATION_PLAN.md).
- [Shared foundation](../../../docs/SHARED_RUST_FOUNDATION_PLAN.md).
- [NN update procedure](../../../docs/NN_RULE_CHANGE_UPDATE_PROCEDURE.md) if a real model exists.

Use the checkout's actual supported facade, test tiers and tools. The audit plan
specifies proposed facilities; do not assume they are already implemented or
invent a CLI command. Verify availability before using one.

## Workflow

1. **Freeze intake.** Confirm intended rule/source/adjudication, branch and
   baseline/target Python rules plus Rust revision/variant. Record exact engine,
   Python runtime/lock, content/catalog/fixture and model identities. Infer routine
   choices from context; continue independent work while resolving genuinely
   missing rule meaning or artifacts.
2. **Classify authority.** Rust is the live engine and presumed correct; the
   pinned versioned Python engine is the current reference except witnessed
   scoped known errors. Do not self-declare a broad Python exception to obtain
   a green test. Separate as-played, original-contract and new-contract audits.
3. **Add failing evidence.** Write a minimal regression and version-gate/
   unaffected negative tests. Cover nearby optional/mandatory, prompt, scope,
   event/order and physical-instance cases. Prove the expected result from
   the reference or an explicit known-error/adjudication record.
4. **Implement narrowly.** Keep semantic rules, mechanical moves, pin/exporter
   updates and artifact admission reviewable. Follow upstream Python rule integers;
   use a Rust subrevision for semantic Rust differences/fixes. Preserve active
   games' pinned contract, current command/retry/replay/clock behavior and privacy.
5. **Verify code and artifacts.** Run documented format/lint/native/rustdoc,
   tooling, fixture integrity and affected service/full differential/restore
   tiers. Regenerate only into new target-version destinations. Map/explain
   changed cases; preserve old hashes/artifacts and unique coverage.
6. **Audit history once per target.** Replay recorded operations in each target
   once, comparing complete canonical decision-space hashes to original recorded
   hashes. Check selected legality and same-pass state/effect/result evidence.
   Do not rerun the original engine for every comparison. Missing hashes may
   justify a separately recorded one-time original backfill; otherwise report
   the coverage gap. Never force an illegal continuation or treat its suffix
   as verified.
7. **Classify and retain findings.** Flag witnessed original-rule violations,
   decision-space exposure, expected new-rule changes, known Python errors,
   unresolved adapter/engine mismatches and uncheckable cases separately.
   Record first witness, identities and exact population/sample/counts.
   Do not mutate old winners, Elo, training labels or player reputations merely
   because a candidate differs. Generate safe scoped annotations, not external
   emails/messages unless explicitly instructed.
8. **Assess downstream impact.** Record current-game/client rollout, fixtures/
   content, statistics/Elo and dataset disposition. If an NN exists, apply the
   NN procedure/skill within the authorized scope and resource budget. No NN
   means “not applicable,” not fabricated retraining evidence.
9. **Prepare a reviewable result.** Commit authorized changes/evidence on the
   intended work branch. Record release/rollback checks and remaining gates.
   Publish/deploy/notify only when already authorized by the user's task/session;
   do not infer a new approval requirement from this skill.

## Non-negotiable checks

- Version/hash identity binds the artifact and rule meaning; never relax it to
  load incompatible data or overwrite frozen expected hashes.
- Hash the complete issued legal space, not a search cap, sampled subset or UI
  labels. Normalize opaque IDs only through a proved semantic binding.
- A decision-space match alone is not proof of state/result correctness.
- Same seed alone is not cross-engine RNG equivalence.
- Reports distinguish expected, executed, matched/different, missing, unsupported
  and unreachable cases. Sampled coverage is not all-game certification.
- Recording/evaluation may not consume gameplay RNG or mutate the live root.
- If a required tool is absent, record that implementation gate and finish
  unaffected authorized work. Do not claim the planned audit or safe rollout ran.

## Final handoff

Report the rule/version decision, changed code/tests/artifacts, real commands
and counts, historical scope/findings, current-game and data/NN dispositions,
commit/artifact identities and unresolved limitations. Distinguish implemented,
verified, deferred/not applicable and actually released.
