---
name: nn-rule-change-update
description: >-
  Assess and update a GoA2 neural evaluator after a rule, oracle, content,
  feature, legal-action or reward change. Use to choose revalidation, fresh
  retraining, warm-starting the current model or an earlier compatible model,
  with immutable data/model lineage, split protection, bounded training and
  target-rule admission.
---

# Update a neural model after a rule change

State that you are applying this skill when first using it. Use only within the
user's authorized model-update task and declared compute/data constraints.

## Read first

Read repository/path instructions, then:

- [NN rule-change procedure](../../../docs/NN_RULE_CHANGE_UPDATE_PROCEDURE.md).
- [Rule-update procedure](../../../docs/RULE_ENGINE_UPDATE_PROCEDURE.md).
- [Neural-search plan](../../../docs/NEURAL_SEARCH_IMPLEMENTATION_PLAN.md), especially model readiness.
- [Database/admission plan](../../../docs/DATABASE_TRAINING_STATISTICS_PLAN.md).
- [Client NN stage](../../../docs/CLIENT_SIDE_BOT_EXECUTION_PLAN.md).

Confirm which pipeline/model/artifacts actually exist. Procedures and trait
stubs are not evidence of an implemented NN.

## Workflow

1. **Establish applicability.** If no real model exists, record not applicable,
   preserve the impact record and use the admitted heuristic. Otherwise bind
   baseline/target rule contracts, feature/action/boundary/reward semantics and
   exact model/runtime/data/ancestor identities.
2. **Assess changed knowledge and targets.** Read historical single-replay
   findings, including missing/partial coverage. Separate legality/policy
   changes, stable-value/outcome changes, distribution shifts and representation
   changes. Decision hashes alone cannot certify old value labels.
3. **Admit target data.** Re-extract/revalidate eligible data or generate bounded
   new target-rule games. Quarantine broken/incomplete labels; do not attach an
   old winner to an unexecuted counterfactual path. Preserve usage restrictions,
   lineage/start/seed groups and held-out/ancestor-exposure records.
4. **Choose candidate initialization.** Compare as appropriate: unchanged weights
   after revalidation, fresh training, current-model warm start and an earlier
   compatible ancestor. Record the exact chosen artifact and semantic tensor
   mapping. Shape/name equality alone does not establish compatibility.
5. **Define transfer and budget.** Declare transferred/reset heads/features and
   fresh optimizer/scheduler by default. Exact optimizer/run resume requires
   unchanged validated run identity; it is not a rule-change shortcut. Freeze
   resource ceilings, hypotheses, replicates, stopping/promotion criteria and
   dataset manifests before training.
6. **Run bounded adaptation.** Use the existing validated pipeline and isolated
   training/self-play namespace. Record all outcomes and artifact provenance.
   Do not silently expand into an unbounded sweep, train on reserved evaluation
   games or inherit unknown provenance as “clean.”
7. **Validate actual usefulness.** Check new-rule witnesses, unaffected cohorts,
   policy/value/terminal semantics, privacy, reference/export/native and any
   claimed browser conformance. Run held-out paired Rust play and equal-work/
   equal-time/resource controls with honest failure/censoring counts.
   Verify incremental features/NNUE accumulators against full recomputation.
8. **Admit and hand off.** Create a new immutable target-compatibility artifact/
   record. Unchanged weights retain original training provenance and are not
   described as retrained. Activate only for compatible jobs/games, invalidate
   old caches and test rollback to an evaluator admitted for the same rules.
   Client native/browser gates remain separate.
9. **Report disposition.** A rejected/deferred model leaves a safe heuristic
   option for the new rules and preserves old compatible models for pinned
   old-contract games. Do not claim NN update success because engine tests pass.

## Required safeguards and output

Never overwrite/relabel an old model, change game rules to suit weights, mix
model identities in one search or leak future/private state into inference
features. Warm-starting an earlier model does not erase its dataset exposure.

Report initialization choice, exact rules/model/data ancestry, transferred/reset
parameters, budget/actual run counts, held-out/new-rule/unaffected evidence,
runtime/platform scope, compatibility/rollback and remaining limitations.
Actual training/deployment must stay within authorization already provided;
a missing implementation/artifact is a recorded blocker, not an invented run.
