# Neural search and incremental implementation plan

Status: proposed implementation plan; no training run, model admission or neural
search release is claimed complete. Written 11 November 2026. This is plan 4 of
five, following the [shared foundation](SHARED_RUST_FOUNDATION_PLAN.md),
[client execution plan](CLIENT_SIDE_BOT_EXECUTION_PLAN.md), and
[database/data plan](DATABASE_TRAINING_STATISTICS_PLAN.md).

Deliver in four stages:

1. Stable feature representation and working simple evaluators at explicit
   policy/value evaluation points.
2. A reused or adapted framework for neural inference, model evaluation and
   reproducible experiments.
3. Controlled exploration of neural architectures.
4. Incremental implementation and scaling of the selected end-to-end system.

The order is deliberate. A network must not conceal an incorrect search hook,
hidden-information leak or changed workload. A useful small trained model can
be admitted before large-scale generation, large networks or distributed workers
are justified.

## 1. Goals, current baseline and scope

Build a search system whose evaluation can progress from parametric functions
to learned models without changing game rules, command transport, persistence
or the meaning of the training examples.

Assume cleanup is finished. Reuse the foundation's observation-only agent,
typed commands, hypothetical worlds, feature/evaluator identity, budgets and
correctness harness. Reuse the database plan's canonical games, immutable
datasets, lineage/splits and admission rules. Local neural execution is stage 3
of the client plan; it stays blocked until the actual NN implementation/model
deliverables in section 9 exist.

**Current games must keep working.** Pin the implementation baseline's
rules/content/runtime identity. Preserve exact legality, recipient scope,
physical-card identity, clocks, current save/replay and command behavior.
No rules correction, oracle/Python update, historical model compatibility or
reinterpretation of an existing artifact is part of this work.

The planning base is documentation commit `51ddb947`, above a remote source
base that predates the separate cleanup continuation. The reviewed Rust search
contains bounded resolution alpha-beta, sampled planning/hidden-world logic,
hardcoded evaluator choices and explicit work/stop information. Its module
layout and hook locations must be reconciled with the cleaned implementation.

Existing Python AI code has useful policy/value contracts, stable-transition
semantics, data validation and paired-evaluation machinery. Reuse tested ideas
or components where they satisfy the current Rust contracts; old verification
counts and model artifacts are not acceptance evidence for this implementation.
The authoritative rules runtime remains Rust. Python may support offline
training, analysis and controlled orchestration, not live gameplay execution.

### Search and learning are separate choices

AlphaZero suggests learned policy/value with search and controlled self-play.
Stockfish NNUE suggests low-cost evaluation, sparse features and incremental
accumulators alongside a strong search. Neither implies that GoA2 is a
perfect-information, strictly alternating two-player game.

Keep the root viewer and scoring team fixed, preserve simultaneous commitments,
and sample hidden worlds only from authorized observations. Do not give the
agent actual opponent cards or serialize simultaneous choices into a sequence
that reveals one opponent's commitment to another. A faster/stronger evaluator
does not by itself solve information-set search, opponent modeling or strategy
fusion.

Start by injecting evaluators into the current search where the contract fits.
Treat PUCT/ISMCTS or a different joint-planning solver as a separately identified
search experiment, not a prerequisite for a neural value head or an incidental
part of comparing two model architectures.

## 2. Stage 1 — stable features and proven evaluation points

### 2.1 Versioned semantic representation

Adopt the foundation's Rust-owned semantic feature records, full-recompute
extractor and evaluator ports over `PlayerObservation` and allowed hypothetical
continuations. Add/version only the neural-specific fields, tensor adapters
and head semantics; do not create another observation or reference extractor. Give policy inputs
and stable-value inputs distinct context types. Sharing feature blocks does not
make their temporal meanings interchangeable.

| Feature group | Meaning and required treatment |
| --- | --- |
| Identity | Rules/content, observation/feature schema and supported scope; artifact/provenance metadata rather than accidental learned identifiers |
| Board | Valid cells, axial/cube coordinates, six ordered neighbors, terrain, zones/lanes, objectives and occupancy |
| Entities | Team-relative units, hero pieces, minions, tokens, status/resources and public relationships; physical identity separate from definition |
| Cards/loadout | Permitted private cards and public reveal/upgrade knowledge; explicit unknown/count masks; distinguish equal printed definitions from distinct instances |
| Effects/history | Rule-relevant visible effects, durations, public knowledge and progress; no future reveal or unrelated private context |
| Policy context | Typed current decision, eligible controller/owner, legal candidate features and exact action mapping |
| Value context | Stable boundary kind and actor when meaningful, round/turn, fixed root viewer/team; no candidate list or fabricated prompt |
| Time/config | Only explicitly admitted game-visible information; host wall time, secret seed, credentials and machine state are excluded from learned features |

Specify field types, units, bounds, missing/unknown semantics, vocabularies,
normalization, tensor ordering, padding/masks, coordinate orientation and feature
digest. Freeze semantic identity for a released schema; research changes create
a new version and rebuild compatible datasets/golden vectors. Stable means
explicit and reproducible, not that the first schema may never improve.

Use one full-recompute Rust extractor as the correctness reference. Python
training consumes its versioned records or a separately verified equivalent
vectorizer, not a subtly different set of game heuristics. Keep native IDs/
request handles in validated alignment metadata unless their game semantics are
explicit features; arena slots, allocation order and candidate array position
must not become learned identity.

Policy candidates use typed semantic features: action kind, public card/effect
definition, legal unit/hex target, numeric/option meaning and relevant resources.
Keep candidate IDs unique and bound to the current request. Candidate order
permutation must realign scores to the same commands, not change their meaning.
Preserve meaningful order inside a multi-selection/action sequence.

Use sparse records, entity/hex tables and masks as the shared semantic input.
An architecture-specific tensor adapter may choose dense planes, embeddings,
sparse indices or a graph. Do not force every architecture into one fixed
flattened board or make feature storage depend on a particular neural library.

### 2.2 Explicit evaluation points

Adopt the foundation's [evaluation boundaries](SHARED_RUST_FOUNDATION_PLAN.md#evaluation-boundaries)
and its single core detector/transition anchor. Policy scores actual legal
planning/response/upgrade choices; stable value is candidate-free at ACTOR_READY
or PLANNING_READY, and terminal outcomes bypass the network. Budget cutoff is
a stop reason, not another value point.

Stage 1 extends the shared tests to neural-specific call coverage, feature/head
alignment, targets and score semantics. If a hook is missing, extend the core
facade and its conformance tests rather than adding a second detector. Model
features/recorders cannot infer boundaries from phase/prompt text or add synthetic
inputs. A future partial-decision value head requires a distinct schema/targets;
it cannot reuse stable-transition labels.

### 2.3 Plug in simple parametric evaluators first

Provide separately replaceable interfaces, conceptually:

```text
score_policy(decision_features, ordered_legal_candidates, context)
  -> candidate_aligned_finite_logits

score_value(stable_value_features, fixed_perspective)
  -> finite_expected_outcome_value

rules_terminal_outcome(state)
  -> exact_outcome                  // not a model prediction
```

Reuse the foundation's parametric proof artifacts/ports; extend them for the
neural score convention explicitly rather than adding a second heuristic runner.
Begin with a uniform policy control, a deterministic weighted policy
`w · candidate_features`, and a candidate-free weighted value
`b + v · position_features` with an explicit bounded score transform.
Parameters live in a small validated artifact with feature/score identities.
No NN training is needed for this stage.

Use interpretable features such as known material/resources, lane/objective
pressure and positioning. Their availability follows visibility masks;
an unknown opponent hand is not an empty known hand. The initial heuristic need
not be good enough to claim strength: its purpose is to prove the architecture
and provide a measurable baseline.

Define learned value as expected team outcome in [-1, 1], with exact outcomes
win +1, loss -1 and rules-defined draw 0 when supported. Nonterminal prediction
bounds/precision and terminal-priority handling must be explicit; saturating a
heuristic/quantized score must not manufacture a proven win. Do not confuse this
scale with centipawns or an already calibrated probability. Where draws exist,
`P(win) - P(loss)` does not uniquely identify `P(win)`.

Do not alternate value sign merely by tree depth. Consecutive same-team turns,
reactions, team prompts and simultaneous planning require actual team ownership
and the selected solver's explicit reduction, while the root score perspective
stays fixed.

Policy owns logits; search owns masking/softmax/temperature and sampling under
a versioned recipe. Illegal actions never enter normalization. Empty/forced
choice behavior is explicit; a single legal action may skip a model call with
that fact counted. NaN/Inf, output-size mismatch, foreign candidate IDs and
out-of-range value are typed errors. Fallback use is visible, never a hidden
substitute for missing hook coverage.

Wire the simple evaluators into all declared paths: current planning root
selection/order, resolution choices, controlled continuation policies and
stable leaf evaluation after completed transitions. Foreign continuations use
an explicit information-safe environment policy. Keep evaluator selection
independent for policy and value; value-only NNUE-style search must not require
a policy network.

Provide traces with decision/boundary kind, transition anchor, evaluator ID,
candidate alignment, call/skip/stop reason, work counts and fixed perspective.
Use synthetic cases or restricted diagnostics; do not log real private
observations or secret server state. Demonstrate that controlled parameter
changes reach the intended hooks and can change the prescribed test choice.
A trait that is never called by actual search is not completion.

### 2.4 Stage 1 acceptance

| Gate | Evaluation | Complete when |
| --- | --- | --- |
| S01 Feature contract | Golden current-content observations, native serialization/vectorization, masks/vocabularies, physical identities, candidate permutations and declared scope | Exact reference features/identities or specified numeric tolerances; no layout drift or hidden allocation features |
| S02 Boundary semantics | Planning/partial/second-slot, reactions, same actor, cleanup/upgrades, all-pass, terminal, loop and budget cases | Correct detector/anchor and identical normal rules execution/RNG/events with recording enabled or disabled |
| S03 Working hooks | Uniform/weighted policy and weighted value in real planning/resolution/continuation search; call/skip tracing and parameter perturbation | Policy and value are invoked only at their stated points and can affect intended ordering/choice without changing legality |
| S04 Information and ownership | Paired hidden-state worlds, descendant/team/defender decisions, masked unknowns, clone/restore/cancel and reconnect knowledge | Fixed root viewer/perspective, no secret leakage or branch mutation; native candidate/command contracts preserved |
| S05 Baseline and compatibility | Current game/save/replay/command suites plus fixed-work and timed heuristic reports | Required correctness passes, simple evaluator artifacts and raw baseline measurements exist; no NN needed |

**Exit:** a complete simple-evaluator search can run through the shared
in-process and transported/native harness. Merely defining feature types or
running an evaluator on a static board does not close stage 1.

## 3. Stage 2 — model inference and evaluation framework

This stage covers two complementary meanings of model evaluation:
**running inference correctly**, and **measuring whether it is useful**.
Reuse mature components around a small project-specific contract and harness;
do not build a new general ML platform or let a framework define game semantics.

### 3.1 Reuse assessment and backend selection

Inventory the existing Python model contracts, candidate-free value path,
headless game runner, dataset validation, paired-game protocol and run provenance.
Reuse components only after tests establish current Rust observation/boundary/
outcome semantics and platform support. Historical model/checkpoint compatibility
is unnecessary; incompatible artifacts fail explicitly.

Preferred research setup: PyTorch for offline training/architecture experiments,
with locked versions and deterministic controls recorded where available.
Production inference remains a separate Rust/WASM-compatible choice.
Evaluate a small shortlist against the same real operator/shape requirements:

| Candidate approach | Good fit | Required proof |
| --- | --- | --- |
| Small Rust scalar/SIMD inference implementation | Weighted/MLP/NNUE-like networks with a small allowed operator set and low per-leaf overhead | Export/load and numeric conformance; bounded memory; incremental-state reference tests |
| Portable exported graph, e.g. an admitted ONNX subset | CNN/MLP/graph models where the runtime supports the required shapes/operators | Native CPU and browser/WASM feasibility, operator allowlist, startup/latency/size and dependency/licensing review |
| Existing Rust ML framework/runtime | Models supported without a bespoke operator implementation | Current native/WASM target tests, memory/numeric/budget behavior and actual supported export path |

Choose a backend in an evidence-backed decision record, not from a library's
general WASM claim. ONNX operator coverage/dynamic graph shapes and browser
runtime support are requirements to test, not assumptions. A Python framework
may be the numerical reference without becoming a live server/client dependency.

Keep weights in process/worker memory, reuse immutable buffers and avoid JSON
serialization or a Rust-to-JavaScript callback for every evaluated leaf.
Async inference/batching requires explicit resumable search and cancellation/
deadline integration. GPU kernels and long synchronous runtime calls may not be
preemptible by a cancellation flag. Bound operator/batch/model size, measure
overshoot, and use host job fencing/watchdog behavior where needed rather than
claiming hard instruction-level preemption. CPU scalar inference is the reference; GPU and large
batches are later measured options, not prerequisites.

### 3.2 Artifact, inference and adapter contracts

A model manifest binds model/weights digest, architecture/operator schema,
policy/value feature schemas, boundary/score semantics, rules/content scope,
normalization/vocabularies, precision/quantization, dimensions, parameter
inventory and resource/capability limits. Record initialization/training/data/
export provenance separately from semantic compatibility.

Expose independent `evaluate_policy` and `evaluate_stable_value` methods.
A model may implement either or both; shared trunks are optional. Value forward
must not construct fake candidates or invoke a policy head. Policy output
realigns to the supplied typed candidate IDs. Batch collation respects ragged
candidate/entity sizes and masks; padding is never a legal move or real hex.

Load allowlisted, data-only tensors/graphs with bounded shapes/bytes/operators,
validated digests and declared publisher trust. Client runtime cannot execute
arbitrary Python pickle, downloaded JavaScript, arbitrary WASM or custom model
operators. Trusted offline optimizer checkpoints and public inference artifacts
are different file/authorization classes. Export produces a minimal safe
runtime artifact rather than shipping the research environment.

Golden tests compare extractor output, reference framework, exported scalar
runtime and any optimized backend. Declare tolerance and near-tie action
selection rules before interpreting differences. Quantization is a distinct
artifact/evidence identity; matching tensor shapes does not prove equivalent
decisions, calibration or strength.

Pin one model/feature/score identity to a search job, cache and aggregation.
Replace models only between jobs or by cancelling/restarting the job.
Private accumulators/buffers are branch-local where mutation requires it.
Model loading/failure cannot mutate live state, relax legality or retain a
search tree containing incompatible values.

### 3.3 Experiment and quality framework

Use the admitted immutable datasets from the database plan. A run manifest binds
dataset/split/lineage, extractor/model/search identities, initialization/ancestor,
train/update/evaluation seeds, precision, hyperparameters, budgets, hardware and
source/toolchain. Record attempted/completed/failed/censored counts, resumability
and exact artifact outputs. Do not launch unbounded self-play or training merely
because the framework can run it.

The framework needs:

- Streaming dataset inspection and schema/admission checks, game-level sampling,
  head-specific weights and no future-information/label leakage.
- A bounded train/validate/export loop for a declared model, reproducible seed
  handling, optimizer/parent-weight semantics and atomic run publication.
- Standalone inference tests and benchmarks, including warm/cold process costs,
  actual per-leaf evaluation, batch-size effects and memory.
- Held-out policy ranking and value metrics, plus actual-play evaluation through
  the authoritative Rust engine under controlled agent configuration.
- Paired sides/rosters/seeds and frozen-policy/value ablation controls, with
  complete operational-failure reporting and artifact-bound resumption.
- A model registry recording research candidate, validated, admitted, rejected
  and retracted states. Dataset/model lineage is separate from runtime activation.

Policy metrics may include cross-entropy/KL to an admitted target, top-k agreement
and legal-command alignment. Agreement with played human actions is imitation,
not proof of optimal choices. Current alpha-beta/joint-planning traces do not
automatically define AlphaZero root-visit targets: use an explicit scored-action
distillation recipe, controlled teacher targets, or actual completed MCTS visits
from a separately implemented algorithm. Do not relabel generic node/cell counts
as visits or probabilities.

Value metrics include MAE/MSE/Huber against admitted outcomes, calibration in the
declared outcome/probability convention and stratification by boundary, phase,
map, hero/roster and termination class. Terminal draws require positive rule
support; missing winners, timeouts and round caps are not draws. Never attach the
real game's eventual outcome to arbitrary counterfactual search leaves.
Terminal-supervised value examples come from the actual played trajectory at
admitted stable boundaries; teacher targets need their own provenance.

Separate datasets/targets for policy decisions and stable values. When several
decisions by one viewer precede a stable boundary, record according to a declared
deduplication/weighting recipe so frequent prompts do not accidentally multiply
the same value outcome. Initial setup or a partial commitment is not fabricated
as a completed-transition value sample.

For strength, freeze opponent(s), search, budgets and declared workload; pair
agent-side assignments and use independent held-out start/seed groups.
Report win/loss/rules-defined-draw and censored/failed separately. Compute
uncertainty at the seed/pair/cohort unit, not by treating correlated paired games
or repeated positions as independent samples. Predeclare sample/stopping and
promotion criteria, accounting for multiple architecture/hyperparameter trials;
do not repeatedly inspect the final held-out set until a winner appears.

Keep three comparisons distinct: equal completed work, equal wall-clock budget,
and training/inference resource cost. A larger network can improve held-out loss
while reducing searched work and playing strength. Benchmark actual decisions
and complete-game reliability, not only accelerator examples/second.
Research arena results stay in their own evaluation cohort and do not change
the production human Elo ledger.

### 3.4 Stage 2 acceptance

| Gate | Evaluation | Complete when |
| --- | --- | --- |
| M01 Reuse/backend decision | Existing code assessment and the same representative operators/shapes on shortlisted runtimes | Documented keep/adapt/reject choices, native CPU path and browser portability evidence/limits |
| M02 Inference conformance | Real small trainable NN, separate policy/value forwards, masks, corrupt artifacts and reference/export/runtime comparisons | A trained bounded smoke model exports and evaluates correctly; a stub/random output is insufficient |
| M03 Experiment integrity | Fixed admitted synthetic/current-data cohort, rerun/resume/identity mismatch, split/parent and failure cases | Exact manifests/counts and truthful completion; no held-out leakage or silent resumption under changed identity |
| M04 Quality harness | Held-out metrics, paired real Rust games and uniform/heuristic/value-only/policy-only controls | Reproducible reports distinguish inference correctness, prediction quality, strength and operational failures |
| M05 Deployment boundary | Model load/switch/error/cancel, memory/operator limits and native/WASM feasibility probes | No Python/SQL/training dependency in ordinary gameplay builds; defined numeric and resource contract |

**Exit:** a bounded end-to-end framework works with a genuinely trained small
network and controlled data. That smoke network is engineering evidence, not
automatically a useful/admitted model for client plan E01.

## 4. Stage 3 — explore neural architectures

### 4.1 Controlled shortlist

Start with cheap baselines and compare architectures under the same semantic
inputs, data/splits, training budget, head targets and frozen search.
Architecture-specific representations are explicit adapters with schema IDs.
Value-only, policy-only and two-head models are all legitimate experiments.

| Family | Initial experiment | Main question / risk |
| --- | --- | --- |
| Linear and small MLP | Sparse/global features; candidate scorer and/or stable value | Establish a cheap learnable baseline and the cost of learning versus hand weights |
| NNUE-style sparse network | Active feature indices, small accumulator, clipped/nonlinear layers; initially value-only | Can exact incremental updates cut per-leaf cost on GoA2 state changes without losing useful relationships? |
| Hex convolution | Masked board planes with global/card/entity side inputs | Does local spatial structure improve prediction enough to justify feature/inference cost? |
| Relational graph/message passing | Hex/unit/card/effect nodes and typed allowed edges with pooled context/candidate readout | Can variable maps and long-range relations be modeled efficiently with bounded graph size and supported operators? |
| Hybrid | A compact spatial/relational encoder plus sparse/global head, or separate policy/value models | Is a small mixed design a better quality/latency/memory tradeoff than a shared large trunk? |

Do not begin with an unconstrained transformer/model-size sweep. Add another
family only when a diagnosed limitation and a bounded experiment justify it.
No architecture is selected merely because AlphaZero or Stockfish used it.

NNUE here means an efficiently updatable network design, not chess's exact
feature layout or weights. GoA2 effects, resource changes, upgrades, summons,
phase transitions and information reveals can invalidate many features at once.
Measure sparse update density and full-reset frequency before assuming
incremental evaluation will dominate.

### 4.2 Hex-grid convolution is a representation problem, not an unsolved prerequisite

Use the current engine axial convention, e.g. `(q, r)` with cube
`s = -q-r`. Freeze neighbor order in the schema. One valid direction set is:

```text
D = [(1,0), (1,-1), (0,-1), (-1,0), (-1,1), (0,1)]
y(q,r) = activation(b + W_center*x(q,r)
                     + sum(W_d*x(q+dq,r+dr) for d in D))
```

Embed the finite hex board in a rectangular tensor with a valid-cell mask.
Off-board cells are zero/masked and are distinct from empty playable cells.
Reapply the validity mask after every layer; biases or messages must not
propagate through padded/nonexistent cells and then re-enter the board.
A square 3x3 implementation may use a fixed mask selecting center and six
neighbors, or use six explicit shifts/gathers. Unused square offsets are not
learnable shortcuts. Prevent wraparound at tensor boundaries.

Test the tensor implementation against the engine's exact six-neighbor graph,
including irregular map edges, holes and multiple lanes/zones. Geometry tests
establish coordinates/message support, not that the network decides legal
movement: the rules engine remains authoritative.

Attach global game/phase context and nonlocal card/unit/effect relationships
rather than assuming local board planes contain everything. Masked pooling
must account for valid cells so larger padded rectangles do not bias scale.
A graph encoder may instead use the same hex-neighbor edges plus explicit
typed relationships, with bounded nodes/edges and no hidden-information edges.

Rotation/reflection augmentation is allowed only for transformations proven to
preserve map terrain, objectives, teams, zones, action coordinates and rules.
Do not assume every GoA2 map has sixfold symmetry. Verify inverse transforms
map predicted actions back to the original legal candidate exactly.

### 4.3 Experiment matrix and selection

For each experiment declare a hypothesis, frozen baseline, variable being
changed, feature/architecture identity, training budget/seed replicates and
held-out decision/actual-play evaluation. Report learning curves, parameter and
artifact bytes, activation/accumulator memory, feature cost, scalar/SIMD latency,
completed search work and paired-play uncertainty.

Required ablations include:

- Hand-weighted versus fitted linear/small MLP control.
- Value-only, policy-only and combined learned heads with a fixed opposite head.
- Candidate-ordering prior versus learned continuation policy, held separately.
- Shared trunk versus separate heads/models where one head's cost dominates.
- Full recompute versus incremental NNUE path with identical weights/features.
- Geometry/relationship features versus corresponding removed blocks.
- Reference precision versus proposed quantization, including near ties.
- Frozen search/equal-work versus equal-time comparisons.

Maintain the existing search as one baseline. Explore PUCT/ISMCTS or a different
simultaneous solver only with its own algorithm ID and contract tests:
information-set consistency, hidden-world resampling, opponent/environment policy,
team perspective, joint choices and budget accounting must be explicit.
Changing algorithm, model and training targets together cannot isolate an
architecture effect. Where MCTS is introduced, define actual completed root
visit counts and control temperature/noise/virtual loss separately.

Sampled worlds share the declared job budget. An incomplete first world is not
a completed all-world root score; a depth/iteration aggregate must state its
coverage. Learned continuation calls remain information-safe and use
domain-separated RNG streams from root/tree sampling, world construction and
live gameplay. A counterfactual revelation changes only that hypothetical branch,
not what the real root agent is permitted to remember.

Select a Pareto tradeoff over correctness, prediction/strength, latency, memory,
download size and portability. It is acceptable to choose a small NNUE/value
model or retain the simple evaluator when larger models do not earn their
cost. Record failed/negative experiments without silently discarding their
operational-failure rate.

### 4.4 Stage 3 acceptance

| Gate | Evaluation | Complete when |
| --- | --- | --- |
| A01 Architecture coverage | Bounded prototypes of linear/MLP, NNUE-style, hex CNN and graph, or evidence-backed technical rejection of a family | Actual comparable reports and explicit rejected scope; implementation-free speculation is insufficient |
| A02 Geometry/features | Hex-neighbor reference, repeated masks, pooling, irregular maps, candidate transform and information-safe relation tests | Exact geometry/action mapping and no off-board/hidden-data shortcuts |
| A03 Scientific comparison | Declared dataset/budgets/replicates, ablations, same-search and equal-time/work comparisons | Reported uncertainty, resource costs and limitations; no held-out selection contamination |
| A04 Selection record | Pareto comparison plus chosen initial architecture/runtime/feature/score scope | Reviewable selected design and non-selected alternatives with measured reasons |
| A05 Search validity | Current reference tests plus any separately changed algorithm/joint/hidden-world semantics | Rules/privacy/current-game correctness remains intact; algorithm change is not disguised as a model gain |

**Exit:** an evidence-backed initial design and an implementation recipe.
Large-scale training or adoption of every prototype is not required.

## 5. Stage 4 — incremental implementation and scaling

Implement the selected design as a sequence of bounded, independently verified
rungs. Each rung keeps the preceding evaluator/search/data identity available
as a reproducible reference; obsolete artifact compatibility is not required.
Scale only the dimension being evaluated.

| Rung | Implementation scope | Evidence to advance |
| --- | --- | --- |
| L0 Contract baseline | Stage-1 parametric search, golden features/boundaries and single-worker deterministic harness | S01–S05; actual call/work traces and current-game preservation |
| L1 Small end-to-end model | Bounded admitted dataset, one selected small model, one trainer, CPU reference export/runtime and a complete controlled run | M01–M05; actual played trajectory -> dataset -> train -> export -> Rust search works without manual relabelling |
| L2 Admitted native NN candidate | Held-out current-game evaluation, calibration/strength/reliability report, safe artifact loader and bounded native inference | Model-admission checklist in section 9; suitable model can now enter client-plan stage 3 |
| L3 Measured state/inference optimization | Profile-driven representation improvements, buffer reuse, optional sparse deltas/NNUE accumulators and scalar/SIMD kernels | Exact reference equivalence, before/after actual-search cost and budget/cancellation behavior |
| L4 Broader data/model scope | More maps/heroes/rosters, curated hard cases and larger datasets/network sizes one dimension at a time | Declared supported scope, split-safe comparable evaluations and capacity/cost limits |
| L5 Throughput scaling | Streaming shards, generation/inference batching, optional accelerators and then bounded multi-actor generation/training workers | No data duplication/omission, valid backpressure/leases, measured cost and no starvation of live service |
| L6 Product rollout and iteration | Native client integration, browser scalar/SIMD validation where supported, opt-in/shadow/canary and controlled replacement | Client-plan E gates, monitored model identity/resources/failures and tested rollback to a compatible admitted evaluator |

L0–L2 deliver a real useful-model candidate and may unblock native NN client
integration without waiting for large networks, GPU serving or distributed
self-play. L3–L6 are staged engineering/product follow-through, not a reason
to withhold a validated small native model. Browser deployment still needs the
client plan's actual browser acceptance; a native artifact is not a browser
release.

Each run manifest sets hard ceilings for games/positions/rows, optimizer updates,
wall time, CPU/GPU time, memory/disk, workers and artifact sizes, plus stopping
and failure criteria. Start with a tiny synthetic fit/smoke and a bounded
single-worker real-game pilot before expanding cohorts or launching a sweep.
Choose concrete numeric ceilings from measured L1 costs and available hardware
before running each experiment; this document does not invent a cloud budget
or automatically start paid/unbounded work.

A training/generation loop needs an explicit stop after a declared iteration
count/resource budget. Fresh weight initialization, parent-weight initialization,
optimizer resume and full-run resume are different operations with separately
validated provenance. Resuming under a changed data/schema/model/search identity
must fail or create a new run, not silently continue the old one.

### 5.1 State and feature optimization

Use profiling to decide whether the bottleneck is engine transition/clone,
feature extraction, network inference, candidate enumeration or orchestration.
Optimize the measured component and preserve a full-recompute/clone reference.

Possible measured changes include typed contiguous/structure-of-arrays fields,
immutable interned public definitions, bounded scratch buffers, dirty feature
sets, incremental accumulators and make/unmake logs. They are internal
representations, not new persistence/wire formats. Do not add every optimization
up front or use this plan to reopen an unverified interpreter rewrite.

If using make/unmake or feature deltas, record every affected component,
including global/team/phase knowledge, effects and physical-card aliases.
On apply/undo/clone/restore, incremental features and accumulators must match
full recomputation. A move's visible displacement alone is not an adequate
invalidation set. Copy-on-write/shared immutable weights cannot imply shared
mutable accumulators between seats, branches or sampled worlds.

Cache keys include model/schema/content identity, boundary kind, fixed viewer/
perspective, relevant knowledge and search/continuation semantics. Never persist
private process keys as public game identities. Quantization, model replacement,
changed visibility or changed features invalidates incompatible cached values.

#### Deferred cards and upgrades

Treat an undetermined node as a constrained family of concrete witness plans.
Retain each witness's cards, upgrade history and joint binding constraints;
independent lists must not invent incompatible card/upgrade combinations.

Encode identical shared spatial features once, then use conditional card/loadout
features and nonlinear heads for each relevant binding. N01 declares these
feature dependencies; N06 evaluates the factorization. A positional head can
prioritize route families. Backed-up values retain the declared stable-boundary
and binding semantics. NNUE-style partial accumulators may reuse contributions
where exact feature dependencies permit it.

Refine when legality, effects, available continuations or evaluator inputs differ.
Equal NN embeddings/scores do not prove rule equivalence. Preserve existing
grouping guards until replacement is validated; inserting a full conditional NN
score into a shared grouping key may destroy useful sharing. Learned priorities
are not admissible pruning bounds without proof.

Deferred computation must preserve commitment timing. A choice made before
hidden information is revealed stays consistent across indistinguishable worlds;
do not select the best upgrade separately in each sampled world. Preserve a
legal concrete incumbent, and resolve the selected binding into actual issued
commands before play.

L3/I02 tests compare bounded grouped versus fully concrete enumeration, including
identical positions with different hands/bonuses, incompatible bindings,
information-set consistency and final command legality. Count duplicate witnesses
correctly in policy/target statistics. Implement shared encoding over existing
witnesses first; justify further symbolic execution with measured benefit.

### 5.2 Batching and distributed generation

Add batching only after measuring full decision latency, queue wait, cancellation
and padding/copy overhead. Bound batch wait by job deadlines and avoid
head-of-line blocking urgent human-game decisions behind offline throughput.
The normal native/browser agent must retain an efficient serial CPU path.

Generation actors and trainers operate in the database plan's isolated
self-play/evaluation namespaces. Pin policy/value model IDs per game/run and
record every controller switch. Actors cannot silently mix a new policy with
old target identities or count retries as new independent games.

Use fenced work claims, unique case/game/admission IDs, atomic complete-game
publication and idempotent dataset ingestion. A retried actor may regenerate a
case, but only one validated source instance contributes under the declared
recipe. Replay buffers retain lineage/split/age and weighting metadata;
eviction cannot turn held-out groups into TRAIN. Backpressure bounds unpublished
spools and shard queues; incomplete games remain failed/censored, not training
wins or successful throughput.

Parameter-server/async policy iteration, stale actor weights, replay mixing and
parallel optimizer updates are later explicit algorithms with recorded
staleness/consistency and learning-rate semantics. They are not a transparent
increase to a worker-count setting. Compare them against the bounded
single-worker pipeline before adoption.

### 5.3 Stage 4 acceptance

| Gate | Evaluation | Complete when |
| --- | --- | --- |
| I01 Vertical slice | A complete bounded current-game generation/admission/train/export/Rust-inference/search/evaluation run | Immutable linked manifests, real trained weights and truthful completed/failed/censored counts |
| I02 Optimization | Baseline versus each representation/kernel/delta change, randomized apply/undo/clone/restore and full references | Same required features/scores/commands/work or explicit numeric contract; demonstrated actual-search benefit |
| I03 Scaling | Declared rung/resource ceilings, worker retry/crash, backpressure, batch deadline/cancel and data split tests | No duplicate/omitted admitted cases or unbounded work; measured cost/resource tradeoff |
| I04 Admission and rollback | Real candidate checklist, prior compatible evaluator, failed load/update and current-game continuation | Safe identity-bound promotion/retraction with visible fallback; no game/schema/rules change |
| I05 Product handoff | Native CPU contract plus export/runtime/browser capability report and client integration references | Client stage-3 dependencies are real; its actual native/browser product gates remain distinct |
| I06 Reproducible evidence | Clean build/current required suites, exact run/artifact/dataset hashes, metrics and decision records per completed rung | No hidden skipped/missing cases, changed workload or model identity presented as equivalent evidence |

## 6. Data, supervision and information boundaries

Use the database plan's authoritative accepted-game/result records and immutable
dataset manifests. Keep environment, purpose, lineage/leakage group, completion,
consent/usage and split admission distinct. Production games, self-play, CI,
manual analysis and arena evaluation do not share an automatic TRAIN stream.

Policy training examples bind observation, actual legal candidates, selected
action and the declared target source at that exact decision. Stable value
examples bind the actual viewer/team, typed boundary/anchor and admitted game
outcome or explicitly identified teacher target. Head-specific weighting is
declared at game/sample level so an implementation's minibatch packing cannot
quietly redefine loss weights.

Do not assume client-reported visits, values, policy logits or model names are
verified because its move was accepted. Controlled runners produce trustworthy
search targets; server-validated human actions may support a separate imitation
recipe. Full private archives are source material for a trusted builder, not
inference features for the trainer.

Construct inputs using information available then, including accumulated public
knowledge. Final revealed hands, post-game ratings, future actions and outcome
labels cannot enter policy/value inference features. An actual terminal state
is handled by rules; training nonterminal samples use the declared boundary
before the terminal outcome was known.

Sampled-world state is hypothetical. Project it through the fixed viewer's
allowed knowledge at that hypothetical point and distinguish it from actual
trajectory supervision. Counterfactual results may support a declared teacher/
bootstrap recipe, not a false claim that the actual game visited that state.
Document strategy-fusion/opponent-policy approximations independently of the
guarantee that no actual server secret enters observation-only search.

Split at stable lineage/start/seed-family groups, not rows or arbitrary exported
game IDs. Freeze validation/test/promotion groups and account for parent-model
data exposure. Report inherited-data limitations honestly. Model/dataset
correction or usage retraction propagates through the database registry;
a new artifact digest does not erase prior exposure or affected ancestors.

## 7. Determinism, runtime safety and current-game regression

The deterministic scalar/serial path is the reference. Separate RNG streams
for game setup, search world sampling, policy sampling/exploration, parameter
initialization, data order and evaluation cohorts. Record exact recipe/version
and seed, not only a single ambiguous global seed.

Floating-point GPU reductions and some parallel libraries may be nondeterministic.
Record backend/determinism mode and measured tolerance/distribution. Golden
feature/geometry/candidate identity remains exact where its types permit it.
A noisy result does not justify weakening rules/replay identity tests.

All inference, feature extraction, world construction, forced continuation and
search work participates in step/node/depth/world/memory/deadline/cancellation
limits. Count completed transitions/world aggregates and neural calls separately
from attempted nodes. A faster search that shortened the horizon, used fewer
worlds, pruned extra legal candidates or counted cancelled work differently is
a changed workload, not a like-for-like optimization.

Budget exhaustion uses a valid completed result or an explicit legal fallback.
It does not score a malformed partial engine frame, label an interrupted rollout
terminal or extend a live clock while waiting for a model. WASM resumable slices
preserve state/RNG/work accounting and yield for messages; native scheduling
keeps cancellation/network handling responsive.

Run the completed cleanup's required quick, service, differential and restore
tiers for affected boundaries, plus neural hook/feature/inference tests.
All selected fixture families have nonzero executed counts and no unexplained
missing cases. Use the current-content hero/map/mode/roster/prompt cohort and
declare any narrower model scope explicitly; a narrow research model cannot
masquerade as universal current-game support.

Test observation privacy, legal candidate mapping, simultaneous choices,
rejected-input nonmutation, physical-card aliases, ordered events, save/replay,
clock/receipt behavior, clone/worker isolation, model errors and current client
contracts. Training/tests use isolated namespaces and synthetic/admitted data,
not private live matches copied into public fixtures.

## 8. Implementation packages and decisions

Use the [dependency slices](PLANS_INDEX.md#dependency-slices): research smoke
runs need the data/admission contracts and a conforming bounded local backend,
not a production PostgreSQL rollout. The Rust game/feature/boundary contract is
shared; neural tensor adapters, head targets and experiment runners live here.

All packages are open. Owners are roles to assign.

| ID | Stage / owner | Deliverable | Depends on | Completion evidence |
| --- | --- | --- | --- | --- |
| N01 | 1 / engine-search | Adopt shared identity/features/extractor; specify neural tensor/head schema and cross-adapter golden vectors | Foundation F01/F02/F06 | S01/S04 against the same reference extractor |
| N02 | 1 / engine-search | Adopt shared detector/ports and parametric artifacts; extend neural call/head/target coverage through the same core hooks | N01, foundation F06 | S02–S05 and shared-detector conformance; no second implementation |
| N03 | 2 / ML-runtime | Reuse assessment, backend/operator/export decision, CPU inference and artifact loader | N01–N02 | M01/M02/M05 with actual bounded trained smoke network |
| N04 | 2 / ML-data-evaluation | Run manifests, train/validate/export harness, metrics, paired controls and model registry | N02–N03, database admission contracts | M03/M04 and reproducible bounded runs |
| N05 | 3 / ML-search | Small MLP, NNUE, masked hex-CNN and graph experiments/ablations | Accepted stages 1–2 | A01–A03 with measured rejection/acceptance of each family |
| N06 | 3 / engine-ML | Initial architecture/precision/runtime selection and separately identified search experiments | N05 | A04/A05 and explicit supported scope/tradeoffs |
| N07 | 4 / ML-engine | One bounded complete implementation, current-data trained native candidate and admission report | N03–N06, actual required dataset/pipeline components | L1/L2, I01/I04 and section-9 checklist |
| N08 | 4 / engine-runtime | Profiled state/feature/incremental/SIMD work and witness-preserving deferred card/upgrade sharing | N07 and frozen reference | L3/I02, concrete-enumeration/commit-timing equivalence and measured benefit |
| N09 | 4 / ML-data-operations | Broader scope, bounded generation/training throughput and worker/batch implementation as justified | N07, database storage/isolation | L4/L5/I03/I06 and declared resource ceilings |
| N10 | 4 / client-release | Native/browser NN handoff, model registry rollout/retraction and ongoing evidence | N07 plus relevant N08/N09, client stages 1/2 | L6/I04/I05 and client-plan E gates for each advertised host |

Model preparation, a trained research prototype, core NN readiness and a
browser-ready neural bot are different milestones. Do not mark the latter
complete by implementing the first trait or making a GPU produce logits.

### Required decision records

Before closing the corresponding stage, record:

- Current feature/scope/boundary and expected-outcome semantics.
- Backend reuse/selection and the safe export/numeric/operator contract.
- Target provenance, split/ancestor exposure and run resource/stopping budgets.
- Fixed search baseline and separately changed algorithms or continuation policy.
- Architecture/precision selection and quantified resource/quality tradeoff.
- Model admission thresholds and rollout/retraction policy.

Choose concrete thresholds before inspecting promotion results. They may be
noninferiority plus a meaningful resource improvement, or a declared strength
improvement within latency/memory constraints. Record sample size/uncertainty,
operational failure ceiling and tested hardware rather than inventing Elo,
simulations/sec or universal browser performance promises.

## 9. What unblocks client-side neural evaluation

Client plan stage 3 depends on a **full working NN implementation**, not just
this document or its stage-1 interfaces. The minimal readiness milestone is an
accepted L2/native candidate with:

- [ ] A real trained GoA2 model within an explicit current rules/content and
      hero/map/mode scope, produced by a working bounded end-to-end pipeline.
- [ ] Immutable dataset/split/feature/boundary/model/export/runtime identities,
      usage/provenance records and actual training/evaluation outputs.
- [ ] Separate policy/value capabilities as advertised; candidate-free stable
      value, legal policy alignment and exact rules-terminal handling.
- [ ] A safe data-only artifact, compatible CPU runtime, bounded loading and
      golden reference outputs on representative current-game decisions/boundaries.
- [ ] Held-out prediction, matched real-game quality/reliability and latency/
      memory evidence meeting the predeclared admission criteria.
- [ ] Native scalar conformance, documented numeric/quantization/tie semantics
      and branch/accumulator correctness if incremental inference is used.
- [ ] A portability/export report stating which operators and feature layouts
      can run in WASM, known blockers and the browser capability work still needed.
- [ ] Explicit registry admission/retraction and a compatible evaluator rollback;
      no silent replacement of live jobs, rules or current game state.

Large distributed training and every L3–L6 optimization are not prerequisites
for this small full-model milestone. Native client integration can then begin
its E02/E05 work. Browser integration additionally needs a viable WASM runtime
and its own E03–E06 evidence; neural-core readiness does not claim those product
tests have already passed. This avoids a circular requirement that a completed
client release must exist before neural implementation is considered ready
for client integration.

An artifact with random weights or a tiny synthetic fit proves infrastructure,
not this checklist. If the model fails admission, keep heuristic clients working
and record the research result rather than disguising fallback as neural success.

## 10. Completion reporting and handoffs

Each completed package/stage records commands, source/toolchain/hardware,
current game/content identity, dataset/model hashes, expected/executed/missing/
failed/censored counts, raw feature/inference/performance metrics and decisions.
Tests at the wrong timepoint or comparisons under changed budgets do not close
the stated gate. Writing/committing the plan itself does not claim implementation.

All five plans and their [ownership/dependency index](PLANS_INDEX.md) are written.
Follow the [rule update](RULE_ENGINE_UPDATE_PROCEDURE.md) and
[later NN update](NN_RULE_CHANGE_UPDATE_PROCEDURE.md) procedures for changes.
They distinguish revalidated weights, fresh training and current/earlier-model
warm starts. The [audit plan](RULE_VERSION_AUDIT_IMPLEMENTATION_PLAN.md) separates
original-label equivalence from new target-derived trajectories. None of these
documents is evidence that training, retraining or deployment has occurred.

Prior art and reviewed implementation:
[current Rust search](../rust/crates/goa2-core/src/interpreter/search.rs),
[current native bot](../rust/crates/goa2-core/src/interpreter/live_game/bot.rs),
[earlier stable-transition learning contract](AI_LEARNING_CONTRACT.md),
[earlier policy/value architecture](AI_SEARCH_AND_LEARNED_RUNTIME.md),
[inference contracts](../src/automata/models/contracts/inference.py) and
[paired-evaluation protocol](../src/automata/evaluation/protocol.py).
Those references identify reusable ideas/source, not inherited model formats,
historical compatibility promises or current neural implementation evidence.
