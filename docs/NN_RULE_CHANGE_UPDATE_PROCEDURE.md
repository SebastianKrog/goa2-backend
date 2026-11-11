# NN update procedure after a rule change

Status: procedure for later, once a real NN/model pipeline exists.
Written 11 November 2026. This does not launch training or claim that an NN is
implemented. Use with the [rule-update procedure](RULE_ENGINE_UPDATE_PROCEDURE.md),
[neural plan](NEURAL_SEARCH_IMPLEMENTATION_PLAN.md) and
[NN rule-change skill](../.agents/skills/nn-rule-change-update/SKILL.md).

## 1. Trigger and invariants

Run this impact assessment when a rule, Python/Rust subrevision, reference
exception, public content, feature/observation semantics, legal-action encoding,
reward/outcome, boundary definition or model runtime can affect inference or
training. A rules change does not automatically require full retraining, but
compatibility must be demonstrated rather than assumed from unchanged shapes.

If there is no NN, record “not applicable: no model,” preserve the change/data
impact record, and continue with the admitted heuristic. An interface or smoke
network is not a deployed model that needs artificial retraining.

Keep active games on their pinned rules/runtime and compatible evaluator.
An old model may remain valid for those games while a new model is prepared for
new rules. Do not downgrade the new rules to fit a preferred old model, relabel
an incompatible artifact, or silently mix old/new scores within a search.

## 2. Freeze impact and candidate sources

Record old/new full rule contracts, exact source/content identities, exception
registry, feature/boundary/action/score schemas and actual model/runtime/data
versions. Link the single-replay historical-audit findings and completeness.
Identify both direct changed behavior and distribution shifts in unaffected
positions or future outcomes.

For every candidate initial model, capture artifact digest, supported rules/
features, training datasets and split groups, ancestor lineage, previous
evaluation scope, export/precision and optimizer availability.

Consider at least these initialization candidates where artifacts permit:

| Path | Use | Required evidence / caution |
| --- | --- | --- |
| Revalidate existing weights | Behavior/feature changes may not require new weights, and new-contract acceptance actually passes | New compatibility attestation and all affected-scope checks; do not claim retraining |
| Fresh training | Target semantics/features changed substantially, prior provenance is unsuitable, or a clean baseline is needed | New initialization/data lineage and a bounded from-scratch reference |
| Warm-start current model | Most representation/knowledge transfers to the new target | Explicit compatible-parameter mapping, fresh optimizer by default and new-contract training/evaluation |
| Warm-start earlier model | An earlier ancestor has a better-compatible schema, less affected training or a better measured starting point | Exact chosen ancestor and rationale; earlier does not automatically mean cleaner or stronger |
| Partial transfer/reset | Only selected feature blocks/trunk/heads are compatible | Semantic mapping report, reset parameters/heads and controlled comparison with full/fresh alternatives |

Use a predeclared bounded comparison, not an endless ancestor search on the final
test set. If suitable prior provenance/permission is unavailable, do not infer it
from a filename; use a new clean lineage or record the limitation/blocker.

## 3. Data revalidation and regeneration

The database admission registry is authoritative for source/result/lineage,
purpose, consent/usage and held-out groups. A rule-change audit can produce
new-contract trajectories/features during its single target replay; there is
no need to replay the original engine again just to compare stored hashes.

Classify each source cohort:

- A complete target replay preserves the recorded path, relevant state/events
  and terminal result: eligible for a new target-version dataset only after
  explicit admission and feature/target re-extraction.
- Decision spaces match but state/result semantics changed or are unchecked:
  old value labels are not certified as equivalent. A completely executed target
  path may supply its own new labels through a separate derived-data recipe.
- Recorded action becomes illegal or semantic prompt/chance alignment is lost:
  do not attach the old winner to the target's hypothetical trajectory. Quarantine affected
  labels and generate new complete target-contract games.
- Missing original hashes with a complete, valid target execution: historical
  equivalence is unproved, but a new scripted replay-derived trajectory may be
  admitted under a separate recipe, using its actual target states/outcome,
  features and lineage. It is not fresh human/self-play evidence and cannot
  inherit original search visits as target policy labels.
- Incomplete/sampled/unreachable target execution: retain truthful coverage.
  Selected supported policy examples need their own recipe; no unexecuted
  terminal outcome or complete-game value certification may be borrowed.
- Known original-contract defect: quarantine affected source/target labels and
  record its result/correction disposition rather than simply changing the
  recorded winner.
- Old-contract data retained for old-contract use: keep its original identity;
  it is not automatically new-rule training data.

Recompute legal candidates/policy alignment and stable-value features/labels
under the new schema/contract. Changed action meanings, terminal/reward rules
or evaluation boundaries require updated targets and affected head treatment.
Never treat numeric feature-column equality as semantic compatibility.
Preserve current-game winner truth separately from counterfactual labels.
If chance probabilities/start distributions changed, a legally executed old
sample is not automatically an unbiased new-distribution example. Regenerate
under the target by default, or use an explicitly validated off-policy/weighting
recipe; never infer equivalent sampling from the same seed/outcome.

Add targeted new-rule cases and independently generated target games. Retain
verified unaffected rehearsal data when admissible to limit catastrophic
forgetting; “unaffected” needs evidence, not a card-name filter alone.
Follow current masks, recipient knowledge and actual stable-transition semantics.
Finished private replay/future revealed information must not enter inference
inputs.

Freeze new dataset manifests, semantic hashes, exclusion/admission reasons and
source-to-row mapping. Keep split groups stable across old/new versions,
duplicate games, forks, changed compression and regenerated seed families.
Training on an old validation/arena family makes it unsuitable as an unseen
promotion family later. Parent weights may already contain old data exposure:
declare it and select genuinely reserved evaluation groups where provenance
supports that claim. Missing ancestor provenance is not proof of no exposure.

Old artifacts remain immutable. Retractions/usage changes propagate through the
registry and affected runs/models according to data policy; warm-starting does
not erase the influence or provenance of earlier training.

## 4. Weight transfer versus optimizer/run resume

Warm-start means initialize a **new target run** from identified compatible
weights. Default to a fresh optimizer/scheduler and explicit initialization for
new/reset tensors. Record every transferred/dropped/reset tensor, shape, dtype,
feature/embedding meaning and head semantics. Matching names/shapes alone is
insufficient, especially after vocabulary/coordinate/action-order changes.

Policy and value heads are assessed separately:

- New legal actions/candidate features may require retraining/resetting policy
  parameters even if a shared trunk is reusable.
- Changed victory/reward or stable-boundary meaning may invalidate value weights
  despite identical input/output dimensions.
- Sparse/NNUE feature index changes require a proved semantic remapping or reset.
- Trunk reuse/freezing, head-only adaptation and gradual unfreezing are
  experiment variants, not guarantees against new-rule errors.

Exact run/optimizer resume is only for unchanged run identity with compatible
data/model/target/optimizer semantics and a validated checkpoint. It is not the
default way to bridge a rule change. Reusing optimizer moments across changed
rules needs its own explicit experiment; never present it as an ordinary resume.

For a “no weight change” decision, publish a new immutable compatibility/
validation record bound to the target contract while retaining the original
training-rule/data provenance and unchanged weight digest. Do not edit the old
manifest to pretend the model was trained under new rules.

## 5. Bounded training/adaptation and validation

Before training, freeze hypotheses, target dataset, initial-model choices,
hyperparameters/seed replicates, update/game/wall/CPU/GPU/memory/disk limits,
stopping criteria and promotion criteria. Run the existing bounded pipeline;
do not launch automatic unbounded self-play or a new framework solely for this
update. Regeneration/training uses isolated self-play/test namespaces, never
the production human Elo stream.

Compare at least an admitted heuristic and a fresh/reference option appropriate
to the change. Compare current-versus-earlier warm starts where proposed.
Report learning curves and total data/compute, not only final validation loss.

Validation must include:

- New-rule regression and nearby negative cases, including the exact defect/
  historical witnesses that motivated the update.
- Held-out affected and unaffected hero/map/mode/roster/phase cohorts.
- Policy legality/alignment, candidate permutations, unknown/private-information
  masks and stable-value/boundary semantics.
- Value calibration in its declared outcome convention, not fabricated draws
  for failed/censored games.
- Fixed-policy/value-only/policy-only ablations where needed to identify which
  component adapted.
- Paired actual Rust games with frozen opponents/seeds and declared equal-work
  versus equal-time budgets; uncertainty and operational failure rates.
- Reference export/native CPU and any advertised WASM scalar/SIMD conformance,
  precision/near-tie behavior and resource/cancellation limits.
- Full-recompute versus any incremental NNUE/feature accumulator across
  apply/undo/clone/restore, phase changes, effects, upgrades and model switch.

Do not select hyperparameters or ancestors repeatedly against the final reserved
promotion set. Check that new-rule improvement did not merely trade away
unaffected play, increase failures or shorten the search workload.
A candidate can be rejected even with better prediction loss.

## 6. Admission, rollout and completion

Admit an artifact only with the neural plan's real-model readiness evidence
and target-specific current-game/feature/score identities. Publish safe
data-only weights/operator manifests and reference vectors, not the research
environment or arbitrary executable model code.

Activation is per new job/game-compatible evaluator. Cancel obsolete work and
invalidate model-dependent trees/caches/accumulators; never aggregate old/new
model scores under one identity. New-rule games may run the heuristic until a
model passes. Native and browser clients each pass their advertised product
gates; native success alone does not certify a browser build.

Rehearse retraction/rollback to an evaluator **admitted for the same game rule
contract**. An older NN is a valid rollback only with that compatibility.
Keep active old-contract games separate. Observe model identity, resources,
fallback/failure and new-rule behavior under controlled rollout; do not silently
switch model/rules in a current player's game.

Completion report:

- [ ] Impact and initialization decision: revalidation, fresh, current warm-start,
      earlier-model warm-start or explicit partial transfer.
- [ ] Data/target admission, quarantine, source audit coverage and split/ancestor
      exposure are recorded with immutable identities.
- [ ] Transfer/reset/optimizer semantics and bounded run results are reproducible.
- [ ] Affected/unaffected prediction, paired-play, reliability and resource
      criteria pass, or rejection/heuristic-only disposition is explicit.
- [ ] Safe artifact/reference vectors, native/browser scope, admission and
      compatible rollback are recorded.
- [ ] No old artifact/history was overwritten or falsely relabelled.

A rule update may complete with NN deployment deferred and a safe heuristic
disposition. It may not claim that NN retraining/revalidation succeeded when
only the engine tests passed.
