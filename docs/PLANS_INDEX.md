# Five-plan index and contract ownership

Documentation review, 11 November 2026. All five plans are written; implementation
packages remain open until their evidence exists. This is navigation and an
ownership map, not a sixth plan or a duplicate interface specification.

| Plan | Entry point | Owns |
| --- | --- | --- |
| 1. Foundation | [Shared foundation](SHARED_RUST_FOUNDATION_PLAN.md) | Common Rust contracts, observation/commands, semantic features, core boundary detector, parametric proof and execution limits |
| 2. Clients | [Client execution](CLIENT_SIDE_BOT_EXECUTION_PLAN.md) | Native -> WASM -> NN hosts, delegation/ownership, connection lifecycle, packaging and per-host acceptance |
| 3. Storage/data | [Database plan](DATABASE_TRAINING_STATISTICS_PLAN.md) | Live PostgreSQL, archives/outbox, training admission/storage, statistics/Elo and test sessions |
| 4. Neural search | [Neural plan](NEURAL_SEARCH_IMPLEMENTATION_PLAN.md) | Tensor/head/target semantics, inference/experiments, architecture comparison and incremental scaling |
| 5. Updates/audits | [Rule procedure](RULE_ENGINE_UPDATE_PROCEDURE.md), [NN procedure](NN_RULE_CHANGE_UPDATE_PROCEDURE.md), [implementation plan](RULE_VERSION_AUDIT_IMPLEMENTATION_PLAN.md) | Python-aligned/Rust rule revisions, single-replay audits, findings, fixtures and model/data updates |

Plan-5 skills: [rule-engine-update](../.agents/skills/rule-engine-update/SKILL.md)
and [nn-rule-change-update](../.agents/skills/nn-rule-change-update/SKILL.md).
They follow the procedures; the R packages still require implementation.

## Contract ownership

Implement shared contracts once; consumers add adapters and their own tests.

| Contract | Owner | Consumer boundary |
| --- | --- | --- |
| Rule/content identity and initial current-game binding | F01 | R01/R07 adds multi-revision registries/dispatch; every host/store/model binds the contract |
| Typed legal space, decision/physical references and command encoding | F02/F04 | R02 owns canonical hash bytes/profiles over those semantics, not another legality API |
| Observation/knowledge and hypothetical worlds | F02/F03 | Clients/search use authorized inputs; exports preserve viewer/time provenance |
| [Evaluation boundaries](SHARED_RUST_FOUNDATION_PLAN.md#evaluation-boundaries) and transition anchor | F06 core detector | N02 extends neural call/head/target coverage; no competing detector |
| Semantic features, full-recompute extractor, evaluator ports and score convention | F06 | N01/N03 owns tensor adapters/model/export; sparse, plane and graph layouts are not one universal tensor |
| Budgets, cancellation, branch isolation | F07 | Client owns workers/browser lifecycle; NN owns backend integration/overshoot |
| Prepare/durable accept/retry | F05 on current store/test adapters | D02/D03 supplies PostgreSQL/multi-instance guarantees |
| Client seat lease versus scheduler lease | B02 delegation contract; D03 persistence/fences | Renewal is not gameplay; takeover fences obsolete jobs |
| Game/result/session/lineage and admission | D01/D04/D07/D08 | NN consumes admitted manifests; audits do not silently rewrite original results |
| Statistics and rating ledger | D05/D06 | Audited corrections require adjudication; one effective contribution per rated game |
| Audit hashes, target starts, adapters/ranges/coverage | R02–R06 | Foundation exposes nonmutating hooks; storage retains compact private evidence |
| Model readiness and rule-change adaptation | Neural L2/section 9; NN procedure | Client E gates certify each native/browser product independently |

## Dependency slices

Plan numbering is a design sequence, not a requirement to finish an entire
predecessor before any useful implementation.

- F01 plus observation/command contracts enable independent consumers.
- Live database D02 needs F01/F04/F05, not reconstruction, NN tensors or WASM.
  F05/F08 can use current SQLite/disposable adapters; no PostgreSQL dependency
  cycle is intended.
- Native B03 and neural hook work need F03's observation-only constructor,
  F06's shared proof evaluator and F07. Prove F03 early; private checkpoints
  cannot stand in for authorized reconstruction.
- Foundation transport/browser feasibility tools are conformance work.
  B04/B07 owns actual installers/browser releases, not F08.
- NN smoke/pilot work needs the data/admission contracts and a conforming bounded
  local backend. Immutable local manifests/shards can suffice; production
  PostgreSQL rollout is not a prerequisite. Actual production sources require
  the validated export/registry path before training admission.
- Neural L2 can unblock B08 without large networks/distributed training.
  Native NN release needs its native E gates; browser release additionally
  needs WASM/browser E03 and the other relevant browser evidence.
- R02/R03 consumes shared legal types/journal hooks. Historical scanning and
  multi-version routing remain R work, not hidden F08 prerequisites.

F08 closes shared conformance. Each consumer still supplies its own product/
deployment evidence. A design document never completes an implementation gate.

## Evidence and compatibility

The foundation's [evidence/scope contract](SHARED_RUST_FOUNDATION_PLAN.md#evidence-and-scope)
owns common identities, counts and current-game coverage. Link exact unchanged
evidence; rerun checks for affected changes. Consumer gates add distinct workload/
failure tests rather than copying generic engine regressions.

“Current” includes in-progress admitted contracts after new games select a new
rule revision. Correct pinning and declared audit retention are required;
blanket support for obsolete APIs/models/formats is not.

The update procedures distinguish original-history equivalence, complete
target execution and admission of a new scripted target trajectory. Missing
original hashes limit the first, not necessarily the latter two. A suffix
without target-prefix evidence is not a whole game; an incomplete target path
has no borrowed terminal label. Metadata identity stays bound to a digest's
envelope rather than forcing all cross-version semantic hashes to differ.

## Decisions still required

| Decision | Resolve in |
| --- | --- |
| Reconstruction feasibility for advertised decision families | F03 |
| Canonical encoding, physical/prompt mappings, target-start/chance recipes and comparison profiles | R02/R04 |
| NN backend/operators, schemas, architecture and numeric convention | N01/N03/N06 |
| Training budgets, promotion thresholds and reserved cohorts | N04/N07 before promotion results are inspected |
| Advertised OS/browser/model capabilities | B04/B07/B08–B10 |
| PostgreSQL baseline, capacity/latency, retention and RPO/RTO/key recovery | D01/D09/D10 |
| Account linking, ranked eligibility, initial Elo and data-usage policy | D01/D06 |
| Concrete rule adjudications, known Python errors and rollout disposition | Rule procedure/R01, per update |

These are explicit implementation decisions, not measured successes. Proposed
platform/database/Elo defaults are starting choices pending their acceptance.

## Review record

The review moved core boundary semantics into F06, clarified semantic-feature
versus tensor ownership and independent dependency slices, specified validated
target starts/ranges, separated metadata from semantic hashes and original-label
reuse from derived target data, fixed Elo correction slots, and made per-host
NN releases explicit. Repeated chronology/reminder paragraphs were trimmed.
Only documentation and skills changed; no rule/game/model/runtime changed.
