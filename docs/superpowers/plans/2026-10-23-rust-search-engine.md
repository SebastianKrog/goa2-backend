# Rust engine for iterative game search

Status: implementation in progress. Verified reference/codec/plain-geometry foundations
are tracked in [rust/README.md](../../../rust/README.md). Complete native game
execution, search and performance gates remain unfinished.
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
- Prefer WSL for builds, tests, benchmarks and worker execution; keep artifacts
  on its Linux filesystem when practical. Exclude NN/model/training tests.
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

Milestones 3â€“6 can use a declared small roster for development, but do not declare
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


## Implementation progress: ordinary combat closure

The isolated native interpreter now executes ResolveCombat, DefeatUnit,
RemoveUnit and TriggerGameOver. Seventy new frozen Python handler traces check
complete state and event order, including rewards/assists, life loss, markers,
multi-piece ownership, card completion and annihilation. The combined primitive
corpus has 233 traces and 461 operations. Native worker-copy and interrupted-frame
restore checks cover private combat mutations and generated continuations.
Planning and combat now share unresolved heroes/turn state. Runtime movement and
stat checks reject newly enabled unported aura dependencies after card completion.

This is partial milestone 3 progress. Attack/reaction windows, defense-card and
full effect execution, the complete phase pipeline, reversible search state and
search remain open. No full two-turn throughput or playing-strength result is
claimed. Validation excludes neural-network/model-training tests and runs native
checks under WSL; the pinned Python oracle still uses its matching Windows runtime.


## Implementation progress: defense windows and discard orchestration

Native ReactionWindow now reproduces the card/pass menu and context for ordinary
hands, including primary-defense blocking, Initiative-based blocks, item/marker
stats and canonical piece-owner routing. DiscardCard executes card lifecycle,
public revelation and turn discard history before the queued AFTER_CARD_DISCARD
and BEFORE_DEFENSE hooks. Planning and discard use the same native card history.
Action-type restoration and generated discard/passive bindings are executable
and survive envelope export/restore. Immutable reaction catalogs are shared by
worker clones.

The frozen Python registry supplies passive trigger/config metadata. A matching
unported passive or primary defense/on-block effect is explicitly unsupported;
those bodies are not treated as no-ops. Full AttackSequence orchestration, actual
passive/defense effects and active-effect protection remain incomplete. The
combined primitive corpus now has 456 cases and 1,175 operations, including 223
new defense/discard cases across all 33 starting hero decks. Release-worker full
state/request/event parity passes. Native trace tests stream individual cases;
input-only fixture catalogs are shared to keep memory bounded as coverage grows.
No NN tests or live lab service changes are part of this work.


## Implementation progress: attack orchestration

Native AttackSequence now executes its target selection and complete ordered
reaction/actor/pre-action/defense/combat/on-block/restore recipe. Optional attacks
queue an unchanged-identity follow-up only after a successful target choice, so
skips cannot reuse a previous defender or run actor switches. Generated frames
carry typed effective range/damage/target bindings; compact snapshots restore
those bindings at automatic-step boundaries and during defense prompts.

Attack metadata uses the actual performed card (owner-specific lookup before
global/current-card fallback), preserves rule-version-9 basic-card semantics,
and recomputes explicit stats/bonuses against the bound board actor. Native
CheckTargetDefeated reads the per-attack log mark with optional owner-normalized
killer matching. Forty-nine new frozen handler traces bring the primitive corpus
to 505 cases and 1,270 operations, including all maps, multiple attacks, piece
actors, item/marker stats and extra-card lookup distinctions. Selected Python
attack/oracle rules tests pass without NN dependencies.

This remains partial milestone 3. Eligible passive and primary-defense/on-block
effect bodies still reject explicitly; full card action/phase execution,
reversible search state and the search/performance gates remain open.


The transition fixture now uses a schema-2 index plus content-addressed complete
case files. Migration retained the original full-corpus semantic SHA-256 and
checked every snapshot for equality and ordered content. Index and case hashes
are independently validated by Python; tests reject modified cases, malformed
paths, duplicate names and modified index metadata. Native per-scenario tests
load only their requested envelope, while full parity still examines every
complete state/request/event record. This changes offline fixture storage only.

Validation after the attack/index work: 49 native tests pass in WSL; 59 selected
Python rules/oracle tests pass; all 505 indexed cases match native release output
across 1,270 operations. A fresh complete Python export regenerated every case
hash and the retained full-corpus semantic SHA-256 exactly. Strict Clippy,
formatting checks and locked release compilation pass. No NN tests were run.


## Implementation progress: primary defense bodies and singleton markers

Aspiring Duelist, Stop Projectiles and Bain's Perfect Getaway/Close Call/Narrow
Escape now execute natively through the complete attack flow. Typed child frames
preserve temporary defender actors and context flags; card linkage is emitted
only for steps that declare the field. Shared marker placement/removal handles
all three marker types, singleton replacement, source identity, piece ownership,
AFTER_PLACE_MARKER ordering and copied-action substitution. Marker mutations feed
later defense stats immediately. Reflect/Deflect Projectiles' defense flag bodies
are also native, while their custom on-block discard bodies remain unported.
The pinned registry distinguishes inherited empty on-block methods from custom
ones; eligible unported bodies still fail explicitly.

Offline atomic-prefix traces now compare each intermediate state and event
against Python's existing stop-before-step hook. Hard execution budgets are
unchanged. Ninety-seven new real-handler cases bring the corpus to 602 cases and
1,485 operations. Faceup upgrade fixtures exercise actual effect bodies; separate
facedown cases verify the no-op path. Native tests check worker independence,
restoration at each effect boundary and explicit rejection of custom on-block
bodies. This remains partial rule coverage; complete effects/phases, reversible
search state, search, integration and throughput/strength gates remain open.

Validation after the defense/marker work: 52 native unit/integration tests pass
in WSL, and the release worker matches all 602 indexed traces across 1,485
operations. Forty-three selected Python oracle/marker/defense tests pass. Strict
Clippy and formatting checks pass; gameplay Python source and live lab services
were not modified. Neural-network/model-training tests remain excluded.
The fixture writer now prunes only superseded generated hash files after its
new index is safely installed; tests verify unrelated files are retained.


## Implementation progress: card selection and projectile forced discard

The native interpreter now compiles ordinary CARD selectors into typed filters
and scans physical containers in Python order, including aliases and repeated
containers. Visibility, action/color/tier, basic/active/item, identity and
lifecycle filters preserve Python behavior. Optional skips, mandatory aborts,
auto-selection and selected-card metadata use the same request/resume flow.
Nonempty generic predicate filters and non-string context identity coercions
still reject explicitly.

ForceDiscard snapshots its victim into generated selector/discard frames, so
later context changes cannot redirect the prompt or mutation. Piece-addressed
prompts resolve to their owning player while retaining the raw victim in child
steps and prompt text. Existing discard history and passive checks are reused.
Active-effect shields and immunity remain unsupported.

Reflect/Deflect Projectiles now run their complete ordinary on-block body:
optional enemy selection at the current effective range, followed by forced
discard. Deflect excludes the attacker. Temporary defender actor binding and
source-card linkage survive intermediate export/restore. This extends primitive
rule coverage; full card/phase execution, reversible state, search and all
throughput/strength gates remain open.


Validation after the card-selection/projectile work (2026-10-24): all 54 native
unit/integration tests pass in WSL; 80 selected Python oracle/rules tests pass.
The regenerated release corpus matches all 714 cases and 1,924 operations,
including full semantic state, requests and ordered events. Strict Clippy,
formatting and whitespace checks pass. The frozen Python gameplay sources and
live lab/game/bot services were not modified; no NN tests were run.


## Implementation progress: native secondary card-action execution

ResolveCard menus now execute ordinary secondary actions in the native stack:
piece choice, passive checks, MoveSequence, AttackSequence, Fast Travel, Clear
and Hold. Immutable child programs are compiled at import; small typed bindings
carry runtime operands without generating JSON in advance. Menu and continuation
encoding is shared with existing diagnostics. Fast Travel propagates the actual
card identity into its generated relocation and destination-filter records.

Shared MultiSelect supports ordered UNIT/UNIT_OR_TOKEN choices, minimum/maximum
counts, DONE/SKIP, duplicates, invalid retries, saved selections, no-actor and
empty-candidate paths. Clear uses its actual repeated-selection/ForEach/removal
children. Token removal updates occupancy, retains the supply entity and emits
the ordinary TOKEN_REMOVED event. Linked active-effect cleanup is implemented
in later milestones below.
Generated templates and per-iteration copies preserve distinct identities.

Facedown card text is a no-op as in Python; registered primary bodies and the
unregistered-effect fallback remain explicit unfinished closures. Empty effect
IDs on faceup cards reject rather than silently skipping the required fallback.
Full phases/turn-end, aggregate context aliases, active effects, reversible state,
search, integration and the throughput/strength gates remain open.


Validation after secondary-action integration (2026-10-24): all 59 native
unit/integration tests pass in WSL. The release worker matches all 870 indexed
Python traces across 2,542 operations; 156 new cases cover the complete secondary
action path, including actual hero/minion defeats from Mrak's registered
upgrades and runtime item bonuses. Defeat assertions prevent empty-target
fixtures from standing in for combat coverage. Thirty-nine selected non-NN
Python oracle/card-choice/decision-context/piece tests pass.

Worker-isolation tests cover independent action and multi-select choices.
Restoration tests cover generated movement recipes and every Clear removal
boundary, preserving complete semantic state and ordered events. The primary
text guard distinguishes facedown no-ops from unported faceup fallback behavior.
Strict Clippy, Rust/Python formatting, Python lint and whitespace checks pass.
The pinned gameplay Python sources and live lab/game/bot services are unchanged.
No neural-network/model-training tests were run.


## Implementation progress: reviewed primary recipes and displacement

The interpreter now dispatches primary card text through a native immutable
recipe catalog. Ten pure builders are frozen using the pinned public
CardEffect.get_steps_with_stats API: Noble Blade, Arcane Whirlpool, Liquid Leap,
Magical Current, Stranger Tide, Onslaught, Hit and Run, Leaping Strike, Poisoned
Dagger and Poisoned Dart. Explicit hero/card/stat operands are compiled once;
advance binds typed instructions rather than constructing JSON or calling Python.
Registered but unported effects still reject. Unregistered effects use Python's
actual movement/attack/log/no-op fallback, including the printed-range default.
State-dependent builders require explicit native logic, not sample-state freezing.

New shared primitives implement PlaceUnit, SwapUnits and RecordHex. Placement
matches occupancy-versus-terrain behavior, unplaced entities, tokens, virtual
boards and loaded-map errors; swapping preserves location insertion order.
Spawn metadata and spawn-adjacency filters support the teleport restrictions.
Noble Blade uses context adjacency, empty-neighbor and movement-prevention
filters. Literal/key precedence remains lazy. Generated primary frames retain
their literal hero and already-computed stats through export/restore.

The 182 added Python cases cover complete effects/action wrappers, actual
hero defeats, all-map teleports, item bonuses, zero/negative range and zero
primary value, optional/invalid choices, piece victims and every instruction
boundary. Direct displacement/recording cases cover no-op and failure paths.
The indexed corpus now contains 1,052 cases and 3,288 operations.

Corpus merging now reads one complete case at a time, verifies every content
hash and preserves the exact schema-1 semantic SHA-256 while atomically
installing the schema-2 index. Tests cover replacement/order/Unicode and failure
before index swap. No snapshots or semantic fields were pruned for memory.
Full phases, active effects, remaining cards/passives, aggregate alias support,
make/unmake, hashing, search, integration and performance/strength gates remain open.


Repeated frozen-catalog parsing during imports is now cached through immutable
OnceLock values shared by interpreter, reaction, planning and diagnostic code.
Only individual step records are cloned into session-local programs; symbols,
registers and mutable frames remain private. Source/schema/operand checks remain
in the loaders. A controlled alternating comparison of two release workers,
12 warm import-only requests each for primary-noble_blade-items, measured median
import-plus-protocol costs of 198.66 ms uncached and 113.41 ms cached (1.75x for
this workload). No simulation steps ran. This does not measure the two-turn
simulation, search throughput or strength gates.


Validation after the primary/caching work (2026-10-24): all 62 native
unit/integration tests pass in WSL, including the complete indexed corpus.
The cached release worker matches all 1,052 Python cases and 3,288 operations
with complete semantic state, request and ordered-event parity. Thirty-three
selected non-NN Python oracle/effect/placement tests pass. Catalog authenticity,
streaming merge safety, worker independence, per-instruction restoration and
loaded-map error behavior have dedicated checks. Strict Clippy, Rust/Python
formatting, Python lint and whitespace checks pass. The frozen gameplay Python
sources and live lab/game/bot services were not modified; no NN tests were run.


## Implementation progress: pushes, behind-target geometry and seven more cards

PushUnit now executes plain displacement with source/target/distance precedence,
integer coercion, straight-line checks, edge/obstacle stops and obstacle-crossing
landing trims. It preserves no-op context, zero/negative-distance events,
collision flags, push_victim_id and AFTER_PUSH ordering. Tokens use TOKEN_PUSHED
and skip the ordinary unit passive check. Active-effect/mine traversal remains
an explicit unported closure. ComputeHex follows Python floor division; invalid
cube results fail rather than silently rounding. Presence-based adjacency tags,
recursive unit OR and SpaceBehindEmpty filters are native.

The reviewed primary catalog now also executes Rogue Wave, Tidal Blast, Blink
Strike, Mad Dash, Brutal Jab, Mighty Punch and Savage Kick (17 effects total).
Ranged/melee operands are found by comparing both card flags through the public
effect API. Only a direct attack-step flag difference is accepted; shape changes
need explicit native logic. Typed flags freeze into exported continuations.

The 280-case primary corpus includes positive movement/defeat assertions, both
ranged flags and optional pre-move paths. Another 49 direct cases cover push,
vector and targeting edge cases. The merged index contains 1,199 complete cases
and 4,085 operations. Worker isolation and per-instruction restoration tests now
include push, blink/dash and ranged-operand boundaries. Full phase/active-effect,
remaining-card/passive, reversible-state, search and performance gates stay open.


Validation after the push/card work (2026-10-24): all 65 native tests pass in
WSL, including the full indexed corpus. The release worker matches 1,199
Python reference cases and 4,085 operations with complete semantic state,
request and ordered-event parity. Thirty-four selected Python oracle/effect/
vector tests pass. Strict Clippy, Rust/Python formatting, Python lint and
whitespace checks pass. Frozen gameplay Python sources and live services
remain unchanged. NN tests are excluded. These checks cover the implemented
rule closure; complete-game simulation and search remain unfinished.


## Implementation progress: after-action minion return

ReturnMinionToZoneStep now compiles to native ordered lane-specific returns.
A single breadth-first traversal finds nearest walking destinations and retains
the shortest path length for the generated MoveUnitStep; no repeated path query
is needed to rediscover that distance. If walking fails, a separate connected-map
search finds nearest empty placement destinations. Favored-team/ID ordering,
team-owned retryable prompts, processed-minion continuations, no-op cases and
nonmandatory relocation match the reference. The original plain-only guard has
since been replaced by actor-aware traversal, movement/displacement legality and
connected placement fallback (see the effect-aware minion return update below).
Uncompiled card aura restrictions still reject explicitly.

Generated move/place/return frames carry typed bindings, folded into ordinary
wire records on export. Restoration is checked before every instruction,
including an impossible return followed by a legal return in another lane.
Cloned workers share immutable prompt definitions and keep choices, positions
and continuations private. Thirty-six new reference cases (107 operations)
cover all six maps and generated boundary states, with positive relocation
assertions. The complete corpus now contains 1,235 cases / 4,192 operations.
The source capture retains original set serialization order; copying before
capture could otherwise reorder nonsemantic set arrays independently of Rust.

All 68 native tests pass in WSL, including the complete indexed corpus; 62
focused non-NN Python rules/oracle tests pass. Strict Clippy and Python lint pass.
Full turn/round orchestration, remaining rules/effects, reversible mutation and
hashing, search, integration and performance/strength gates remain open.

The locked release build also passes full state/request/ordered-event parity
for all 1,235 cases and 4,192 operations. Corpus fingerprint:
`fb119aaec6b2ebcc8b4892727ea994afd5a53210759d0cae68c7b4a8a65e0441`.
Rust/Python formatting, whitespace and frozen-source integrity checks pass.
Gameplay Python and live lab/backend/bot services remain unchanged.


## Implementation progress: minion battles, lane pushes and blocked spawns

Native battle rules now execute CheckLanePushStep, LanePushStep,
MinionBattleStep, ChooseMinionRemovalStep and ResolveDisplacementStep. Zone/lane
and spawn definitions are immutable shared data; Battle Zones and wave counters
are private mutable world state and export through ordinary state boundaries.
Supported instruction closures can execute from CLEANUP as well as RESOLUTION.
This does not yet implement the surrounding phase dispatcher or round cleanup.

The coordinator computes simultaneous lane outcomes before mutation, flips every
triggered counter once (clamped only by the multi-lane rule), applies throne
precedence and post-push zone totals, and performs reference tie remedies without
moving zones. Single-lane last-push semantics remain separate. Spawn assignment
uses two passes, preserving roster/type/lane/supply order and preventing a blocked
spawn from reserving a minion subsequently placed at an open point. Spawn-token
removal and direct spawns do not manufacture game events. Displacements retain
favored-team order, origin/unit selection, own-lane placement and exact prompt
context. Invalid group choices retain pending input as Python does; invalid
single-unit destinations and casualty choices clear it. Minion battle counts
include One Man Army and removal prompts recalculate ordinary/heavy eligibility.
The original active-effect battle guard has since been replaced by source-verified
exclusion counting, token-linked cleanup during pushes and topology-aware spawn
displacement (see the effect-aware battle update below). Casualty removal follows
the existing RemoveUnit semantics; battle removals do not invent defeat protection.

Seventy-six independent real-handler cases add 232 operations, including all
six maps, cleanup-phase roots, simultaneous victory/tie remedies, counter edge
cases, spawn supply/blockers/tokens/terrain, duplicate origins, forced/selected
casualties and registered ultimate bonuses. Positive fixture assertions check
actual outcomes and spawned counts. The indexed corpus contains 1,311 cases /
4,424 operations. Native tests restore before every instruction and team prompt,
check frozen target/counter continuation fields and verify worker independence.
The affected Python rules/oracle suite passes 87 tests; NN tests are excluded.
Full phase/round orchestration, active effects/remaining cards, reversible state,
hashing, search, integration and performance/strength gates remain open.

An additional source audit preserved empty Battle Zone IDs as missing-zone
sentinels, even if the board contains an empty-key zone. Explicit push/return
regressions cover both paths. The same truthiness rule is shared by all native
battle-lane lookups; direct casualty zone-key lookup remains separate, as in
Python. Frozen gameplay Python and live services remain unchanged.

Final validation for this addition (2026-10-24): all 71 native tests pass in WSL
with locked release builds, including full semantic state, request and ordered-event parity for all 1,311 cases / 4,424 operations. The preceding debug suite
also passed; the final two sentinel regressions are included in the release run.
The release worker separately matches all 76 battle cases (232 operations).
Corpus fingerprint:
`382e726fcc266eef915c8be92624e8279937a78283b5d4fff7e847270c45ee8b`.
Strict Clippy, Rust/Python formatting, Python lint, whitespace checks and frozen
source integrity pass. Eighty-seven affected Python rules/oracle tests pass.
No NN tests were run, gameplay Python was not changed, and no live service was
restarted. Complete game/search capability remains unadvertised.


## Implementation progress: action handoff and next-turn boundaries

Planning diagnostics and executing sessions now share one native actor/phase/
owner/buffer state. Actor selection and revelation logic are reused rather than
reimplemented. The executor runs confirmation, finalization, minion return/lane
checks, current-initiative selection, team ties and coin timing, hero respawn,
and advancement into planning. Generated continuations retain literal hero/tie
bindings through export/restore. Finalization resolves physical card slots,
resets the correct played/ultimate passive counters, clears actor/piece/context
and preserves a guess/reveal until a different hero finishes. Boundary metadata
and full ordered position snapshots are typed mutable state. End-turn discard
history clearing differs from standalone AdvanceTurn, matching the reference.
Automatic empty-hand passes reuse revelation and actor scheduling. End-phase
battle/check/cleanup frames can be queued, but round cleanup/upgrades remain an
explicit unsupported execution boundary. No full-game/search capability is set.

Mandatory aborts now preserve confirmation/finalization and pending combat with
its defender-to-attacker restore, instead of clearing the whole action chain.
Respawn supports tokens, marked points, ordered nearest-space fallback, passing,
invalid retries and multi-piece supply. Empty-dictionary retry staging follows
Python's exact prompt behavior. Registered aura hook identities are frozen from
stat/movement/action-prevention method overrides; ordinary resolved cards inherit
empty hooks and no longer block later actions. Actual custom hooks stay guarded.
A dedicated authenticity test checks the registry method identities.

Fifty-three new Python-handler cases add 368 operations. Every map has a complete
four-hero resolution-to-planning trace; positive checks require actual movement,
matching turn snapshots and advancement. Other cases cover actor/tie ordering,
items, facedown initiative, off-board heroes, aborts, confirmations, passive uses,
card slots, guess lifetimes, turn logs, auto-passes, respawns and queue boundaries.
The indexed corpus now contains 1,364 cases / 4,792 operations. New native tests
restore every instruction/prompt, verify worker isolation and exact aura gating,
and prevent unsupported round cleanup from counting as a completed simulation.
Sixty-eight affected non-NN Python rules/oracle tests pass. Full round/planning
API integration, remaining effects/cards/spells, aggregate aliases, make/unmake,
hashing, search, adapter and performance/strength gates remain open.

All 75 native unit/integration tests pass in WSL release mode, including the full
1,364-case indexed corpus. Strict Clippy and the 68 selected non-NN Python tests
pass. The previously run debug core suite also passed before the new restoration
checks were added. Corpus fingerprint:
`fb3e7e114d71eca8ac9c6bb3d0b83a34a9348af819047eaf3dfd6a2443375590`.

The separate release-worker protocol check also matches all 1,364 cases and
4,792 operations with full semantic state, request and ordered-event parity.
Rust/Python formatting, whitespace and frozen-source integrity checks pass.
Gameplay Python and live lab/backend/bot services remain unchanged; NN tests
were excluded throughout this addition.


## Implementation progress: cleanup, upgrades and complete ordinary rounds

The next native rule slice connects typed planning commits to action execution,
end-phase cleanup, mandatory ordered upgrades and the next planning round. Shared
planning state now exposes commit, uncommit, pass and finish commands, including
Emmitt's optional second card and synchronous all-pass turn advancement. The
isolated planning diagnostic retains its explicit boundary behavior; executable
sessions consume the same continuations through the native stack.

`round.rs` implements marker/card cleanup, exact physical-card retrieval,
nonpersistent token-pool removal, cumulative mandatory leveling, pity coins,
pending upgrades and sparse ordered item reveal snapshots. Upgrade choices use
the reference lowest-tier/color pairs and full physical-card details. A batch is
validated against a private native preview before pending input or live state is
changed; application rechecks ordered tiers and preserves master-deck aliases.
The two registered unlock hooks are native: Unbound Fury's enraged overrides and
Ready for War's silver retirement/gold retrieval and source events. Direct
cleanup and post-upgrade RoundReset preserve their distinct defeated-list rules.

The frozen source manifest is unchanged. New authenticity coverage checks the
ultimate hook catalog against the actual registry. There are 138 round traces
(446 operations), including both upgrade choices for all 33 heroes, threshold
levels/gold, malformed batches, sparse items, markers, token-pool aliases,
unlocks and all-map end-phase transitions. Twelve game traces (1,077 operations)
use real Python planning commands through hold-based full rounds on all six
maps, two consecutive rounds, invalid/undo/two-card planning and empty-hand passes.
Native tests restore every cleanup/upgrade instruction and prompt, check atomic
invalid-batch retries and private worker choices, and restore planning branches.

The canonical corpus contains 1,514 full-state/request/event traces and 6,315
operations. Fingerprint:
`9d4d143c5ae61bc06bc5c1039c2649272d3f35bc715c3a01e66f94c539215a56`.
All 79 native unit/integration tests pass in locked WSL release mode, including
the complete corpus. Strict Clippy passes. All 74 selected non-NN Python
round/upgrade/unlock/oracle tests pass. Gameplay Python remains unchanged.

C: filled during fixture generation. Only generated duplicate copies with
verified canonical counterparts were removed. Case export now replaces files
atomically and merging can link verified immutable snapshots on the same volume,
with a copy fallback. Tests cover interrupted writes and orphaned-link retries.
Full semantic snapshots and the original oracle source identity are retained.

This closes the ordinary round pipeline only. Active effects, custom aura and
passive hooks, remaining primary/spell bodies, aggregate alias semantics,
make/unmake and hashing, evaluation/search, optional adapter and speed/strength
gates remain open. General game-transition/search capabilities remain false.
No NN tests were run, no live services were restarted and no commit was made.

Final external protocol validation also passes all 1,514 cases / 6,315 operations
with exact full semantic state, request and ordered-event parity through the WSL
release worker. Rust/Python formatting, lint and frozen-source integrity checks
pass. The active implementation goal remains incomplete for the gates above.


## Implementation progress: active flat/count card auras

Native `CardAura` definitions now execute Metamorphosis and Unlimited Firepower.
Card selection follows the real face-up level-eight ultimate/resolved-played
rules, preserving duplicate physical aliases. Counts use team-bearing adjacent
entities with no invented immunity filter. Multi-piece initiative aggregates
distinct enemies across all on-board pieces even when queried through one piece;
movement uses the bound piece where applicable. Flat range/radius bonuses retain
the original fixed melee-range behavior and apply to off-board owners as in Python.

Movement actions freeze Metamorphosis's pass-through permission in typed child
frames, including movement-path/straight-line predicates and exact wire export.
Imported/restored continuations retain the frozen flags. Raw effect-side moves
receive no implicit movement aura. Tests restore every instruction and input,
change aura activation after action start, remove counted enemies after range
selection and verify worker clones share definitions/programs but not mutations.
The former global aura flag is now explicitly `has_uncompiled_auras`.

Thirty-one added scalar/card-stat cases cover both hooks, all six maps, lifecycle
and face rules, item/marker stacking, off-board owners, tokens, duplicate aliases
and multi-piece aggregation. Fourteen real-handler traces add 49 operations,
including actual obstacle crossings, isolated failed moves, a ranged minion kill
and aura-dependent initiative handoff. Public-hook specification checks verify
the native assumptions against the frozen registry. Source identity is unchanged.

All 81 native unit/integration tests pass in locked WSL release mode, including
1,528 full semantic-state/request/event traces and 6,364 operations. The added
14 aura traces also pass through the separate release-worker protocol. All
10,016 stat/model queries and 37 lifecycle operations pass that protocol.
Ninety-five affected Python hero-rules/oracle tests, strict Clippy, Rust/Python
formatting and Python lint pass. Trace corpus fingerprint:
`bc9a5defd317c302c7b2f178cb57e607c26d095e66e3ed336dbb30ccdbc4873d`.
Model corpus fingerprint:
`acdda2d00037ba9c13f67199d6cbcf0a4e955f935ef8252f4a47945ac454933d`.

Big Sodding Gun still needs performing-card-aware conditional stat plumbing;
Witching Hour needs action prevention and the active-area stat-effect lifecycle.
Those hooks remain guarded. Active effects, remaining card/passive/spell bodies,
aggregate aliases, make/unmake, hashing, evaluation/search, adapter and the full
performance/strength gates remain open. General game/search capabilities remain
false. NN tests and live services remain untouched; no commit was requested.

At the user's request, obsolete generated trace/mismatch copies and old Rust test
scratch directories were removed. Current aura artifacts, current test scratch,
canonical fixtures and active WSL build caches were kept. All 1,528 canonical case
files remain present after cleanup. Approximately 88 MB was reclaimed on C:.


## Implementation progress: conditional stats and a real basic primary

Native stats now have separate ordinary-turn, live-context and explicit-card
entry points. The shared performed-card resolver uses immutable interned register
keys, reads the current registers on every call, prefers owner-specific physical
lookup, then global lookup, then the hero's turn card. Menus, attack expansion,
reaction values, movement/fallback stats and basic-attack metadata use that path.
Explicit card-stat queries use the supplied physical card instead of a nested
action's card. Invalid physical handles reject; no mutable context cache is kept.

Big Sodding Gun's conditional +2 Attack/+2 Range is native, restricted to a basic
card with declared primary Attack. Face/lifecycle behavior follows the frozen
public hook. Point Blank Shot is now a native reviewed primary recipe, bringing
the closed catalog to 18 effects / 62 steps. The new CheckAdjacency primitive
preserves direct-position semantics for pieces, literal/key precedence, missing
positions, scalar identity behavior and falsy short-circuiting. Active topology
effects remain guarded. Actual blocked attacks push surviving adjacent targets;
targets defeated during combat no longer satisfy adjacency.

Twenty-three added stat cases cover context overrides, owner preference,
global/turn fallback, foreign and piece owners, facedown cards, inactive sources,
physical aliases and explicit views. Thirty-five handler traces add 77 operations:
actual conditional-range kills, changing context between attacks, primary versus
fallback stat behavior, adjacency edge cases and a defended primary followed by
a push. Native tests restore every instruction and prompt across nested attacks
and primaries, keep worker registers private, check invalid card handles and
ensure supporting the stat hook does not hide its unported eligible push passive.
The frozen reference source identity remains unchanged.

All 84 native unit/integration tests pass in locked WSL release mode, including
1,563 exact full-state/request/event traces and 6,441 operations. All 35 added
traces and all 16,482 queries across 57 model/stat cases pass the separate release
worker protocol, along with the existing 37 lifecycle operations. All 41 affected
Python Sabina/oracle tests, strict Clippy and Rust/Python formatting/lint pass.
Canonical trace fingerprint:
`3bf6d7714895eb4f51ca1e5399ebcb70ba44f886ed8c96c5fd5b79ee0b0e82a8`.
Model corpus fingerprint:
`f656209f7297a5f80f65ed0ce9adb01d90b610702ad3b2c25d67b120ed0cdb2e`.
Primary catalog fingerprint:
`a59a39ec0c973e30b6eeb0579627cdd456ffdd200f6604102e5971c5849d6c52`.

Remaining: Big Sodding Gun's AFTER_PUSH discard-or-defeat passive, Witching Hour,
active-effect lifecycle/scopes, other card/passive/spell bodies, aggregate aliases,
make/unmake and hashing, evaluation/search, adapter and full speed/strength gates.
Truthy non-string performed-card identities and conditional spell-face stats
remain explicit unsupported cases. Completed request-identifier lifetime across
export/restore also remains an audit item before live adapter integration.
General game-transition/search capabilities remain false. NN tests and live
services were not touched; no commit/push was requested. Current caches and test
artifacts remain intact as requested.


### Implementation progress: mandatory discard-or-defeat

Implemented `ForceDiscardOrDefeatStep` as a native typed expansion into the
existing selection, discard and defeat programs. Mandatory prompt wording,
owner/piece identity, off-board guards and literal/context/current-actor kill
credit match the frozen Python implementation. Generated prompt and defeat
bindings survive instruction-by-instruction export/reimport. Truthy non-string
victim/killer coercion and active-effect immunity still reject explicitly.

Added 20 reference traces / 37 operations with asserted discard, defeat, no-op
and kill-credit outcomes, including three instruction-prefix traces. The oracle's
owner-level multi-piece defeat records defeat but leaves pieces when removal
names only the owner; the Rust engine preserves this reference behavior.
Focused worker protocol verification passed all 20 cases. Extended native
restore testing covers every instruction in the 17 distinct behavior cases.

Added-trace fingerprint:
`3ee939b013879bdd3628c46197f33a62eac65034afa2580bd895ceae394686e3`.
Full trace corpus: 1,583 cases / 6,478 operations, fingerprint
`7430c8c2cc8f7dcd6f36dc090d4deaeb00f2aa31532fe6ef0cd13c5de4ea5375`.

Strict Clippy passed. Selected oracle/Sabina Python tests: 41 passed, no NN tests.
Full locked WSL release regression: 84 tests passed, including complete parity
for 1,583 traces / 6,478 operations (CLI corpus test 89.86 seconds). Rust fmt,
Black (29 files), Ruff and Git whitespace checks passed. Frozen source identity
was verified unchanged, and the tracked gameplay source diff remains empty.
Big Sodding Gun's passive dispatch still needs implementation; this completes
its discard-or-defeat primitive, not its eligible passive closure. All broader
search/engine gates remain open. No live service changes or commit/push.


### Implementation progress: passive scan, offers and Sabina after-push

Ported the provider/offer/usage pipeline to native typed programs. Provider scans
preserve played-card order, level/face/lifecycle/usage gating and ultimate aliases.
Offers resolve the current owner's live card again; optional offers use exact
CONFIRM_PASSIVE prompts/context and YES/NO/SKIP behavior. Removed or face-down
waiting cards do not execute. Marking occurs only after a nonempty body and uses
the reference's current actor, including explicit-owner/marking asymmetry.
Counters update physical cards shared by their true aliases, privately per worker.
Frozen registry config metadata now includes optional flags and prompt text, with
an exact public-API authenticity test. Immutable prompts compile once per world;
runtime advancement constructs no JSON instructions and calls no Python.

Big Sodding Gun's body now executes after actual pushes: enemy hero victims
produce discard-or-defeat, while friendly heroes, minions and absent victims do
not consume usage. Off-board enemies still emit the no-op body and mark usage.
Real Point Blank Shot, repeated pushes, collision-blocked pushes, multi-piece
victims, regular/ultimate aliases and explicit hero overrides match Python.
The passive/discard cycle is linked through a once-compiled session program,
preventing recursive expansion while retaining real generated frame boundaries.

Added 35 complete traces / 116 operations with instruction-prefix snapshots and
asserted outcomes. Added native every-instruction restore, independent-worker,
waiting-card revalidation and unported-provider guard tests. Reused the restore
runner across the existing nested-context and discard-or-defeat tests.

Added trace fingerprint:
`d62b86177e99f52bfee723377ae426694f75908a14d4a30cba9b251591930ba6`.
Full corpus: 1,618 traces / 6,594 operations, fingerprint
`3cec1de9f966583c18b90204bbf81fdaced48759803d4552a36aa57cedd6af13`.

Validation: 88 native tests passed in locked WSL release, including all complete
frozen traces (CLI corpus comparison 90.83 seconds). The selected Python
oracle/Sabina tests passed 43 tests; the final provider-predicate assertion passed
separately. Strict Clippy, Rust fmt, Black (30 Python files), Ruff and Git
whitespace checks passed. Frozen gameplay source identity remains unchanged.
NN tests, running services and active caches were left alone. No commit/push.

Remaining: other passive bodies, runtime predicates, active-effect providers and
lifecycle/scopes, Witching Hour, other card/spell bodies, aggregate aliases,
make/unmake and hashing, evaluation/search, adapter and full speed/strength gates.
Truthy non-string passive victim coercion is explicitly unsupported. Consumed
request-token lifetime across export/restore still needs its separate audit.
Global game-transition/search capabilities remain false; the engine is unfinished.


### Implementation progress: consumed request and generated-frame identity audit

Reproduced a real stale-input bug before fixing it: completing the first number
prompt, advancing one instruction, exporting without any pending request frame,
and restoring caused the later confirmation to reuse `native-request-1`.
The new regression failed with both old/new IDs equal, then passed after the fix.

Native exports now retain versioned `native_resume` request/step high-water marks
outside semantic game state. Restore preserves consumed identity history even
with an empty stack. Fresh Python exports omit this native-only block; live
continuation IDs still establish the initial lower bound. Generated template
IDs reserve their numeric generation prefix as well. Native restore rejects
future resume versions, malformed values, counters behind live frames and
exhausted counters. Callers must preserve this block when resuming a native
session; removing it is a fresh state import rather than a lossless native resume.

Request wire strings now match exactly. A padded/signed numeric spelling cannot
answer a canonical generated request; a literal imported spelling is accepted
only as the exact live ID. This is session-lineage transport metadata; it does
not alter materialized game state, definitions, physical aliases or rule values.
It is not a replacement for the remaining make/unmake or search-state hash work.

Added six native tests: the failing consumed-ID regression, exact generated and
imported wire matching, completed continuations with no stack, generated-template
prefixes, regressing/exhausted history and codec shape/version/semantic isolation.
Validation: 94 tests passed in locked WSL release, including 1,618 full reference
traces / 6,594 operations (CLI corpus comparison 91.38 seconds). Strict Clippy
passed. The final release protocol preserved native metadata, resumed a high-water
mark of 17 at request 18, and matched the frozen Python semantic/request/event
record exactly after normalizing ephemeral identity. Frozen source identity
was verified unchanged; tracked gameplay sources have no diff. No new Python
rules or NN tests were needed, and no running service or active cache was changed.

This closes the consumed-request-token restore audit identified in the previous
progress sections. Other card/passive/spell bodies, active effects, aggregate
aliases, make/unmake/hashing, evaluation/search, adapter and speed/strength gates
remain unfinished. Global game-transition/search capabilities remain false.
No commit/push was requested.


### Implementation progress: tactical Sabina primaries and bounded repeats

Expanded the reviewed pure primary catalog from 18 to 29 effects (99 top-level
steps): Back to Back, Listen Up, Roger Roger, Ready and Waiting, Troop Movement,
Marching Orders, Path to Victory, Close Support, Covering Fire, Shootout and
Bullet Hell. Real reference cases assert swaps, two/three minion moves, actual
hand-discard logs and one/two follow-up minion removals, retaining removal's
no-reward behavior. This is not complete Sabina or general card support.

Implemented native MayRepeatOnce/MayRepeatNTimes through their common wire type,
including mandatory lane-push checks before each repeat, optional decisions,
partial/negative/exhausted counters, nested fresh template copies and restored
lane-check flags. ValidateRepeat preserves scalar Python AND values. Active
repeat-prevention effects and aggregate context alias results reject explicitly.
The existing ForEach and new repeat instruction share one template-copy path,
with frozen primary operands and private mutable frames/counters per worker.
No runtime Python or JSON instruction construction was added.

Added 127 tactical primary traces / 1,001 operations and 27 repeat/validation
traces / 97 operations. Added the previously absent ordinary Point Blank Shot
matrix (11 traces / 64 operations), complementing existing conditional/passive
coverage. The focused primary generator can export only a requested effect
subset without duplicating common fallback/primitive traces. The primary
trace module now supplies 418 cases across 29 reviewed effects and its common
fallback/primitive/all-map cases.

Four native tests add every-instruction restore for the new primaries and
repeat continuations, nested-worker isolation and explicit unsupported guards.
Focused release-worker protocol comparisons passed all 165 added cases /
1,162 operations. Locked WSL release regression passed 98 tests, including all
1,783 complete traces / 7,756 operations (CLI corpus comparison 109.12 seconds).
The selected Python oracle, Sabina, repeat and lane-push tests passed 61 tests.
Strict Clippy, Rust fmt, Black (31 Python files), Ruff and Git whitespace checks
passed. Frozen source identity remains unchanged; tracked gameplay sources
have no diff. NN tests and running lab/backend/bot services were untouched.

Primary catalog fingerprint:
`7e4b5f52f32651131fd6487bc93dc80465dc6933ca480f6314b6c4d334416810`.
Tactical trace fingerprint:
`645a256f68e210ee2cb35cff8c44671048fd13b89c46f8c4f810855df12bf776`.
Repeat trace fingerprint:
`2e0a8fd5d6bd8748e571c3142d4401b36e1315061d73d224d41a50b67ce40850`.
Point Blank Shot matrix fingerprint:
`6bb6607c55126b842e4b18a71a77187a3cf954f5771f9fb5591b2104ffed8470`.
Full trace fingerprint:
`d1d512cfb470a09544da1202b2d3833a3a6cd133e20a8f071442128665944549`.

Disk handling: C: reached about 32 MB free while retaining full semantic traces.
Applied transparent NTFS compression only to generated canonical JSON case files;
no active files/caches were removed and no semantic fields were pruned. Compact
reported 1,249,739,462 bytes stored in 395,557,049 bytes (about 854 MB saved).
All 1,783 case hashes were then verified unchanged. C: had about 926 MB free.
Hard links still share immutable data with focused working artifacts.

Remaining: other card/passive/spell bodies and predicates, active-effect
providers/lifecycle/scopes, Witching Hour, aggregate aliases, make/unmake and
hashing, evaluation/search, optional adapter and speed/strength gates. The
engine/search objective remains unfinished; global game/search flags stay false.
No commit/push was requested.


### Implementation progress: count/collect queries and conditional attacks

Added typed CountStep, CollectUnitsStep, PlayedCardFilter and
CheckContextConditionStep. The unit-filter compiler is shared with selection,
preserving explicit filter order and implicit self/immunity rules. Count and
collect intentionally bypass selection's intrinsic can_be_targeted check;
queries with off-board actors still work. Count's default skips immunity,
collection's default respects it. Hex counts and other target families retain
the frozen source's candidate behavior. The handler's centralized skip gate
preserves prior output; the leaf Count.resolve skip/reset branch is not executed
by the actual reference handler and was not substituted for it.

Played-card predicates read both the current turn slot and the current card,
including facedown masks, combined action/color matching, no-criterion presence,
and per-piece candidates sharing an owner. Added a shared one-based HeroRecord
turn-slot getter. Conditions preserve True/None, numeric booleans and nonnumeric
fallback to zero. Float/int comparison does not round an i64 threshold to f64,
so it matches Python above 2**53 and at signed 64-bit boundaries. Nonfinite
numbers and unported query filter/effect closures reject explicitly.

The primary catalog now supports Quickdraw, Gunslinger, Dead Shot, Shield Bash
and Counterattack: 34 reviewed effects / 118 top-level steps. Assertions cover
actual bonuses, old/resolved/hidden target cards, live pieces, preexisting bonus
context, forced discards and empty-hand defeats. Ordinary primary matrices were
added alongside the targeted conditional cases, keeping the generator/corpus
coverage aligned.

Added 41 query cases / 41 operations, including 882 scalar condition checks;
80 targeted primary cases / 425 operations; and 55 ordinary primary cases /
227 operations. Four native tests cover instruction-boundary restoration,
private live card/turn/occupancy queries, malformed filter flags/criteria and
nonfinite comparison guards. The final release protocol matches all 176 added
cases / 693 operations. Locked WSL release regression passed 102 tests, including
all 1,959 full traces / 8,449 operations (CLI corpus comparison 113.88 seconds).
The selected Python oracle, played-card, collection and Sabina tests passed 58
tests. Strict Clippy, Rust fmt, Black (33 files), Ruff and Git whitespace checks
passed. Frozen source identity is unchanged; tracked gameplay source has no diff.
NN tests and running services were untouched; no commit/push was requested.

Primary catalog:
`278822087f2c61045b41fad73e753258a997ef9d109288d0a72ac66c552c844b`.
Query traces:
`514f2371627cc105485e876673fdcf051396997abc7d6e9f633416b830fe4ba6`.
Targeted primary traces:
`caf469c6b7bbbf99a0ee1fc482746da0fbb4bef88eb933920d39a8257fec27f9`.
Ordinary primary traces:
`52ebde09baabbf6e376ae5d4cf4ed7646ac42e0df0215b0f94fccef72dfddd92`.
Combined added traces:
`161fb9a1e05ec81b1b2974910d49a6f2ee6af7bb28dcdda4b6cbab347b846b58`.
Full trace corpus:
`81be7fe169c104107f1bd64579dd2108287dbb6cb730218aecac6a8f8c657e05`.

Kept complete snapshots and hard-linked focused artifacts. Transparent NTFS
compression reduced the 176 new case files from 83,235,721 to 28,286,976 bytes;
all 176 content hashes were verified afterward. No active file or cache was
removed. C: had about 821 MB free after compression.

Remaining: other card/passive/spell bodies and predicates, active-effect
providers/lifecycle/scopes, Witching Hour, aggregate context aliases, make/unmake
and hashing, evaluation/search, optional adapter and speed/strength gates. The
full engine objective is unfinished; game-transition/search flags remain false.


### 2026-10-24: ordinary retrieval and shared premature expiry

Ported RetrieveCardStep as a native instruction with physical target lookup:
played slots first, then discard, then current card; bulk retrieval snapshots
ordered discard contents. Reuses Hero model movement while preserving sparse
slots, aliases, face/initiative resets, raw piece actor events, existing discard
history and no-op conditions. Retrieval events omit Alternative Timelines-only
metadata. Reference-generated cases verify non-string card values are no-ops.

The shared script-side expiry helper clears the base active flag after retrieval
or discard, following the reference manager's global printed-ID lookup order.
Independent same-print copies on another hero and enraged overrides are covered.
Active-effect expiry and repeated physical bulk retrieval requiring partial
failure remain guarded before mutation; no reference behavior is approximated.

Steady Advance, Unwavering Resolve, Devoted Followers and Fresh Converts now
execute from reviewed recipes. Catalog: 38 effects / 136 top-level steps.
Added 31 direct cases / 31 operations, 54 targeted support-skill cases / 196
operations and 44 ordinary primary cases / 74 operations. All three focused
corpora passed the actual release worker protocol with complete state/request/
event parity. Three native tests cover every-instruction restoration, cloned
worker isolation and pre-mutation rejection. Python oracle/retrieval/Sabina
rules tests: 68 passed. Strict Clippy, Rust fmt, Black and Ruff pass. Frozen
source identity is unchanged and tracked Python gameplay source has no diff.

Primary catalog:
`fdb9e90c6e1bfed65fb22bce56c5bcc99681c25cf7ea87bc15fdd4fb1baa8aef`.
Direct retrieval traces:
`10af88dbd7cd9689e3858abd0e38c4bd288dc1e78c96d6c3f2fe1025138566b3`.
Targeted primary traces:
`bed36378eff247d7bff25a71a23ea921858419c627647f3f7bf57f90c582a343`.
Ordinary primary traces:
`ea1425a43f522d5684fe18691a3404619626af6a229e8c403d7aac2445b0e1c0`.
Combined 129 cases / 301 operations:
`192da12886082ce93fb41cfde933adb887d52133b80d9d56a4d1bb81f11bc9dc`.
Full 2,088 cases / 8,750 operations:
`78cb39fbe3ed1b365f0a63525ba49b5cdf67870db2dedf6c285cda35a8d9cecb`.

Transparent NTFS compression retained all 129 new immutable case files,
39,627,495 logical bytes stored in 13,426,688 bytes; content hashes passed after
compression. Current fixtures, caches, scratch and live services were preserved.
Only obsolete prior test output was removed at the user's request.

Remaining scope is unchanged: other primary/passive/spell effects, active-effect
lifecycle/providers, aggregate aliases, make/unmake and hashing, evaluation,
iterative team-aware search, hidden/simultaneous planning, parallelism, optional
validated adapter and the 1,000 full two-turn simulations/sec gate. General
transition/search capability flags remain false; the full objective is unfinished.


Final regression for the retrieval addition: locked WSL release workspace suite
passed all 105 tests, including complete protocol comparison of all 2,088 cases /
8,750 operations in 117.47 seconds. This corpus-validation duration measures I/O
and reference comparisons, not native search throughput. No NN test or running
service was changed; no commit/push was requested.


### 2026-10-24: native active-effect storage and lifecycle

Added `effects.rs`: complete typed immutable payload definitions behind Arc,
ordered private effect instances, explicit durations, activation flags, source /
physical card / token identities, timestamps and temporal activity checks.
World import/export now carries this typed store. The ordinary card expiry
helper delegates to the shared premature-card-removal path.

Ported manager lifecycle operations: activate/deactivate by card and ID;
collect/remove natural turn expiry; duration expiry; premature removal by card,
source and ID; stale cleanup. Natural expiry returns finishing instructions in
source order, while after-cards-played triggers fizzle outside their dedicated
window. Premature removal never returns finishing steps. Token rows survive
source/duration sweeps; bound card effects follow the physical card owner even
when their performer differs. Card owner lookup includes extra-turn cards, while
card flag updates use the frozen global printed lookup without that slot.

Preserved non-obvious source distinctions: passive effects always count active;
cardless effects ignore the explicit flag; an empty card link is false for
activation/owner lookup but not the stale-cleanup None check; an empty token link
is still token-bound. ID activation changes only the first matching row, generic
ID removal removes all, source expiry removes matching instances, and duration
expiry removes colliding IDs while refreshing only explicitly expiring card
links. Tests retain these semantics instead of normalizing rows.

The new `effect_trace` operation is differential diagnostics, not a game/search
capability. Frozen Python reference: 41 cases / 286 operations, covering every
effect category and duration, full semantic state after every operation,
finishing order, aliases, ownership, token exemptions and restored worlds. Two
native tests add full-state parity and cloned-worker isolation with shared
immutable definitions. Actual release-worker protocol parity passed all 41.
Python EffectManager/oracle-tool tests passed 59; frozen source remains unchanged.

Lifecycle corpus SHA-256:
`c4fe67b6549760aa8157805e11741e4f68b28f753640fdfd8aae4e6a8238803a`.

Effect creation, concrete behavioral consumers and finishing-payload scheduling/
execution remain unfinished. Live session guards still reject active-effect
closure, and all general transition/search flags remain false. Remaining full
scope includes other card/passive/spell bodies, aggregate aliases, reversible
state/hashing, iterative team-aware search, hidden/simultaneous planning,
parallelism, optional validated integration and speed/strength gates.


Final validation: locked WSL release workspace suite passed all 107 tests,
including all 2,088 transition traces / 8,750 operations (CLI corpus comparison
119.75 seconds) and the separate 41 lifecycle cases / 286 operations. Fixed an
older action-guard test to use a valid Python-generated effect instead of an
invalid placeholder object, so it continues to test the behavioral guard after
typed import. Strict Clippy, Rust fmt, Black (33 files), Ruff and Git whitespace
checks pass. The new 48,379,837-byte fixture was compressed in place to
15,323,136 bytes and its content hash reverified; active files/caches retained.
No NN test, live service, commit or push was involved.


### 2026-10-24: native EffectManager creation

Added EffectCreateSpec and CreationContext: compile a fully expanded, validated
creation template at the outer boundary; instantiate without JSON inside native
rules. Instances share immutable definitions while origins, IDs, timestamps and
activation remain private. Added native next_entity_id import/export and checked
allocation of the exact eff_e_N ID. Restored and cloned worlds retain their own
counters; exhaustion rejects before mutation and does not block deduplication.

Creation matches the frozen manager's card/type/affects key, including dormant
rows and its complete no-refresh behavior. Cardless/empty card links always
create another instance. Existing non-unresolved cards activate immediately;
missing/extra-only card lookup follows the reference global scan. Scope origins
bind to acting pieces only for non-global, non-fixed origins; barrier origins
normalize independently. Attack-origin metadata uses the actually performed
card's basic tier property, including hidden and nested owner/extra-card cases.

Added 82 creation cases / 284 operations. Combined lifecycle/creation corpus:
123 cases / 570 operations, all passing the persistent release-worker protocol.
Two additional native regressions verify shared templates with private origin /
timestamp / counter state, restore/recreation and atomic counter exhaustion.
Large effect test fixtures load once at runtime instead of being embedded in
test binaries. Locked WSL release workspace suite passed all 109 tests, including
all 2,088 transition traces / 8,750 operations (CLI comparison 172.67 seconds).
Selected Python EffectManager/oracle tests: 59 passed. Strict Clippy, Rust fmt,
Black (34 files), Ruff, and unchanged frozen-source verification pass.

Combined lifecycle/creation SHA-256:
`2bf2b9710289ace3059421167fff98522f9043b91b84fc5754603ead30b7cfd7`.
Both immutable fixture copies were compressed in place: 206,033,306 logical bytes
stored in 64,995,328 bytes; hashes reverified after compression.

The user reported Windows Git crash dialogs and repeated WSL source-drive tips.
Windows error records identify installed Git 2.24.1.2 at Program Files/Git as the
crashing binary. The available bundled Git 2.53.0.windows.3 successfully reads
this checkout; use its explicit executable path for subsequent Windows Git work
rather than the older PATH entry. No global software/settings change was made.
WSL build outputs are Linux-local but source remains on /mnt/c, explaining the
performance tip. Avoid copying large snapshots into WSL while C: is constrained.
The old cache cleanup is tracked separately; active Rust data is preserved.

CreateEffectStep input binding, gameplay consumers, finishing-payload scheduling,
remaining card/spell/passive rules, aggregate aliases, reversible state/hashing,
search, optional validated integration and full simulation/strength gates remain
unfinished. General transition/search flags remain false. No NN tests, live
service restart, commit or push was involved.


### 2026-10-24: CreateEffectStep and pre-migration checkpoint

Added typed CreateEffectStep compilation/execution with source-derived default
records and enum catalogs. Bindings cover literal/context cards and precedence,
origin action, dynamic/piece origins, token lifecycle linkage and copied-token
substitution, exceptions, public color and nested finishing-color freezing.
Creation events retain split/color metadata and the source's suppression when
an existing effect ID collides. The native rule path uses no JSON/Python.

Resolution-only creation/context/log traces can restore active effect rows;
planning diagnostics and gameplay/input consumers remain explicitly gated until
their active-effect semantics are ported. General transition/search flags stay
false. Added three native regressions for instruction restoration, frozen worker
payloads and rejection of unported input/controller paths.

The actual release worker passed 64 new full-state/request/event cases / 67
operations. The frozen default contract and selected Python oracle/manager tests
passed 60 tests. New full regression corpus: 2,152 cases / 8,817 operations;
SHA-256 cf93b3801e70b46ff9dc9db19bdd6db379d07da0f693d59b5c47b56d567ca5a0.
Effect-step corpus SHA-256:
4deea67932b7a2d0c2d2f3cb9c13ba9dd8f39fcd81e531d1d852fec4e7bf0f8e.
Frozen defaults SHA-256:
560bac090b0d8891865c7d11a0303d3a2ed7856ed83a3856ae934d8465aacda6.
The new three native tests and complete expanded regression still need execution;
the prior full suite passed 109 tests. This checkpoint does not claim completion.

User requested a commit before migration. The WSL copy at
/home/sebastian/projects/goa2-backend-bots is staging until the Windows checkout
is committed and that commit is synchronized and verified. Keep the Windows
checkout and its frozen Python environment as a backup/reference. No lab or
frontend settings or live services are changed by the migration. Remaining engine,
search and performance scope is unchanged.


### 2026-10-24: committed checkpoint and WSL migration verified

Committed all current Rust engine work as 92510d5 before the final migration
synchronization. The initial file copy remained staging until the committed
Windows state was synced and checksum-verified. Git HEAD and branch matched,
and the Linux checkout was clean before migration documentation updates.
Active development now lives at /home/sebastian/projects/goa2-backend-bots
(Windows access: \\wsl.localhost\Ubuntu\home\sebastian\projects\goa2-backend-bots).
The Windows original remains an intact committed backup/frozen Python runtime.
Virtual environments, disposable caches and ignored diagnostics were not copied;
all source, Git history and canonical fixtures were retained. Local Git identity
was preserved and core.autocrlf=input selected for Linux. Tracked CRLF text was
normalized to LF without changing Git-normalized content.

Cargo confirmed source filesystem ext2/ext3 and compiled the actual crates from
/home/sebastian/projects/goa2-backend-bots/rust. Full locked release workspace
suite passed all 112 tests, including the three new effect-step regressions,
all 2,152 handler cases / 8,817 operations (CLI comparison 112.14 seconds),
and 123 manager cases / 570 operations. Strict Clippy and Rust fmt passed.
The preserved Windows Python runtime successfully verified the migrated source
through the WSL file share against the unchanged frozen manifest. No NN tests,
lab/frontend configuration changes, service restarts or pushes were involved.

The full engine/search objective remains unfinished; continue development only
in the Linux checkout, using the Windows original as backup/reference. Pending
work includes active-effect gameplay consumers and finishing scheduling, other
card/passive/spell rules, aggregate aliases, make/unmake and hashing, evaluation,
iterative team-aware search, hidden/simultaneous planning, parallelism, optional
validated integration and full-simulation performance/strength gates.


### 2026-10-24: token-linked effect removal and Linux-local oracle

Implemented real RemoveTokenStep effect cleanup: remove instances linked by
token_id, source_id or scope.origin_id only when the token has a board position.
Independent barrier anchors do not count. Preserve unrelated duplicate-ID rows,
card-active flags and finishing payloads; report the actual removed count in the
TOKEN_REMOVED event. Inactive and temporally expired rows are still removed.
Resolution-only create/remove/context/log stacks can restore active rows; other
unported gameplay/controller paths remain guarded. Native tests verify each
instruction boundary, create/remove composition and private worker state.

Added 51 full handler cases / 53 operations, including every effect category,
link type, no-op target, gates/context precedence, off-board tokens, repeats,
card flags, fixed scope origins and premature finishing suppression. Both frozen
Windows and Linux runtimes generated identical cases. The actual release worker
passed every new state/request/event comparison. Canonical corpus now contains
2,203 cases / 8,870 operations, SHA-256:
9c776a5c734ce4d2437135291afbebbf2f47dbb7a305ddfdfdfccb73c37a4352.
The full locked release workspace run passed all 114 native tests, including
the complete corpus (CLI comparison 103.98 seconds).

Created a Linux-local reference environment at
/home/sebastian/.cache/goa2-rust/oracle-venv using the already installed CPython
3.11.17 and exact frozen Pydantic 2.13.4, pydantic_core 2.46.4 and jsonpatch 1.33.
Source/runtime verification retained the unchanged manifest identity. All 60
focused Python oracle/EffectManager tests passed there. This avoids repeated
Windows-to-WSL source reads; the Windows runtime remains a backup/cross-check.
Strict Clippy, Rust formatting, Black, Ruff and whitespace checks passed.
No NN dependencies/tests, live service changes, commits or pushes in this slice.

Remaining full objective is unchanged: active-effect gameplay consumers and
finishing scheduling, remaining card/passive/spell rules, aggregate aliases,
make/unmake and hashing, evaluation, iterative team-aware search, hidden and
simultaneous planning, parallelism, validated integration, strength tests and
1,000 complete two-turn simulations/sec. General transition/search flags remain
false; token-removal parity does not establish those broader requirements.


### 2026-10-24: native effect stats, immunity and topology consumers

Added shared typed effect behavior compiled at the state/program boundary.
Native queries use no JSON/Python: all effect scope shapes/relations, positive
item doubling, area stat modifiers, basic-action bonuses, and ordered conditional
modifiers. Item/marker/card-aura contributions precede conditional thresholds.
Multi-piece owner/piece recursion preserves performed-card fallback and counts
each positional effect once for aggregate initiative. Explicit card queries keep
the reference's recursion precedence. Dynamic exceptions/primary operand binding
update the shared-definition projection through copy-on-write.

Implemented split/isolation connectivity, including source live-position versus
isolated-hex fallback and one-way caster interaction. Heavy/action/attack immunity
is shared by effect stat/scoping checks, including token controllers, named
subjects, basic/nonbasic attacks and attacker exceptions. Topology intentionally
ignores effect activity/duration as the frozen reference does; action immunity
uses the explicit active flag while attack immunity also checks duration.
Card aura positional counts and minion defense now consult topology. Illusion
minion equivalence and unported card aura hooks remain explicit guards.

Effect activity reads live world round/turn. Round reset synchronizes this clock;
a native full-round regression checks the synchronization. Added real
CheckAdjacencyStep execution through the same topology implementation, with
restore-at-instruction and private-state regressions. Other unported gameplay
and action controllers remain guarded; this does not enable general search.

New source-verified stat corpus: 134 cases / 28,675 queries, SHA-256:
7c8768ef68aeaef25b8da643a698f8837d67929f3a51508eb0e9fb814d581450.
The actual release worker matches every scalar/card/minion-defense result.
Added 17 full adjacency handler cases / 18 operations, including literal/context
IDs, one-way isolation, immunity exceptions, off-board/unknown endpoints and
instruction prefixes. Actual release worker matches full state/request/events.
Main corpus: 2,220 cases / 8,888 operations, SHA-256:
a76430a1fc10dd2d5e3f04d2a0ff1cd3e3392def789c9ec9258c1e3f7f238c96.

Reference defect discovered: radius immunity combined with topology can recurse
until Python raises RecursionError. Preserved one source-verified rejected case
in effect-errors.json. Native scope resolution returns an explicit unsupported
error at bounded recursion depth; this case is not counted as a correct gameplay
transition. Do not silently repair/re-pin the frozen Python reference.

All 60 focused Python oracle/manager tests, strict Clippy and exporter linting
pass. The full locked release workspace regression passed all 119 native tests,
including all 2,220 handler cases / 8,888 operations (CLI comparison 128.00 seconds).
Rust formatting and whitespace checks also pass. No NN tests,
Python engine changes, live restarts, commits or pushes in this slice.

Remaining objective is unchanged: other gameplay effect consumers, finishing
scheduling and remaining card/passive/spell rules; aggregate aliases; make/unmake
and hashing; evaluation, iterative team-aware search, hidden/simultaneous
planning and parallelism; optional validated integration; full-game strength and
1,000 full two-turn simulations/sec. Completion is not established by these
bounded rule/transition correctness tests.


### 2026-10-24: native displacement legality and real placement/swap

Added typed validation.rs with shared can_displace verdicts corresponding to
Python can_be_moved/pushed/swapped/placed. Native queries consume typed effect
behavior, not JSON: placement-prevention category, live timing/scope, immunity,
requested displacement type, and source/actor team and self flags. Preserve the
source's evaluation order and first blocking effect ID/reason/source. Off-board
entities bypass effect checks; missing entities/occupied placement destinations
produce the exact plain denial. Token immunity/controller and raw actor-team
semantics remain distinct as in the source.

The displacement_legality worker operation compiles one world for a query batch
and emits full diagnostic verdicts only at the boundary. This is displacement
restriction checking, not full path/action legality. Move/push path/terrain,
action availability and passive-consumer closures remain unfinished.

Enabled PlaceUnitStep, SwapUnitsStep and primitive SetActorStep in guarded
resolution roots containing active effects. Placement preserves occupancy
prechecks, missing/skip targets, optional/mandatory failure and token events;
swap validates both participants before either occupancy mutation. Actor false
values use Python truthiness: placement falls back to the target, swap to literal
system, token event actors become null, and SetActor saves null for empty actors.
Live/fixed/multi-piece effect origins are resolved after each mutation/restore.

Source-verified displacement-validation.json: 159 cases / 85,701 full verdicts,
SHA-256 b77bcc19d464633b0922593fdb2a2d3f38a6b07d99a79d64a99f5007890e7f9d.
Actual release worker matches all allowed flags, reasons, blocker/source IDs.
Added 318 real placement/swap cases / 636 operations, SHA-256:
4a5957f32a96b2126fb3485e8b59ff3116970945670de5ddb72fa52a49bc6c1f.
Actual worker matches complete state/request/event records, including prefixes.
Added native full-verdict parity, private-origin workers, instruction restoration
and blocked-second-swap-participant atomicity tests. Canonical corpus now has
2,538 cases / 9,524 operations, SHA-256:
2fc6967645d5c8bd59430d2a625120a202d28b527f23d020efbfa6fd1475d912.
The full locked release workspace regression passed all 124 native tests,
including all 2,538 transition cases / 9,524 operations (CLI comparison 110.49
seconds). Strict Clippy, Rust formatting, whitespace and Python formatting/lint
checks pass, as do all 60 focused oracle/EffectManager Python tests. Also passed all
27 selected reference validation/displacement/placement rules tests (87 Python
tests total in this slice).

No Python engine changes, NN tests, live service restarts, commits or pushes.
The full engine/search objective remains unchanged and unfinished: other effect
consumers and finishing orchestration, remaining card/passive/spell rules,
aggregate aliases, make/unmake and hashing, evaluation, iterative team-aware
search, hidden/simultaneous planning, parallelism, validated integration,
strength gates and 1,000 full two-turn simulations/sec. General transition/search
flags remain false.


### 2026-10-24: native action restrictions, repeat prevention and movement caps

Added native action/repeat/movement verdicts using typed immutable effect
payloads and each worker's live position/context. Preserve replacement-action
prerequisites, effect-before-card-hook precedence, explicit card-color exceptions,
first blocking effect metadata, bound owner identity and unbound any-piece
permission. Movement caps preserve raw board-presence requirements, action-only
flags, negative/zero maxima and stable first-winner ties. The action_legality
batch diagnostic compiles a world once and exports JSON only at the boundary.

Ported Witching Hour's card-specific prevention hook, with source-derived hook
catalog coverage. Ordinary negative attacks remain legal: reference cases compute
and offer ATTACK values -16 and -14, and native tests require negative-valued
options to remain present. Witching Hour explicitly prohibits negative attacks
within its radius; that exception is not generalized. If Python blocks negative
attacks without a specific rule, record a reference defect rather than reproducing
it. No such general prohibition appeared in these reference cases.

Action-menu/recipe diagnostics can evaluate active effects. Actual general
ResolveCard execution with effects remains guarded pending movement, targeting,
passive and other consumers. Enabled real ValidateRepeatStep inside the bounded
effect-aware resolution closure, retaining scalar AND behavior and context order.
Added per-instruction restore and worker-isolation tests with moving sources.
The aggregate context-alias gate remains in place.

Source-verified action-validation.json contains 198 states / 306,816 complete
verdicts and 31 menus / 1,169 menu/action-recipe operations. SHA-256:
bd8ceee56244fc816ca6bf24abfd67dfe885d2def968fd352299e0e56fde4c6c.
175 real repeat-validation cases / 350 operations are now in the canonical
transition corpus (2,713 cases / 9,874 operations), SHA-256:
432e5cb1557be5421c6f832ed42893733ef775117bb2a36b9d0eac76d750d67a.
The standalone transition subset SHA-256 is:
30d65463649dfa3584e64b94f0b8b6a1622f37c7fc72fca34580f207c396475e.

New source cases exposed an origin-zone lookup mismatch: existing concrete tile
zone IDs (including null) take precedence over membership fallback at both scope
ends. Fixed it; effect-stats.json now covers 136 states / 29,103 queries, SHA-256:
f1fa19af6b73f2f6b28535f9e3641bc623f9f2206ff867ad72545c33da01ebe5.
Release-worker checks match every new verdict/menu/transition and all 85,701
previous displacement verdicts. The full locked Rust workspace regression passed
128 tests; complete CLI transition comparison took 104.10 seconds. Strengthened
negative-value assertions passed a further focused test. All 87 focused Python
oracle/rules tests, strict Clippy, format/lint and whitespace checks pass.

No frozen Python rule changes, NN tests, live service restarts, commits or pushes.
The full objective remains unfinished: remaining effect consumers and finishing
scheduling, remaining card/passive/spell behavior, aggregate aliases, make/unmake
and hashing, evaluation and iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning and parallelism, optional validated integration,
strength gates and 1,000 complete two-turn simulations/sec. General engine/search
capabilities remain false; these correctness timings are not search throughput.


### 2026-10-24: actor terrain, stateful walking and real movement with effects

Added native terrain.rs: actor-relative static barriers, enemy empty-space
obstacles, petrified occupied terrain, passable token lookup and token supply
fallback. Preserve raw source/actor teams, timing/immunity, separate barrier and
scope origins, fixed-scope exceptions and entity-versus-pool precedence. Existing
map tiles and virtual missing tiles retain the Python board contracts. A new
reference case exposed the virtual missing-tile terrain behavior; native terrain
queries now match it instead of incorrectly treating missing tiles as non-terrain.

Native movement path/reachability batches share one BFS per query. Every edge
must respect topology connectivity and visibility from the movement's original
space, so ZERO cannot bridge a complete crossing of a reality split. Preserve
explicit empty topology identity lists, actors that differ from moved units,
off-map origins, final-obstacle exemptions and passable intermediate tokens.
A moved token cannot traverse a mine when moved identity is provided, matching
rule version 9's validator. The batch converts no JSON in its traversal and
retains no query cache after a mutation.

Real MoveUnitStep now composes displacement permission, optional action
permission, movement caps, topology-aware paths, straight-line checks and
actor-specific crossed obstacles. Preserve mandatory-abort versus optional
continuation, ordinary invalid-path no-ops, success/context behavior, live source
movement and exact movement events. Enabled direct MoveUnit within the bounded
active-effect resolution closure; full movement sequences, selection filters,
other effect consumers and mine-path choice/trigger payloads remain separate
work. Actual movement still rejects on-board passable tokens, including tokens
found only in supply, until the mine execution closure is ported.

terrain-validation.json: 196 states / 42,728 queries / 183,848 scalar answers,
SHA-256 0c3ab5625215eccff4d9c38b347105a54de2a764ba0b1d7a545c3bfc76a094da.
756 full movement cases / 1,512 operations, standalone SHA-256:
25fb9ac6196acce721492f39e9cf37d6332950c83464d0a0c326a35e15f5c9b5.
Export asserts actual successful ordinary moves and mandatory/optional abort
outcomes. The release worker matches complete states/events, including reality
crossing attempts and straight moves. Native tests add full query parity, live
private barrier origins, mutation-fresh batches, a passable-token corridor,
supply-only mine guards and restoration at every instruction for 68 traces.
Canonical corpus now contains 3,469 cases / 11,386 operations, SHA-256:
e2f006255c3266c0937d9ed557e382fc1e400ad96081f58db64ab0ecc50e2f98.

Focused reference suite passes 166 Python oracle, rules, barrier, terrain,
topology and mine-pathfinding tests. Strict Clippy and formatting/lint checks
pass. No frozen Python engine changes, NN tests, live service restarts, commits
or pushes. Full locked release workspace regression passed all 134 native tests,
including all 3,469 full handler cases / 11,386 operations. The complete CLI
comparison took 112.36 seconds; this is a correctness-boundary workload, not
search throughput. All prior action/stat/displacement/model/geometry/planning
corpora and the new terrain/restore tests pass.

The full engine/search objective remains unchanged: remaining gameplay effect
consumers, mine/finishing scheduling, card/passive/spell behavior, aggregate
aliases, make/unmake and hashing, evaluator and iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning and parallelism, optional
validated integration, strength tests and 1,000 full two-turn simulations/sec.
General game/search flags remain false. These correctness checks do not establish
full-game execution or search throughput.


### 2026-10-24: movement sequences, hex filters and controlled input

MoveSequenceStep now combines computed movement stats, scoped movement caps,
card auras and movement-aura-zone grants at expansion. Freeze effective range
and pass-through permission into generated selection/movement frames and preserve
them across continuation export/restore. MoveUnit still checks current
movement/displacement permission at execution. Preselected destinations, optional
skips, straight/full-distance constraints and invalid/retried choices match the
source. Actual mine choice/trigger execution remains separately gated before a
movement sequence can issue an unexecutable prompt.

Ported the existing compiled HEX filter set's effect consumers: actor obstacles,
movement paths with explicit moved-unit identities, topology-aware range,
adjacency, spawn adjacency and straight-line predicates. Physical/geometric
ranges deliberately bypass topology. Inverse straight-line predicates negate
the complete topology-aware condition. Tide of Darkness uses the source's
unlocked ultimate/static-effect-ID test, including facedown ultimates. Pure
filter/path scans can inspect passable tokens without running mine payloads.
Unit-targeting/card selections and other unported filter families remain gates.

Added common CONTROL_NEXT_ACTION request routing using the exact actor,
resolution-owner, original turn-card, scope-origin and explicit active-flag
checks. Do not substitute generic temporal activity: the frozen source uses
only the control row's is_active flag here. First matching effect wins. The
native request owner becomes the controller, but choices/legality and discard
actor metadata retain the original hero; wire context adds controlled_hero_id.
Request identity and routing reconstruct after restore. Bounded effect-aware
roots now admit HEX/NUMBER selections and confirmation prompts. The existing
compile-time Fast Travel filter guard remains authoritative; an initially
added duplicate check was removed rather than maintained alongside it.

hex-filters.json: 201 states / 19,497 scans / 1,208,717 candidate verdicts,
SHA-256 b19d529d1d577aab8194dbdf6dc47f199c5da07d3cd359c7a96bfc673ee932aa.
movement_sequence_traces.py adds 1,045 real handler cases / 5,347 operations,
including 64 controlled-prompt cases / 352 operations. Standalone SHA-256:
2431d2c4c5214cdd092fb19e8d048920aea548550d21703136deaa10dbfa7e30.
All new filter and full state/request/event records match the release worker.
Success/skip source assertions reproduce their frozen cases exactly when using
the generator's identical deepcopy boundary (zone set iteration order is
preserved rather than sorted). Native tests add restoration of 51 sequence and
64 controlled-prompt traces at every instruction, live source movement after
aura expansion, worker-private control ownership/original metadata, explicit
geometric/inverse-line rules and early unsupported-filter rejection.

Canonical transition corpus: 4,514 cases / 16,733 operations, SHA-256:
e35a4b118d1bed6254ad44fa5dda4dd4f2486b46747193fe70242d772c401de3.
All 290 focused Python oracle, movement, topology, battle-zone and Hanu effect
tests pass. Strict Clippy and Python lint/format checks pass. The complete locked
release workspace regression passed all 142 native tests, including every one
of the 4,514 transition cases / 16,733 operations. Full CLI comparison took
161.39 seconds; this is a correctness-boundary timing, not search throughput.
All prior model/stat/displacement/planning/movement corpora and the new
filter/restoration tests pass.

No frozen Python rule changes, NN tests, live restarts, commits or pushes.
The complete engine/search objective remains unfinished: mine paths/triggers,
remaining targeting/gameplay/phase/finishing consumers, remaining card/passive/
spell rules, aggregate context aliases, make/unmake and hashing, evaluator,
iterative team-aware minimax/alpha-beta, hidden/simultaneous planning,
parallelism, optional validated integration, strength gates and 1,000 complete
two-turn simulations/sec. General game/search flags remain false; these
correctness checks do not establish full-game execution or search throughput.


### 2026-10-24: standalone mine paths and explicit trigger closure

Ported one-pass mine-aware BFS with shared immutable sets and parent-linked
paths. Preserve per-edge source topology (distinct from ordinary movement's
original-space restriction), actor obstacles, raw moving identity/team and
entity-first passable supply fallback. Stable entity/supply ranks prevent
symbol-interning history changing native set order. No cached result crosses a
world mutation. Exact path/option ordering and mathematical sets match 920
queries across 23 states.

Ported standalone MinePathChoice and explicit TriggerMine instructions. Choice
prompts preserve raw string IDs, nested source wire metadata and invalid-input
retry identity. Ordered triggers preserve literal/context precedence, arbitrary
ID removal, token-linked effect expiration, event order, rollback flags and
forced-discard chains, including duplicate/off-board blast source quirks.
World-aware linking compiles discard consumers only for possible blast queues;
dud-only active-effect queues avoid unreachable combat dependencies. Possible
blast queues with active/potential effects reject before mutation until the
effect-aware discard/shield/passive closure is implemented. Continuations carry
victim/source bindings explicitly across export/restore, and workers keep their
mine choices private.

The source-reproduced mine-reference-errors fixture records implicit multi-mine
hash-order nondeterminism and generic numeric option-ID conversion rejected by
MinePathChoice. Choice-only fixtures normalize declared mathematical sets;
explicit trigger queues/events and all option/path order remain exact. Frozen
Python sources and their pin are unchanged. Ordinary negative attacks remain
legal; a general Python prohibition is a reference defect, not a porting target.

Automatic MoveUnit detection and mine-choice/requeue integration remain gated.
Remaining gameplay/phase/finishing consumers, card/passive/spell rules, aggregate
aliases, make/unmake and hashing, evaluator, iterative team-aware minimax,
hidden/simultaneous planning, parallelism, validated integration, strength gates
and 1,000 complete two-game-turn simulations/sec remain unfinished. General
game/search capability flags remain false. No live services were restarted.


Latest mine verification: all 150 locked release workspace tests pass, including
4,631 complete transition cases / 17,261 operations. Canonical corpus SHA-256:
6fa1afe326791dadc91163e24909172ef7986129c01abd505e5f9db5471211be.
The full CLI comparison took 154.63 seconds; this measures correctness-boundary
work, not search throughput. All 307 focused Python oracle/rules/effect tests,
strict Clippy, Python lint and formatting checks pass. Automatic movement/mine
integration and the full engine/search requirements remain unfinished.


### 2026-10-24: automatic movement/mine scheduling

MoveUnit now integrates the mine BFS and explicit trigger closure: source supply
ownership detection, straight crossings, flexible path choices, requeue with a
step-scoped prechosen flag, relocated-unit events and literal victim/queue
snapshots for TriggerMine. Frozen sequence range/unit bindings and fresh runtime
copy identities survive export/restore. Imported labels remain explicit, while
generated copied frames receive independent ephemeral identities. A second move
still detects its own mines. Ordinary movement no longer blanket-rejects all
on-board passable tokens, including supply-only identities.

Link potential blast discards from deployed entities, explicit prechosen queues
and potential placements; do not compile irrelevant off-board roster blast
consumers for ordinary dud-only effect roots. Runtime uncompiled blast checks
remain before relocation. Effect-aware blast discard/shield/passive rules are
still gated. General game/search capability flags remain false.

Source fixture coverage adds 66 cases / 396 operations, including exact ordered
multi-mine straight/explicit queues. Flexible detected paths contain at most one
enemy mine, avoiding the source's known implicit hash-order nondeterminism;
ordered events are never normalized away. Three source defects now reproduce in
mine-reference-errors.json: the previous hash-order and numeric-helper failures,
and literal destination loss when generating a mine path choice. The latter is
an explicit compatibility fixture. Frozen Python source and pin are unchanged.

Remaining scope still includes effect-aware mine discards, other gameplay/phase/
finishing consumers, remaining card/passive/spell rules, aggregate context alias
arena, make/unmake plus hashing, evaluation, iterative team-aware minimax,
hidden/simultaneous planning, parallelism, validated integration, strength gates
and 1,000 complete two-game-turn simulations/sec. NN tests remain excluded;
live lab/backend/bot services are untouched.


Automatic movement verification: all 153 locked release workspace tests pass,
including 4,697 full transition cases / 17,657 operations. Canonical SHA-256:
afb2455e6068ae5a945acbda59570931cba40ff6031dc4eb649f10f486b5bac3.
Full CLI comparison took 151.26 seconds; this is correctness-boundary work,
not search throughput. All 307 focused Python oracle/rules/effect tests pass,
including reproduction of the three mine reference defects. Strict Rust/Python
lint, formatting and whitespace checks pass. No NN tests, frozen Python source
changes, live restarts, commits or pushes. Full engine/search requirements and
effect-aware blast discard closure remain unfinished.


### 2026-10-24: effect-aware forced discard and shield closure

Ported creator-owned active discard-shield lookup, exact played/current ordering
and printed-ID deduplication, forced-discard actor immunity, eligible-hand-first
behavior and filtered shield alternatives. Basic/nonbasic forced selections
snapshot the eligible hand IDs plus shield IDs, disable the basic predicate for
shield alternatives, and serialize that state with the generated selection.
Unfiltered selections retain live hand behavior. Discard can find the selected
played/current shield even from nominal HAND, record public reveal/discard
history/context and expire all linked effects prematurely without finishing
steps. Card flags and physical aliases remain worker-local.

Blast discards now admit supported existing active effects. Compile passive
providers only when an eligible offer requires them; unrelated active effects
or grants for other triggers no longer force unreachable combat compilation.
Provider eligibility preserves creator/active/card/trigger/usage rules and
ordinary played/ultimate seen-ID precedence. Eligible unported passive bodies
remain explicit, with preflight for supported initial roots. Dynamically created
blast-effect closures remain gated, as do complete reaction windows and other
unported gameplay/phase/finishing consumers.

Reference coverage adds 103 real handler cases / 680 operations, SHA-256:
94c596b194f80ede9896ad7e78107236175d6d5145f258f5d28ccc4b436d6182.
Registered Mrak physical cards cover hand/shield choices, filtered hand snapshots,
current/facedown/duplicate/multiple shields, inactive/old-active/off-scope rows,
empty hands, immunity, linked expiry and explicit/movement blast chains. Source
assertions prove nonbasic shields can absorb basic-card discards without touching
the hand or running finishing steps, and grants are offered only for matching
triggers. NN tests, source modifications, live restarts, commits and pushes are
excluded. Ordinary negative attacks remain legal; general Python prohibition
would be a reference defect.

Full scope remains open: eligible passive/remaining card/spell bodies, complete
reaction/combat/phase/finishing consumers and dynamic effect closure, aggregate
context aliases, make/unmake and hashing, evaluation, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallelism, validated
integration, strength gates and 1,000 complete two-game-turn simulations/sec.


Effect-aware discard verification: all 158 locked release workspace tests pass,
including 4,800 full transition cases / 18,337 operations. Canonical SHA-256:
5983b21fe85936de3500fa767da69b7c1b4cde880f6f35e98b2a64449025b656.
Full CLI comparison took 180.31 seconds; this is correctness-boundary execution,
not search throughput. All 353 focused Python oracle/rules/effect tests pass,
including the real Mrak shield tests. Strict Rust/Python lint, formatting and
patch whitespace checks pass. Frozen Python sources, their pin and live services
are unchanged. No NN tests, commits or pushes. The full engine/search objective,
eligible unported passive bodies and dynamic blast-effect closure remain open.


### 2026-10-24: granted provider execution and Wild Hunt

Added live provider IDs to passive offers/usage markers and typed mutable effect
usage/grant fields. Scanning preserves source creator/active/grant/card/usage
and seen-card rules; executing a waiting offer re-resolves the current provider
and global carrier card. Copied abilities use their creator, not the physical
card owner. Explicit markers require only matching provider/card IDs, including
inactive rows and absent global carriers. Provider usage never increments the
foreign physical card and remains private across search-worker clones. Future
make/unmake and hashing must track this mutable effect counter independently of
the immutable definition payload. Hero finalization's counter reset includes
its own provider rows; full active-effect phase execution remains a separate gate.

Refactored body dispatch into immutable per-effect/per-owner programs compiled
only for relevant trigger families. Wild Hunt's exact source recipe is frozen
from its registered get_passive_steps method. It performs an optional exact
straight two-space selection and effect-side move, with no movement-action aura
inheritance. Generated frames retain literal owner origins and restore them at
instruction boundaries. Other eligible bodies remain guarded, instead of fabricating an empty passive result. The subsequent combat milestone
adds active-effect Big Sodding Gun support.

The provider corpus contributes 202 complete handler cases / 1,267 operations,
SHA-256 74700ceaff60f72f2d366dcf8a680bd66fd8a29a74a814a91314b854158cdea2.
Coverage includes copied/facedown foreign carriers, direct/scanned offers,
active/scope/creator/card flags, unlimited and negative usage, deduplication,
missing actors/positions, exact movement, action-only/non-action caps, mine
crossing and provider removal before marking. All full worker records match
Python. Source assertions prove creator ownership, independent usage and
inactive/missing-carrier marking; no NN packages or tests are involved.

Full scope remains open: remaining passive/card/spell bodies, complete dynamic
active-effect gameplay/combat/phase/finishing consumers, aggregate context arena,
make/unmake plus hashing, evaluator, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallelism, validated integration, strength gates
and 1,000 complete two-game-turn simulations/sec. These are correctness checks,
not evidence for full-game/search completion or throughput. Live lab/backend/bot
services and frozen Python engine sources remain unchanged.


Provider/Wild Hunt verification: all 162 locked release workspace tests pass,
including 5,002 full transition cases / 19,604 operations. Canonical SHA-256:
2959d73e4ead1d8a9c7d344f9a88706b4a848b13f04ff9f9c7cc44135bacd03b.
Full CLI comparison took 154.59 seconds; this measures correctness-boundary
serialization/execution, not search throughput. All 359 focused Python oracle,
rules and real card-effect tests pass, including Mrak and Silverarrow tests.
Strict Rust/Python lint and formatting checks pass. Frozen Python source/pin,
live services and NN dependencies are unchanged; no commits or pushes. The full
engine/search objective, remaining passive/card/spell bodies and complete
active-effect phase/combat/dynamic-effect closure remain unfinished.


### 2026-10-24: effect-aware defeat, minion saves, bounties and granted Gun

Ported source/card ownership expiry on defeat without finishing steps, preserving
all token-bound rows (including an empty non-null token ID). Minion protection
preempts defeat mutation and prioritizes mandatory totem sacrifice. Card-save
prompts re-evaluate live providers and qualifying hands; accept/skip/invalid
results retain Python's different scoring/removal/bounty paths. Retry frames
carry only minion, killer and tried IDs, so imported pending flags do not leak
into a different provider. Granted Big Sodding Gun can execute hand-only forced
discard or defeat; provider removal before marking remains visible.

Bounty beneficiary is the existing subject-or-source property, not a separately
serialized protected_unit_id. Payout uses live positions, team and topology,
with typed mutable per-row max_value. Deduplicated creation cannot reset spent
coins. Future make/unmake and hashing must track this counter along with passive
usage. Empty-stack game-over states with effects can restore. Remaining illusion
minion-defense consumers reject explicitly; complete attack/reaction and phase
consumers remain unfinished.

Added 132 frozen Python handler cases / 542 ordered operations, subset SHA-256
71639d8299f0ef59aea9f77f805a88337b772d3dc22df24102bf3d36db0ef7bc.
Coverage includes dynamic creation, all minion types, source/card/token ownership,
stale/declined/imported protection choices, live provider removal, bounty
exhaustion/topology/order, terminal restore and signed negative combat.
Explicit source assertions confirm ordinary negative attacks are legal; no
general negative-attack Python defect appeared. All 433 focused non-NN Python
oracle/rules/card-effect tests pass.

The full engine/search objective remains active. Outstanding requirements include
remaining card/passive/spell bodies, complete effects/phase/finishing semantics,
aggregate aliases, make/unmake and hashing, evaluator, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallel workers, validated
integration, strength gates and 1,000 complete two-game-turn simulations/sec.
Frozen Python engine source/pin and live services are unchanged.


Fixture publication now keeps superseded immutable snapshots. A full verification
run exposed that pruning after an index swap could delete a file still needed by
a reader holding the previous index. Export/merge regression tests now require
both old readers and the new index to remain readable; reclamation belongs in
an explicitly offline cleanup. No active fixtures are pruned during publication.


Effect-aware combat verification: all 164 locked release workspace tests pass,
including all 5,134 complete handler traces / 20,146 operations. Canonical index
SHA-256: 2bfec22f0ae454a3039d23b91e1de67cc76722d138291cf1b3fa55ffd1453bc3.
Independent full CLI state/request/event comparison also passes (409.97 seconds
while running the native suite concurrently); this is correctness-boundary
serialization/execution, not a search-throughput measurement. All 433 focused
Python tests pass, including negative combat, real Brogan/Swift/Sabina effects
and safe snapshot publication. Strict Clippy, Rust formatting, Black/Ruff and
patch whitespace checks pass. Frozen source/pin and live lab/backend/bot services
are unchanged. NN tests/dependencies remain excluded; no commits or pushes.
The complete engine/search objective remains unfinished with the scope above.


### 2026-10-25: actor-bound illusion defense and effect-aware reaction windows

Ported illusion-minion defense using the live actor's canonical hero identity,
provider source/direct-active flag, connected first ring and source team. No
hardcoded hero, token owner, temporal/scope/subject restriction or second-ring
bonus is introduced. All combat, reaction and offline diagnostic consumers pass
the actor explicitly, so worker branches and temporary defense actors remain
independent without duplicate cached actor state.

Reaction menus now append active played/current shield cards in source order and
compute stats with the actual offered physical card, including basic-action and
Initiative bonuses. Accepting a shield uses ordinary discard/expiry/passive
handling; a stale provider reopens the reaction prompt. Supported primary defense
flags, marker bodies, combat outcomes and actor restoration work with effects.
Body programs link projectile dependencies only when a card can reach that
on-block consumer. Bounded active-effect roots exclude locked upgrades; ordinary
phase roots still include all possible future upgrades. Relevant active-effect
projectile targeting remains guarded rather than replaced by a no-op.

Added 200 frozen real-handler cases / 1,425 operations; subset SHA-256
23f95fc058c7e4fbecf1ee3a3c360a350f967841b0fd292fa98614f6d96f82c8.
Coverage spans all 33 hero decks with Defense/Initiative effects, shield paths,
source/piece/actor/topology/ring variants and complete supported Arien, Wasp and
Bain defense chains. Source tests assert selected-card bonuses and live shield
invalidation. Native tests restore every new case and isolate actors, cards,
providers and pending choices between workers.

Full objective remains open: complete effect-aware targeting/attacks/projectile
reactions, remaining card/passive/spell/phase/finishing bodies, aggregate aliases,
make/unmake, hashing, evaluator, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallelism, validated integration, strength gates
and 1,000 complete two-game-turn simulations/sec. This milestone is correctness
coverage, not proof of full-game support or search throughput.


Illusion/reaction verification: all 168 locked release workspace tests pass,
including every canonical handler transition and instruction restoration for all
200 new cases. All 600 focused non-NN Python oracle/rules/card-effect tests pass.
The production passive-catalog comparison runs in an isolated process so effects
registered by test modules during collection cannot pollute the source catalog.
The private shield/pass test observes the discard before a later passed attack
correctly defeats the hero and expires its remaining card-owned effects.

Full CLI comparison passes all 5,334 cases / 21,571 operations. Canonical index
SHA-256: 1c602cb333606d1ef140fbe1765a5f4db600c768c671f53bf857e15e2fab1cba.
It took 427.99 seconds alongside the native suite; this measures correctness
serialization/execution, not search throughput. Strict Clippy, Rust formatting,
Black/Ruff and whitespace checks pass. Frozen Python source/pin and live services
are unchanged; no NN tests/dependencies, commits or pushes. Full engine/search
capability flags remain false and the complete objective remains unfinished.


### 2026-10-25: effect-aware targeting, resumable pre-actions and projectile attacks

Implemented source-specific ordered candidate pools, illusion type/team handling,
LOS targeting, actor/attack immunity, movement/placement predicates and topology
identity propagation. Intrinsic validation uses the override actor but predicate
filters retain the current actor; unit tokens bypass intrinsic validation. Single
UNIT selectors append Illusions, while multi/query UNIT pools do not. Source
activation distinctions (direct flags versus validator timing) are preserved.

Replaced the pre-action no-op stubs needed by full attacks. Movement children
come from the real Python resolver and bind typed hero/range values and exact
prompts; source fields restore without opaque custom gameplay metadata.
Disruptors keep processed IDs on the same requeued frame, consume matching rows
without finishing steps, update carrier activation, and re-evaluate hand, scope,
immunity and board presence. Empty-hand defeat credits the provider; defender
abort policy preserves the attacker. Existing Reflect/Deflect targeting now runs
with active effects. Full attack traces cover movement, leaving scope, linked
expiry, multiple costs, defeat and mine discards during a defense chain.

Frozen source batch: 66 states / 29,780 complete results, SHA-256
a12410da81f268fdce8c63b7138c0ad51861714e82a823e415449dea2853e4e3.
Added 482 real-handler traces / 2,902 operations, SHA-256
69dc87592a7d2680f03dd5ac295e2953a1ab450a3059aac129400888dde4119e.
All these source/native comparisons pass. Native tests add full batch comparison,
private worlds, instruction restoration and forked pending-disruptor continuations.

The original objective remains active: remaining card/passive/spell/filter
implementations, all phase/dynamic-effect/finishing semantics, aggregate aliases,
make/unmake and hashing, evaluator, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel workers, validated integration, strength
gates and 1,000 complete two-game-turn simulations/sec. Full-game/search flags
remain false; source-comparison timings are not throughput claims.


Targeting/pre-action verification: all 171 locked release workspace tests pass,
including every canonical transition and instruction restoration for all 482 new
cases. All 657 focused non-NN Python oracle/rules/card-effect tests pass. The
66-state / 29,780-result targeting batch also matches the native worker exactly.
Generic effect creation already preserves the pre-action payload fields; no
special-case compiler copy or frozen Python engine edit was needed.

The full independent CLI comparison passes 5,816 cases / 24,473 operations.
Canonical index SHA-256:
890b01218bb6efe0ab8482a9f383d4e17305a1aac2456e5e9e72c81e69049659.
It took 395.44 seconds alongside native verification; this is correctness-boundary
serialization/execution, not search throughput. Strict Clippy, Rust formatting,
Black/Ruff and patch whitespace checks pass. The frozen source/pin and live
lab/backend/bot services are unchanged; no NN tests/dependencies, commits or
pushes. Ordinary negative attacks remain legal. The full engine/search objective
and all remaining gates above remain open.


## Effect-aware automatic minion return

The native return instruction now handles active effects using the existing typed
legality, terrain and topology rules. Walking destinations check displacement
permission and non-action movement caps; traversal uses the minion's actor-aware
obstacles, passable tokens and start-to-each-space connectivity. If no legal walk
exists, placement searches through connected map spaces and checks the destination's
placement permission. The current actor governs displacement restrictions, with
missing/empty actors falling back to the returning minion. Bare tile obstacles
still govern landing candidates, matching the source's intentional distinction.
The generated move reuses the BFS path length rather than rerunning shortest-path
queries. Uncompiled card aura restrictions remain explicit failures.

`scripts/rust_oracle/minion_return_effect_traces.py` adds 238 real-handler cases /
695 operations, including displacement/immunity scopes and duration variants,
movement caps with action-only toggles and missing/empty actors, topology, terrain,
barriers and passable-token variants. Four forced corridors assert actual relocation
and distinguish walking from placement when a token blocks the only route. All
238 cases match the release CLI's full state, requests and event order. Permanent
native checks restore every instruction and fork independent waiting return choices.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/minion_return_effect_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/minion_return_effect_traces.py verify -- "$worker"
```

This removes one turn-finalization dependency; full effect-aware phase orchestration
still requires scheduled expiry/finishing and remaining gameplay consumers.
The full engine/search, alias arena, make/unmake and hashing, evaluator, hidden and
simultaneous planning, iterative team-aware search, parallel workers, validated
integration, strength gates and 1,000 complete two-game-turn simulations/sec remain
required. Negative attacks remain legal. The frozen Python source is unchanged;
NN tests/dependencies and live lab/backend/bot processes are untouched.


Effect-aware return verification: all 172 locked release workspace tests pass,
including complete state/request/event comparison for 6,054 canonical cases /
25,168 operations and every-instruction restoration of the new return cases.
The full CLI regression test finished in 184.03 seconds; this is correctness
verification, not search throughput. The separate release-worker comparison
passes all 238 new cases / 695 operations. All 657 focused non-NN Python tests,
strict Clippy, Rust formatting, Black/Ruff and patch whitespace checks pass.
Canonical index SHA-256:
91e5f72204b7fdf44348c9722e1a7f63294c6a3f86599e26aac7276e90bf3470.
The standalone return corpus SHA-256 is
7eeb1a14310352dca879b91bcff1c9defbbe905beec28e1b085365f30c5f0932.
Full engine/search completion and performance remain unproven and open.


## Effect-aware minion battle and lane pushes

Battle primitives now run with active effects in resolution and cleanup phases;
immediate cleanup, upgrades and reset are implemented below. Scheduled turn/end-phase
finishing orchestration remains guarded. Native counting implements MINION_BATTLE_EXCLUSION:
sum caps by raw source ID in effect order, find adjacent enemy minions connected
to that source, and reserve excluded IDs per enemy team across sources. This
consumer deliberately uses the stored active flag and raw entity team/position;
it does not apply temporal expiry, effect scope, subject or immunity filtering.
Overlapping origins can therefore produce different totals when effect order
changes. Nonpositive summed caps do nothing. One Man Army remains additive.

Lane pushes remove token-, source- and scope-origin-linked effects when a directly
spawned minion overwrites a placed token. This uses the existing token-removal
semantics without inventing events, finishing payloads or carrier-card resets.
Blocked-spawn displacement searches connected neighbors without an actor or an
origin-to-every-space constraint. It offers bare empty landing spaces first,
then the generated mandatory Place instruction validates the chosen destination;
placement prevention can abort the remaining steps, matching Python.

`scripts/rust_oracle/battle_effect_traces.py` adds 322 real-handler cases / 1,375
operations. These rerun the original 76 battle/lane/spawn scenarios with active
effects, check three kinds of linked token effects, cover 159 displacement-rule
states, and exercise exclusion caps, summed/overlapping origins, scope/duration,
raw-team/token ownership, immunity, topology and One Man Army. All new records
match the release worker's complete state, requests and event order. Source-only
contract tests pin the order-sensitive overlap counts and direct activation rule.
Native tests cover restoration after every generated instruction and independent
casualty choices in cloned workers sharing immutable definitions.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/battle_effect_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/battle_effect_traces.py verify -- "$worker"
```

This removes the battle dependency of effect-aware phase progression. Remaining
work still includes scheduled expiry/finishing, all remaining card/passive/
spell/filter bodies, aggregate context aliases, make/unmake and hashing, evaluator,
iterative team-aware minimax/alpha-beta, hidden and simultaneous planning, parallel
workers, validated integration, strength gates and 1,000 complete two-game-turn
simulations/sec. Ordinary negative attacks remain legal. The frozen Python engine
and live lab/backend/bot services are unchanged; NN tests/dependencies are excluded.


Effect-aware battle verification: all 174 locked release workspace tests pass,
including complete state/request/event comparison for 6,376 canonical cases /
26,543 operations. The full CLI regression took 188.83 seconds; it measures
correctness-boundary replay, not search throughput. The independent release-worker
comparison passes all 322 new cases / 1,375 operations. All 758 focused non-NN
Python rules/oracle/card-effect tests pass, including Wuk exclusions, end phase,
lane pushes, lane-bound respawns and double-lane endgames. Strict Clippy,
Rust formatting, Black/Ruff and patch whitespace checks pass.
Canonical index SHA-256:
a22adaf54a29ac948f6ad441407064e6d16b48427fd5634791214461dfa7f8cb.
Standalone battle corpus SHA-256:
496ac2c27a1b253901f365093fa562361b1d22f8654d7f42588e33b69e53bb37.
The frozen source/pin remains unchanged. No live-service restarts, NN dependencies,
commits or pushes were needed. The full engine/search goal and all remaining
requirements above remain open.


## Immediate effect-aware round cleanup, upgrades and reset

EndPhaseCleanup now composes the native lifecycle consumers in source order:
expire THIS_ROUND effects, reset markers, sweep stale effects, expire effects
linked to played/current cards, retrieve cards, clear temporary pooled tokens,
then purchase levels and handle upgrades/reset. Its second duration sweep
intentionally ignores returned finishing payloads, as Python does; it does not
own THIS_TURN expiry or scheduled end-phase finishing execution. Token-bound
effects retain their duration exemption, including a non-null empty token ID.
Card expiration still removes matching token-bound card effects when required.
Stale sweeps preserve the source's carrier-flag behavior, and discarded cards are
not treated as played/current cards during the explicit card-expiry sweep.

Each TOKEN_REMOVED event now contains the actual count of linked effects removed
at that token's position in pool order. Persistent/unpooled/unplaced tokens and
duplicate physical pool references follow the same source behavior. Effects that
survive cleanup remain in the private world throughout upgrade previews, chosen
cards, item gains, ultimate hooks and reset into planning.

The compiler now builds only reachable upgrade/reset continuations for these
active-effect phase roots, avoiding irrelevant future action/Fast Travel programs.
Ordinary full phase roots retain their full continuation set. Reaction dependency
selection accounts for cleanup/upgrade roots that can introduce deck cards. Active
effect planning boundaries can be saved/restored; commit/uncommit/pass/finish
commands still reject the unported planning effect closure before mutation.
Cleanup-only sessions also reject planning when their last effect expires but
full planning continuations were not compiled; restoring that effect-free saved
boundary compiles ordinary planning and enables its commands. Scheduled
turn/end-phase finishing remains a separate, explicit requirement.

`scripts/rust_oracle/round_effect_traces.py` adds 216 real-handler cases / 602
operations: 138 cleanup/upgrade scenarios rerun with persistent effects, plus 78
duration/activation/card-link/token/finishing cases. All registered heroes and
both upgrade faces are retained; the six map cases enter directly at cleanup,
while the existing ordinary full-flow map cases remain unchanged. All new cases
match complete release-worker state, requests and events. Source contracts verify
ignored versus retained finishing payloads, ordered token event counts and empty
token bindings. Native tests restore every instruction, fork upgrade choices and
verify planning guards reject each command without mutating persistent state.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/round_effect_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/round_effect_traces.py verify -- "$worker"
```

The full engine/search objective remains open: scheduled expiry/finishing and
remaining phase/dynamic-effect consumers, all remaining card/passive/spell/filter
bodies, aggregate context/step aliases, make/unmake and hashing, evaluator,
iterative team-aware minimax/alpha-beta, hidden and simultaneous planning,
parallel workers, optional validated integration, strength gates and 1,000 complete
two-game-turn simulations/sec. Negative attacks remain legal. No frozen Python
source change, NN dependency, live-service restart, commit or push is required.


Immediate cleanup verification: all 176 locked release workspace tests pass,
including full state/request/event comparison for 6,592 canonical cases / 27,145
operations. The final CLI replay took 199.35 seconds; this is correctness-boundary
replay, not search throughput. The independent release-worker comparison passes
all 216 new cases / 602 operations. All 760 focused non-NN Python rules/oracle
checks pass. Strict Clippy, Rust formatting, Black/Ruff and patch whitespace
checks pass. Canonical index SHA-256:
c05a749a999471caa67b1eec2105257ea7d5482516fd58bbe4529437ea0b49d8.
Standalone cleanup corpus SHA-256:
87cebeead6b5a617c71767019d777d1cc3d2d72576d26d8888da79e51e66a7f4.
Source/pin integrity is unchanged. Scheduled finishing/alias identity and all
remaining full engine/search requirements remain open. No NN dependencies,
live-service restarts, commits or pushes were introduced.


## Typed object identity arena foundation

`objects.rs` adds worker-local typed object handles, ordered list/dictionary/model
storage, shallow context snapshots, alias assignment, field updates, list append/
pop and dictionary deletion. Mutation application consumes interned symbols and
typed values; JSON is confined to import/export/diagnostic boundaries. Equal
values are never used as object identity. A cloned arena preserves sharing within
the worker and copies mutable nodes privately, including handles detached from
the visible state. Frozen models reject writes; model/step setters require an
existing field; cycle-producing assignments reject before semantic mutation.

The opt-in source exporter in `scripts/rust_oracle/shared_objects.py` records
`object_bindings`: kind plus compact-state paths for each list, dictionary and
live/frozen model object. Singleton bindings retain kind information before a
snapshot or assignment makes an object shared. Cards/tokens retain the existing
physical instance table. Tuple containers remain immutable value data while
their mutable children retain identity. This stage does not implement generic set
mutation. Validation checks paths, kinds, overlap and identical ordered contents.
Ordinary source exporters remain unchanged until runtime bindings are migrated.

`scripts/rust_oracle/object_arena_traces.py` supplies 12 real-source scenes / 49
mutations. They cover shared and independent equal lists/steps, duplicate textual
step IDs, one step shared by stack and effect finishing payloads, shared pending
input dictionaries, hero/hand/effect objects, shallow snapshots, immutable tuples,
frozen models, detached handles and dictionary insertion order. Expected values,
identity paths and ordered keys come from actual Python objects, not Rust. The
`aggregate_trace` diagnostic matches all records. Native tests also check private
forks, visible-identity boundary roundtrips and malformed/cyclic/frozen rejection.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/object_arena_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/object_arena_traces.py verify -- "$worker"
```

Live arena execution now accepts identity-bearing resolution envelopes for
SetContextFlag, AddContextValue, RecordTarget, static LogMessage, NUMBER
SelectStep, ForEach, SetActor, RestoreActionType and RestoreActionContext
instructions with recursively compiled template bodies. These sessions read/write context and step pending state
through arena handles. Dense-world compilation still rejects direct identity-
bearing imports; this closed executor uses a read-only ownership/controller
projection and guards writes through aliases to unbound world/step-operand lists.
A context dictionary that itself aliases unbound world data also rejects.
Other instructions and phase flows retain their import guards. Boundary binding
indices remain local to each import and are rebound on restore. Private cloned
checkpoints retain their handles. Dense world slots, other mutable operands,
aggregate repeats and scheduled finishing execution still need integration.

The original full engine/search scope remains open: remaining phase/dynamic-
effect consumers, card/passive/spell/filter bodies, make/unmake and hashing,
evaluator, iterative team-aware minimax/alpha-beta, hidden/simultaneous planning,
parallel workers, optional validated integration, strength gates and 1,000 complete
two-game-turn simulations/sec. Negative attacks remain legal. The frozen Python
engine and live lab/backend/bot processes are unchanged; no NN dependencies,
commits or pushes are introduced by this foundation.


### Required arena integration contract

- Use one symbol namespace per worker for arena values, native registers, world
  slots and frame operands. Runtime binding must not serialize/reparse JSON.
- Bind model/list fields to the authoritative dense storage or migrate them to
  arena storage. A second by-value copy would reintroduce the alias bug. Include
  hero collections, effect counters, marker fields and mutable step operands.
- Rebind a position slot when movement gives it a new immutable Hex value; do
  not change an older frozen object retained by a snapshot or context variable.
- Frames that represent the same physical step must share a mutable handle.
  Independent copied steps may have equal data and identical textual step IDs.
  Read live mutable operands through that handle when source semantics require it.
- Stored effect steps and executing frames must share pending-input/step data
  while rule-version-9 turn effects linger. Preserve source actor wrappers,
  FinishedExpiringEffect abort boundaries and deferred AdvanceTurn removal.
- Rebind handles on a fresh boundary import. Private cloned checkpoints retain
  their own handles, including detached nodes; exported visible binding indices
  are not persistent program identities.
- Replace aggregate repeat/snapshot guards only after shallow-reference behavior
  and source value equality are verified, including physical card/model cases.
- Promote alias-aware capture in gameplay oracle exports and revalidate full
  phase traces before removing the dense-world/frame import guards.
- Journal arena allocation/slot/link changes for make/unmake; include copied
  physical instance rows, copied-program bindings and operand-protection entries.
  Hash rule-relevant mutable alias topology, not allocation/table order, and
  exclude immutable definition data. When world/card mutation binds, physical
  deepcopy must read current runtime payloads rather than the import snapshot.
  The final performance gate remains complete two-turn game simulations, not
  aggregate mutation or serialization throughput.


Object arena verification: all 179 locked release workspace tests pass, including
all 6,592 canonical gameplay traces / 27,145 operations and the separate 12 arena
scenes / 49 mutations. The full CLI gameplay replay took 207.07 seconds; it is a
correctness replay, not search throughput. The final release worker independently
matches arena values, identity topology and ordered keys. All 762 focused non-NN
Python tests pass. Strict Clippy, Rust formatting, Black/Ruff and patch whitespace
checks pass. The gameplay index remains unchanged:
c05a749a999471caa67b1eec2105257ea7d5482516fd58bbe4529437ea0b49d8.
Arena index SHA-256:
d137bf6846416caf5b2dff0f537dcb31a5654261b561e9274343964126bcbe23.
Frozen Python source/pin and live services remain unchanged. Runtime arena
bindings, scheduled finishing and the full engine/search requirements remain
open; this storage milestone does not advertise gameplay alias or search support.
No NN dependencies, commits or pushes were introduced.


## Live shared-object scalar execution

`interpreter/object_execution.rs` binds actual execution-stack steps, context
containers and pending request/input fields to worker-local arena handles. Shared
stored effect steps observe pending-state changes immediately; independently
copied steps with equal textual IDs remain distinct. Successful, automatic and
skipped number choices clear both pending fields. Reusing a shared step later in
the stack requests input again with a new request ID. Invalid input preserves the
pending payload and repeated requests reuse their current ID, as in Python.

SetContextFlag assigns the live payload reference; RecordTarget appends that same
reference to an existing or newly allocated list. Future Set payloads therefore
observe earlier list mutations. Tuple targets remain immutable, models retain
Python truthiness, and shallow snapshots keep shared children. Symbols use one
namespace; generated request IDs stay numeric internally. Mutable nodes are
private to worker clones and immutable protected-slot metadata is shared. Runtime
execution does not serialize JSON or clone aggregate Set payloads.

The supported live binding is deliberately closed to scalar/context operations
and NUMBER selections in resolution. An arena remains authoritative for context,
stack and stored step state. The dense projection is used only for read-only
ownership/controller lookup; planning commands and writes through context aliases
to world/compiled-operand lists reject until those consumers bind. General dense
world mutation, dynamic scheduling, aggregate repeats and finishing-step execution
remain unfinished. Ordinary gameplay traces retain their current exporter.

`object-execution.json` adds 18 frozen-source stack scenarios / 41 operations.
It compares full state, requests, events, errors and explicit object identity,
including duplicate textual step IDs, stored finishing references, invalid/stale
input, repeated prompts, auto/skip/abort, condition skips, shared/new lists, tuple
and model behavior, future Set payloads and numeric addition. Native tests restore
every boundary, rebind transport IDs, fork different choices, check early budgets
and reject world-list writes and context roots aliasing world dictionaries.
Five source tests check the underlying contracts, narrowly typed request
normalization and physical-instance capture boundaries. Generic references into
mutable card/token payload descendants reject during source capture until those
slots have identity bindings; direct physical card/token references still work.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/object_execution_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/object_execution_traces.py verify -- "$worker"
```

Live-execution corpus SHA-256:
472b7e0d268c41e888a268d7c27abfcd55f514bb396b331a83bcbbcf407d8a31.
The full engine/search objective and complete two-game-turn performance gate
remain open. Negative attack values remain legal; a general Python rejection
must be recorded as a source bug, not copied into Rust. No frozen Python changes,
NN dependencies, service restarts, commits or pushes are part of this work.


Live arena execution verification: all 186 locked release workspace Rust tests
pass, including all 6,592 canonical gameplay traces / 27,145 operations and the
18 live-identity scenarios / 41 operations. The full CLI test replay took 205.75
seconds, a correctness measurement rather than search throughput. The final
release worker independently matches both the live-execution corpus and the
12 storage scenes / 49 mutations. All 767 focused non-NN Python rules/oracle
tests pass. Strict Clippy, Rust fmt, Black/Ruff and whitespace checks pass.
Frozen Python source/pin integrity and the canonical gameplay index are unchanged.
Full dense-world bindings (including physical-payload child links), scheduled
finishing, remaining rule bodies and the full search/performance gates remain
open. Capability flags stay false for complete game transitions and search.
No live services, NN dependencies, commits or pushes were changed.


## Live ForEach execution and template deepcopy

The arena executor now runs ForEach against live context list/tuple sources and stores
current_index on the actual shared step. Each item assignment retains its original
object/physical-instance reference. Subsequent iterations re-read the live list;
the final iteration decides whether to requeue before its body executes, matching
Python when a body appends/replaces items. Negative indices, missing/empty sources,
tuple items, conditions, nested loops and shared versus independent loop counters
are covered.

Each template is deep-copied separately, with a fresh memo per template root.
Copies preserve internal sharing and retain textual step IDs while receiving
private mutable handles. Nested copied programs reuse immutable compiled
instructions. Pending payloads belong only to arena nodes, so frame templates no
longer retain or clone stale aggregate input. Operand protection is computed by
the copied role: detached copies of world lists are mutable payloads, while
unbound compiled operands remain guarded.

Physical card/spell/token copies receive distinct runtime instance handles and
share immutable definitions. Their payloads are immutable within this closed
executor; private instance rows are materialized only at export. Boundary restore
and worker forks preserve these identities without using printed IDs as object
identity. The diagnostic instance_bindings groups visible reference paths
independently of physical table allocation order, so comparisons check physical
sharing as well as values and generic object topology.

Object bindings now record deepcopy_shared for plain Hex, whose source custom
__deepcopy__ returns itself. This behavior is captured by exact source type, not
inferred from coordinate-shaped values or frozen status. Frozen subclasses still
deep-copy their fields and mutable descendants. Alias-aware corpora were
regenerated with that metadata; frozen Python source remains unchanged.

The live corpus now has 43 source scenarios / 159 operations, including nine
payload-copy variants, physical card/token copies, nested copies, dirty/prompting
templates, list mutation, shared counters and a 1,200-step loop stopped/resumed at
the real 1,000-step boundary. Native checks restore every operation and verify
private physical allocations and input choices. Source checks verify reference
and copy semantics directly.

Current live corpus SHA-256:
fba6da46d7ecfc94b6ff73b0e2a4c3dc89074cc9a32e233a986859ddd3e5779a.
Current 12-scene / 49-mutation storage corpus SHA-256:
1001eb3d2c8202ef1a3f9070e5983b9701a8506e69ac65a41a569c15e77361a9.

This expands context/control execution. Dense world slots, physical-payload child
links, other mutable step operands, repeats, other dynamic scheduling, finishing,
remaining phase/card/passive/spell/filter rules and all original search/strength/
performance requirements remain open. The final gate is still 1,000 complete
two-game-turn simulations per second. Complete game-transition/search capability
flags remain false. Negative attacks remain legal, and general Python rejection
must be noted as a source bug instead of copied into Rust. No NN tests/dependencies,
live-service restarts, commits or pushes are introduced.


ForEach verification: all 189 locked WSL release workspace Rust tests pass,
including all 6,592 canonical gameplay traces / 27,145 operations and the expanded
43 live-identity scenarios / 159 operations. The final CLI replay took 204.49
seconds; this is correctness/serialization replay, not search throughput. The
final release worker independently matches the live and 12-scene / 49-mutation
storage identity corpora. All 773 focused non-NN Python rules/oracle tests pass.
Strict Clippy, Rust fmt, Black/Ruff and whitespace checks pass. Frozen Python
source/pin and the canonical gameplay index remain unchanged. The full original
engine/search objective remains open; capability gates stay false. No live
services were restarted and no commits or pushes were made.


## Nested action scopes and actor header binding

The live executor implements push_action_context, RestoreActionContextStep and
RestoreActionTypeStep. Scope pushes save exactly the eight source action fields,
using separate present/value entries and retaining references to lists, models
and physical instances. Restoration distinguishes absence from present-null,
restores original references and leaves unrelated context changes intact. Scope
stack mutations are visible through retained aliases; action-type restoration
pops its shared list without deleting the empty key. The diagnostic push operation
calls this actual engine bookkeeping helper; nested card/action legality and body
construction remain separate work.

Mandatory aborts now discard remaining steps while restoring discarded action
scopes. A RestoreActionContextStep with other_hero_action retains the boundary so
that action can unwind and the outer text resumes. Own-action failures continue
unwinding the outer text. Remaining scopes without explicit restore frames unwind
after the stack is exhausted. Copied loop bodies use the same logic. This is the
closed context/control program: combat/finalization/finishing boundaries retain
their guards until those consumers bind, rather than being discarded by this
limited abort path.

SetActor binds current_actor_id in the authoritative arena state and updates the
derived planning actor used by prompt/controller lookup. It saves the previous
actor before reading actor_key, including when the key and save key coincide.
Ordinary and arena execution share actor and prompt-owner rules. Empty literal/
key overrides follow source truthiness; a missing actor-key target leaves the
actor unchanged, while a missing prompt-owner key falls back to the current actor.
Controller remapping is verified across actor swaps. Other dense world slots,
collections and effects remain unbound/read-only in this execution mode.

The live corpus expands to 86 real-source scenarios / 258 operations. New checks
cover scope pushes/restores, shallow aliases, nullable/missing fields, shared
stacks, nested own/other aborts, a paused abort boundary, copied-loop failures,
action-type stacks, actor changes, save/read ordering, empty override strings and
controller ownership. Trace records now also compare ordered context keys, in
addition to full state, request/event/error data and generic/physical identity.
Every boundary restores; native tests additionally verify private forked scope
rows, source parity in ordinary execution and rejection of unsupported helpers
before state mutation.

Current live corpus SHA-256:
bd7543443c99beec4716d4331383746e30a1d708c2c9eaa1ae132a9e33c6bc9f.
The storage corpus remains 12 scenarios / 49 mutations, SHA-256:
1001eb3d2c8202ef1a3f9070e5983b9701a8506e69ac65a41a569c15e77361a9.

The full original engine/search objective remains active: other dense-world and
physical-child bindings, complete dynamic/finishing/phase and rule bodies,
make/unmake and hashing, evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel workers, optional validated integration,
strength gates and 1,000 complete two-game-turn simulations/sec. No NN work or
live-service restarts are part of this checkpoint. Frozen Python source remains
the unchanged oracle; negative attacks remain legal.


Action-scope verification: all 193 locked WSL release workspace Rust tests pass,
including full state/request/event replay for 6,592 canonical cases / 27,145
operations and the 86-case / 258-operation live identity corpus. The final CLI
replay took 204.05 seconds, a correctness/serialization measurement rather than
search throughput. The final release worker independently matches both live and
12-case / 49-mutation storage identity corpora. All 779 focused non-NN Python
rules/oracle tests pass. Strict Clippy, Rust fmt, Black/Ruff, patch whitespace and
frozen-source integrity checks pass. The Python engine/pin and canonical gameplay
index are unchanged. Remaining full world/finishing/rule/search and performance
requirements stay open; complete game-transition/search capability flags remain
false. Negative attacks remain legal. No live services, commits or pushes were
changed.


## Live effect lifecycle and after-reveal scheduling

The arena now owns effect collection replacement, linked physical-card active
flags, and stored finishing-step execution. Natural duration expiry returns the
original finishing-list handles; premature ID removal returns no finishing text.
Old collection aliases remain detached lists of the original effect models.
Physical-card copies snapshot scalar overrides at copy time, so later activation
changes do not rewrite a copied card. Reimport binds every reachable stored step,
including steps retained through aliases to removed effects.

Turn expiry has a read-only collector returning ordered IDs and original
finishing lists, plus the source's remove-now helper. The collector preserves
rule-v9 semantics: callers must leave effects active while their finishing text
runs and remove the collected IDs in deferred AdvanceTurn. Automatic turn
advancement and the complete arena phase scheduler are still unfinished.

The after-cards-played helper now removes all trigger instances by effect type,
refreshes linked cards, and queues only payloads due exactly one turn later in
the same round. It retains original mutable steps and wraps each runnable payload
in fresh SetActor/FinishedExpiringEffect steps. Repeated IDs never remove unrelated
non-trigger effects; repeated references to one effect schedule its same payload
again. Stale/future/cross-round triggers fizzle, and token-bound after-reveal
triggers follow the source's dedicated firing path. This is the real source helper
used by revelation; wiring all revelation/retrieval/next-actor transitions into
the authoritative arena remains required.

Generated wrapper recipes compile once on import. Runtime scheduling uses typed
arena allocations and numeric generation IDs, with no JSON compilation or state
reconstruction. Finishing abort markers preserve the following payload or outer
tail after a mandatory failure. Selection state remains shared through effect,
context and stack references while worker clones stay private. Detached numeric
option lists are read live if they change before a stored selection is requeued.
Freshly queued programs regain immutable-operand guards for the still-unbound
instruction families.

The live corpus contains 142 source scenarios / 428 operations, comparing full
state, requests, events, errors, context key order, physical references and generic
object identity. Its SHA-256 is
42fe944b2089a9c6da719abe66b76c5bfbca02651a4784c6be565eba118e5c39.
The 12-case / 49-mutation storage corpus and the 6,592-case / 27,145-operation
ordinary gameplay corpus are unchanged. All 202 locked release workspace Rust
tests and 791 focused non-NN Python rules/oracle tests pass. The final CLI replay
took 204.52 seconds, a correctness/serialization measurement rather than search
throughput. The final release worker independently matches both identity corpora.
Strict Clippy, Rust fmt, Black/Ruff, whitespace and frozen-source integrity checks
pass. No frozen Python source, lockfile or source pin changed.

Remaining scope includes all dense-world/physical-child bindings, complete
turn/round/finishing and dynamic rule execution, remaining card/passive/spell and
filter bodies, make/unmake, canonical hashing, evaluation, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallel workers, validated
integration and strength gates. The performance requirement remains 1,000
complete two-game-turn simulations per second; correctness replay time is not
search throughput. Full game-transition and search capabilities remain false.
Negative attacks remain legal; a general Python rejection must be recorded as an
oracle defect. No NN work, source repinning, live-service restarts, commits or
pushes are part of this checkpoint.


## Arena turn boundaries

The live arena now executes rule-v9 `end_turn`, deferred `AdvanceTurnStep`,
empty-hand auto-pass and all-pass revelation. Turn finishing text runs before its
collected effects are removed. Position snapshots use a new dictionary containing
the original Hex objects; discard logs and completed planning buffers detach their
old containers. Played-card slots mutate their original lists, including fixed
empty turn slots and physical-card state changes during auto-pass.

Round-end entry expires THIS_ROUND effects before queuing their original finishing
steps and the battle/check/cleanup tail. Those remaining consumers, nonempty
initiative/card-resolution continuations and automatic revelation of committed
cards still reject explicitly. Parked round work restores and rejects execution
before popping a step or mutating rule state. Public planning commands remain
guarded until their authoritative arena bindings are complete.

The live corpus now contains 170 current-oracle scenarios / 512 operations,
SHA-256 `fe48993ec034e02dc8eeaed919afcebd0eb4109932d9beed449cc2dbb69f4592`.
The final release worker independently matches both live and storage identity
corpora. All 208 locked release workspace Rust tests and 797 focused non-NN
Python rules/oracle tests pass. The final CLI gameplay replay took 200.51 seconds,
a correctness/serialization result rather than search throughput. Strict Clippy,
Rust fmt, Black/Ruff, whitespace and frozen-source integrity checks pass. Only rule
version 9 is supported; the frozen Python source/pin and older corpora are unchanged.

Complete rules/phase execution, remaining dense-world/physical-child bindings,
make/unmake, hashing, evaluation, iterative team-aware search, hidden/simultaneous
planning, parallel workers, validated integration/strength gates and 1,000 complete
two-game-turn simulations/sec remain open. Full transition/search flags stay false.
Negative attack values remain legal. No NN work or live-service restart is included.

The scheduler uses one cold-compiled actor/abort wrapper recipe across natural
turn, round and after-reveal finishing. Deferred expiry IDs are read from their
actual live list, including shared-list modifications before execution. Generated
counter reservations precede changes to effects/logs; every automatic instruction
continues to obey the real step/deadline budget. Arena ownership is temporarily
held by one execution helper and restored on errors, allowing stack-internal
lifecycle calls without a second mutable step-state representation.

All-pass revelation binds the source hero model and its played-card list rather
than rebuilding a JSON state. It records the physical current card in the fixed
turn slot, preserves pre-existing slots, clears the current-card field, and updates
the derived world lookup. Repeated passes can advance to Cleanup synchronously;
due after-reveal payloads instead pause that progression until their queued
FindNextActor executes. New frames use numeric private generation IDs and retain
all original mutable finishing-step handles. The frozen source remains untouched.

Remaining phase work is concrete: bind actual committed-card reveal/history and
two-card retrieval, nonempty actor/tie/card/confirmation/finalization continuations,
minion battle, lane push, cleanup/upgrades/reset, and every world/model collection
those steps mutate. Lift each guard only after full state/request/event/identity
comparisons, every-boundary restoration and private-worker mutation checks pass.
The full original engine/search objective remains active; this checkpoint is not
complete phase closure or a throughput result.
