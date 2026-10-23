# Rust engine for iterative game search

Status: implementation plan, not an implemented or benchmarked Rust engine.
Date: 2026-10-23.

## 1. Objective and decisions

Build a native Rust rules/simulation engine optimized for repeated legality
queries and game-tree search. Use the legality-optimized Python backend as the
rules oracle. Keep Python as the authoritative live game server and use Rust to
propose actions, with Python validating the final submission.

The starting search will be iterative-deepening, team-aware minimax with
alpha-beta pruning. Add an explicit simultaneous-planning layer and sample
hidden worlds from the acting player's available information. Separate the
rules engine from the search algorithm so minimax, fixed-policy rollouts and a
future information-set search can use the same verified simulator.

The long-term throughput goal remains **1,000 complete two-game-turn simulations
per second** on the user's machine. Alpha-beta nodes/second, legality queries,
and completed simulations are different measurements; none substitutes for the
others. A language change alone is not evidence that this target is achieved.

Initial constraints:

- Preserve actual game rules, including intermediate decisions, reactions,
  mandatory failures, phase changes, hidden information and card/token lifecycle.
- Keep the existing two-full-game-turn horizon and current scoring as the
  reference configuration. Count turns from before the selected root action;
  include turn-ending root actions and round rollover.
- Use no neural network, model runtime or training dependency.
- Keep all inner-loop rules, move generation and evaluation in Rust. Python calls
  per node or per simulated decision would undermine the design.
- Do not restart the lab, change its selected bot or replace the live backend
  while implementing or benchmarking. Integration is an explicit later step.
- Initially add the workspace inside `goa2-backend-bots`; do not create another
  fork or modify the regular backend checkout as part of this plan.

## 2. Reference implementation and baseline

The following checked-out code is the oracle boundary, rather than the older
codebase map, which is marked stale:

| Concern | Reference |
| --- | --- |
| Actual state transitions | `src/goa2/engine/session.py`, `handler.py`, `phases.py`, `steps/` |
| Rules, effects and statistics | `rules.py`, `validation_*.py`, `filters_*.py`, `stats.py`, `topology.py`, `src/goa2/scripts/*_effects.py` |
| Candidate and sequence legality | `src/automata/runtime/legality.py`, `docs/LEGALITY_CHECKS.md` |
| Batched movement validation | `validate_movement_paths` in `src/goa2/engine/rules.py` |
| Hidden-world construction | `src/automata/runtime/determinize.py`, `src/goa2/domain/card_knowledge.py` |
| Search state and decisions | `src/automata/decision.py`, `src/automata/search/contracts.py` |
| Existing search and horizon | `src/automata/search/ismcts/engine.py`, `config.py` |
| Material/value baseline | `src/automata/search/heuristic.py` and the lab's `src/bot_evaluation.py` |
| Saved rules semantics | `src/goa2/domain/rules_version.py` |

`601df58` (`Search optimization`) is the earlier search checkpoint. The
legality-optimized oracle baseline is commit `00becf4a68615afa410f8eabe0c841809af0427b`
(`Optimize batch and sequence legality checks`). Before creating oracle fixtures,
record that exact revision, dirty-tree digest if applicable, rules version,
data export hash and fixture schema version. **Do not pin only `601df58` and
call it the legality oracle.**

Measurements already obtained, with their scopes kept distinct:

| Workload | Recorded result |
| --- | ---: |
| Python: 1,000 full iterations, two-turn recorded search position | 57.35 s, about 17.4 iterations/s, before the latest legality changes |
| Python: later 200-iteration search check | 6.96 s; same root visits, scores and choice as the earlier reference |
| 572 range-4 movement destination checks | 395.08 ms independently; 1.07 ms batched |
| 64 distinct sequences sharing five decisions; one valid tail, 63 invalid tails | 148.74 ms independently; 2.82 ms batched |

The latest Python validation was 3,555 engine/domain and affected search tests,
followed by all 29 focused legality tests after error-classification tightening.
Rerun against the frozen oracle revision; these counts are historical evidence,
not a substitute for Rust parity tests.

## 3. Workspace and ownership boundaries

Proposed layout:

```text
rust/
  Cargo.toml                     # workspace, locked dependencies
  crates/
    goa2-core/src/
      ids.rs                     # compact checked identifiers
      definitions.rs             # immutable maps/cards/hero definitions
      state.rs                   # mutable instance records
      decision.rs                # typed decisions and canonical action keys
      geometry.rs                # adjacency, distances, bitsets, traversal
      rules/                     # validation, topology, stats, effect lifecycle
      steps/                     # stack execution and continuation frames
      effects/                   # explicit hero/card implementations
      undo.rs                    # reversible mutation journal
      hashing.rs                 # incremental search fingerprints
      codec.rs                   # versioned import/export boundary
      capabilities.rs            # supported state/rule closure
    goa2-search/src/
      alphabeta.rs
      planning.rs                # simultaneous commitments
      beliefs.rs                 # world sampling/information boundaries
      ordering.rs
      evaluation.rs
      transpositions.rs
      budget.rs
      parallel.rs
    goa2-search-cli/src/main.rs   # persistent worker and diagnostic commands
  tests/fixtures/                # minimized, versioned oracle cases
  benches/                      # fixed corpus and reproducible benchmark runner
scripts/rust_oracle/              # Python export, replay, comparison and shrinking
src/automata/runtime/rust_search.py # later optional Python adapter
```

Keep `goa2-core` independent of Python, HTTP, frontend state and search policy.
Keep the evaluator and search strategy independent of transition implementation.
Use safe Rust first. Unsafe code, custom allocators, SIMD and compressed wire
formats require measured justification and their own correctness tests.

## 4. Native state and transition representation

### Immutable definitions

Export static definitions once per content/rules revision. Rust owns compact
read-only tables for cards, heroes, maps, zones, lanes and compiled effect
instructions. Runtime references are checked integer IDs, not strings.

A printed card definition is distinct from a physical card instance. Two cards
with the same printed ID can be independent objects; collections refer to
instance IDs. Preserve aliases among hand, deck, played slots, pending commits,
spellbook and effect bindings. Scenario overrides produce distinct definitions;
do not silently replace them with the registry's standard card.

Export roster-dependent token supply through the Python engine. Include every
upgrade/spell token type a roster can acquire, preserve required entity-numbering
semantics, and retain deployed scenario entities. Do not port only the current
board's tokens or restore all heroes' unused supply.

### Board and geometry

- Dense `HexId` values with cube-coordinate conversion only at import/export.
- Precompute adjacency, coordinate distances, zone membership, lane order,
  spawn metadata and useful geometric range masks.
- Use variable-length word bitsets. The recorded map has **572 tiles**, so a
  single 64-bit or fixed 512-bit board is insufficient.
- Keep occupancy, terrain and effect-dependent legality distinct. Derived masks
  are invalidated by every relevant mutation.
- Preserve off-map behavior, occupied-destination versus obstacle-crossing
  flags, passable tokens/mines, actor-relative barriers, dynamic empty obstacles,
  movement origin restrictions and directional topology isolation.
- Never replace topology with an undirected connected-components shortcut:
  ZERO-region interactions, movement-origin visibility and actor-specific
  immunity can change what a traversal permits.
- Support arbitrary registered map sizes and multiple lanes; reject capacities
  explicitly rather than truncating an export.

### Mutable state

Use compact arrays/arenas for hero instances, pieces, minions, card instances,
tokens, modifiers, active effects, pending planning and phase/turn counters.
Use typed context registers and scoped action frames instead of string-keyed
lookups in the hot path. Every register must have an explicit import/export and
undo mapping; arbitrary Python context fields cannot simply disappear.

Carry multi-piece ownership separately from board pieces. Decisions may belong
to a hero other than the current actor: reactions, team prompts, controlled
cards and nested actions must preserve both identities.

Keep the Python stack semantics. Implement small Rust step/continuation enums
for automatic execution until a player decision, terminal state or requested
search boundary. Port effect timing and scope, not only the final card outcome.
Static exports can describe data, but Python effect functions require explicit
Rust implementations or a verified instruction compiler; they cannot be
translated by merely copying their names.

### Make/unmake

Start with compact state copies as a correctness baseline, then introduce a
reversible mutation journal:

1. Mark journal and temporary-arena lengths before an action.
2. Record old values for all changed fields, collection order/length, occupancy,
   phase/context frames, effects and rule RNG state.
3. Execute automatic rules and expose the next decision.
4. Undo to the exact mark on backtracking or cancellation.

Log complete small records first; use finer deltas only after profiling.
Do not clone an entire world at every child. Do not share mutable records across
workers. On every differential fixture, undo must restore full semantic state,
legal choices and a recomputed fingerprint exactly, including after exceptions
and mandatory aborts.

## 5. Legality and sequence interfaces

Proposed core interface, with concrete types finalized in milestone 1:

```rust
fn legal_actions(state: &State, out: &mut Vec<Action>) -> Result<Decision, Unsupported>;
fn check_actions(state: &State, candidates: &[Action], out: &mut Vec<Verdict>);
fn apply_until_decision(state: &mut State, action: Action, undo: &mut UndoLog)
    -> Result<Transition, TransitionError>;
fn unmake(state: &mut State, mark: UndoMark);
fn check_sequences(state: &mut State, sequences: &[Sequence], out: &mut Vec<SequenceVerdict>);
```

Generate one prepared legality context for an unchanged decision: actor, source
card, effective stats, geometry, permissions and reachable/target masks. Batch
queries use bitset membership and typed comparisons. Return stable canonical
ordering where Python's ordering affects deterministic ties or continuation.

Sequence checks use a prefix trie plus make/unmake. Execute shared prefixes
once, fork worker-local state only when parallelizing, prune descendants after
the first illegal decision, and preserve the first failing index. A terminal
input need not execute its consequences if the current engine decision proves
it legal. If a caller requests its resulting state, it must actually execute.

Retain a three-way result for oracle checks: legal, illegal, unsupported/unknown.
Python `legal=None` is not a positive or negative oracle label. Propagate internal
engine errors distinctly. Include structured rune/upgrade batches, optional
skip/done, second-card planning, correct decision ownership and stale request IDs.
Runtime request IDs are checked at the Python boundary; native internal decisions
use stable typed identities instead of minting UUIDs at every search node.

## 6. Search design

### Resolution: team-aware alpha-beta

Use fixed root-perspective values and explicit MAX/MIN nodes. This is safer than
negating the score after every input: the same team can make several consecutive
decisions, while defense and control effects can transfer a decision mid-action.

Classify each boundary by its actual decision owner:

- MAX: an action controlled by the root's coordinated team model.
- MIN: an adversarial opponent decision.
- Fixed policy: an explicitly configured ally/opponent controller.
- Chance: genuine rule randomness with known probabilities.
- Automatic: no player decision; execute the rules directly.

Make the team/controller model explicit. The current Python ISMCTS folds other
heroes into an environment policy; adversarial minimax intentionally changes
that behavior. Rules parity does not require the new bot to choose the same
move as ISMCTS. Preserve a fixed-policy compatibility mode for benchmarks.

Iterate over **completed game-turn horizons**, initially one then the full
reference two turns, before attempting deeper levels. Intermediate card/action
choices do not consume a full turn. Forced choices do not manufacture depth.
At cutoff use the precise current reference boundary semantics; do not replace
it with a superficially similar planning or action boundary. Settle necessary
rules and reactions exactly as the oracle requires.

Order all legal actions without excluding them: prior principal variation,
transposition move, immediate wins/kills, tactical responses, then the current
heuristic and stable tie order. Alpha-beta pruning is allowed; approximate
beam limits and selective pruning are separate experiments, not exact minimax.
After the baseline matches exhaustive tiny trees, consider principal-variation
search and aspiration windows with correct re-search on bound failure.

### Hidden information and simultaneous planning

Ordinary minimax assumes a fully specified sequential game. GoA2 also has hidden
loadouts, facedown commitments and simultaneous choices. Treat this as an
explicit search problem rather than pretending server-visible private cards are
available to the bot.

1. Build the root observation from public facts and the acting hero's permitted
   private information. Preserve teammate secrecy where Python does.
2. Sample consistent worlds from card/loadout provenance, public reveals,
   upgrades and known commitments. Use the same world sample set across root
   alternatives to reduce comparison noise.
3. In the first resolution-only search, run alpha-beta inside each sampled world
   and aggregate comparable root-action values. This is a heuristic determinized
   search, **not an exact solution of the imperfect-information game**.
4. Group unrevealed planning commitments into a simultaneous epoch. Construct
   candidate team/joint commitments from the same pre-reveal observations;
   neither side gets to inspect the other's chosen hidden commitment.
5. Begin with a documented pure maximin security calculation on small joint-action
   matrices. Test that opponent choices cannot condition on an observed private
   commit. Add a mixed-strategy finite-matrix solver when mixed policies are
   required; verify it against exhaustive small games before larger planning.
6. Use iterative action-set expansion only as an explicit budgeted approximation
   for large matrices. Record coverage and unfinished work; do not report an
   incomplete matrix as an exact minimax value.
7. Track strategy fusion: independently optimal continuation in each sampled
   world can assume knowledge the actual player lacks. Share policies across
   indistinguishable information sets where supported, compare against the
   existing ISMCTS baseline, and retain an information-set search option if the
   determinized minimax policy plays worse despite greater speed.

A minimum over adversarial actions is different from an expectation over belief
samples. Define any risk-sensitive aggregation separately. Rule chance,
search-world sampling and heuristic tie-breaking use separate RNG streams.
Planning support is a required milestone; resolution-only speed is not completion.

### Iteration, deadlines and fallback

Before deep search, establish one Python-validated legal root action and an
initial native heuristic ranking. Check a monotonic deadline and cancellation
flag at bounded node/step intervals, including inside long automatic sequences.
Reserve time for transport, final Python validation and submission.

Publish progress only after a coherent unit completes: a root alternative across
the required world samples, or a whole search depth. Preserve the best fully
completed result. A partially evaluated branch must not overwrite it simply
because its provisional score is high. Return completed depth, sample coverage,
principal variation, node counts, deadline status and fallback reason.

On unsupported rules, incompatible content, engine mismatch or worker failure,
fall back for the **whole root decision** to the existing validated Python path.
Do not call Python deep inside a Rust branch or assign a favorable value to an
unsupported child. If a deeper branch becomes unsupported, discard that native
search result. Partial coverage is a development stage, not a claim of full support.

## 7. Evaluation, hashing and reuse

Port the existing evaluator first with a versioned recipe ID and differential
numeric tests. Retain the lab's current ordering: real hero/minion kills outrank
positioning; minion range with a remaining gold/red card outranks hero range,
which outranks minion range without those cards. Preserve the friendly defense
minion and enemy ranged exposure adjustments. Initially use simple printed
attack range exactly as the current lab evaluator does. Do not retune weights
while measuring the engine port.

Use incremental state fingerprints, initially 128-bit Zobrist-style keys plus
verification signatures. Recompute an independent canonical checksum in tests.
Hash every rule-relevant mutable fact: physical card identity/lifecycle, hidden
sampled commitments, effects and expiry, piece ownership, positions, occupancy,
terrain, stack/continuation frames, context registers, turn/phase, flags and any
rule RNG state. Qualify keys by immutable definition/content identity and rules
version. Printed token/card data belongs in shared definitions, not every key.

Transposition entries carry remaining game-turn horizon, node/controller model,
root perspective, evaluator version, exact/lower/upper bound, best action and
completion status. Separate concrete-world state reuse from information-set
policy statistics. Never combine values from incompatible world assumptions,
belief scopes or evaluator recipes. Do not treat one world's decision as known
optimal across all worlds.

Path-dependent guards, repetition history and remaining step budgets must be
represented or verified before reuse. Bound cache memory; never store interrupted
searches as exact values. A cache hit cannot bypass legality, expiry, advancement
limits or deadlines, and cannot invent completed-iteration counts.

## 8. Python oracle and differential validation

### Oracle adapter

Add an offline adapter over the frozen Python engine that can:

- Export definitions, a typed state, decision ownership and legal actions.
- Check batches with `check_moves`, `check_sequences` and
  `validate_movement_paths`, preserving unknown verdicts.
- Apply a submitted decision and export the next decision, full semantic state,
  phase/turn counters, ordered semantic events and player-visible projections.
- Replay complete traces and minimize a Rust/Python disagreement.

**Legality parity alone is insufficient:** the existing batch helpers return
verdicts, not a proof that Rust computes the same post-action state. The adapter
must expose transition snapshots at every decision and relevant automatic
boundary. Also compare optimized Python movement against its retained general
BFS so a shared shortcut bug is not silently blessed.

Transport normalization is narrow and typed: map ephemeral request IDs and
Python step identity labels to stable trace labels. Preserve meaningful entity,
card-instance and effect IDs. Do not strip every `id`, erase hidden lifecycle
fields, sort order-sensitive collections, or compare only public views.

### Corpus and test gates

Seed fixtures from the library's decision states and saved reproductions, then
add generated games and explicit boundary cases. Include:

- All supported maps, lane configurations, roster sizes and hero combinations.
- Every hero, upgrade tier, spell, ultimate and effect family; roster token supply.
- Planning, two-card commits/done, reveal, initiative ties/inversion, reactions,
  defeat, respawn, minion battles, lane pushes, upgrades and terminal outcomes.
- Mid-card import, nested actions, changed acting/deciding hero, controlled actions,
  mandatory abort/restore, multi-piece heroes and passable token/mine behavior.
- Barriers, topology splits/isolation, dynamic obstacles, ongoing modifiers,
  end-of-turn/round expiry and source-card removal.
- All supported rules versions; unsupported historical versions reject explicitly.
- Hidden-world samples from public information and no-leakage tests: changing
  actual hidden cards while preserving the player's observation must not change
  the bot's inputs or seeded policy merely by reading server-private state.

For each supported fixture, compare legal action sets and canonical order,
legal/illegal batch verdicts, first sequence failure, post-action state and event
order. Randomly walk both engines with the same canonical actions and explicit
rule randomness. On mismatch, save the smallest seed/state/sequence as a permanent
regression before continuing the port.

Add exhaustive tiny-board action enumeration and finite-tree minimax versus
alpha-beta tests. Add make/unmake round trips, incremental-hash recomputation,
TT-on/off equivalence, cache invalidation, deadline/cancellation and worker
isolation tests. Python's current passing suite is a source of fixtures and
expected semantics, not a test suite Rust automatically inherits.

## 9. Integration and parallel execution

Start with a persistent Rust worker process and a versioned request protocol.
Load immutable definitions once; send one root observation/world-sampling request
and return a canonical action plus diagnostics. Use JSON at this outer boundary
first, never in the search loop. Profile before considering a Python extension
or binary transport. Do not spawn a new process for each simulated decision.

The Python adapter verifies root identity, actual current player/request and final
action legality before submitting through the existing backend path. Detect stale
roots and cancel obsolete searches. Expose the new bot as an explicit optional
choice only after shadow-mode parity; do not change the default bot silently.

Parallelize independent worlds/root alternatives after serial parity and budget
handling are stable. Each worker owns mutable state, undo log, arenas and initially
its transposition table. Share immutable definitions. Preserve deterministic
single-thread runs; merge results in stable action/sample order. Measure one
worker before scaling to physical cores and then logical threads. Bound aggregate
workers across simultaneous games so multiple searches do not oversubscribe the
machine. Defer shared lock-heavy transposition tables until contention is measured.

## 10. Performance gates

Use release builds on the same reference machine. Record CPU, worker count,
build flags, source/data revisions, corpus hashes, seeds, horizon, evaluator,
controller model, world count, deadline, memory and cold/warm conditions.

Measure these separately:

| Metric | What must be included |
| --- | --- |
| Legality queries/s | Valid and invalid candidates; dynamic-effect cases; no Python per candidate |
| Transitions/s | Actual complete automatic advancement to the next real decision |
| Full two-turn simulations/s | Determinization, action generation/selection, all intervening rules and final evaluation |
| Minimax nodes/leaves/s | Clearly defined native search expansion/evaluation counts, not simulations |
| Decision latency/quality | Root import, sampling, search, worker overhead, final Python validation; p50/p95 and fallback rate |

A native compatibility-rollout benchmark must execute the same two-turn rules,
reference evaluator and policy semantics as Python. Use fixed seeded random
streams for comparisons. Recorded action/RNG tapes may help differential testing,
but a benchmark that merely replays preselected actions excludes policy work
and cannot establish the full-simulation target.

First reproduce the saved 1,000-iteration workload without a deadline in both
engines. Then run a fixed diverse corpus, reporting per-position results and slow
percentiles rather than only an opening-position average. Include dense combat,
planning, effects-heavy and multi-lane positions. Report cache hits separately;
do not count incomplete, zero-horizon or fabricated repeated visits as completed
simulations. Reference settings cannot be shortened to pass a timing gate.

Success requires demonstrated 1,000 full two-turn simulations/s on the declared
reference workload, plus disclosed corpus performance and unchanged semantics.
If the broader corpus misses the target, document the remaining gap and keep the
performance work incomplete. Separately assess alpha-beta decision quality at
fixed wall time against Simple and the current non-neural ISMCTS; engine speed
and playing strength are independent acceptance gates.

## 11. Implementation milestones

Each milestone produces reviewable code, fixtures and recorded measurements.
Correctness gates precede replacing Python behavior.

| Milestone | Deliverables | Exit gate |
| --- | --- | --- |
| 0. Freeze reference | Pin legality commit; oracle manifest; baseline corpus/runner | Python regressions pass; reproducible baseline and content identity |
| 1. Native data boundary | Workspace; checked IDs; definitions/state codec; capability manifest | Lossless semantic round trips; mid-action import rejects unsupported state explicitly |
| 2. Geometry and legality | Bitsets, traversal, targeting/stat dependencies; batch and trie checks | Differential legality and invalidation pass on ordinary and spatial-effect fixtures |
| 3. Core transitions | Phase/step interpreter; card lifecycle; combat, movement, reactions, minions, upgrades | Trace/state/event parity for a complete four-hero game and boundary corpus |
| 4. Reversible execution | Undo journal; incremental hashing; allocation instrumentation | Exact restoration and hash recomputation under generated backtracking walks |
| 5. Serial iterative search | Reference evaluation; fixed-policy compatibility benchmark; team-aware alpha-beta; deadlines | Exhaustive-tree/TT equivalence; completed best-so-far result; no hidden budget bypass |
| 6. Hidden/simultaneous planning | Observation exporter; consistent beliefs; planning epochs; tested maximin/matrix policy | No-leakage and simultaneous-choice tests; supported planning/second-card games |
| 7. Full rule coverage | All heroes/maps/effects and declared rules versions; minimized regressions | No unsupported transition reachable for the advertised coverage; full parity corpus passes |
| 8. Parallelism and tuning | Worker-local execution; measured allocation/layout/ordering improvements | Deterministic serial baseline retained; real throughput/latency improvement; bounded memory |
| 9. Lab integration | Optional Rust bot adapter; shadow mode; Python final legality check; fallback telemetry | End-to-end games, stale-root/deadline/failure tests; explicit enablement only |

Milestones 3–6 can use a declared small roster for development, but do not declare
the engine complete from a special-case Arien/Brogan opening or exclude difficult
heroes from the final coverage claim. Keep separate progress for rule parity,
coverage, full-simulation throughput and adversarial playing strength.

## 12. Completion checklist

- [ ] Python oracle pinned to the legality-optimized revision and content export.
- [ ] All declared maps, heroes, effects, phases and rules versions covered.
- [ ] Batch/sequence legality, transition states and event order match Python.
- [ ] Make/unmake, hashing, caches and parallel workers preserve independence.
- [ ] Actual decision ownership and consecutive same-team choices handled.
- [ ] Hidden information and simultaneous planning treated explicitly and tested.
- [ ] Two-game-turn reference horizon and evaluator preserved in parity benchmarks.
- [ ] Iterative search returns the best completed result within its real deadline.
- [ ] Unsupported branches and stale roots trigger whole-decision fallback.
- [ ] Full-simulation target measured honestly; minimax counters kept separate.
- [ ] Playing strength evaluated at fixed time without neural models.
- [ ] Optional integration validated without silently replacing the live lab bot.

## Algorithm references

The algorithm choice is a starting implementation decision, subject to the
correctness and strength gates above.

- Knuth and Moore, [An Analysis of Alpha-Beta Pruning](https://www.sciencedirect.com/science/article/pii/0004370275900193)
  (1975): primary analysis of alpha-beta correctness and search behavior.
- Cowling, Powley and Whitehouse, [Information Set Monte Carlo Tree Search](https://eprints.whiterose.ac.uk/id/eprint/75048/1/CowlingPowleyWhitehouse2012.pdf)
  (2012): primary treatment of hidden information, simultaneous moves and
  determinization/strategy-fusion limitations. This motivates the information
  boundary and comparative ISMCTS gate; it does not establish that alpha-beta
  will be stronger for GoA2.
