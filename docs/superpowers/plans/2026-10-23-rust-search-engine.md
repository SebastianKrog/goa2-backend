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
the ordinary TOKEN_REMOVED event; active-effect cleanup remains unsupported.
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
nonmandatory relocation match the reference. Active effects, aura restrictions
and passable-token movement still reject instead of being approximated.

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
Active-effect exclusions/topology/protections remain explicitly unsupported.

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
