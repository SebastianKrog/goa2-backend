# Rust engine for iterative game search

Status: implementation in progress. Verified reference/codec/plain-geometry foundations
are tracked in [rust/README.md](../../../rust/README.md). Complete native game
execution, search and performance gates remain unfinished.
Date: 2026-10-23.

## 1. Objective and decisions

Build a native Rust rules/simulation engine optimized for repeated legality
queries and game-tree search. Use the legality-optimized Python backend as the
rules oracle only. Rust generates, ranks and searches candidates. Python
checks rule behavior on selected decisions, sequences or game traces. The
sole current deliverable is a correct, efficient search
engine that can return useful decisions within its budget. Develop and verify
it offline.

1. **Engine acceptance.** Establish rules parity, legal best-so-far decisions,
   hidden-information correctness and useful throughput and latency on the
   reference workload and a diverse corpus.
2. **Playing strength.** Once the engine passes those gates, compare and tune
   search methods and parameters until the bot is a credible opponent.

Layout Lab integration and server API integration are deferred outside this
plan's completion gates. Any future work on live adapters, API endpoints,
controller options, UI, or live storage and submission will be scoped
separately. The local worker protocol and offline Python oracle remain tools
for testing and measurement.

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
- Keep implementation and benchmarks offline. Defer Layout Lab and server API
  integration until the engine and playing-strength work is accepted.
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

The native live-session checker now walks a shared-prefix trie under rollback,
reports the first illegal or unknown decision, and validates terminal supported
inputs without submitting them. It supports typed planning moves and the
enumerated options represented by native requests. The CLI exposes `check_sequences`, with physical
card references matching the stack trace protocol. The differential verifier
matches Python legality, checked counts, and failing indexes for 65 sequences,
covering planning, chained number selections, runes, full upgrade batches,
respawn hexes, team-owned actor prompts, multiselect completion, confirmation,
spellbook selection, and ordered rune inscription batches. The inscription
trace also checks partial progress, prompt context, object identity, and the
completion event. Snorri's Inscribe the Runes also matches Python through
card text and normal action dispatch. Native planning choice generation reads the
live hand and commitment fields. Sequence checks use those choices to reject
foreign, missing, and already-committed cards before dispatch. Broader
differential coverage remains open.

Retain a three-way result for oracle checks: legal, illegal, unsupported/unknown.
Python `legal=None` is not a positive or negative oracle label. Propagate internal
engine errors distinctly. Include structured rune/upgrade batches, optional
skip/done, second-card planning, correct decision ownership and stale request IDs.
Runtime request IDs are checked at the Python boundary; native internal decisions
use stable typed identities instead of minting UUIDs at every search node.

## 6. Search design

### Resolution: team-aware alpha-beta

The native session now has an experimental resolution search. It expands typed
live inputs under rollback, scores material from the root team's perspective,
and retains the last completed iteration when a node limit or deadline stops
search. Before the first iteration completes, it retains the best fully
evaluated root move. This implementation counts branching decisions and stops
at planning. Forced inputs use the node and time budgets without consuming
search depth, including when they follow the last branching decision.
An optional fixed-policy continuation scores leaves at a one- or two-game-turn
cutoff and uses first-legal joint choices at later planning boundaries. Before
trying the turn horizon, the search uses at most one eighth of its node budget
(rounded up to at least one node) to score immediate successors. A score at
completed depth zero can therefore come from an immediate successor or a fully
evaluated root move from an interrupted first iteration. The resolution CLI
labels scores as immediate, decision_depth, or turn_rollout.
The CLI's rollout_game_turns diagnostic matches Python's first-legal stopping
round, turn, phase, decision count and full semantic leaf state for both
horizons on Forgotten Island and Magma and Ice. Each map starts from both its
opening and turn four before cleanup and upgrades. The comparison sorts only
unordered board zone hex and neighbor sets. Reproduce the eight cases with
rollout_parity.py and the pinned oracle runtime. Adversarial planning across
turns and bot integration remain open.

Use fixed root-perspective values and explicit MAX/MIN nodes. This is safer than
negating the score after every input: the same team can make several consecutive
decisions, while defense and control effects can transfer a decision mid-action.

Classify each boundary by its actual decision owner:

- MAX: an action controlled by the root's coordinated team model.
- MIN: an adversarial opponent decision.
- Fixed policy: an explicitly configured ally/opponent controller.
- Chance: genuine rule randomness with known probabilities.
- Automatic: no player decision; execute the rules directly.

Make the team/controller model explicit. Preserve a fixed Rust continuation
policy for reproducible benchmarks. Verify the resulting actions and rule
transitions with Python.

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

The Rust session can project player-scoped facts using stable printed card IDs.
It includes public hero resources, positions, and reveals plus the requesting
hero's own cards. It includes each hero's public hand count, as Python does.
Other heroes' card identities and commitment buffers are excluded. The
ordinary planning sampler rejects a reconstructed pool smaller than the
public count and still considers unrevealed choices in the joint epoch. A source-pinned catalog now enumerates ordinary upgrade
paths and filters them by public items and card reveals. Nonstandard hero
provenance stays unavailable. Seeded ordinary planning hands are reconstructed
from the sampled path and public played/discarded cards. Facedown cards in those
zones leave reconstruction unavailable. The sampled hands describe the pool
before hidden commitments. Ordinary planning worlds can install those hands
in an independent native worker, preserving revealed played and discarded
physical cards in their existing zones. The search_sampled_planning CLI operation
exposes the bounded matrix for one seeded sample. Turn-horizon matrices prune
a row once its known worst reply ties or loses to a complete earlier row. A
joint transition validates a complete set of
choices against one pre-reveal state and applies it under rollback in roster
order. The bounded pure maximin evaluator now scores supplied team and opponent
plans on one sampled world through native resolution. It reports complete
rows and evaluated cells. Native generation can cap the joint action sets
and reports their uncapped sizes. The requesting hero's choices vary first.
Scoped planning can aggregate across worlds. Broader world reconstruction
remains open.

Joint planning can also complete an already open second-card slot. In that
case first_card is the next card to submit, or null to finish planning, and
second_card is null. The existing first commitment is preserved.

#### Public evidence and upgrade batches

The sampled-planning CLI accepts optional per-hero upgrade_checkpoints. Each
entry supplies a completed level and cumulative ordinary card-item totals
in Attack, Defense, Movement, Initiative, Range, Radius order. The CLI also
accepts public_upgrade_batches with the hero, revision, and before/after
checkpoints. Replay orders merged records by revision, ignores exact
duplicates and rejects conflicting records or impossible chronology. Both
worker paths use the replayed totals as constraints. The offline runner
captures completed batches from Python's public reveal snapshots and passes
them into subsequent searches. It tracks standard-provenance heroes from
level one, and abandons cumulative inference if a reveal is missed or the
public delta is inconsistent. The recorder can save and restore a versioned
JSON checkpoint bound to a game ID, including the public baseline for an
unrevealed batch. The verifier reloads mid-batch and after reveal. Live
capture and storage wiring remain open.


The upgraded rollout verifier uses Python's level-award logic and submits
legal 3-to-6 and 1-to-8 batches through the simultaneous upgrade prompt.
The latter includes six card choices and the automatic ultimate unlock.
Across all six bundled maps, aggregate evidence replay with duplicates
produced the same Rust plan as direct checkpoints. Python accepted each
plan. The split 3-to-6 case also exercises out-of-order history.

1. Keep a replayable public evidence stream per hero, ordered by game event or
   observation revision. Record level and visible item totals before and after
   each completed upgrade batch, public card reveals, and changes in visible
   played, resolved and discard slots. Include public hand size and facedown
   slot counts where the rules expose them. Carry this evidence across rounds,
   reconnects and saved checkpoints. If observation begins midgame, start from
   all hypotheses consistent with the public snapshot.
2. Extend each surviving loadout once per level crossed, using the observed
   before and after levels as the bounds. Three-level batches are expected in
   play, and the same algorithm handles larger legal batches up to the level
   cap. Apply each crossed level's rules, including ultimate unlocks, and
   filter the histories against the aggregate public item change. For example,
   a level 3 to 6 jump crosses levels 4, 5 and 6. If all three rewards become
   visible together, keep every legal assignment consistent with the totals.
   Narrow the assignment when an intermediate checkpoint or a later reveal
   distinguishes the histories. Account separately for publicly attributable
   item gains from effects or abilities.
3. Apply visible card identities as constraints on ownership as well as zone
   membership. A revealed Tier II card can prove a historical upgrade even
   after a Tier III replacement. A card currently visible in a played or
   discard slot constrains the pool and is unavailable in hand. Use those
   constraints even when a reveal-history entry is missing. Handle known
   returns, swaps and other card changes according to their rules. Preserve
   uncertainty when evidence is missing or provenance is unsupported.
4. Cache the resulting beliefs by public evidence revision. Keep upgrade
   histories distinguishable when they constrain later deductions. Group
   equivalent current card pools for search and define their sampling weights
   explicitly. Separate uncertainty about card ownership, current hand
   membership and facedown commitments. Use a uniquely inferred pool
   directly and sample only the unresolved facts. On contradictory evidence,
   surface the inconsistency and use a documented public-only fallback.

#### Team knowledge modes

Every bot in every mode knows the full static card catalog for all heroes,
including printed effects, upgrade pairs and item rewards. Use one of four
mutually exclusive modes per team.

| Mode | Information and planning behavior |
| --- | --- |
| **True rules** | Each bot sees its own private cards and public evidence. Allied choices and hands follow standard visibility. |
| **Coordinated** | Same-team bots share exact hands and commitments. Search chooses a joint plan for those bots. Allied humans' facedown choices remain uncertain. |
| **Coordinated + show** | Coordinated bots show their committed choices to allied humans before public reveal. Start search without waiting for human choices and model those choices as uncertain. This is the preferred mode for interactive play. |
| **Coordinated + know same-team humans** | Coordinated bots can see allied humans' committed cards before public reveal. Treat those commitments as fixed inputs and optimize the bots' joint plan around them. An allied human's uncommitted hand stays private. This mode has the most information for team play. |

Opponents' private cards stay outside the observation in all modes. Sharing
with humans is limited to the show mode. Build the search observation at a
trusted boundary from the public evidence and the mode's explicitly permitted
private facts. Give the planner control only over bot slots. In know mode,
handle uncommitted humans with a bounded wait or an uncertain-choice policy.
Key search results by observation revision and revalidate if an allowed
commitment is withdrawn or replaced. Keep the mode and evidence revision in
cache keys so knowledge cannot cross modes, teams or viewers.

The sampled-planning CLI accepts an optional bot_team object with mode and
bot_ids for trusted callers. Scoped planning searches bot commitments against
uncertain replies and returns only bot commitments. Optional worlds selects
one to eight equally weighted worlds from the request seed. The planner uses
the same worlds for every candidate and maximizes the mean of the per-world
worst evaluated replies. A row is eligible for scoring only when all capped
replies have been evaluated in every sampled world. Scoped planning can
prepare worlds and score a candidate across them with one to four workers.
Workers share a deadline and draw nodes from one atomic cap. A worker can
use nodes other worlds leave unused. Scores merge in seed order, and only
fully evaluated candidates are eligible. The trusted worker
operation show_bot_commitments projects committed printed card IDs for an
allied human viewer in show mode. Verify this projection offline with a
trusted roster. Authentication, live revalidation and UI delivery belong
to deferred integration.

#### Search and verification

1. Build the root observation from public facts and the acting hero's permitted
   private information under the configured team knowledge mode.
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
   indistinguishable information sets where supported. Compare a Rust
   information-set policy with determinized minimax in Rust-only play tests
   if strategy fusion limits playing strength.

Resolution search accepts an optional planning_sample_seed for a turn rollout.
At each future planning epoch it reconstructs ordinary hidden hands from the
branch's public state and keeps the perspective hero's hand. The seed is
stable across alternatives at the same epoch. The same optional public
upgrade batches or checkpoints used for planning search can constrain those
rollouts. This option samples future planning only. Resolution decisions before that boundary still use the
imported state, and unsupported hand provenance rejects the search.

A minimum over adversarial actions is different from an expectation over belief
samples. Define any risk-sensitive aggregation separately. Rule chance,
search-world sampling and heuristic tie-breaking use separate RNG streams.
Planning support is a required milestone; resolution-only speed is not completion.

### Iteration, deadlines and fallback

Before deep search, establish one Python-validated legal root action and an
initial native heuristic ranking. Check a monotonic deadline and cancellation
flag at bounded node/step intervals, including inside long automatic sequences.
Reserve time for transport, final Python validation and submission.

The diagnostic worker starts the deadline when it receives a complete request
line. JSON decoding, upgrade replay and native root import consume that budget.
Native import checks the deadline on entry and exit. The Python diagnostic
client can bound a complete exchange, including pipe I/O. On timeout it
terminates and retires the directly launched worker so a partial response
cannot be reused. The scoped oracle run uses a 12-second outer budget for
a 10-second native request. The live adapter still needs to reserve its own
time for final Python validation and submission.

Publish progress only after a coherent unit completes: a root alternative across
the required world samples, or a whole search depth. Preserve the best fully
completed result. A partially evaluated branch must not overwrite it simply
because its provisional score is high. Return completed depth, sample coverage,
principal variation, node counts, deadline status and fallback reason.

Scoped planning prepares a fallback from the live controlled slots before
sampling hidden worlds. A deadline or cancellation during world preparation
returns that unscored fallback with prepared and requested world counts.
The diagnostic Python caller continues to use its own first-legal fallback
when no native score was completed.

Resolution search reports whether it solved the tree, completed the requested
depth, reached the node limit or deadline, or was cancelled. The stop reason
is separate from score kind and completed depth. An interrupted iteration can
still return the earlier completed result or a scored fallback.

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

The search requests accept an optional evaluation field. material_position is
the existing default. lab_attack_range_v1 uses the verified lab evaluator
throughout native search, including immediate fallbacks and rollout leaves.
Nonterminal scores use a fixed-point scale of 999,999 so alpha-beta comparisons
stay integral and terminal wins and losses retain priority.

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

For search-only changes, generate candidates in Rust and send selected actions
or sequences to Python for rule validation. Use focused accepted-choice checks
when they cover the risk, and complete game or boundary replays when state,
event order, or cumulative behavior matters. Batch validation where it reduces
oracle overhead. Keep Python out of the search loop and skip exhaustive Python
move enumeration in routine search verification. Time only Rust in performance
benchmarks and compare playing strength only against Rust opponents. Earlier
cross-language timings below are historical.

When Rust rule execution or move generation changes, expand differential
coverage around the affected rules. Use targeted action-set comparisons only
when checking that generation includes all required choices. Compare verdicts,
first sequence failure, post-action state and event order as relevant. Save
the smallest reproducer for any mismatch.

Use exhaustive tiny-board enumeration and finite-tree minimax versus
alpha-beta tests within Rust. Add make/unmake round trips, incremental-hash recomputation,
TT-on/off equivalence, cache invalidation, deadline/cancellation and worker
isolation tests. Python's current passing suite is a source of fixtures and
expected semantics, not a test suite Rust automatically inherits.

## 9. Offline verification and parallel execution

Start with a persistent Rust worker process and a versioned request protocol.
Load immutable definitions once; send one root observation/world-sampling request
and return a canonical action plus diagnostics. Use JSON at this outer boundary
first, never in the search loop. Profile before considering a Python extension
or binary transport. Do not spawn a new process for each simulated decision.

The offline harness verifies root identity, the current player and prompt,
and final action legality against Python. Exercise stale roots, cancellation,
worker failures and fallback with local fixtures. These checks require no
running lab or server API.

The offline Python planning validator rejects stale revisions, unauthorized
heroes, physical cards outside the searched hand and results without a
completed score. It checks the complete decision prefix with Python sequence
legality. The offline resolution validator also requires the imported live
prompt token, an eligible bot responder, and a legal selection. The live
caller must supply trusted ownership and revisions and hold the state stable
through validation and submission.

The offline rollout verifier's shadow mode submits real planning and
resolution snapshots to a persistent worker and checks returned moves with
Python. A separate first-legal control run verifies the shadow observer did
not change play. Four-turn runs on Forgotten Island and Magma and Ice
validated 8 planning and 75 resolution recommendations without fallback.
The diagnostic uses a two-second native and four-second transport budget.
Live opt-in remains separate.

The opt-in native play diagnostic applies validated recommendations through
the Python engine, using the same first-legal fallback for a rejected result.
With a two-turn rollout horizon it applied 8 planning and 76 resolution
recommendations over four turns on each of the same two maps, with no
fallback. One resolution choice differed from Python's first option. This
exercises submission and advancement offline. Playing-strength comparison
remains open.

With future-planning sampling enabled, the same offline play run again
validated all 84 recommendations without fallback. The iterate build's
end-to-end resolution latency was 224–241 ms p50 and 276–315 ms p95 across
the two maps (38 decisions each). Planning was 253–277 ms p50 and 318–491 ms
p95 (four decisions each). These timings include Python envelope export and
its oracle source verification, worker request and response, and final
Python validation. Submission, scheduling and full live latency remain
unmeasured.

Verifying the frozen sources once per offline run preserves per-state lossless
export checks. On 12 alternating pairs for one Magma and Ice opening, identical
envelopes took 78.1 ms with per-export verification and 42.2 ms with a verified
export session. The four-turn sampled play rerun validated all 84 decisions.
Resolution p50 was 192–208 ms and planning p50 was 221–230 ms.

In the historical versus-Simple smoke test, Arien's team used native search
with lab_attack_range_v1 and the opposing team used the pinned Python
heuristic. That heuristic also handled native fallback in those runs. Both four-turn games reached
round 2 after 16 native planning and 41 native resolution choices with no
fallback. This is a matchup integration check. Longer side-swapped games,
complete resolution information masking and comparison with non-neural
ISMCTS remain open.

The side-swapped Simple matchup exposed team-addressed prompts beyond
initiative ties. Search, choice generation and input validation now use
the same ownership check for team addresses. Twelve-turn games on both
maps with Rust on each team reached round 4 with 96 validated native
planning and 236 resolution choices and zero fallback. All four games
were still in progress at the cutoff with six lives per team.

Historical benchmark before Python was restricted to rules validation.
The matchup runner also supported non-neural Python ISMCTS. Four side-swapped
four-turn games used the same lab evaluator, two-turn horizon and configured
350 ms search budget. Rust applied 32 planning and 79 resolution choices
without fallback. These are search budgets, with export and validation
outside the native timer and total native latency reported separately. All
games were unfinished with equal lives at the cutoff. This verifies the
comparison path without establishing relative strength.

The current verifier no longer runs Python opponents or times Python
rollouts. Earlier matchup and paired-timing measurements are historical.
The current offline Python path validates or replays rule transitions.

The offline_match worker operation runs both policies and the entire game
transition loop in Rust. A depth-zero policy is the fixed first-legal baseline.
Four side-swapped four-turn smoke matches on Forgotten Island and Magma and Ice
reached the next round with zero fallback. The search candidate used depth one,
a two-turn continuation, the lab evaluator, a 4,096-node cap and a one-second
deadline. All matches remained undecided. This verifies the runner, with
full-game strength evaluation deferred until engine acceptance.

The match worker can export its chosen commitments, resolution selections
and final state for untimed Python replay. Four side-swapped 12-turn runs
on the two smoke-test maps validated 513 planning batches and resolution
inputs in total. Python selected no moves in these replays and reached
the same final states.

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

Time complete Rust rollouts under a declared two-turn horizon, evaluator and
continuation policy. Include sampling, candidate generation and policy work.
Use fixed seeds and validate representative actions or replays in Python
outside the timer. Record action or RNG tapes for validation where useful.

Reproduce the saved 1,000-iteration workload in Rust if the saved position is
available. Then run a fixed diverse corpus, reporting per-position results
and slow percentiles rather than only an opening-position average. Include dense combat,
planning, effects-heavy and multi-lane positions. Report cache hits separately;
do not count incomplete, zero-horizon or fabricated repeated visits as completed
simulations. Reference settings cannot be shortened to pass a timing gate.

Success requires demonstrated 1,000 full two-turn simulations/s on the declared
reference workload, plus disclosed corpus performance and unchanged semantics.
If the broader corpus misses the target, document the remaining gap and keep the
performance work incomplete.

### Playing-strength tuning after engine acceptance

Use the verified simulator and accepted engine configuration as the control.
Compare search methods and tune depth, turn horizon, world count, action-set
width, move ordering, opponent and continuation policy, evaluation weights,
and time allocation. Treat the two-turn reference configuration as a fixed
parity and performance benchmark. Candidate playing configurations may vary.

Compare Rust search configurations against a fixed Rust baseline at fixed
wall-clock decision budgets. Use paired, side-swapped games across seeds,
maps and rosters, including difficult midgame positions. Use Python solely
to validate selected decisions or replay rule outcomes. Report completed-game outcomes, move
quality on reviewed positions, fallback rate and decision latency. Set the
strength acceptance threshold before evaluating the final candidate and
confirm the chosen settings on held-out games. Short smoke games verify
execution only.

## 11. Implementation milestones

Each milestone produces reviewable code, fixtures and recorded measurements.
Engine correctness and efficiency gates precede playing-strength tuning.

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
| 8. Parallelism and engine performance | Worker-local execution; measured allocation/layout/ordering improvements | Deterministic serial baseline retained; real throughput/latency improvement; bounded memory |
| 9. Playing strength | Compare and tune search methods and parameters offline | Credible play under the agreed budget, with results against a fixed Rust baseline and held-out validation |

Milestones 3â€“6 can use a declared small roster for development, but do not declare
the engine complete from a special-case Arien/Brogan opening or exclude difficult
heroes from the final coverage claim. Keep separate progress for rule parity,
coverage, full-simulation throughput and adversarial playing strength.

## 12. Completion checklist

- [x] Python oracle pinned to the legality-optimized revision and content export.
- [ ] All declared maps, heroes, effects, phases and rules versions covered.
- [ ] Batch/sequence legality, transition states and event order match Python.
- [ ] Make/unmake, hashing, caches and parallel workers preserve independence.
- [ ] Actual decision ownership and consecutive same-team choices handled.
- [ ] Hidden information and simultaneous planning treated explicitly and tested.
- [ ] Public upgrade batches and reveals survive replay and reconnect. Test
  single gains, two-level batches, a 3-to-6 jump, and the largest legal batch.
  Verify ambiguous assignments survive aggregate evidence and narrow when
  new evidence arrives. Cover effect-derived items,
  reveals that collapse a pool, and resolved/discarded cards without history.
- [ ] Test all four knowledge modes at the offline observation and worker boundaries.
  Swapping hidden human or opponent cards with public evidence held fixed must
  preserve same-seed, same-budget choices unless the changed commitment is
  expressly visible in know mode. Check that controlled teammates' permitted
  cards can affect joint plans, and that show mode starts without human input
  and publishes only allied bot commitments.
- [x] Two-game-turn reference horizon and evaluator preserved in parity benchmarks.
- [ ] Iterative search returns the best completed result within its real deadline.
- [ ] Unsupported branches and stale roots trigger whole-decision fallback.
- [ ] Full-simulation target measured honestly; minimax counters kept separate.
- [ ] After engine acceptance, select and validate playing settings at fixed
  decision budgets using Rust-only opponents and a fixed Rust baseline.

## Algorithm references

The algorithm choice is a starting implementation decision, subject to the
correctness and strength gates above.

- Knuth and Moore, [An Analysis of Alpha-Beta Pruning](https://www.sciencedirect.com/science/article/pii/0004370275900193)
  (1975): primary analysis of alpha-beta correctness and search behavior.
- Cowling, Powley and Whitehouse, [Information Set Monte Carlo Tree Search](https://eprints.whiterose.ac.uk/id/eprint/75048/1/CowlingPowleyWhitehouse2012.pdf)
  (2012): primary treatment of hidden information, simultaneous moves and
  determinization/strategy-fusion limitations. This motivates the information
  boundary and informs comparison of Rust search methods.


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


## Committed-card revelation and actor selection

The live arena now reveals first/second commits, retains public card identities
as a tuple of frozen records, and orders two-card retrieval before after-reveal
triggers and actor selection. Existing knowledge records keep their identity;
completed planning buffers detach. Retrieval mutates the original physical card
and hand/slot/discard containers, preserving the source step's finished input.
Cleared initiative overrides are omitted rather than emitted as null, including
when an independent copied card retains its old override.

Ordinary and arena execution share initiative ranking and tie decisions. The live
path handles reversed initiative, mutable candidate lists, team choice, invalid
input retries, direct-location versus multi-piece respawn scheduling, actor/owner
headers and the delayed coin flip. Each boundary evaluates its ranking/tie rules
once. Actual card, confirmation, finalization and respawn bodies remain parked;
importing them does not compile unrelated gameplay/passive closures, and attempting
to execute one rejects before changing the step or state.

The live corpus has 211 source scenes / 598 operations, SHA-256
`be1972ce5f667f0eefb37fe270f20e0b4354ded3cfc0ecb6ac4e89be3e0382f2`.
All 214 locked release workspace Rust tests and 804 focused non-NN Python
rules/oracle tests pass. The final CLI replay took 177.59 seconds, a correctness/
serialization result rather than search throughput. The release worker independently
matches both identity corpora. Strict Clippy, Rust fmt, Black/Ruff, whitespace and
frozen-source integrity checks pass. Frozen Python source/pin and
both older corpora remain unchanged. No live services were restarted.

Public planning commands, remaining action/world/physical-child bindings and full
turn/round execution are still required, followed by make/unmake, hashing,
evaluation, iterative team-aware search, hidden/simultaneous planning, parallel
workers, validated integration/strength gates and 1,000 complete two-game-turn
simulations/sec. Full transition/search capabilities remain false. Negative
attacks remain legal; general Python rejection must be documented as an oracle
bug rather than reproduced in Rust.

The revelation binding replaces only source-replaced containers and keeps in-place
hand/played/discard mutations on their original nodes. Physical scalar overrides
now support field deletion; copies snapshot both values and omissions. New public
knowledge entries are frozen model nodes under an untracked tuple container,
matching source capture and deepcopy semantics. Source model code is unchanged.

A parked-step cold compiler preserves actual wire fields and input/condition
state while compiling only a harmless frame shell. It restores the real parked
family and loop-template metadata immediately; no sanitized marker leaks into
saved state. This lets actor handoffs/restoration be verified independently of
unported card bodies without activating their passive/stat/action consumers.
Unsupported bodies retain explicit runtime guards. Warm execution uses typed
handles/recipes and never compiles or reconstructs JSON.

The 41 added scenes cover first/second commits, idempotent frozen knowledge,
unknown/missing players, pass-slot override clearing, two-card retrieval and
pending-state retention, due trigger order, reverse activation/date/token
predicates, missing candidates, ties/duplicate pools/invalid inputs, live candidate
mutation, delayed flips, multi-piece ownership, skipped parked frames and the
open/done/two-card planning gates reached by auto-pass. New native contracts check
worker isolation, omission/copy snapshots, frozen alias topology and non-mutating
parked-action restore. Existing full gameplay coverage stays active.

Next phase work: bind the actual card chooser/body, confirmation, finalization,
respawn, battle/lane/round cleanup and upgrades/reset; integrate public planning
commands through authoritative card/request/collection bindings. Preserve all
identity and phase-order comparisons before lifting the corresponding guards.
The original complete engine/search and performance objective remains active.


## Live pass and finish-planning commands

The shared-object path now accepts the public pass and finish commands. Pass
validates phase, hero, prior commitment and an empty live hand before mutation;
it replaces the starting-position request dictionary, keeps surviving request
rows shared, refreshes mutation protection for the new live graph, and adds the
pass to the original pending dictionary. Finish is a
no-op outside planning, requires a pending key (including an explicit pass), and
appends to the original done list only once. Both commands run the native
revelation transition when all players have closed planning. The second-card
open check reads the authoritative arena hand rather than a derived vector.

The live source corpus now contains 224 scenes / 614 operations, SHA-256
`3f84f5370b50f5d2daf11349bc7223659b952a39736ea45235ef5083c9ae3009`.
The 13 added scenes cover invalid phase/hero/hand/commitment, repeated commands,
unknown pending keys, request dictionary detachment and retained rows, ordinary
last-player revelation and a two-card hero explicitly closing planning.

Commit and uncommit still require live physical-card/collection bindings. Actual
card/confirmation/finalization/respawn bodies and full round execution remain
open, as do the dense-world/physical-child completion, make/unmake, canonical
hashing, evaluator, iterative team-aware minimax/alpha-beta, hidden/simultaneous
planning, parallel workers, integration/strength gates and 1,000 complete
two-game-turn simulations/sec. Full game-transition and search capabilities stay
false. The frozen Python engine/pin and live services are unchanged; negative
attacks remain legal. No NN work is included.

Verification: the independent release worker matches all 224 live scenes / 614
operations, including full state, requests, events, physical references and
aggregate identity. Ten focused frozen-source planning/revelation/tie contracts
pass. Strict Clippy (`--all-targets -D warnings`), Rust fmt, Black/Ruff and
whitespace checks pass. The frozen source SHA-256 remains
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.

The canonical 6,592-case / 27,145-operation gameplay replay also passes. Native
coverage includes invalid-command atomicity, finish idempotence, detached request
guards, protected surviving rows and restoration at every live-operation boundary.
These are correctness/identity checks, not complete-game search throughput.

Rust verification covers 217 passing tests across the workspace run and final
focused reruns, including the corrected 224-scene CLI replay and request-guard
refresh. Ten focused Python contracts pass. This checkpoint is uncommitted.


## Live card commitment and withdrawal

Public commit and uncommit now use the authoritative arena. Commit preserves
source no-op/invalid-input boundaries, first-versus-second commit request cleanup,
physical card identity, facedown/round-played state and initiative overrides.
Withdrawal is LIFO for two-card heroes: it appends the same card to the original
hand, clears its override, restores the first current card or clears current,
and removes only the first matching done entry. Ordinary planning now follows
the same first-occurrence done-list behavior.

Typed synchronization refreshes derived hero card collections from the arena
after in-place edits, including shared hands across heroes and a hand aliased to
a deck. Worker forks retain private cards, hands, buffers and cache state. Request
cleanup is shared with pass; existing dictionaries detach only where Python
replaces them. Warm commands use typed handles and perform no JSON reconstruction.

The live corpus has 247 scenes / 668 operations, SHA-256
`b464e42768468bdc3b0079208aa41813baa4667ed7be007eea6995444881f279`.
The 23 new scenes cover take-back cycles, wrong phase/hero/card, repeated commands,
passes, request aliases, copied overrides, shared collections, explicit two-card
done/second commits, last-card closure, duplicate done entries, aliased buffers
and spell-card nonmembership. All 23 scenes / 54 new operations independently
match the release worker before full replay.

Foreign same-printed-card value equality and runtime copied-card planning rows
still require native model/instance bindings; those shapes reject explicitly
before mutation rather than silently treating physical identity as model equality.
Actual card chooser/body, confirmation, finalization, respawn, battle/lane/round
execution and remaining dense-world/physical-child bindings remain open. The
original make/unmake, hashing, evaluator, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel workers, integration/strength gates and
1,000 complete two-game-turn simulations/sec requirements remain open. Full
transition/search capabilities stay false. Negative attacks remain legal. Frozen
Python source/pin and live services are unchanged; no NN work is included.

Card-planning verification: the independent release worker matches all 247 live
scenes / 668 operations, including state, request, event and identity records.
All 11 focused frozen-source planning/revelation/tie contracts pass. Strict
Clippy, Rust fmt, Black/Ruff and whitespace checks pass. The frozen source hash
remains `76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
No live services were restarted and no new commit/push was performed.


Next live phase bindings must preserve these source contracts:

- `ConfirmResolutionStep` honors both rollback flags before consuming pending
  input, consumes invalid truthy input before re-prompting, and targets its explicit
  hero rather than inheriting a controller blindly.
- `ResolveCardStep` checks actual hero/card/board presence, populates the current
  card context and derives offered actions through live validation/stats before
  scheduling registered bodies and passive/pre-action work.
- `FinalizeHeroTurnStep` resolves the original card slot, activates that card's
  effects, resets card/effect passive counters, conditionally clears guess/reveal
  state, clears the existing context dictionary and actor/owner/piece headers,
  then schedules minion return, lane push and actor selection in that order.
  Its turn-ended event and shared collections must remain source-identical.

Keep unsupported dependency bodies explicit until their authoritative world and
physical-child bindings exist; do not advertise complete transitions/search from
successful import or from phase scheduling alone.

Final card-planning checkpoint: all 221 locked release workspace Rust tests pass,
including the 6,592-case / 27,145-operation canonical gameplay replay and restore
checks at all 668 live-operation boundaries. The independent live worker replay
and 11 focused Python contracts also pass. These correctness/serialization checks
are not evidence of the final two-game-turn search-throughput target. Changes
remain uncommitted; the full engine/search objective remains active.


## Live resolution confirmation

`ConfirmResolutionStep` now executes on the live arena without compiling unrelated
card bodies. Both rollback flags short-circuit before input consumption. Ordinary
confirmation consumes only truthy mapping input, retains a completed step's
request ID and preserves references to detached pending dictionaries. Invalid
selections re-prompt; stale submissions reject through the normal request path.
The prompt uses the step's exact hero ID, with shared controller routing applied
only when source actor/owner/effect rules require it. Piece and unknown literal
owners are retained rather than normalized to another hero.

Mandatory aborts now stop at confirmation and finalization boundaries instead of
discarding those steps. Existing foreign-action context boundaries remain intact.
Truthy non-mapping confirmation input rejects before the step is popped, unless
a rollback flag auto-confirms; that bypass preserves the original dirty input.

The live corpus contains 279 scenes / 757 operations, SHA-256
`a16e2cfb613b83f1762991237133ec747264e1181f8790dae0789610324f377b`.
The 32 added scenes independently match 89 source operations covering retries,
retained request/input state, truthy/falsy rollback flags, explicit/controlled
owners, skipped conditions, mandatory aborts, scope restoration, loop copies and
repeated shared steps. Native fork tests check independent requests and decisions;
a separate source contract proves rollback bypass of malformed pending data.

Card-action selection/body, finalization, respawn and complete battle/lane/round
execution remain unbound in the live arena. Remaining model equality and runtime
physical-child bindings, make/unmake, canonical hashing, evaluator, iterative
team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel workers,
integration/strength gates and 1,000 complete two-game-turn simulations/sec remain
required. Full transition/search capabilities stay false. Negative attacks remain
legal; the frozen Python source/pin and live services are unchanged. No NN work or
new commit/push is included.

Final confirmation verification: the locked release workspace run passes all 223
then-present Rust tests, including the 6,592-case / 27,145-operation canonical
replay and restore checks at all 757 live boundaries. The additional final
ordinary/live confirmation parity test passes across 19 scenes, giving 224
passing Rust tests across that run and the focused follow-up. The ordinary path
now preserves falsey pending input while prompting instead of clearing it.
All 13 focused frozen-source contracts and the independent 279-scene / 757-operation
worker replay pass. The standalone release worker is rebuilt with the final fix;
its full-transition/search flags remain false. Strict Clippy, fmt, Black/Ruff,
whitespace and frozen-source checks pass. These are correctness checks, not
complete-search throughput. This checkpoint remains uncommitted.


## Live hero-turn finalization

Finalization now resolves the original played-card slot, activates effects linked
to the resolved printed card, resets positive passive counters on the hero's
played/ultimate cards and owned effects, conditionally clears guess/reveal state,
and clears the existing context dictionary plus actor/owner/piece headers. It
emits the source turn-ended event and queues minion return, lane checking and
actor selection in that order. Source card-active flags remain independent from
effect activation. Unported continuation bodies still reject before execution.

Slot assignment does not require printed-card model equality: duplicate printed
instances are now accepted for this primitive, with their physical identities
preserved. Existing collection/equality guards remain on operations that actually
need value-based membership/removal. Typed card-cache synchronization is reused
across planning and finalization, including shared played lists, while each
hero's resolved-turn count remains independent.

The frozen get_hero function normalizes HeroPiece IDs to their owning hero.
Shared lookup and ordinary planning/revelation now follow that rule. Raw caller
IDs remain planning keys and event/guess/reveal predicates where Python uses them;
effect-counter ownership follows the resolved hero's ID. Source and native cases
cover both owning-hero state changes and raw-key distinctions.

The live corpus has 311 scenes / 800 operations, SHA-256
`a337611e6ada4b26e3769e00dd20a097894d79d69754d1f6756a3b46a18f6f97`.
The 28 finalization and four piece-planning scenes independently match 43 source
operations. They cover slot padding/replacement, positive/nonpositive counters,
linked/shared/duplicate-ID effects, guess/reveal retention, context aliases,
unknown and piece IDs, shared slots, duplicate printed cards, repeated execution
and skipped conditions. Native tests additionally cover worker isolation,
generation exhaustion before mutation, cache coherence and guarded continuation
restoration; an ordinary/live comparison checks piece-owned cards and raw buffers.

Card-action selection/body, respawn, minion-return/lane/battle/round consumers and
remaining dense-world/physical-child/value-equality bindings remain open. The
original make/unmake, canonical hashing, evaluator, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallel workers,
integration/strength gates and 1,000 complete two-game-turn simulations/sec remain
required. Full transition/search capabilities remain false. Negative attacks
remain legal. Frozen Python source/pin, dependencies and live services are unchanged;
no NN work or new commit/push is included.

### Reboot checkpoint — 2026-11-01

Work stopped at the user request for a system reboot. No remaining owned cargo
workspace test/CLI replay process was found; session 91754 is no longer available.
Its final CLI result could not be recovered, so the full workspace run is not
claimed complete. The observed serial run passed 145 core tests and all 42 live
execution tests (800 operation restoration boundaries), plus the other core
integration suites. The CLI replays still need a confirmed result. Fifteen focused
Python contracts passed. The full independent 311-scene worker replay passed
before the latest slot-cache synchronization change.

An earlier parallel run exhausted WSL memory while rustc overlapped eager fixture
loading. The integration fixture loader now streams cases and filters the small
index before loading ownership subsets; this last harness edit remains uncompiled
and unvalidated. After resuming: format and run strict Clippy; compile the live
integration test separately, then run its 42 tests with measured peak RSS and no
overlapping compilation; finish the CLI tests; rebuild the standalone release
worker and recheck its full live replay. Verify frozen-source and whitespace
checks, then record final results. This is test-harness memory work, not evidence
of complete-search throughput. All implementation changes remain saved locally
and uncommitted. No live service was restarted.

### Resume verification — 2026-11-01

The streaming integration loader is now formatted, compiled and verified under
strict all-target Clippy. All 42 live tests passed, including restoration at all
800 operation boundaries. The separately compiled executable completed in
247.47 seconds with measured peak RSS 71,248 KiB (`/usr/bin/time`), compared with
the earlier observed eager-loader RSS of 4,181,456 KiB. The loader retains only
the index and one case at a time, and filters ownership subsets before loading
case files. Assertions and case coverage are unchanged. This fixes verification
resource use; it is not an engine/search performance measurement. Frozen-source
and runtime verification still returns
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
CLI replay and final standalone-worker verification are still pending at this
checkpoint.

Final resumed validation: all three CLI tests passed in 277.49 seconds, including
the canonical 6,592-case / 27,145-operation replay and live 311-scene /
800-operation replay. Together with the observed serial core/integration results
and the resumed 42-test loader run, all 231 current Rust tests have passing
results across those runs. The standalone release worker was then rebuilt and
independently passed the live-object Python verifier with the exact corpus hash
above and state/request/event/identity parity. The initial independent command
used the generic stack normalizer, which does not traverse live object bindings;
its request-ID alias mismatch was a runner selection error. The correct entry
point is `object_execution_traces.verify`, not `stack_traces.verify`, for this
corpus. No engine workaround or oracle alteration was made. Strict Clippy, Rust
formatting, whitespace and frozen-source checks pass. Changes remain uncommitted.

Next implementation boundary: bind automatic minion return to the live arena.
Follow frozen `ReturnMinionToZoneStep.resolve`, including raw processed-list
identity, favored-team/ID ordering, invalid-input retry, one-destination
automation, team-owned choices, placement fallback and fresh continuation steps.
Reuse the existing dense return planner only after authoritative context/world
bindings are current; the live execution context must not be replaced with the
stale ordinary Registers cache. Movement/placement child steps need executable
physical bindings, with source differential fixtures before lifting the guard.
Complete lane/battle/round/card/respawn execution and the original full search
and performance gates remain required; the overall engine goal is not complete.


## Live automatic minion-return controller

`ReturnMinionToZoneStep` now runs on the live arena: raw processed IDs,
favored-team/string-ID ordering, own-lane zone selection, walking versus legal
placement fallback, automatic single destinations and team-owned choices reuse
the dense planner. Invalid truthy mapping input clears only pending input;
falsey input survives prompting and successful input remains visible through
retained aliases. Original processed lists are not mutated. Continuations receive
fresh lists with the chosen minion appended, and relocation precedes continuation.
No-path/no-space/inside/offboard/missing-zone cases finish without aborting.

The controller shell compiles to one live instruction rather than ordinary
relocation or mine child programs. Generated destination Hex
models preserve Python's explicit shared-deepcopy identity; other frozen models
retain their ordinary copy semantics. Automatic movement legality is not an
action check and does not read Registers context, so this adapter uses an empty
typed context rather than the stale ordinary-session projection. Card collection
caches are refreshed before effect-aware planning. No JSON runs in the warm path.

The live corpus is now 342 scenes / 840 operations, SHA-256
`67fbede19bec0f5d87676d1bcbe5b9e34fa0b74f8e8e3fee9b1875fb71ded7c8`.
The 31 added frozen-source scenes independently match 40 native operations,
including alias retention, retries, opposite destination choices, repeated/skipped
steps, multiple teams/minions, duplicate processed IDs and effective movement
caps. Three focused Python contracts pass. Native tests cover worker isolation,
generation exhaustion before mutation, Hex copy identity, cold dependency
isolation and restore/rejection at the relocation boundary. Final workspace
validation is pending at this checkpoint.

Movement/placement child bodies remain explicitly guarded before pop or mutation.
They must receive authoritative live position/entity/token bindings and full
source-compatible events and follow-ups next. Complete lane/battle/round/card/
respawn execution, remaining dense/physical bindings, make/unmake, canonical
hashing, evaluator, iterative team-aware minimax/alpha-beta, hidden/simultaneous
planning, parallel workers, integration/strength gates and 1,000 complete
two-game-turn simulations/sec remain required. Full transition/search capability
flags remain false. Negative attacks remain legal. Frozen Python source/pin,
dependencies and live services are unchanged. No NN work or new commit/push.

Relocation follow-up details from the frozen source: `GameState.move_unit`
delegates to `place_entity`, validates destination before mutation, preserves the
existing locations dictionary, stores the exact destination Hex object and
updates old/new tile occupancy. Live bindings must preserve that target alias
rather than allocate an equal replacement. `PlaceUnitStep` emits UNIT_PLACED or
TOKEN_MOVED according to the actual entity, retaining the source actor semantics.
`MoveUnitStep` clears success_output_key before its skip check, so the current
generic skip adapter cannot substitute for that instruction-specific ordering
when its full body is bound. It must also preserve crossed-obstacle output,
movement events and mine-path/requeued-step/trigger snapshots for hero movement.
Minions do not trigger hero mine-path logic, but that does not remove the full
hero/token relocation requirements from the engine objective.


## Test memory, parallelism and compile-time optimization

User requested another resource/iteration pass before continuing engine bodies.
The live controller remains 342 scenes / 840 operations with the same corpus
hash; no Python source/pin or fixture content changed. All 235 tests passed in
the pre-final optimization release run. Its serial runtime was 423.12 seconds
with peak RSS 1,387,072 KiB. Buffered fixture loading reduced the live suite from
approximately 200 seconds to 94.14 seconds and the storage suite from 24.92 to
1.37 seconds. A counted read benchmark on the 3,360,170-byte loop-deep-model case
measured 3,360,171 file reads / 881 ms unbuffered versus 412 reads / 32 ms buffered;
all decoded output hashes matched. These are fixture/test results, not search
engine throughput.

The CLI verifier now consumes one actual snapshot per operation and borrows
identity bindings during normalization. The public worker still collects exactly
the same response. A shared test-only iterator pool runs independent cases with
private sessions; worker panics fail the test and exact scene/operation counts
remain asserted. At the user's request its default is four workers, capped by
available CPUs. `GOA2_ORACLE_WORKERS=1` restores serial case execution. The WSL
wrapper defaults `RUST_TEST_THREADS` to 2 while respecting environment/CLI
overrides. The first shared-pool full run passed all 235 tests in 241.20 seconds
with peak RSS 1,393,280 KiB, before the later large-corpus/cache optimizations.

The separate `iterate` Cargo profile inherits release semantics with optimization
level 2, no LTO, 16 codegen units and incremental compilation. Release ThinLTO
and codegen-units=1 remain unchanged for final performance measurements. First
iteration-profile compilation of every test binary, including its fresh dependency
cache, took 33.70 seconds. Subsequent actual test-code edits across both crates
rebuilt in 11.61 seconds; a narrower shared-helper edit took 6.16 seconds. A real
codec/body change rebuilt the test workspace in 25.74 seconds, and the latest
two integration-test edits took 2.30 seconds. Earlier release rebuilds took
72–82 seconds for all test binaries and 38–44 seconds for the worker.

A further audit found oversized whole-corpus caches. Isolated effect tests used
1,393,272 KiB peak RSS; validation used 1,214,444 KiB. A streaming first pass cut
these to 231,916 and 25,296 KiB. Named lookups then improved further: unrelated
case bodies and unused expected snapshots are skipped during deserialization,
with only explicitly reused inputs retained. Full parity scans remain unfiltered
and retain all exact comparisons. Effect, displacement and action arrays assert
123, 159 and 198 cases; action/travel/menu scans assert 38, 13 and 31; spatial
scans assert 201. Action menus no longer allocate the unrelated 72 MB query
array, and spatial tests no longer cache the entire 47 MB corpus.

The immutable embedded oracle manifest is now parsed once, and reserved-reference
validation borrows definition/instance maps instead of cloning them. All checks
and error conditions remain; this does not cache mutable game state or skip
validation. Strict Clippy passes. The final measured default-profile run, release replays and independent source
verification completed successfully; results follow below. No live service was restarted; no
new commit/push. Full engine/search scope remains open.

Final optimization verification (2026-11-01): the default WSL workflow, without
worker/thread overrides, passes all 235 tests in 213.96 seconds with peak RSS
935,672 KiB. That compares with 423.12 seconds / 1,387,072 KiB for the measured
precompiled buffered-release serial run: about half the elapsed time and one
third less peak test memory. This combined workflow comparison changes both
profile and concurrency; it is not an isolated engine-performance claim.

The final isolated effect suite passes all four tests in 9.70 seconds with
201,368 KiB peak RSS (before: 9.68 seconds / 1,393,272 KiB). Validation passes all
five in 2.88 seconds with 21,504 KiB (before: 2.92 seconds / 1,214,444 KiB).
Streaming therefore preserves their runtime while removing most retained data.
The final named reader skips unrelated bodies and unused expected values; full
parity readers still compare every original expected value and assert all case
counts. Bounded parallelism is now the default, with explicit serial overrides.

All three optimized release CLI tests pass in 71.40 seconds, covering canonical
6,592 cases / 27,145 operations and live 342 scenes / 840 operations. The rebuilt
release worker independently passes Python's live-object verifier with unchanged
corpus hash `67fbede19bec0f5d87676d1bcbe5b9e34fa0b74f8e8e3fee9b1875fb71ded7c8`
and state/request/event/identity parity. Frozen-source/runtime verification still
returns `76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
Strict Clippy, Rustfmt, whitespace and frozen Python/lockfile checks pass.
The iteration/release build caches together occupy approximately 2.1 GiB.

No tests were disabled, no NN dependencies or tests were introduced, and no live
services were restarted. All changes remain uncommitted. Resume actual engine
implementation at the live movement/placement consumers; the original complete
engine/search and 1,000 complete two-game-turn simulations/sec requirements are
still open, with full transition/search capabilities false.


## Live placement execution

`PlaceUnitStep` now executes against the authoritative arena and dense spatial
state. Target/unit/key resolution reads live step/context fields, preserving
falsey fallback and short-circuit behavior. Occupied destinations finish without
mandatory abort; validator denial (including occupied self-placement) aborts only
when mandatory. Effect-aware displacement checks retain source actor semantics.
Units, pieces, owning heroes, minions and tokens use the existing entity registry;
tokens emit TOKEN_MOVED and other targets UNIT_PLACED with source from/to values.

The existing locations dictionary and old/new tile models mutate in place. An
immutable destination Hex is reused exactly; a dictionary destination becomes a
fresh immutable Hex without changing the original input or ignored extra fields.
Retained old Hexes and independent snapshots survive unchanged. When the turn
snapshot aliases the locations dictionary, its derived cache is refreshed too.
Ad-hoc virtual coordinates preserve source behavior, including multiple entities
at one off-map point; loaded-map out-of-range destinations reject before pop or
mutation. Shared/mismatched tile bindings and malformed coordinate types remain
explicitly guarded. These guards do not silently substitute a different rule.

Placement has a dedicated shell-compiled opcode and does not link unrelated
movement/mine child bodies. Tile handles compile in one ordered arena pass,
avoiding repeated per-tile dictionary searches/JSON decoding; the immutable
binding table is shared across worker clones. Minion return now executes its
placement fallback, including restoration after the controller expansion. Walking
remains guarded until its own full body and follow-up dependencies are bound.

The live corpus is 539 scenes / 1,234 operations, SHA-256
`74e54562da1de0635f4fe2e1be770f331a16fe93bd615dd28edf13f72d07968e`.
The 197 added frozen-source scenes independently match 394 operations: 159 effect
variants, 29 alias/input/abort cases, all six repository maps, and three completed
minion fallbacks. Coverage includes active/dormant/time-linked restrictions,
scopes/immunity, unknown/offboard/entity-kind targets, dirty ignored input,
dictionary coercion, retained aliases, repeated/skipped steps and foreign-action
abort boundaries. Four focused source contracts pass. Native tests verify fork
isolation, live destination reads, dense/tile cache coherence, snapshot aliases
and guarded errors before mutation. Final complete Rust and worker verification is recorded below.

Full movement/mine-path execution, card actions/primary/passive/spell bodies,
respawn and lane/battle/round consumers plus remaining dense/physical bindings
remain open. Make/unmake, canonical hashing, evaluator, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallel search, integration/
strength gates and 1,000 complete two-game-turn simulations/sec remain required.
Transition/search capabilities stay false. Negative attacks remain legal, frozen
Python/pin and dependencies remain unchanged, and no live service was restarted.
No NN work or new commit/push is included.

Final placement verification: all 239 default `iterate` workspace tests pass,
including restoration at all 1,234 live-operation boundaries. All three optimized
release CLI tests pass (66.51 seconds), covering the canonical 6,592-case /
27,145-operation replay and the expanded live corpus. The rebuilt release worker
independently matches all 539 live scenes / 1,234 operations with full
state/request/event/identity parity and the corpus fingerprint above. The four
focused Python contracts pass; strict Clippy, Rustfmt, Black/Ruff, whitespace and
frozen-source/lockfile checks pass. Frozen source/runtime verification remains
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
This is correctness verification, not a complete-search throughput measurement.
Changes remain uncommitted.


## Live walking execution

Implemented live walking, sharing placement's spatial mutation primitive instead
of duplicating location, tile and aliased-snapshot updates. Unit/destination
fallbacks, displacement/action/movement restrictions, topology/actor-aware paths,
straight alignment/path validation, crossed-obstacle output and success lifecycle
read actual arena state. Python's outer handler skip and the movement body's
post-clear skip are separate boundaries; both are covered by source scenes.
Generated minion walking now completes and restores successfully.

Added 171 source-derived scenes / 331 operations, bringing the live corpus to
710 / 1,565; semantic SHA-256
`5dd7c85aea03a83d3ed3bd050ecf76ce3a6a9986100ffc3f07934a147cb28cc3`.
All added scenes independently match full state/request/event/identity snapshots.
Private worker destinations, dense/cache coherence, and unbound mine guards
before stack/success mutation have dedicated native tests.

Next bind live mine-path choice/requeue and snapshot trigger execution. Full
card/primary/passive/spell bodies, respawn and lane/battle/round consumers, physical
bindings, make/unmake, hashing, evaluation, iterative team-aware search,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/sec remain required. Do not advertise
complete transition/search capability. Frozen Python, dependency/runtime pins,
negative-attack semantics and live services remain unchanged. No NN work or
commit/push is included.

Final validation: all 241 default `iterate` workspace tests pass, including the
canonical 6,592-scene / 27,145-operation replay and restoration at every live
operation boundary. After tightening malformed-mine-queue guards, both focused
native walking tests pass again. Five focused Python contracts pass. The final
optimized release worker independently matches all 710 live scenes / 1,565
operations with full state/request/event/identity parity; index SHA-256
`7f03b42cf416cb468cee8734d7c86ce9b2f4a059467376ee40b6e783b0442db5`.
Strict Clippy, Rustfmt, Black/Ruff and whitespace checks pass. Frozen source and
runtime verification remains
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`;
`src/` and `rust/Cargo.lock` remain unchanged. This verifies correctness, not
complete-search throughput. Changes remain uncommitted.


## Live mine choices, triggers and forced-discard expansion

Implemented arena-backed mine path choices, retries and retained pending values;
movement detection, shallow requeue and literal trigger snapshots; ordered mine
removal, tile/snapshot updates and linked-effect list replacement; rollback
reanchor boundaries; and forced-discard expansion with immunity, basic filters,
shields and literal victim bindings. Card selection/discard consumers are staged
source steps and reject before popping until their live bodies and follow-up
chains bind. Dud chains complete. Do not describe blast discard resolution as
complete.

Added 157 source-derived scenes / 551 operations. The live corpus is now
867 scenes / 2,116 operations, semantic SHA-256
`2ad558df025638e239fa07ddf32db0aaf41fa01248e7334623f2b30dfc02dc29`, index
`d204e996f6cdc395dcf24f1b4d540eabf45fff073eb93cc56acdbedc62ee851e`.
Independent new-case comparisons match full state/request/event/identity records.
Only declared mathematical choice sets are normalized; ordered literals, events,
options and path arrays remain exact. Native tests exercise private requests and
choice lists, shallow child sharing, snapshot victim/queues and pre-pop guards
through restoration. Source contracts additionally inspect detached old lists and
duplicate effect IDs, pending input and shield/basic/immunity expansion.

Next bind general card selection/discard, including passive/expiry chains. Full
card/primary/passive/spell bodies, respawn and lane/battle/round consumers, remaining
authoritative physical/dense bindings, make/unmake, hashing, evaluator, iterative
team-aware search, hidden/simultaneous planning, parallel search, validated
integration/strength gates and 1,000 complete two-game-turn simulations/sec remain
required. No complete transition/search capability is advertised. Frozen Python,
locked dependencies/runtime, negative attacks and live services remain unchanged.
No NN work or new commit/push is included.

Final validation: the 244-test `iterate` workspace run passes. After correcting
one fixture to perform an actual detached-list append, all 44 restoration tests
and all three CLI tests pass again; the canonical 6,592-scene / 27,145-operation
corpus remains covered. All seven focused Python contracts pass. The final
optimized release worker independently matches all 867 live scenes / 2,116
operations, including state/request/event/object-identity parity and the
fingerprints above. Strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen
source/lock checks pass. Frozen source/runtime verification remains
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
Card-selection/discard consumers and complete search support remain incomplete.
This is correctness validation, not a complete-search throughput measurement.
Changes remain uncommitted.


## Live intrinsic CARD selection

Implemented live arena card-selector fields, direct referenced-context readers,
and shared ordinary/live candidate rules. Covered owner/hero precedence and piece
normalization, all intrinsic card-property/container rules, masking, shields,
ordered/duplicate candidates, metadata outputs, optional/mandatory outcomes,
automatic selection and exact accepted/invalid pending-input lifecycle. No JSON
or whole-context projection enters the warm candidate path. Nonempty generic
CARD predicates remain guarded before popping.

Reused the existing source selector matrix via an optional non-disruptive export
callback: 77 new scenes / 278 operations, including every registered hero.
Expanded old mine/shield flows by 112 operations through actual card prompts and
selections. Combined live corpus: 944 scenes / 2,506 operations; semantic SHA-256
`40a922c1ebd9da5b93aba93b500550b60657e3525707b538d392ab9b73c49691`, index
`7b90f9e9aa4a14efcd2ad3fbdd0d17b183343f733047cc6aefe1c0a5a3cb7cff`.
New-case and expanded-flow comparisons independently match full source snapshots.
Native tests check live private lists, metadata and unbound-predicate guards. The
blast/victim regression now completes selection and verifies discard guards
before pop across restore.

Next bind discard execution and expiry/passive chains. First inspect/implement
source model-value equality for current/played/hand selection/removal rather than
substituting physical instance identity. Preserve list aliases, physical fields,
public reveal/log ordering and global first-card expiry behavior. Generic CARD
predicate closure remains required. Full card/primary/passive/spell bodies,
respawn/lane/battle/round consumers, authoritative dense/physical bindings,
make/unmake, canonical hashing, evaluation, iterative team-aware search,
hidden/simultaneous planning, parallel search, validated integration/strength
gates and 1,000 complete two-game-turn simulations/sec remain open. Complete
transition/search capability stays false. Frozen Python/runtime/lock and negative
attacks remain unchanged; no live service restart, NN work or commit/push occurs.

Final validation: all 247 default `iterate` workspace tests pass, including the
canonical 6,592-scene / 27,145-operation replay and restoration at every one of
the 2,506 live boundaries. All eight focused Python contracts pass. The rebuilt
optimized release worker independently matches all 944 live scenes / 2,506
operations with full state/request/event/object-identity parity. Strict Clippy,
Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass. Frozen
source/runtime verification remains
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
Discard, generic CARD predicates and complete-search work remain open. This is
correctness validation, not a complete-search throughput result. Changes remain
uncommitted.


Historical discard equality prerequisite (resolved by the boundary extension below), verified in the frozen runtime: two deep-copied
cards with `_definition_revision` values 0/0 compare equal. Changing only one
private revision to 1 makes equality false while both `model_dump(mode="json")`
and `export_state_with_shared_objects` outputs remain equal. The previous boundary
therefore omitted observable Python equality state. Do not implement discard using
only physical identity or the visible `CardInstance` fields. Extend oracle
boundary equality metadata and typed native state, then verify equal/distinct
physical cards, private revisions, metadata dictionary equality, and
current/played/hand precedence against Python. Preserve the frozen engine/pin;
this is a boundary/representation task, not permission to change the oracle.


### Equality-aware live discard

The offline card boundary now preserves model class, private definition revision
and whether metadata retains JSON equality semantics. Typed state and physical
deepcopy/export/restore preserve this rule state. Native comparisons use complete
static definitions and dynamic fields, ignore dictionary insertion order, preserve
list order, and compare booleans/integers/floats using Python equality without
rounding large integers. Unsupported non-JSON physical metadata stays guarded;
missing private metadata is never assumed to be revision zero.

Live discard resolves literal/context hero and card references, normalizes piece
ownership, follows source container fallback and current/played value-equality
precedence, and mutates actual physical cards and lists. All derived hero card
collections and shared discard-log entries refresh from those mutations. It
records public identity once, appends every discard to the log, replaces linked
effects by row (including duplicate IDs), cancels the globally first matching
card's raw active flag, and queues the real after-discard passive step. Premature
expiry never runs finishing text. Old effect-list aliases remain detached data.

The equality corpus has 26 source cases / 104 comparisons, SHA-256
`c879e9d86ff93cf9cd0bd276a0f3f47e43d3ac4b8be2bae2b75a736511ab2e37`.
The 25 discard source scenes include equal but distinct current/played cards,
private revisions, metadata ordering, shared/cross-hero lists and logs, duplicate
effect IDs, global first-card expiry and ignored/skipped operands. Combined live
corpus: 969 scenes / 2,531 operations; semantic SHA-256
`0a111880f903eac6e0516763a239cfa144f54ececc283797f8c35a0b5ea569bf`, index
`20d19c4458bb2f33e0bfe9c52624d05ee1a4c5d0b77a722a9ed50f24b87bfd80`.

Next bind live passive scanning/offering and its context-dependent eligibility,
then continue remaining card/primary/passive/spell/filter bodies, respawn and
lane/battle/round consumers, authoritative physical/dense bindings, make/unmake,
canonical hashing, evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search and integration/strength gates.
The 1,000 complete two-game-turn simulations/sec requirement remains unproven.
Complete transition/search capabilities stay false. Frozen Python/runtime/lock,
negative attacks and live services remain unchanged. No NN work or commit/push.


Next live passive binding details (source inspection): keep ordinary played-card
alias duplicates, suppress only later provider scans by printed ID, and preserve
played/ultimate/provider ordering and explicit hero ownership. Separate source
eligibility and offer creation from passive-body execution; the existing native
scanner currently rejects eligible effects other than Big Sodding Gun/Wild Hunt.
Port all eight custom eligibility predicates: Jinx (own active effect, movement
validation and reachable destination), Legend of the Skies (turn slot), Battle
Fury (discard context and primary-action legality), The Archwizard (performing
card and spellbook Wish), Rune Mastery (runes and defense hook), Bullet Time
(basic-action context), Reign of Winter (versioned genuine defeat evidence), and
Dragon Knight (performable faceup skill). Default predicates still return true.
Do not treat successful no-passive scans as complete passive support. Revisit
ordinary lifecycle list removals as well: remaining physical-identity comparisons
must use the same Python model-equality contract wherever the source does.


Validation: all 255 Rust workspace tests pass, including the canonical
6,592-scene / 27,145-operation replay and restoration across 969 live scenes.
The optimized release worker independently matches all 969 live scenes /
2,531 operations and all 26 equality scenes / 104 comparisons against frozen
Python. Seventeen distinct focused Python contracts pass (live execution,
equality and codec). Strict Clippy, Rustfmt, Black/Ruff and whitespace checks
pass. Frozen source/runtime identity remains
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`;
`src/` and `rust/Cargo.lock` are unchanged. This validates the implemented rule
boundaries, not complete-game/search support or the required search throughput.
Changes remain uncommitted.


### Live passive discovery, offers and usage

Live arena execution now scans played cards, unlocked ultimates and active-effect
providers in source order. Ordinary duplicate card aliases remain duplicated;
only provider scans are deduplicated by printed ID. Owner overrides, masking,
usage limits, provider identity, default prompts and unknown/missing cards match
Python. Optional offers issue real requests and retain pending input/request
fields on decline. Accepted/mandatory passive bodies remain guarded before pop;
this does not advertise complete passive-body or full-transition support.

All eight custom eligibility predicates are bound: Jinx, Legend of the Skies,
Battle Fury, The Archwizard, Rune Mastery, Bullet Time, Reign of Winter and Dragon
Knight. Card-specific permission uses the explicit target card and empty typed
registers, preventing stale context and avoiding a whole-context copy. Movement
and current rule-v9 defeat-log predicates reuse native rule state. The frozen
metadata exporter records all custom predicate methods and verifies that every
previous continuation recipe remains unchanged.

Passive marking updates actual physical cards or the first matching provider,
including every derived effect row aliased to that same provider object. Separate
equal provider objects remain independent. Counter overflow is guarded before
pop across restoration. Completed supported mine/shield discard chains now
continue through the passive scan; offers requiring unbound bodies still stop
at that next boundary.

The 246 new passive scenes / 492 operations cover every configured registered
passive and all eight custom predicates, negative permission branches, pending
offers, providers and usage aliases. Existing mine/discard scenarios gain 108
operations. Combined live corpus: 1,215 scenes / 3,131 operations; semantic SHA-256
`aa3bcb6dbbe44e8fd669e77668323e39aa925c899456d65076b81aef11d86e46`, index
`6fc403a047733657ace01b364c16410917e081aea3dd2c40a789a34135cfab94`.
Original finishing fixtures now receive stable IDs because Python's default
`id(object())` can collide with later generated IDs. The frozen engine is not
changed. Live corpus export streams one case at a time; a compatibility test
checks identical snapshot/index/semantic identities against the previous writer.

Next bind dynamic passive bodies and their MoveSequence, generic selectors,
ForceDiscardOrDefeat, and primary-action consumers. Remaining complete
card/primary/passive/spell/filter bodies, respawn/lane/battle/round execution,
authoritative dense/physical/model-equality bindings, make/unmake, canonical
hashing, evaluator, iterative team-aware minimax/alpha-beta, hidden/simultaneous
planning, parallel search, integration/strength gates and 1,000 complete
two-game-turn simulations/sec remain required. Transition/search capabilities
stay false. Frozen Python/runtime/dependency locks, negative attacks and live
services remain unchanged. No NN work or new commit/push is included.


Final validation: all 258 Rust workspace tests pass, including the canonical
6,592-scene / 27,145-operation replay and restoration across all 1,215 live
scenes. All 13 focused source contracts pass. The rebuilt optimized release
worker independently matches all 1,215 live scenes / 3,131 operations with full
state/request/event/object-identity parity. Strict Clippy, Rustfmt, Black/Ruff,
whitespace and frozen source/runtime/lock checks pass. Frozen source identity
remains `76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
This verifies the implemented rule boundaries, not complete passive bodies,
complete-game/search support or the required search throughput. Changes remain
uncommitted.


### Accepted passive construction and live rune selection

Accepted/mandatory offers now use cold source-derived continuation recipes with
live typed owner, previous-card, radius, rune and eligible-card values. Fifteen
direct body constructors and Cloak's generic attack/movement branches are bound;
Flurry/Cloak branches that rebuild registered card effects remain guarded before
pop. Empty bodies do not queue a usage mark. Nonempty bodies queue the original
source order followed by MarkPassiveUsed. Original pending mappings/tokens remain
observable through aliases exactly as in Python.

Recipe allocation uses no JSON in the warm path. It preserves nested filter
models, lists, dictionaries and ForEach template step kinds, assigns transport
identities to all generated steps, and binds nested programs in the private
worker map. Owners substitute during allocation; dynamic lists refill their
fresh slots. Cold definitions remain shared across workers. Tests cover Jinx's
actual actor switch, Legend's live previous-card flag, Dragon Knight's dynamic
skill list and permission recheck, modified/negative radii and raw-radius March
of Nature semantics. Negative attack values remain legal.

ChooseRune now executes live, including empty/duplicate options, automatic single
choices, retries and optional metadata. Consuming any truthy pending mapping
clears pending_input while retaining pending_request_id and detached aliases;
invalid input retries, and unmatched metadata writes null while unmapped values
retain their old output. Empty actor IDs and piece owners produce exact prompt
ownership and rune-selection decision context. Rune Mastery completes through
choice and usage marking; A Complicated Profession completes its supported
forced-discard chain and marks usage afterward. Other consumers remain guarded.

The 153 new scenes / 343 operations cover accepted source bodies, executable
prefixes, complete supported passives and rune input/metadata/ownership behavior.
Combined live corpus: 1,368 scenes / 3,474 operations; semantic SHA-256
`45d6ce90ce130fcd097ccd815123ce67e25e29e404545c91dda5925e75f87491`, index
`e2ff428bb3ed31de80e38c3dd62fff6ad4d4f32f955333929e19c601218bc677`.
Source recipe export proves all previous metadata/continuations unchanged before
adding the new structures. Shared-child or unknown-method recipe drift fails
explicitly rather than losing object identity.

Next bind the movement/general HEX/UNIT selectors, ForceDiscardOrDefeat,
primary-action, spell, color, defeat, attack and token consumers, together with
the general card-body builder required by Flurry/Cloak. Complete remaining
card/primary/passive/spell/filter bodies, respawn/lane/battle/round execution,
authoritative physical/dense/model-equality bindings, make/unmake, canonical
hashing, evaluator, iterative team-aware minimax/alpha-beta, hidden/simultaneous
planning, parallel search, integration/strength gates and 1,000 complete
two-game-turn simulations/sec remain required. Complete transition/search
capabilities stay false. Frozen Python/runtime/lock and live services remain
unchanged. No NN work or new commit/push is included.


Final validation: all 260 Rust workspace tests pass, including the canonical
6,592-scene / 27,145-operation replay and restoration across all 1,368 live
scenes. Three new focused source contracts pass. The rebuilt optimized release
worker independently matches all 1,368 live scenes / 3,474 operations with full
state/request/event/object-identity parity. Strict Clippy, Rustfmt, Black/Ruff,
whitespace and frozen source/runtime/lock checks pass. Source identity remains
`76702801afa0ac816e2ee0fc269d78cbea28c677f1128912b981cc9d4498cd43`.
This validates body construction, rune execution and the supported complete
passives; unbound consumers/rebuilds, complete-game/search support and required
search throughput remain open. Changes remain uncommitted.


### Live movement macros and general HEX selection

Movement macros now allocate the source-derived selector/filter/move sequence
with fresh step identities and exact model/container kinds. Live arena fields
supply actor, destination, mandatory flags, straight/full-distance constraints,
computed range, movement caps and pass-through permissions. Preset destinations
retain source truthiness. Living Tsunami, Wild Hunt and Jinx now execute their
movement continuations through passive usage marking.

HEX selectors read current filter fields and only the context slots those
filters reference. The native scan supports range, movement path, straight line,
clear line, obstacles, adjacency, spawn and fast-travel predicates. Fast travel
checks shared action prerequisites, color exceptions, topology and actor-aware
obstacles, including physical context-card precedence. Requests, invalid inputs,
optional skips, mandatory failure and automatic selection preserve source order
and pending-input aliases.

The boundary now captures board dictionary key Hex identity independently of
Tile.hex. Equal-valued private keys stay private until selected; visible key
aliases and restored worker forks retain identity. Missing automatic-selection
identity and malformed bindings reject before popping. Sparse typed context
reads avoid materializing complete contexts or symbol-sized register arrays.

The 3,244 new source scenes / 14,678 operations independently match native
state, request, event and identity traces. Coverage includes the ordinary effect
and topology matrix, all six real maps, passive movement, key identity, and
explicit fast-travel card exceptions. Source template regeneration proves that
only four movement recipe shapes were added. Frozen Python source and dependency
locks remain unchanged; no live service restart, NN work, commit or push.

UNIT selectors, remaining card/primary/passive/spell bodies and consumers,
respawn/lane/battle/round execution, authoritative physical/model-equality
bindings, make/unmake, canonical hashing, evaluator, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallel search,
integration/strength gates and 1,000 complete two-game-turn simulations/sec
remain required. Complete transition/search capabilities remain false.


Final validation: all 262 Rust workspace tests pass, including the unchanged
6,592-scene / 27,145-operation canonical replay and restoration at every one of
18,152 live operations across 4,612 scenes. All nine focused source contracts
pass. The optimized release worker independently matches the entire live corpus
with full state/request/event/object-identity parity. Live semantic SHA-256:
`70816e564ba04c89737318b8f7e60e2ae17cd356d25e599c72dabe72e86c2a4b`;
index: `af8cbf4c8a45a0e7f80291304fa1c72d953cd9cc3509b91657cb5a7bb090da7c`.
Strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
This validates the implemented movement/HEX boundaries, not complete engine or
search support. Changes remain uncommitted.


### Live UNIT and UNIT_OR_TOKEN selection

Unit selectors now read current arena filter fields and sparse referenced context
slots, reusing native targeting, line-of-sight, immunity, illusion equivalence,
displacement and played-card rules. Intrinsic targeting uses the overridden
player while relative filters retain the current actor. Empty and absent actor
IDs remain distinct, and piece prompts normalize to their owning hero.

Automatic self-exclusion and immunity filters follow the source order and
presence checks. Nested OR/AND filters retain short-circuit semantics, including
empty composites; primary operand substitution traverses both composite kinds
and binds the source card used by fast-travel color exceptions.
Selections preserve candidate order, optional/mandatory behavior, retries,
automatic choices and pending aliases. Reusing the same selector reads updated
history lists and actor state; worker forks remain private. Unknown filter
families and unsupported aggregate/non-string context coercions still guard
before popping. Remaining filter families must still be bound.

The new 345 source scenes / 1,476 operations match native state, request, event
and identity traces. Three source contracts check candidate ordering, prompt
ownership, automatic input aliases and repeated live selectors. Corpus merging
now reuses the canonical encoding already verified for each case instead of
serializing it twice. Unicode identity and corruption/atomic-index regressions
pass. Combined live corpus: 4,957 scenes / 19,628 operations; semantic SHA-256
`022f4a63772e814dc0438d31faeafcce1d65bc0830ec02dab2d86b343455f5ab`, index
`2156c705d852174817a0b72053750632e450bb8882142cbc37bcceca89155f35`.

Remaining unit/card/hex filter bodies, live query/collection consumers,
ForceDiscardOrDefeat and other card/primary/passive/spell bodies, remaining
phase/world execution, authoritative physical/model-equality bindings,
make/unmake, canonical hashing, evaluator, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/sec remain required. Complete engine
and search capabilities remain false. Frozen Python and locks, negative attacks,
and live services are unchanged. No NN work, commit or push is included.


Final validation: all 265 Rust workspace tests pass on the final build, including
the unchanged 6,592-scene / 27,145-operation canonical corpus, restoration at all
19,628 live operations, private UNIT worker forks, unsupported-filter guards,
and the nested primary fast-travel color-exception regression. All 14 focused
source and corpus-integrity contracts pass. The rebuilt release worker matches
all 4,957 live scenes with full state/request/event/object-identity parity;
four private verification workers completed that comparison in 162 seconds.
Strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
The implemented selector boundary is verified; complete engine/search support
and required search throughput remain open. Changes remain uncommitted.


### Live Count, CollectUnits and scalar query conditions

Noninteractive queries now bind current arena target types, filter fields and
referenced context values, reusing the typed query and targeting rules. UNIT
queries use actual units; UNIT_OR_TOKEN queries include tokens and apply their
normal equivalence predicates. Queries do not add SelectStep's intrinsic
can-be-targeted check. Count and Collect keep their distinct default immunity
flags. HEX counting uses native spatial predicates; unsupported candidate
categories produce source-compatible zero/empty results without evaluating
irrelevant predicates.

Successful collection allocates a fresh arena list each time. Existing aliases
retain old results, while ForEach reads the new live list. Repeated queries see
changed actors and updated context history. Handler-level conditional skips leave
old outputs untouched. Scalar conditions read live fields and preserve Python's
bool/numeric comparisons, exact float/int boundaries, nonnumeric-as-zero behavior,
unknown operators and True-or-null output values. Query preflight guards execute
before popping or replacing context. Filter binding and sparse input collection
are shared with UNIT selection instead of duplicated.

The 188 new source scenes / 638 operations cover effect/illusion/immunity states,
query flags, target categories, handler gates, fresh list aliases, worker forks,
ForEach, fast-travel counts and scalar conditions. Fresh Converts, Devoted
Followers and Steady Advance contribute actual source-built query/condition
prefixes at three minion populations; their remaining card bodies are not yet
claimed complete. Combined live corpus: 5,145 scenes / 20,266 operations;
semantic SHA-256 `239ec5426f2d2f2416ea3c28bfebf9edd95d59cf0298558720fdfe9f42ff5b7e`,
index `48ae05ac0c15885b2913e0c3122ac7b67ee3c0bd01078a2445099d7b52d1f200`.

Remaining card/primary/passive/spell and filter bodies, other live consumers,
remaining phase/world execution, authoritative physical/model-equality bindings,
make/unmake, canonical hashing, evaluator, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/sec remain required. Complete engine
and search capabilities stay false. Frozen Python/runtime/locks, negative
attacks and live services are unchanged. No NN work, commit or push is included.


Final validation: all 267 Rust workspace tests pass, including the unchanged
6,592-scene / 27,145-operation canonical replay and restoration at every one of
20,266 live operations across 5,145 scenes. All 18 focused source and corpus
contracts pass. The rebuilt release worker independently matches the full live
corpus with state/request/event/object-identity parity; four bounded private
verification workers completed that comparison in 167 seconds. Strict Clippy,
Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass. This verifies
the implemented query/condition boundaries and source-built card prefixes;
complete engine/search support and required throughput remain open. Changes
remain uncommitted.


### Live normal card retrieval and complete imported card bodies

RetrieveCard now follows the source played/discard/current precedence and takes
a snapshot of discard order for retrieve-all. Its typed preflight simulates
lifecycle flags before model-equality removal, preserves played-card holes,
clears an equal current card, and refreshes every role affected by shared lists.
Private definition revisions and Python-compatible metadata equality remain
part of those comparisons. Mutations preserve physical card and list identity;
no serialized state is used while advancing.

Premature expiry removes matching effect rows in source order, replaces the
active-effect list even for zero matches, updates the first globally resolved
printed card's active flag, and does not execute finishing payloads. Batch
retrieval accounts for shifting effect indices and updated card roles. Events
retain source order and raw actor identity. Preflight currently clones the typed
world and shadows only the three edited lists; it does not clone the arena.
Replacing that world clone with journaled deltas remains an optimization task.
Unsupported equal-looking card metadata, source partial-failure cases and aliases
that create non-card lifecycle entries remain explicit guards before popping.

All 49 new source scenes / 194 operations match native full state, request,
event and identity traces. Imported source-built Fresh Converts, Devoted
Followers and Steady Advance bodies now complete through retrieval and optional
movement, with enabled/disabled conditions and invalid-input retries. This
verifies body execution; the general native card-body builder remains required.
Combined live corpus: 5,194 scenes / 20,460 operations; semantic SHA-256
`4149d549a5f1d6fc017f035ce47408e6753b4475328c7c177a0ea4dea2a50afa`, index
`cceffc74afcf26ba2ebf3a4b55ac1ca3a6dd4fa7c11109ef4ce4781a4eb005fa`.

Remaining card/primary/passive/spell and filter bodies, live consumers and
phase/world execution, full physical/model-equality and error semantics,
make/unmake, canonical hashing, evaluator, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/sec remain required. Complete engine
and search capabilities remain false. Frozen Python/runtime/locks, negative
attacks and live services remain unchanged. No NN work, commit or push is included.


Final validation: all 269 Rust workspace tests pass, including the unchanged
6,592-scene / 27,145-operation canonical replay and restoration at every one of
20,460 live operations across 5,194 scenes. All 22 focused source and corpus
contracts pass. The final release worker independently matches the full live
corpus with state/request/event/object-identity parity; four bounded verification
workers completed that comparison in 166 seconds. Strict Clippy, Rustfmt,
Black/Ruff, whitespace and frozen-source/runtime/lock checks pass. The imported
card bodies and retrieval boundary are verified; general body construction,
remaining rule/error semantics, complete engine/search support and required
throughput remain open. Changes remain uncommitted.


### Live token supply, relocation, removal and overflow

Native live PlaceToken/RemoveToken now follow source pool order, destination
occupancy, owner fallback and output-key precedence. Supply placement changes the
physical pooled token; relocation preserves its ownership, immunity and linked
effects. Dense rule lookups use private copy-on-write token property overrides,
so subsequent selection sees the new owner/immunity without changing another
worker. A distinct physical pool copy with the same printed ID does not mutate
the misc-entity instance. Clone/restore coverage exercises selection immediately
after placement.

Overflow constructs the source Select/Remove/Place continuation, preserving
mandatory selection, owner overrides, invalid-input retries and fresh step IDs.
Actor-scoped substitution suppresses authored immunity on direct placement;
the source overflow retry authors the effective token kind and retains immunity.
Removal replaces the active-effect list and removes token/source/scope-origin
links without executing finishing payloads or clearing card activation.

All 76 new source scenes / 212 operations match native state, request, event and
physical/object identity traces. Coverage includes all 15 token kinds, frozen Hex
and dictionary destinations, virtual placement, occupied/offboard/missing cases,
falsy owners, relocation, physical copies, dynamic selection, effect detachment,
and substitution retries. Generated instruction metadata reproduces exactly;
previous recipe metadata is unchanged apart from the new overflow recipe and
token enum metadata.

Combined live corpus: 5,270 scenes / 20,672 operations. Semantic SHA-256
`533b2382c747d41552ac31ba3a07b7c89ab6797f51dd19d8df5324f09923c6a1`;
index `9b292ff8dea89e87aeb66e1037b346f7d566ddca633af7239009f2557bfd9da2`.
Unsupported aggregate/float owner coercions and source partial-failure paths
remain guarded. Other token/marker step families, remaining rule/body/phase
execution, complete engine/search support and the required two-turn throughput
remain open. No frozen Python changes, NN work, live-service restart, commit or
push is included.


Validation: all 270 Rust workspace tests pass, including restoration at every
live operation and the unchanged 6,592-scene / 27,145-operation canonical corpus.
All 26 focused source contracts pass. The independent release comparison passes
all 5,270 live scenes / 20,672 operations with full state/request/event/identity
parity (four bounded workers, 183.7 seconds while the Rust suite also ran).
Strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
Changes remain uncommitted; complete engine and search capabilities remain false.


### Native token lines and trails

Live PlaceTokensInLine and PlaceTokenTrail now generate typed source-equivalent
continuations. Lines resolve literal/context/actor origins in source order, scan
past terrain and occupied spaces in all six directions, and stop at the board
edge (or the source 50-step cap). Source zero/negative counts still select the
first empty space. Trails include the origin and exclude the destination,
compare exclusions by coordinates, and defer occupancy/terrain behavior to the
individual PlaceToken children instead of pre-filtering their target list.

Both operations reconcile old supply before any placement. Their Select/Remove
pairs use the effective token kind and owner override. The retry is a deep copy
of the authored operation, retaining its step ID and fields; line targets are
recomputed after removals. Output dictionaries have source allocation identity;
unused trail output dictionaries are not allocated. Clone/restore tests exercise
invalid-input retries and parent-worker isolation.

All 58 new source scenes / 397 operations match native state, request, event and
identity traces. The combined live corpus contains 5,328 scenes / 21,069 operations;
semantic SHA-256
`48dd3449b470460488823f7e0e99ea6ba01a4ff0f89e76e04fe10b86420ab61f`, index
`8d0856bba224756b2d98acd93875334bcd509a87798779b0863af5e779cd676c`.

Non-coordinate iterable exclusion members, nontext origin coercions and source
partial-failure paths remain explicit preflight guards. Batch placement and its
completability filters, rock-ultimate/turret/rune and remaining marker operations
remain required, along with the previously listed
remaining body/rule/phase execution, full error/model semantics, engine/search
integration and two-turn throughput target. Complete engine/search capabilities
remain false. Frozen Python, dependency locks, negative attacks and live services
are unchanged. No NN work, commit or push is included.


Validation: all 271 Rust workspace tests pass, including restoration at every
one of 21,069 live operations and the unchanged 6,592-scene / 27,145-operation
canonical corpus. All 30 focused source contracts pass. The independent release
worker matches all 5,328 live scenes with state/request/event/identity parity
(four bounded workers, 175.56 seconds while the Rust suite also ran). Strict
Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
The full corpus exporter now includes both token generators; running its complete
scene inventory reproduces the exact 5,328 library case names and 21,069 operations,
without duplicates. Changes remain uncommitted; complete engine/search support
and throughput remain outstanding.


### Native token batches and feasibility steering

Live PlaceTokenBatch now prechecks total supply, normalized slot counts and a
complete assignment before writing owner context or showing its optional prompt.
It supports mandatory aborts and explicit optional failures, zero-count batches,
opt-in decline, owner overrides, ordered old-token removals, sequential placement
and completion flags. Parent conditions remain scoped to the parent; generated
children use only the source opt-in gate.

Typed HexBatchCompletable and TokenRemovalCompletable rules use register overlays
for hypothetical slot choices and freed-space hints, without mutating the live
world or copying/serializing the whole context. They preserve source tile order,
distinct slots, committed/free/removable spaces and the removal budget. Live
FarthestEmptyAdjacent uses those hints, prior occupied slots and topology-aware
distances (including disconnected infinity ties). Sparse inputs retain only
referenced scalars and coordinate lists.

Source filter models remain physically shared across generated steps and nested
batch grids, while the grids, rows and slot-key lists are fresh containers.
Normalization appends one new obstacle model per slot. Live numeric selectors,
context flag keys and skip/active gates read arena operands so generated prompts,
ownership and opt-in gates use actual runtime values rather than cold placeholders.
Card-role caches refresh before feasibility queries.

All 97 new scenes / 1,189 operations match native state, request, event and object
identity traces. They cover reservation of a later slot, removal of a specifically
required old token, forced removals, infeasible/oversupply cases, optional/mandatory
failure, decline and farthest-adjacent hints. Actual source get_steps batch
boundaries from Stone Grip, Flashback, Déjà Vu and Unstable Timeline complete;
this does not claim their separate attack/swap/cleanup/ultimate consumers are done.
Six source contracts check behavior and model/container sharing. Native tests
exercise generated-graph restore, parent-worker isolation and an unbound slot
predicate rejecting before pop or owner-context writes. Batch feasibility also
rejects inconsistent tile/key geometry before mutation. Movement preserves the
source second live-gate check after clearing its success output; focused golden
regressions cover both active-if and skip-if self-reference.

Combined live corpus: 5,425 scenes / 22,258 operations. Semantic SHA-256
`a4b006f50cb635a028b8b0c0f73d6c99406b62f3b4c7daa3b312d7372f447681`;
index `cb1d166b46de8b013e660bbcaf7d2f51a6994babc71a5352cd6d0eb47bf1abfc`.
Source-generated batch defaults reproduce exactly; prior instruction metadata
remains unchanged. Full corpus regeneration includes the batch generator.

Unbound slot predicate families, degraded/raw-value and source partial-failure
semantics, remaining rock-ultimate/turret/rune/marker consumers, body construction,
rule/phase execution, make/unmake, hashing/evaluation, full iterative search and
integration/strength/throughput gates remain required. Complete engine/search
capabilities remain false. Frozen Python, dependency locks, negative attacks and
live services remain unchanged. No NN work, commit or push is included.


Final validation: all 275 Rust workspace tests pass, including restoration at
every one of 22,258 live operations and the unchanged 6,592-scene /
27,145-operation canonical corpus. All 36 focused source contracts pass.
The corrected release worker independently matches all 5,425 live scenes with
state/request/event/identity parity (four bounded workers, 191.9 seconds while
the Rust suite also ran). Full source scene enumeration reproduces the exact
saved case set and operation count without duplicates. Strict Clippy, Rustfmt,
Black/Ruff, whitespace and frozen-source/lock checks pass. Changes remain
uncommitted; the remaining full engine/search scope and throughput target are
still open.


### Native rock ultimate and complete imported Stone Grip flow

Live OfferRockUltimate now reads the actor hero's current level/ultimate slot,
used-context flag and individual/list hex keys. It preserves roster and on-board
piece order, tests topology in the source enemy-to-rock direction, and uses actor
immunity rather than attack immunity. Each affected piece is retained separately;
multiple pieces of one hero therefore discard from that hero's shared hand.

The source-generated Apply/Skip, condition, flag and ForEach/ForceDiscard
continuation runs through existing discard behavior. Accepting marks the chosen
used flag even for an empty victim hand; declining leaves it available for a
later placement group. The continuation retains frozen discard event behavior. Early gates leave
the previous affected-list object intact, and malformed coordinate guards reject
before pop or context writes. Imported source get_steps Stone Grip bodies now
complete from enemy selection through batch placement and the ultimate, including
locked/unlocked, exhausted supply, decline and invalid-input cases. General native
card-body construction and other Mrak card consumers remain required.

The new flow exposed a trace identity gap: source deepcopy retains a generated
step's textual ID while creating a distinct model. Rust now emits sparse trace-only
(binding index, arena object ID) pairs for generated models. Both normalizers use
those identities, preserve literal native-looking IDs and arbitrary user strings,
and strip the diagnostic field before comparison. These identities do not enter
persisted state or rule execution; copied step-id values remain unchanged.

All 129 new scenes / 1,478 operations match native state, request, event and
model/alias identity traces. Eight source contracts cover eligibility, repeated
offers, empty hands, piece ownership, actor immunity, directional isolation,
complete Stone Grip execution and copied/literal step identities. Native tests
cover private-worker restoration, copied ID value versus model identity, and
atomic malformed-coordinate guards with early used-flag bypass.

Combined live corpus: 5,554 scenes / 23,736 operations; semantic SHA-256
`c361cc7468b6dfc5c739f79e702d4d5cc489b40bf7ebe8d2144c7f189a45a5ec`, index
`f7f65d21da75a5c7dc06755121c57ee8375a718f02bc334f1b2c78c63639e0fb`.
Generated continuation metadata reproduces byte-for-byte (SHA-256
`bb923d9e483eee4ad61a3598a964b1ef9929cb034649d4821bbc39a996dda4fb`).
The full corpus exporter includes the rock-ultimate scenes.

Malformed/raw iterable and source partial-failure semantics, remaining turret,
rune/marker consumers, body construction, rule/phase coverage, make/unmake,
canonical hashing/evaluation, iterative team-aware search, hidden/simultaneous
planning, integration/strength gates and the 1,000 complete two-turn simulations
per second target remain required. Complete engine/search capabilities remain
false. Frozen Python, dependency locks, negative attacks and live services remain
unchanged. No NN work, commit or push is included.


Final validation: all 278 Rust workspace tests pass, including restoration at
every one of 23,736 live operations and the unchanged 6,592-scene /
27,145-operation canonical corpus. All 44 focused source contracts pass. The
final release worker independently matches all 5,554 live scenes with full
state/request/event/model-identity parity (four bounded workers, 249.13 seconds
while the Rust suite and source checks also ran). Full source enumeration
reproduces the exact saved case set and operation count without duplicates.
Strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
Changes remain uncommitted; full engine/search support and the required throughput
remain outstanding.


### Native turret lifecycle and imported Salvage Parts flow

PlaceTurret/RemoveTurret now execute directly against the object arena and native
spatial state. Creation uses source-frozen Turret defaults; reuse retains the
same model, name and obstacle flag while replacing its owner. A small copy-on-write
entity overlay registers new board entities without cloning world definitions.
Turrets remain board entities, excluded from unit/token queries. Placement allows
source-permitted terrain and virtual tiles, and emits an event even when its
position does not change. Literal/context owner precedence, actor fallback,
mandatory aborts, optional failures, output keys and live skip/active gates follow
the frozen source.

Spatial removal is shared with mine cleanup, but effect expiry remains specific
to tokens: removing a turret keeps its model, owner, linked effects and effects
list identity. Locations and snapshot aliases remain shared. Imported source
get_steps Salvage Parts bodies now complete all three choices: place, remove and
move, or remove and retrieve. Native construction of those card bodies remains
outstanding; other Trinkets consumers still require their unbound combat or
multi-selection steps.

The 61 new scenes / 382 operations match state, request, event and model/alias
identity traces. Seven source contracts cover owner precedence, model reuse,
mandatory failures, retained effects, spatial aliases, query classification and
all Salvage Parts branches. Two native regressions cover private-worker cloning
and restore, plus atomic invalid-owner/identity-collision guards.

Combined live corpus: 5,615 scenes / 24,118 operations; semantic SHA-256
`a2028a5e76be95e7307751e91ea4025cf38140342f092a3da6bd697adaf72a33`, index
`086639e293a8d354ba6b7c48bf012ac7dd41f104ae3871c70b448ae054165056`.
Source continuation metadata reproduces byte-for-byte with the added Turret
default (SHA-256
`73767e22f951131459ec97042705037988c5d5ad795194ebdcad21862b657d75`).

The full engine remains incomplete. Remaining source error/coercion semantics,
card/primary/passive/spell/filter bodies, world/phase/round execution,
make/unmake, canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-turn simulations per second are still required. Frozen Python,
negative attacks, dependency locks and live services are unchanged. No NN work,
commit or push is included.


Final validation: all 280 Rust workspace tests and 51 focused source contracts
pass. The Rust suite checks restore at every one of the 24,118 live operations,
plus the unchanged 6,592-scene / 27,145-operation canonical corpus. The rebuilt
release worker independently matches all 5,615 live scenes with full state,
request, event and model-identity parity (four bounded workers, 244.99 seconds
while other validation ran). Full source enumeration reproduces the exact case
set and operation count with no duplicates. Strict Clippy, Rustfmt, Black/Ruff,
whitespace and frozen-source/lock checks pass. Work remains uncommitted; complete
engine/search support and the required throughput remain outstanding.


### Native sequential MultiSelect and turret discard consumers

MultiSelect now reads live filters, bounds, pending input and its persistent
selections list from the arena. UNIT uses actual units and excludes illusion
tokens, unlike ordinary single-select; UNIT_OR_TOKEN includes tokens. Both pools
reuse native targeting and immunity rules. HEX uses the existing typed hex scans
and source coordinate strings, while requests retain the source SELECT_UNIT
shape. Other source target kinds produce an empty pool.

Accepted values append to the original selection list and copy it to output.
The next candidate scan sees that hypothetical append/output through sparse
filter-input overlays, without cloning or mutating the arena during preflight.
Actorless output instead aliases the original list. Done/Skip only finishes once
the minimum is met; exhausting candidates enforces the mandatory minimum, and
maximum checks retain the source's after-acceptance behavior. Handler gates leave
old output intact. Pending input clears independently of injected request tokens;
a completed Done/Skip retains its pending mapping as in Python. Scalar numeric
input uses Python-style text conversion, including float exponent thresholds.
Existing hex symbols are reused to avoid copying the shared symbol table.

The 276 new scenes / 1,900 operations match full state, requests, events and
model/alias identity. Nine source contracts cover candidate pools, actorless and
copied outputs, early completion, bounds, filter re-evaluation, gates, hex option
shape, injected requests and source-built turret discard bodies. Three native
regressions cover private worker cloning/restoration, atomic unsupported-filter
guards with actorless bypass, and early Done with retained request tokens.
Imported Self-Destruct completes through selection, discard loops and turret
removal; Emergency Protocol's discard path also completes. Its empty-hand defeat
path and general native card-body construction remain required.

Combined live corpus: 5,891 scenes / 26,018 operations; semantic SHA-256
`7c33047f422fafb7c78dc87b28f8aaee552cf5d2cb0b39e394e07d024d958ffb`, index
`9ffaa3d51130ed71a12b8120179c270796175e07d0f5a6023fd8bbf80afe58e6`.
Continuation metadata is unchanged. Aggregate-input/source-error semantics,
remaining card/primary/passive/spell/filter bodies, world/phase/round execution,
make/unmake, canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-turn simulations per second remain required. Complete engine
and search capabilities remain false. Frozen Python, negative attacks, dependency
locks and live services are unchanged. No NN work, commit or push is included.


Final validation: all 283 Rust workspace tests and 60 focused source contracts
pass. Restore is checked at every one of 26,018 live operations, alongside the
unchanged 6,592-scene / 27,145-operation canonical corpus. The rebuilt release
worker independently matches all 5,891 live scenes with full state, request,
event and model-identity parity (four bounded workers, 223.98 seconds while other
checks ran). Full source enumeration reproduces the exact case set and operation
count without duplicates. Strict Clippy, Rustfmt, Black/Ruff, whitespace and
frozen-source/lock checks pass. Work remains uncommitted. Full engine/search
coverage and the required complete two-turn throughput remain outstanding.


### Native defeat, minion protection, removal and game over

DefeatUnit now reuses typed combat rewards, assist distribution, bounty and effect
rules, then commits changes through arena handles. Dense mutable rows are staged
with shared immutable definitions; the arena and execution context are not cloned
for consequence calculation. Physical current cards resolve in their existing
played lists, retrieved cards still consume their turn slot, and unresolved,
defeated-hero and defeat-log lists retain source aliases. Piece defeats normalize
to the owner and schedule the source removal IDs. Markers on the victim return;
markers merely placed by the victim remain. Premature effect expiry uses the
physical card owner, exempts token-bound effects, replaces the effect list even
when nothing expires, and never runs natural finishing payloads.

Protected-minion flow includes ordered retries, qualifying-card prompts, discard
costs and token sacrifices. A card save still records the defeat and pays kill
coins; a totem sacrifice does neither. Genuine minion defeats pay eligible bounty
coins. Bounty relations now use the beneficiary's actual team, while assist
recipients come from the killer team's roster. Repeated references to one bounty
model share its coin budget through a search-local arena binding; distinct
models with equal effect IDs remain independent. This fixes a demonstrated extra
coin payout without serializing cache identities.

RemoveUnit only changes board presence and retains models/effects. Game over
sets the winner/condition, clears the original execution/input lists, preserves
retained model aliases and stops all further steps. Terminal shared states can
be restored, including source-inert stale stacks. ForceDiscardOrDefeat now binds
its actual selection/discard or defeat children with explicit, keyed and actor
kill-credit fallback.

The earlier MultiSelect turret-card fixtures left enemies off board and therefore
did not exercise discard children. Twelve cases now place two real enemies and
assert that selection is nonempty. Added Self-Destruct/Emergency Protocol cases
cover real discard, empty hands, both victims and annihilation before turret
removal. Overall this stage adds 245 scenes and replaces those 12 cases: 257
changed cases / 1,896 operations match full state/request/event/model-identity
traces. Eleven source contracts and four native regressions cover rewards,
slots, aliases, source expiry, protection/retries, terminal behavior, kill credit,
team/roster distinctions, shared budgets and private workers/atomic guards.

Combined live corpus: 6,136 scenes / 27,860 operations; semantic SHA-256
`80356afb8ecb621e2e714960e4d3745a66509d816e2188f2cd7c3187ab466d7a`, index
`72353b4f645c532037c386e2f208ce47c423aa88761948f19b3121a9ffa5a3d1`.
Continuation metadata is unchanged. Raw coercion/error semantics, remaining
card/primary/passive/spell/filter bodies, world/phase/round execution,
make/unmake, canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-turn simulations per second remain required. Complete engine
and search capabilities remain false. Frozen Python, negative attacks, dependency
locks and live services are unchanged. No NN work, commit or push is included.


Final validation: all 287 Rust workspace tests and 71 focused source contracts
pass. Restore is checked at every one of 27,860 live operations, alongside the
unchanged 6,592-scene / 27,145-operation canonical corpus. The final release
worker independently matches all 6,136 live scenes with full state, request,
event and model-identity parity (four bounded workers, 323.59 seconds while other
checks ran). Fresh source enumeration reproduces the exact case set and operation
count without duplicates. Strict Clippy, Rustfmt, Black/Ruff, whitespace,
frozen-source/lock and continuation-metadata checks pass. Changes remain
uncommitted. Complete engine/search coverage and two-turn throughput remain open.

Catalog coverage audit: the frozen source has 33 heroes and 596 ordinary card
faces, with 38 reviewed native primary recipes. Thus 558 referenced card-effect
IDs are outside that recipe catalog; this is not a count of bespoke implementations
needed, since shared primitives/default behavior and many passive bodies already
exist. All 22 registered spell effects still need native casting/body dispatch.
The arena's general card-resolution/body-construction path is not connected, so
38 recipes must not be reported as 38 fully playable cards.


### Live combat resolution and attack-local queries

ResolveCombat, SnapshotAdjacentHeroes and CheckTargetDefeated now execute through
arena bindings. Shared combat rules support sparse register writes, negative
attacks, fractional/bool defense values, source flag precedence and unused-value
cancellation. Integer intermediate sums use i128; float/int comparisons retain
precision above 2^53 and handle finite-input overflow without exporting infinity.
Adjacent snapshots apply the source's explicit team, adjacency and immunity
predicates, independently of ordinary target-selection validation. History
queries retain exact victim IDs, normalize killer ownership, and preserve slice,
tuple, scalar-text and alias behavior. Python scalar formatting is shared with
multi-selection. No runtime Python or source JSON evaluation is introduced.

This stage adds 419 scenes / 1,794 operations, ten source contracts and three
native regressions. The combined live corpus has 6,555 scenes / 29,654 operations;
semantic SHA-256 `745e3f00d18bafa9d42edb9cf8bd0a661c9fcb3102c2a907db8e55eb0fb81664`,
index `6a3732a4e1081ed1be16d2d78de43adfe059ef9e1b52138ce348ee96adb5326e`.
All 290 Rust tests and 81 focused source contracts pass. Independent release
verification matches full state/request/event/model identity across the corpus
(four workers, 396.25 seconds alongside other checks). Fresh source enumeration
matches the exact case set and operation count. Canonical fixtures and frozen
Python are unchanged. Continuation metadata remains unchanged.

Attack expansion, reaction windows/defense text and general arena card body
construction remain separate required connections. Primary recipe coverage is
still 38 of 596; no native spell casting/body dispatch is complete. Full engine,
search, strength and 1,000 complete two-turn simulations/second gates remain open.


### Live attack expansion

AttackSequence now expands through arena bindings, sharing typed target/stat
calculations with the existing executor. It preserves negative damage and range,
bonus coercion, current performed-card/basic status, board-piece stat binding,
per-attack flags and exact child order. Defeat marks update the original dictionary;
log lengths support source lists, tuples, dictionaries and Unicode strings.
Optional selection resumes through a shallow model copy: its textual step ID and
filter references are retained while model identity stays distinct. Skipping never
opens defenses or changes actor. Extra target-filter objects remain shared with
the original card/step payload. Unsupported defense children remain explicit
pre-pop guards and can be restored as data; this is not yet a complete attack.

Seventy new source scenes / 86 operations match full state/request/event/model
identity, including conditional auras, nested performing-card context, multi-piece
actors and shared history/filter graphs. Ten focused source contracts pass. Three
native regressions cover private-worker restore, unused-value/atomic bonus guards
and restorable guarded defense continuations. Strict Clippy, Python Black/Ruff and
Rustfmt pass. The expanded corpus has 6,625 scenes / 29,740 operations;
semantic SHA-256 `ecb82f65630acbc477b13cac1cc6792b8030237c4053044d21aff0d8bcfbd76e`,
index `37d0c3ba3c99cda5673a05db13fb9f99e48823c4dbfc0b95f751d06484b2de5b`.
Final validation: all 293 Rust workspace tests pass, including restore at every
live boundary and the unchanged 6,592-scene / 27,145-operation canonical corpus.
Independent release verification matches all 6,625 live scenes / 29,740 operations
with full state/request/event/model identity (four workers, 279.62 seconds while
other checks ran). Fresh source enumeration reproduces the exact names and
operation count without duplicates. The 81 existing focused source contracts and
ten added attack contracts pass. Strict Clippy, Rustfmt, Black/Ruff, whitespace,
frozen-source/lock and unchanged continuation-metadata checks pass.

ReactionWindow, ResolvePreActionMovement/Discard, ResolveDefenseText and on-block
body dispatch remain required to finish the live attack flow. General card and
spell dispatch, all remaining bodies, complete phase/world/round execution,
make/unmake, hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-turn simulations/second remain open. Recipe counts are unchanged.
No frozen Python, dependency locks or live services were changed. Work remains
uncommitted; complete engine/search capabilities remain false.

Next attack-flow integration details from the frozen reaction implementation:

- Separate shared defense-menu/stat computation from arena writes. Read only
  target, primary-defense block, initiative and attack operands; remove all five
  per-defense flags on every source entry, including resumed input.
- Preserve the original action_type_stack list when appending the previous
  action; the existing dense helper's copied list is not an arena implementation.
- Offer hand plus active discard-shield cards using source Python card equality
  for deduplication. CardId identity alone is insufficient for distinct equal cards.
- Keep piece defender IDs for positions and owner IDs for prompt/card ownership.
  PASS/non-hero paths, stale choices, request regeneration and pending model
  aliases need every-boundary fixtures before enabling general continuation.
- Queue DiscardCard before BEFORE_DEFENSE with the owner binding. Then port
  defense-text's off-board guards, defense-specific/fallback builders and actor
  wrappers, followed by on-block bodies. Compare against the frozen Python
  implementation throughout; no oracle source edits or repinning.


### Live reaction windows and generated actor operands

ReactionWindow now executes through arena bindings. Ordered defense menus share
stat calculation with the typed core; active discard-shield cards are deduplicated
by Python model equality before their option stats are computed. Requests preserve
bool, float, negative and absent attack values, including Python prompt formatting.
PASS, non-hero targets, stale choices, request tokens, piece ownership, primary
blocking, initiative defense and the five per-defense resets follow the frozen
handler. A target key that aliases a reset flag is read after that flag is removed.
Selected cards queue the actual DiscardCard then BEFORE_DEFENSE consumers.

The original action_type_stack list is appended in place. Existing scalar/tuple
stacks are retained when the previous action is false and no append occurs;
aggregate previous-action values retain their identity. Invalid writable-stack
and cyclic cases are rejected before popping a step or resetting flags. Pending
payload aliases survive detachment; a reused reaction model preserves a pending
PASS or reopens a discarded-card choice as the source does.

Real AttackSequence integration exposed cached SetActor operands: the generated
actor_key/save_key fields were ignored. SetActor now preflights live fields,
implements save-before-read key aliasing, and preserves scalar conversion and
symbol-table synchronization. Minion/piece/hero actor transitions and custom save
keys now match Python. No runtime Python or JSON state evaluation was introduced.

This stage adds 265 scenes / 2,214 operations, ten source contracts and three
native regressions, and advances the previous parked-reaction regression to the
remaining defense-text boundary. Coverage includes all 33 heroes, distinct equal
and unequal shield cards, all four Razzle piece IDs, shared action/pending graphs,
reused windows and attack-to-reaction integration. Focused native checks pass;
all ten source contracts pass. The combined corpus has 6,890 scenes / 31,954
operations; semantic SHA-256
`6aae08a1e64a41b394a5207e70880e7e95010499f89656a37ec9b949367ba71b`, index
`f9408712da249effaaccea8e9b52ffd61153e1c6974faa1f4526aa58d51fd129`.
Final validation: all 296 Rust workspace tests pass, including restore at every
one of 31,954 live operations and the unchanged 6,592-scene / 27,145-operation
canonical corpus. Independent release verification matches all 6,890 live scenes
with full state/request/event/model identity (four workers, 497.14 seconds while
other checks ran). Fresh source enumeration reproduces the exact case names and
operation count without duplicates. The 91 existing focused source contracts and
ten new reaction/actor contracts pass. Strict Clippy, Rustfmt, Black/Ruff,
whitespace, frozen-source/lock and unchanged instruction-metadata checks pass.

Pre-action movement/discard, defense-text/on-block bodies and general arena
card/spell body construction remain required. Raw coercion/partial-error semantics,
all remaining bodies and filters, full world/phase/round execution, make/unmake,
canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-turn simulations/second remain open. Primary recipe coverage
remains 38 of 596; all 22 spell effects still need native casting/body dispatch.
Engine/search-complete flags remain false. Frozen Python, dependency locks,
continuation metadata and live services are unchanged. Work remains uncommitted.

Next full consumers, based on the frozen source:

- Pre-action movement: resolve literal/key hero precedence, take the first live
  PRE_ACTION_MOVEMENT row with source activity gates, use max_value or one, and
  generate the optional HEX selector plus non-action MoveUnit with exact filter
  models and prompt. Cover real Misa cards, source expiry and piece binding.
- Pre-action discard: retain the processed_effect_ids list on the actual step,
  requeue the same model between triggers, and refresh scope/immunity/board state
  after each discard or defeat. Only a real discard consumes an effect; an empty
  hand defeat retains it and credits its creator. Respect abort_on_defeat=False
  during defense and the source's first-matching-effect order.
- Then connect defense-specific builders and get_steps fallback with source
  actor wrappers/off-board guards, followed by on-block builders and general
  card resolution. Preserve the full body catalog and performance/search goals;
  normal no-op branches do not establish complete consumers.


### Live pre-action movement and reentrant disruptors

ResolvePreActionMovement and ResolvePreActionDiscard now execute through arena
bindings. Their activity/scope predicates are shared with the typed core.
Movement takes the first active provider, preserves max_value-or-one semantics
and exact optional selector/filter/movement payloads, and treats the move as a
non-action. Source negative-range prompt/no-movement behavior is retained.

Disruptors update the original step's processed_effect_ids list and requeue that
same model between actual discard/defeat children. Every entry refreshes live
hand/board/card state and re-checks effect activity, scope and immunity. Duplicate
IDs are processed once; safe empty hands retain effects. A real discard removes
all rows with that ID, refreshes carrier active flags and retains detached effect
aliases without running premature finishing text. Empty-hand defeats retain the
provider and credit its creator. Off-board abort behavior respects defender mode.
Processed IDs are explicitly mutable step state; unbound role aliases, invalid
appends and child-generation exhaustion are guarded before authoritative writes.

This stage adds 173 scenes / 1,818 operations, including registered Misa
Focus/Discipline/Mastery and Trinkets Disruptor Jolt/Pulse/Grid builders, mines,
source expiry/immunity, multi-piece binding, shared/reused steps and processed
lists, and 32 full attack/pre-action integrations up to defense text. All new
scenes match full state/request/event/model-identity traces. Ten focused source
contracts and three native regressions cover behavior, private-worker restore,
processed-list atomicity and generation exhaustion. Strict Clippy and Python
lint pass. The combined corpus has 7,063 scenes / 33,772 operations;
semantic SHA-256 `7055ec1bb625675e743a6f0b4ac96e8c090e6806cead7038a06ee5b3c09293a4`,
index `df07817830c09e3e48ed36bcb2b3d25f0158bd2f2be5853dfa1009a245fc6322`.
Final validation: all 299 Rust workspace tests pass, including restore at every
one of 33,772 live operations and the unchanged 6,592-scene / 27,145-operation
canonical corpus. Independent release verification matches all 7,063 live scenes
with full state/request/event/model identity (four workers, 365.23 seconds while
other checks ran). Fresh source enumeration reproduces the exact case names and
operation count without duplicates. The 101 existing focused source contracts
and ten new pre-action contracts pass. Strict Clippy, Rustfmt, Black/Ruff,
whitespace, frozen-source/lock and unchanged instruction-metadata checks pass.

Defense-text/on-block builders and general arena card/spell construction remain
required, along with raw coercion/partial-error semantics, all remaining bodies
and filters, full world/phase/round execution, make/unmake, canonical hashing and
evaluation, iterative team-aware minimax/alpha-beta, hidden/simultaneous planning,
parallel search, integration/strength gates and 1,000 complete two-turn
simulations/second. Primary recipes remain 38 of 596 and all 22 spells still need
native casting/body dispatch. Engine/search-complete flags stay false. Frozen
Python, dependency locks, instruction metadata and live services are unchanged.
Work remains uncommitted.

Next defense-body fidelity requirements confirmed in the frozen source:

- Distinguish a defense builder returning None (invoke get_steps fallback) from
  an explicit empty list (no fallback). Unknown/facedown/missing cards and effects
  skip before body/stat evaluation where the source does.
- Defense/on-block hooks compute stats for the attacked board ID only when its
  owner matches the defender; otherwise use the owner. A get_steps fallback
  computes owner-based primary stats separately, so do not reuse piece stats.
- Builders run while the current actor is still the attacker. Only the returned
  steps are wrapped in live SetActor steps with the defender and the source's
  distinct defense/on-block save keys.
- Preserve source-card binding on returned/nested steps and token-effect
  exemptions. The defense-text handler checks board presence; on-block does not
  add that guard. On-block gates on both primary defense and block success.


### Live singleton markers and spatial swaps

The authoritative arena now executes `place_marker`, `remove_marker`, and
`swap_units` from live step/context operands. Marker placement mutates the
existing model, creates a singleton only when missing, preserves the original
marker dictionary and all shared-model aliases, and refreshes every corresponding
native stat-cache row. Removal retains the model and emits the source event even
when the marker is already in supply; missing markers remain absent. Copied-action
suppression uses the shared performer policy, while placement retains the raw
textual target (including `SKIP`), the current actor/system source, and the exact
post-placement passive trigger. Bain's registered ultimate integration follows
source behavior for all three marker types, including empty hands.

Swaps validate both exact unit IDs in source order, honor mandatory aborts,
remove both entries before placement, reuse the original immutable Hex objects,
and preserve dictionary/tile/model aliases and insertion order. Self-swaps are
real delete/reinsert operations and emit an event. Token, hero-piece, off-board,
immunity, live actor/context, repeated-step and snapshot-alias cases are covered.
No token-removal or movement hooks are introduced by swaps.

The new frozen-source corpus adds 274 scenes / 1,233 operations (175 swap scenes /
708 operations and 99 marker scenes / 525 operations). Independent native
comparisons match full state/request/event/model identity for both additions.
Thirteen source behavioral contracts pass. Three new native regressions check
worker-private live operands, shared marker cache rows and marker-generation
failure atomicity. The combined corpus contains 7,337 scenes / 35,005 operations.
Final validation: all 302 Rust workspace tests pass, including restore at every
one of 35,005 live operations and the unchanged 6,592-scene / 27,145-operation
canonical corpus. Independent release verification matches all 7,337 live scenes
with full state/request/event/model identity (four workers, 381.09 seconds while
other checks ran). Fresh source enumeration reproduces the exact case names and
operation count without duplicates. All thirteen new source contracts pass;
strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
Instruction metadata remains unchanged. Semantic SHA-256
`90da4378db86154f0263c4f0964803c24abec05a52dd24f07826cc372ade9838`,
index `d3b62b28e2705053965d74192f933b29c4de1c3eb32e25736a061207e8bad28b`.

This connects shared dependencies for defense builders; it does not implement
the remaining defense-text/on-block catalog or general arena card/spell bodies.
Primary recipes remain 38 of 596; all 22 spells still require native casting/body
dispatch. The full world/phase/round, remaining bodies/filters, make/unmake,
canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength and 1,000
complete two-turn simulations/second requirements remain open. Frozen Python,
dependency locks and live services are unchanged. Work remains uncommitted.


### Live effect creation and shared typed effect parsing

`CreateEffectStep` now reads its operands from the authoritative arena. A shared
typed record parser serves both cold effect import and live creation; runtime
construction does not serialize through JSON. Dynamic origins, card/action
context, named colors, attacker exceptions, token substitutions, complete effect
payloads, activation, board-piece origins, entity IDs, card flags and creation
events feed the existing native world creation rules. Card-bound duplicates
remain pure no-ops, including dormant instances, and a newly created row whose
ID collides with an existing row suppresses its event without dropping the row.

Finishing payloads preserve the source graph transformations: dynamic color and
nested-list resolution use shallow model copies, each finishing root receives
its own deep copy, and the patched `AnyStep` validator on
`ActiveEffect.finishing_steps` runs the root's nested-step isolation a second
time. This second pass splits immediate repeated child steps even when no color
was changed. Original templates stay untouched. Scope models remain shared
unless dynamic or board-actor binding requires a copy; validated effect lists
are fresh, immutable Hex models retain identity, and the active-effects list is
appended in place. Native rules use typed cache payloads while the arena retains
physical model/list/step identity.

The frozen-source addition covers 152 scenes / 646 operations: all effect kinds,
card lifecycle and duplicate/colliding-ID modes, every scope/origin mode, static
barriers, nested performing cards, nondefault payloads, dynamic colors and
multi-level finishing aliases. All new traces match full state/request/event/
model identity. Eight source behavior contracts pass. Three native regressions
cover worker-private live payload/scope rebinding, constructor isolation and
entity-counter guard atomicity (duplicates need no new counter). The combined
corpus contains 7,489 scenes / 35,651 operations.
Final validation: all 305 Rust workspace tests pass, including restore at every
one of 35,651 live operations and the unchanged 6,592-scene / 27,145-operation
canonical corpus. Independent release verification matches all 7,489 live scenes
with full state/request/event/model identity (four workers, 405.96 seconds while
other checks ran). Fresh source enumeration reproduces the exact case names and
operation count without duplicates. All eight new source contracts pass;
strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
Instruction metadata remains unchanged. Semantic SHA-256
`b967f5ccd1d4781d03c738c6274de855897f0101f51ea57ade06c8409faee255`,
index `f0cea285d789b89195310acc7600466a88a83162296f08859159dd674d7c29a6`.

Defense-text/on-block construction and general arena card/spell dispatch remain
required. Primary recipes remain 38 of 596; all 22 spells still require native
casting/body dispatch. Raw coercion/partial-error semantics, all remaining bodies
and filters, full world/phase/round execution, make/unmake, canonical hashing and
evaluation, iterative team-aware minimax/alpha-beta, hidden/simultaneous planning,
parallel search, integration/strength gates and 1,000 complete two-turn
simulations/second remain open. Engine/search-complete flags stay false. Frozen
Python, dependency locks and live services are unchanged; work is uncommitted.


### Arena defense-text and on-block builders

The arena now constructs the 21 distinct registered defense builders and seven
on-block builders, covering all 26 primary DEFENSE/DEFENSE_SKILL faces. The
legitimate `get_steps()` fallback, Mortimer's Awaken, uses separately computed
owner-based primary stats. An explicit empty defense list does not invoke that
fallback. Unknown, facedown, missing, secondary and failed-block paths retain the
source's gates and evaluation order. Defense text checks board presence;
on-block text does not add that guard. Builders evaluate with the attacker still
acting; generated steps use the distinct source defense/on-block actor save keys
and restore the original actor afterward.

A reproducible cold catalog records all 596 effect method identities and 56
source-derived recipes. Shared recipe compilation/instantiation now preserves
model/list/filter aliases and registers nested stored step programs as well as
ordinary loop continuations. Source-card attribution and token-effect exemptions
come from the actual registered builders. Dynamic ranges/radii/bonuses, bounty
state, Dodger spawn/terrain/Tide/topology rules, Razzle attacked-piece bonuses and
card-owner rune rules are read live. Copied smoke defenses retain the missing
owner-origin result even when the attacked piece is present. Generated finishing
steps and request IDs can be read into typed effect cache payloads without a
JSON round trip.

Awaken required recursive And/Or hex predicates and hex exclusions. Composite
scans preserve empty AND/OR behavior and short-circuit inactive branches. Hex
exclusion retains model provenance: coordinate dictionaries, tuples and nested
lists do not compare as a saved Hex. Sparse contexts and provisional batch
overlays carry that provenance without changing the world. Live context addition
also reads its current key/amount, including Razzle's generated bonus, instead
of using a cached recipe operand.

The source addition has 750 scenes / 5,167 operations: full defense continuations
for every primary defense face, all on-block hooks, complete attack/marker
integrations, repeated effects, actor/card/list aliases, rune choices, foreign
owner ordering, smoke/glitch placement and swaps, four optional zombie
placements, spawn/barrier/topology conditions, copied smoke cards and focused
composite/exclusion probes. Both source additions independently match complete
native state/request/event/model identity. Twelve source behavior/catalog
contracts pass. Three native regressions cover generation failure atomicity,
private live arithmetic and shared filters/stored-program registration. The
combined corpus has 8,239 scenes / 40,818 operations.
Final validation covers all 308 distinct Rust workspace tests: 307 pass in the
final workspace run, and the unchanged every-boundary restore test completed
separately across all 40,818 operations. Its saved run had only an obsolete
unsupported-defense assertion, now replaced by a passing successful-continuation
restore regression. The unchanged 6,592-scene / 27,145-operation canonical corpus
passes. Independent release verification matches all 8,239 live scenes with full
state/request/event/model identity (four workers, 580.52 seconds while other
checks ran). Fresh source enumeration reproduces exact names/counts without
duplicates. All twelve source contracts, strict Clippy, Rustfmt, Black/Ruff,
whitespace and frozen-source/lock checks pass; prior instruction metadata is
unchanged. Semantic SHA-256
`15448401576dd87a275929244b2c28cc8b9e2e3c69277039fba5412b50253f33`,
index `72f683539a23c50ff92f3fd02fd5f91563d855d8c56c23d246221109cf1eeb7e`.

General arena primary/card/spell dispatch, remaining bodies/filters, raw
coercion/partial-error semantics, full world/phase/round execution, legacy dense
bridge retirement, make/unmake, canonical hashing/evaluation, iterative
team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel search,
integration/strength gates and 1,000 complete two-turn simulations/second remain
required. The separate primary recipe catalog stays at 38 of 596 and all 22
spells still require native casting/body dispatch. Engine/search-complete flags
stay false. Frozen Python, locks and live services are untouched; work remains
uncommitted.

### Native resource primitives (2026-11-02)

The authoritative arena now executes `GainCoinsStep`, `LoseCoinsStep`, and
`RestoreSpentLifeCounterStep`. Operands are read from the live step and context
before consumption. Gains preserve signed amounts and boolean event metadata;
losses use the source integer conversion, cap by the current gold, and expose
the actual loss. Life restoration uses the selected hero's team and its starting
supply, emits nothing for nonpositive restoration, and preserves owner-versus-
piece event identities. Changes update existing arena models and native caches;
unsupported arithmetic is guarded before stack/model mutation. Noninteger gold
gains and Python partial-error arithmetic remain outside the supported boundary.

The new source suite has 51 scenes and 207 operations covering repeated shared
steps, dynamic operands, model aliases, guard conditions, empty/missing IDs,
integer-coercible loss amounts, Life caps, and Razzle pieces. Independent source
contracts cover resource outcomes and event semantics. These are prerequisites
for shared card/spell execution, not additional complete card/spell bodies: the
reviewed primary catalog remains 38/596 and full native spell casting remains
0/22. General dispatch, remaining engine behavior, search and the full performance
acceptance gates remain required. No Python source, lock, or live service changed.

Validation: 310 Rust tests pass in the workspace run; the prior expensive
all-corpus restore gate was filtered rather than rerun. The new dedicated
resource test restores all 207 new operation boundaries and verifies worker
isolation. All 8,290 live scenes and the unchanged 6,592 canonical scenes pass
the workspace CLI parity tests. Independent release-mode verification matches
the 51 new scenes, including full state, requests, events, and model identities.
Five independent source contracts, strict Clippy, Rustfmt, Black/Ruff, whitespace,
and frozen-source/lock checks pass. The live corpus is 8,290 scenes / 41,025
operations; semantic SHA-256
`c89d2f3c9b5517c6c9c292fd5573b5f85b8ea81239414e3754c399f28e9be78e`,
index `13a113cb40dca3a0609e88d549a370c453b776d55c30f242c981f94c51457f02`.
Work remains uncommitted, and the full engine/search objective remains open.

### Authoritative primary text dispatch (2026-11-02)

`ResolveCardTextStep` now runs in the object arena. It reads the live hero operand
and that hero's current physical card (the step's literal card ID is deliberately
ignored), writes the current card context even for facedown cards, and uses
owner-based computed stats for registered builders. All 38 reviewed custom
primary builders now allocate their nested source models/steps with physical
identity metadata. The 45 referenced registered base no-op builders remain empty
rather than falling back to generic actions. Unregistered effects use the exact
movement, attack, defense-log, skill-log or no-action fallback, including negative
values, the range-one default, and DEFENSE_SKILL's no-op behavior. Card definitions
now distinguish an absent effect ID from an empty one, as the source does.

The new cold `primary-builders.json` catalog records all 596 ordinary method
identities and the separate 22-name spell registry. Its source generator asserts
that reviewed bodies share no models across separate root steps, retains aliases
within each root, records ranged-dependent attack operands, and uses the public
binding API so created effects retain their actual source card. Generated nested
loops bind their executable programs. Empty bodies need no generated step IDs;
nonempty construction checks ID capacity before changing the stack or context.

The 336 new source scenes / 1,161 operations include construction with ordinary,
negative/buffed, ranged, facedown and skipped cards; every referenced base empty
builder; missing-current/hero and fallback paths; mandatory-fizzle boundaries;
and real placement, swaps, marker placement, retrieval and forced-discard flows.
Nine independent source contracts pass, and release Rust matches all new scenes
with full state, request, event and physical model identity.

This is primary text dispatch, not complete arbitrary-card/action execution.
`PerformPrimaryActionStep`, `PerformCardActionStep` and casting still need their
full native action lifecycle. Remaining custom builders and spell bodies are
still required. Push, record-hex, computed-hex, adjacency-check and repeat-once
continuations in reviewed bodies currently import/restore as explicit parked
families and guard before execution; their arena consumers remain to implement.
The existing dense bridge, remaining full-world/round rules, raw partial-error
semantics, make/unmake, canonical hashing/evaluation, team-aware iterative search,
hidden/simultaneous decisions, parallel search, integration/strength gates and
1,000 full two-turn simulations/second remain required. Engine/search completion
flags stay false. Python sources, locks and live services remain untouched.

Validation: 315 Rust workspace tests pass, including restore/replay at all 1,161
new primary boundaries and the full 8,626-scene live and 6,592-scene canonical
parity replays. The older expensive all-corpus restore test was filtered rather
than rerun. All nine independent source contracts, independent release parity
for the 336 new scenes, strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen
Python/lock checks pass. The combined live corpus is 8,626 scenes / 42,186
operations; semantic SHA-256
`da4ad3904351940abac1ba427fe39e811083631d7f84a70523c39307b4a8d41e`,
index `cd5928e47ce863889aef0edfa81c051fb530187b55ec2380fdd708c6df409747`.
Work remains uncommitted and the complete engine/search objective remains open.

### Coordinate and repeat continuations (2026-11-02)

The arena now executes `RecordHexStep`, `ComputeHexStep`, `CheckAdjacencyStep`
and `MayRepeatNTimesStep`/`MayRepeatOnceStep`. Recording allocates a fresh mutable
coordinate dictionary. Computing a destination uses Python floor division and
allocates a fresh immutable Hex with the source deepcopy-sharing contract, even
when scale zero produces coordinates equal to the target. Adjacency reads direct
entity positions and uses directional topology connectivity; it deliberately does
not resolve Razzle's owner to the acting piece. Missing inputs preserve outputs
for recording/computation and write false for adjacency.

Repeat reads live counters, conditions, actor, template handles and pending input.
It preserves request tokens, increments only on YES, deep-copies each template
root separately without inventing fresh step IDs, and requeues the same live
repeat only while another iteration remains. Lane-boundary checking is queued
before offering the repeat; the lane-push consumer remains separately guarded.
Request/step counter guards precede mutation. The new source alias fixtures
exposed a second source `GameStep` validation at `StepResult.new_steps`: nested
step fields are isolated again after copying. Repeat, ForEach and effect-finishing
validation now share that same native isolation routine rather than duplicating
it or preserving aliases Python splits.

The 99 new source scenes / 1,070 operations cover direct and keyed lookups,
missing/falsy inputs, distinct fresh dict/Hex objects, scale zero/negative values,
topology bridges and directional isolation, Razzle owner/piece identities, repeat
limits/declines/prevention, request handling, live template edits, nested repeats,
ForEach-return validation, lane-check scheduling and 28 complete primary-text
flows. These include Onslaught's actual defeat and follow-up movement, Blink
Strike's computed placement, and Sabina's repeated minion movement. The complete
repeat text flows use no active battle zones; they do not prove lane resolution
or a complete game. Eleven independent source contracts pass.

Push remains parked. General arbitrary-card action/casting dispatch, the other
primary/spell builders and filters, remaining arena world/phase/round consumers,
legacy dense bridge retirement, raw exception/partial-error fidelity, make/unmake,
canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/second remain required. Source-invalid
computed vectors are guarded before consumption; Python's partial-error boundary
is still part of that unfinished fidelity work. Engine/search completion flags
remain false. Frozen Python, locks and live services are untouched; all current
work remains uncommitted.

Validation: 321 Rust workspace tests pass. The dedicated utility test restores
and replays every one of the 1,070 new operation boundaries in private workers.
The older expensive all-corpus restore gate was filtered rather than rerun.
Workspace CLI parity passes the full 8,725-scene live corpus and unchanged
6,592-scene canonical corpus. Independent release parity matches all 99 new
scenes with full state, request, event and physical model identity. All eleven
source contracts, strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen
source/lock checks pass. The live corpus totals 8,725 scenes / 43,256 operations;
semantic SHA-256
`cb692f51f22f44dd969339fc1b373907e59d9cf99664c581ff414e2fdb8dde00`,
index `2e9964c68c9506e80d6f26f4fa8b6c1041f518ee2394a378efe3752c6a05f76e`.
No live service changed and the work remains uncommitted. The full engine/search
objective, including its two-turn throughput gate, remains open.

### Authoritative push execution (2026-11-02)

`PushUnitStep` now executes through the arena. Its typed path calculation uses
actor-specific obstacles, checks topology from both the start and each path edge,
crosses passable tokens, trims illegal landing spaces, and reports actual distance
and collision. The same source displacement validator handles mandatory aborts,
optional denial, heavy minions, tokens, multi-piece heroes and effect restrictions.
Literal/key precedence, source fallback and dynamic integer distance conversion
follow Python. Successful movement updates the existing location/tile containers
with a fresh immutable landing Hex; zero/negative distance keeps the old location
model but still emits the source push event and schedules after-push passives.
Tokens omit that passive, and a token-only push needs no step-generation capacity.

Enemy mines are collected in path order between the start and actual landing.
The context snapshot and queued trigger's literal mine list have separate object
identities, retaining the exact victim even if later work changes shared keys.
Mine triggers execute before after-push passives. Generation capacity is checked
before changing any position, context, or stack. Worker tests also mutate the
live obstacle-ignore operand and verify the root session remains unchanged.

The source suite adds 252 scenes / 1,635 operations: prior push/vector/filter
scenarios, the displacement effect matrix, actor/duration/scope variants,
passable tokens and ordered mines, terrain/edges/ignored obstacles, topology
anchors, and six push-card bodies with ordinary/skip/invalid and explicitly
positive choices. Default-first zero-distance or out-of-position card choices
are retained alongside the six positive trajectories. All nine independent source
contracts and the four native guard/identity regressions pass.

Push is no longer a parked primary continuation. General card/action menus,
re-performance and casting lifecycles remain incomplete, as do other primary and
spell builders, remaining filters and arena world/phase/round consumers, raw
exception/partial-error behavior, dense bridge retirement, make/unmake, canonical
hashing/evaluation, iterative team-aware minimax/alpha-beta, hidden/simultaneous
planning, parallel search, integration/strength gates and the required 1,000
complete two-game-turn simulations/second. Engine/search completion stays false.
Frozen Python, locks and live services remain untouched; work is uncommitted.

Validation: 326 Rust workspace tests pass, including restore/replay at all 1,635
new push boundaries and full parity replays of the 8,977-scene live and unchanged
6,592-scene canonical corpora. The older expensive all-corpus restore gate was
filtered rather than rerun. Independent release parity matches all 252 new
scenes with full state/request/event/model identity. Nine source contracts,
four native guard/identity regressions, strict Clippy, Rustfmt, Black/Ruff,
whitespace and frozen-source/lock checks pass. The live corpus is 8,977 scenes /
44,891 operations; semantic SHA-256
`3a3b2dc3dc226076c3c2be2a83880e4c63b4916b617d04ff9be8c95509004f76`,
index `0a05161bb62436186af4cb970ebe7976e58ba6b2a60880755277593a8d98addf`.
No live services changed, work remains uncommitted, and the full engine/search
objective and throughput gate remain open.


### Authoritative card-action menus and acting-piece selection (2026-11-02)

`ResolveCardStep` now reads live arena hero/card operands and reuses typed action
rules for the source menu and decision facts. Selecting an action writes the
actual current card/action context and allocates source-derived child models in
execution order: piece choice, before-action hooks, primary pre-movement/discard
and text or secondary primitives, then the matching after-action hooks and basic
context fields. Invalid choices preserve pending input while re-prompting.
Facedown cards, missing/offboard heroes, signed attack values and raw piece IDs
retain the frozen source behavior. Execution classification deliberately follows
the source card's primary action even in the unusual raw DEFENSE_SKILL secondary
case; normalized menu decision facts remain a separate calculation.

`ChooseActingPieceStep` filters Razzle pieces using each piece's action legality
and the current physical card. It binds a sole legal piece automatically, accepts
valid pending choices, clears invalid pending input before re-prompting, and
retains the source fallback when no piece qualifies. Native request/step capacity
is checked before changes to context, stack or acting piece. Queued child models
retain nested-step isolation and source object identity across worker copies.

The new oracle generator covers 596 printed card faces' menus and every offered
choice, 12 piece-selection configurations, 14 complete primary action flows and
10 edge scenarios: 632 scenes / 7,268 operation boundaries. Menu coverage checks
recipe construction without claiming all queued card bodies already execute.
The reviewed custom primary catalog remains 38/596, with 45 additional registered
base no-op builders. All 22 spells still require complete casting/body support.
Fast-travel sequence/unit consumers remain guarded; their recipes can be captured
and restored but cannot yet execute in the authoritative arena.

Re-performance/casting, remaining primary/passive/spell/filter bodies and
world/phase/round consumers, raw exception/partial-error fidelity, dense bridge
retirement, make/unmake, canonical hashing/evaluation, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallel search and integration/
strength gates remain open, including the required 1,000 complete two-game-turn
simulations/second. Engine/search completion stays false. Frozen Python and locks
are unchanged; no live services were changed and this work remains uncommitted.


Validation: 333 Rust workspace tests pass, including all 7,268 new
restore/replay boundaries and six native guard/live-operand regressions. The
older expensive all-corpus restore gate was filtered rather than rerun. Workspace
CLI parity passes the complete 9,609-scene live corpus and unchanged 6,592-scene
canonical corpus. Independent optimized release parity matches all 632 new
scenes with complete state, request, event and physical-model identity. Two
independent source contracts, strict Clippy, Rustfmt, Black/Ruff, whitespace and
frozen-source/lock checks pass. Live corpus totals 9,609 scenes / 52,159 operations;
semantic SHA-256 `6085f395f5f48471226f09c97ce084f7c1ef3b8e1d530d86eec9f8a4a7f63e76`, index
`558bf4fbf0eba69312451cdc6e05e0913fd945e8bc5d9aeb18ae32b9d4452ad4`. Frozen Python, locks and live services are unchanged;
work remains uncommitted. The full engine/search objective remains open.


### Authoritative fast-travel execution (2026-11-02)

Fast-travel sequence and relocation steps now execute through the object arena.
The sequence resolves the current board actor, validates Movement/Fast Travel
restrictions, then creates the source's optional destination selector and travel
child, or just the travel child for a truthy preselected destination. Child
operands retain the resolved actor, destination key, granting card and zone-change
flag. Generated-step capacity is checked before consuming the sequence.

Relocation revalidates the destination using safe zones, topology, occupancy,
terrain and actor-specific obstacles. It bypasses movement paths and placement
prevention, emits UNIT_PLACED for the actual board actor, and preserves the source
optional/mandatory rejection behavior. Existing immutable destination Hex models
are reused; dictionary coordinates create a fresh immutable Hex. Position and
tile containers, retained aliases and a shared last-turn snapshot remain coherent.
Direct travel and denied sequences need no generated-step capacity.

The shared travel-card lookup now follows source precedence: an explicit physical
context card wins; a present null/non-card value disables card exceptions; absent
context cards use a truthy granting card ID or the current card ID. An empty
granting ID falls back to the current one. Sequence validation and destination
filtering use the same lookup, avoiding different exception behavior at the two
boundaries. The cold action constructor catalog now includes travel selectors and
relocation children and remains reproducible from the frozen Python source.

Coverage adds 184 source scenes / 917 operation boundaries: all six maps with
ordinary/zone-change travel and first/last/skip/invalid input, direct travel,
minions/tokens/Razzle, revalidation, coordinate identity and snapshot aliases,
action restrictions and card-exception precedence, topology and barrier controls,
placement-prevention exemption and two complete secondary fast-travel actions.
Separate source/native tests check crossing an enemy barrier with its actual
barrier_radius rather than confusing it with scope.range.

Re-performance/casting, remaining primary/passive/spell/filter bodies and
world/phase/round consumers, raw exception/partial-error fidelity, dense bridge
retirement, make/unmake, canonical hashing/evaluation, iterative team-aware
minimax/alpha-beta, hidden/simultaneous planning, parallel search and integration/
strength gates remain open, including the required 1,000 complete two-game-turn
simulations/second. The primary catalog remains 38 custom recipes plus 45 base
no-op faces out of 596; all 22 spells still need complete casting/body support.
Engine/search completion stays false. Frozen Python, locks and live services are
unchanged; this work remains uncommitted.


Validation: 339 Rust workspace tests pass, including restore/replay at all
917 new travel boundaries and five native guard/identity/barrier regressions.
The older expensive all-corpus restore gate was filtered rather than rerun.
Workspace CLI parity passes the full 9,793-scene live corpus and unchanged
6,592-scene canonical corpus. Independent optimized release parity matches all
184 new scenes with full state/request/event/physical-model identity. Five source
contracts, strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock
checks pass. Live corpus totals 9,793 scenes / 53,076 operations; semantic SHA-256
`79d28b93673c88817b33f1668b521548927fba1373aa4b682c016a46efed0be1`, index
`3135237044eb6fdcceb20ad303c533bc3f8f9d1b746dce7671de8ce5ba6444c8`. Frozen Python, locks and live services remain unchanged;
work is uncommitted. Full engine/search completion and the throughput gate remain
open.


### Authoritative primary re-performance (2026-11-02)

`PerformPrimaryActionStep` now executes through the arena. It searches the
performer's current, played, discard, hand, deck and spell collections in source
order, keeps physical card identity, validates the selected card's own action and
color, and computes its stats once. Missing, facedown, effectless or unregistered
cards retain the source skip behavior. Reviewed primary-body construction is
shared with ordinary text dispatch without repeating the card-collection refresh.

Eligible Movement/Attack primaries offer the source Fast Travel/Clear choice,
including exact request metadata and pending-input reset behavior. Replacement
availability uses safe zones or the current actor's Clear candidates as Python
does. A registered card's replacement can execute before that card's own primary
body is ported; Deluge tests this path while its body remains explicitly guarded.
Liquid Leap is retained as a Skill control and does not offer a replacement.

Execution pushes the source's shallow action-context snapshot and schedules the
selected primary effect or replacement followed by full RestoreActionContext.
The snapshot retains outer structured values and aliases, including performer,
card ownership and substitution scope. Recursive exclusions reach unit selectors,
attack filters, repeat/foreach templates and effect finishing steps. Nested
runtime primary performances inherit the key without overwriting an explicit
one. Newly added filters are physical models with independent key-list identities;
returned nested steps follow source revalidation/copy behavior. Step/request
capacity and scope shape are checked before consuming the pending step.

The new 248 scenes / 1,171 boundaries cover all 38 reviewed bodies with ordinary,
item/negative-stat, excluded-target and facedown variants; all 45 referenced base
no-op builders including ultimates; 28 complete primary/exclusion flows; replacement
menus, invalid input and complete Clear/Fast Travel replacement flows; and missing
or relocated selected cards with current-card precedence. Source contracts check
outer alias restoration, lookup, menus and nested exclusions. The constructor
catalog now contains the full context-restore step and exclusion-filter defaults.

General PerformCardAction copying/casting, remaining primary/passive/spell/filter
bodies and world/phase/round consumers, raw exception/partial-error fidelity,
dense bridge retirement, make/unmake, canonical hashing/evaluation, iterative
team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel search and
integration/strength gates remain open, including the required 1,000 complete
two-game-turn simulations/second. The primary catalog remains 38 custom recipes
plus 45 base no-op faces out of 596; all 22 spells still need full casting/body
support. Engine/search completion stays false. Frozen Python, locks and live
services are unchanged; work remains uncommitted.


Validation: 345 Rust workspace tests pass, including all 1,171 new bounded
restore/replay boundaries and five native guard/live-operand/exclusion tests.
The older expensive all-corpus restore gate was filtered rather than rerun.
Workspace CLI parity passes the full 10,041-scene live corpus and unchanged
6,592-scene canonical corpus. Independent optimized release parity matches all
248 new scenes with complete state/request/event/physical-model identity. Seven
source contracts, strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/
lock checks pass. Live corpus totals 10,041 scenes / 54,247 operations; semantic
SHA-256 `64cfd3c911c827c597ee5023e32d5808b5bc9c9c67966651416792a6032ecb01`, index
`c7a7ea788d26537639b16477601b9acd4cf0cf91551b840e5db1660910e804c5`. Frozen Python, locks and live services remain unchanged;
work is uncommitted. Full engine/search completion and the throughput gate remain
open.


### Remaining active Arien primary bodies (2026-11-02)

Seven reviewed pure builders now have native arena recipes: Spell Break, Ebb and
Flow, Dangerous Current, Raging Stream, Violent Torrent, Slippery Ground and Deluge.
This brings the custom primary catalog to 45 of 596 ordinary card-effect IDs,
with 45 additional referenced base no-op faces (506 bodies still unported).
All fourteen active Arien primary effect IDs now have recipes; this does not
claim complete hero gameplay while world/phase consumers remain unfinished.
All 22 spells still require complete native casting/body support.

The new behind-target predicate works for both unit and hex selectors. It reads
the target's direct location, resolves the origin as Python does, establishes
geometric direction, excludes the target itself and checks topology-aware
target-to-candidate distance. Origin-to-target topology is deliberately separate
from that distance. Live length/origin/key operands, all six directions, target
identity edge cases and splits before/after the target have source parity.

Cold source probes now record per-field absent-radius defaults rather than
assuming every radius placeholder becomes null. Required scopes and `radius or 0`
filters receive zero; nullable filters retain None. This also fixes the same
latent zero-default issue in the reviewed Xargatha retrieval builders. The audit
checks that absent radius changes only the recorded scalar paths, retaining the
same recipe/model structure; native execution uses compiled typed paths.

The 200 new scenes / 2,150 operations include expanded ordinary and re-performance
construction, 84 complete text/action/re-performance flows, absent-versus-zero
radius cases and 36 isolated line-filter scenarios. Positive flows exercise
movement-zone and Skill restrictions, real swaps and repeats, before-attack hand
discard, minion defeats and repeated attacks. Separate source/native regressions
exercise defeating a cardless hero behind the attack target. Generation guards,
live-filter worker isolation and both nullable/zero-default radius behavior are
covered. The old unported-body guard now uses Angry Roar, which remains unported.

General PerformCardAction copying/casting, remaining primary/passive/spell/filter
bodies and world/phase/round consumers, raw exception/partial-error fidelity,
dense bridge retirement, make/unmake, canonical hashing/evaluation, iterative
team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel search and
integration/strength gates remain required, including 1,000 complete two-game-turn
simulations/second. Engine/search completion stays false. Frozen Python, locks and
live services are unchanged; work remains uncommitted.


The expanded catalog exposed a legacy dense-import guard that counted inert
primary templates as reachable dynamic effect creation, blocking unrelated Min
actions whenever blast mines were possible. Compiled primary recipes now carry
internal provenance. Explicit effect-creation roots retain their existing closure
check, while a genuinely reached primary effect/mine combination remains guarded
before its frame is popped. Two dedicated regressions cover both outcomes; this
does not claim that the remaining dense dynamic mine/effect closure is complete.


Validation: 352 Rust workspace tests pass, including all 2,150 new restore/
replay boundaries through expanded existing tests and the dedicated bounded Arien
test, plus six native guard/live-filter/radius/defeat regressions. The older
expensive all-corpus restore gate was filtered rather than rerun. Workspace CLI
parity passes the complete 10,241-scene live corpus and unchanged 6,592-scene
canonical corpus. Independent optimized release parity matches all 200 new scenes
with full state/request/event/physical-model identity. The four canonical Min regression cases also pass optimized parity. Seven source contracts,
strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
Live corpus totals 10,241 scenes / 56,397 operations; semantic SHA-256
`aaf42bff8297bd8e40b92f0007e882e0e8f0a4ecc218cdd122ecc4c1a9a1703f`, index
`7b518512eef4388cb8bd7b24f1a7030109478be5c7da880a66f4b6f72cca09fd`. Frozen Python, locks and live services remain unchanged;
work is uncommitted. Full engine/search completion and the throughput gate remain
open.


### Native copied-card action execution (2026-11-02)

`PerformCardActionStep` now builds and runs the supported actions of a physical
foreign card, including the previous game-turn slot used by Mind Grip. Lookup
retains current/played/discard/hand/deck/spell collection precedence and card
identity; neither hero's current card or collections are replaced. Missing
performers/cards and empty slots skip cleanly. Facedown cards expose their current
face, Defense is excluded, and DEFENSE_SKILL offers Skill. Invalid input reopens
the same menu without entering an action scope. Negative computed attacks remain
negative. Copied Fast Travel uses source availability rather than the extra
safe-zone menu gate on ordinary card resolution.

Ordinary and copied menus/lifecycles now share typed rules and recipe binding.
Primary bodies use the copied card's stats with the performer as actor, including
bare primitive fallbacks for unregistered effects. Lifecycle order, before-action
movement/discard, basic/primary/passive hooks, and optional after-resolve suppression
match Python. For multi-piece performers, copied primary builders bind their Hero
owner while secondary/lifecycle operands retain the explicit piece ID. Separate
source/native regressions protect this distinction.

The action scope snapshots/restores all eight outer keys and their aliases;
token substitution and marker suppression are scoped to the performer. Native
step/request capacity and scope shape guards precede stack/context mutation.
Unported primary/spell bodies remain explicit unsupported closures. This adds
the copying dispatch foundation, not Mind Grip's full card body or spell casting.
The body counts remain 45 custom primary recipes + 45 base no-op faces out of 596:
506 ordinary bodies and complete casting/bodies for all 22 spells remain.

Validation: 358 workspace tests pass with only the older expensive all-corpus
restore gate filtered. The new bounded test restores/replays all 1,526 boundaries
across 127 edge/complete-flow scenes; five native tests cover generation guards,
live owner keys, missing-card capacity and piece-versus-owner body bindings.
The complete 10,964-scene / 64,681-operation live corpus and unchanged 6,592-scene
canonical corpus pass CLI parity. Independent optimized release verification
matches all 723 new scenes / 8,284 operations with full state/request/event/model
identity: 596 ordinary-card menus, 35 edge cases and 92 complete flows. Twenty-seven
Python source/fixture integrity tests, strict Clippy, Rustfmt, Black/Ruff and
whitespace checks pass. Live index SHA-256 `2bfcf750870868ff692eea107345135623635cc8bfb168ee67f39f2bcbc40688`;
semantic SHA-256 `8d356a4fce4ce78b1df31efa57106cdaf592f5c2611ef18668f83df24e4bf23e`.

Fixture merging now checks the content hash and exact leading case name directly
on canonical immutable bytes. Alternate formatting/order retains the full parser;
corruption, false names and interrupted publication still fail safely. A matched
20-scene sample (37.3 MB) changed from 1.4388 s / 20.6 MB peak Python allocation
to 0.1919 s / 4.6 MB, with identical hashes. Appending to the full corpus took
69.08 s. These measure fixture preparation, not native search
throughput. Frozen Python, locks, the Windows backup and live services remain
unchanged; work is uncommitted. Remaining primary/passive/spell/filter bodies,
world/phase/round consumers, raw exception fidelity, dense bridge retirement,
make/unmake, canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/second are still required.


### Mind Grip body and previous-turn-slot filter (2026-11-03)

Mind Grip now has a reviewed native primary recipe. Its first choice selects an
enemy hero in range with a nonempty previous game-turn slot, then uses the copied
card executor with Illusion substitution and marker suppression. Its second
choice defeats an adjacent enemy minion. Text, ordinary action and nested primary
entry paths match Python; copied foreign primaries still require their own native
body support and fail explicitly when that closure is unported.

`HasPreviousSlotCardFilter` uses the game turn, not resolved-turn counts. It reads
live played-slot collections, accepts hero-piece IDs through their owning Hero,
and accepts a present card regardless of its facedown/current-card status. Hex
candidates are rejected, including through the live selector compiler. Source
scenarios cover turns one through five, holes, missing/current/discarded cards,
zero/absent range, negative items, invalid choices and nested scope restoration.
Native regressions cover live slot changes with private worker state and primary
generation capacity before context/stack mutation.

Adding the body exposed a dense cold-import regression: its unused copied-action
step was rejected during catalog compilation. The compiler now retains that live
instruction without claiming dense execution support. Inactive frames skip;
reached dense copied actions remain unsupported before their frame is popped.
A dedicated regression covers both outcomes, while unrelated dense imports pass.
This does not complete the dense bridge or replace authoritative arena execution.

Validation: 362 Rust workspace tests pass, with only the older expensive
all-corpus restore gate filtered. The new bounded test restores all 3,036 Mind
Grip boundaries across 115 scenes; existing construction/re-performance tests
expand by nine scenes / 18 operations. Independent optimized parity matches all
125 new/replaced scenes / 3,067 operations with full state/request/event/model
identity. One existing copied-menu case is replaced to include Mind Grip's now
supported primary dispatch. Four source contracts, strict Clippy, Rustfmt,
Black/Ruff, whitespace and frozen-source/lock checks pass. Full CLI parity covers
11,088 live scenes / 67,737 operations and the unchanged 6,592-scene canonical
corpus. Live index SHA-256 `48e3526f1588c5a6319db341c4b20b0bf6f78acf64e99b51fe780069b97fdbc5`;
semantic SHA-256 `ca3abdc51c9f215a5d8c534221076cc27ba01d4f39d32a42fb938289fa5fcec8`.

The reviewed primary catalog is now 46 custom recipes + 45 referenced base no-op
faces out of 596 ordinary card faces (505 remain to port/review). All 22 spells
still require complete casting/body support. Frozen Python, locks, the Windows
backup and live services remain unchanged; work is uncommitted. Remaining card,
passive, spell and filter bodies; world/phase/round consumers; raw exception
fidelity; dense bridge retirement; make/unmake; canonical hashing/evaluation;
iterative team-aware minimax/alpha-beta; hidden/simultaneous planning; parallel
search; integration/strength gates; and 1,000 complete two-game-turn simulations
per second remain required. Engine/search completion stays false.


### Remaining active Tigerclaw bodies, theft and shared radius fallbacks (2026-11-03)

Seven reviewed builders now have native recipes: Backstab, Backstab with a
Ballista, Combat Reflexes, Light-Fingered, Pick Pocket, Master Thief and Blend Into
Shadows. All twelve referenced active Tigerclaw builders now have recipes; this
is not a claim of complete hero/game execution while other engine closures remain.
Flows cover flanking attack bonuses and blocked primary defense, pre-versus-post
attack movement, actual coin theft and gated movement, teleport placement, and
next-turn immunity.

Coin theft now reads live victim, actor override, amount and output operands. It
caps theft at the victim balance, preserves no-op/stale-output behavior, transfers
between authoritative owner models and cached world rows, and emits the exact
source GOLD_GAINED/steal event. Self/piece transfers preserve net gold and raw event
IDs. Checked totals reject overflow before either owner's balance or the pending
frame changes. Shared gold writes remove duplication with existing resource steps.
Inert dense resource templates import; reached unbound resource execution remains
explicitly guarded before pop rather than being represented as successful play.

Terrain predicates reuse the native terrain service, including petrification and
virtual off-board terrain. String unit candidates deliberately use the source
filter's constant wanted-value behavior. Live filter mutations remain private to
workers. Blend's `radius or 2` exposed the need to distinguish nullable radius
from a default on both None and zero. Cold primary catalogs now use schema 2 with
shared typed fallback operands audited from positive/absent/zero source probes.
Both dense and arena binding and dense boundary output use the same map, retain
negative radii, and keep nullable fields nullable. The obsolete arena-only None
path patches were removed. Real dense Blend execution verifies two-space choices
and correct exported ranges when the printed radius is absent. Complete negative
attack flows explicitly retain -2 and resolve, matching the allowed rules.
The dense blast/effect guard regression now enables each effect template before
checking rejection, including conditional immunity templates.

Validation: 370 Rust workspace tests pass; only the older expensive all-corpus
restore gate is filtered. The new bounded test restores all 2,488 boundaries in
161 Tigerclaw/terrain/theft scenes; existing primary/re-performance tests expand
by 63 scenes / 126 operations. Independent optimized parity matches all 231 new/
replaced scenes / 2,707 operations, with complete state/request/event/model identity.
Seven copied-menu fixtures now include their supported primary body choices.
Seven native regressions, thirteen source/coverage/scalar contracts, strict Clippy,
Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass. Full CLI parity
covers 11,312 live scenes / 70,365 operations and the unchanged 6,592-scene canonical
corpus. Live index SHA-256 `4794130c93264d7c1f0b417152d7b1e5d48a53511efc43da57a2592cb0fbd676`;
semantic SHA-256 `84c0369bb0ca5c19d49e1ba7ec62bf80e4ed0b67825a0461efc64a5b796307bb`.

The reviewed body count is 53 custom recipes + 45 referenced base no-op faces:
98/596 ordinary faces, with 498 remaining to port/review. All 22 spells still need
complete casting/body support. Frozen Python, locks, the Windows backup and live
services remain unchanged; work is uncommitted. Remaining bodies/filters and
world/phase/round consumers, raw exception fidelity, dense bridge retirement,
make/unmake, canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/second remain required. Engine/search
completion stays false.


### Remaining Brogan bodies and live card/round queries (2026-11-03)

Eleven more reviewed recipes cover Bullrush, Furious Charge, Throwing Axe,
Throwing Spear, Shield, Bolster, Fortify, War Drummer, Master Skald, Bulwark and
One Man Army's empty builder. All eighteen referenced Brogan builders now have
recipes. The ultimate's empty builder does not prove its separate minion-battle
rule is complete, nor does primary coverage establish full hero/game support.

New native queries count authoritative HAND/DISCARD/DECK/PLAYED collections and
read the root heroes-defeated-this-round list. Card counts preserve actor/key/
literal precedence, repeated entries and played-card holes. Defeat checks return
True for a nonempty list and None for an empty list, using the live root rather
than similarly named combat/context marks. Central handler skipping preserves old
outputs. Worker edits stay private; unbound dense queries reject before pop.

Source flows check straight charges and mandatory failure, negative attacks,
melee/ranged branches, optional discards versus existing-discard spear attacks,
card retrieval, protection colors and radius fallbacks, and low/high coin rewards.
Minion protection is exercised through actual defeat consumers; Bulwark tests
both dormant card-text effects and activated effects, across enemy/friendly/self
actors and both self/friendly targets. Shield/Bolster use `radius or 1`, Fortify
`radius or 2`, and Bulwark `radius or 0`, through the shared operand schema.

Validation: 374 Rust tests and seventeen Python contracts pass. Only the
older exhaustive all-corpus restore gate is filtered; the new Brogan test restores
all 3,712 boundaries in 286 scenes. Primary and re-performance restoration expand
to 466/1,421 and 353/1,391 scenes/operations. The formerly serial primary restore
loop now uses the existing bounded fixture workers with unchanged coverage.
Independent optimized parity checks
395 scenes / 4,040 operations, with complete state, request, event and physical
identity equality. Ten copied menus now expose their supported primary choices.
Full CLI parity covers 11,697 live scenes / 74,295 operations, plus the unchanged
6,592-scene canonical corpus. Strict Clippy, Rustfmt, Python formatting/lint,
whitespace, frozen-source and lockfile checks pass. Live index SHA-256
`007065fb418450145e09347609bbf557736197b5ccff2ce342f099e03095607d`; semantic SHA-256 `06df807dcc588b72892c69700c3fac30f151201753cc112eecfaa0f692096cad`.

Reviewed primary coverage is 64 custom recipes + 45 base no-op faces = 109/596.
The remaining 487 comprise 484 unported registered bodies and three unregistered
source IDs (`over_the_top`, `chaos_incarnate`, `rock_and_a_hard_place`); unknown IDs
are not counted as reviewed no-ops. All 22 spells still require complete casting/
body support. The source-derived remaining-primary inventory is saved alongside
the validation artifact. Frozen Python, live services and the Windows backup are
unchanged; work remains uncommitted. Full remaining bodies/filters and world/
phase/round consumers, raw exception fidelity, dense bridge retirement, make/
unmake, canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations/second remain required. Engine/search
completion stays false.


### Native spellbook lifecycle and Wish continuation (2026-11-03)

The authoritative arena now executes CastSpellStep, WishSequenceStep,
PrepareSpellbookStep and RemovePreparedSpellsStep. Casting selects from the unique
owner's live prepared spells in source order, spends and reveals the same physical
card, records owner/caster/selected IDs, emits SPELL_CAST and queues the normal
copied-card action with after-resolve suppression. Wish queues its counted
continuation before its action and survives nested mandatory failures; completion
removes only its own marker and checks victory after the nested spell finishes or
fizzles. Piece casting keeps raw event IDs; Wish counters/events/nested casters
normalize to the owner Hero exactly as the frozen source does.

Optional removals preserve the mutable removed-ID list, live limits, skip behavior,
accepted-invalid-input re-prompt behavior and new request tokens after accepted
input. They reveal/spend spells without casting them. Preparation returns spent
spells in sorted printed-ID order, re-hides them, expires only their linked effects
without finishing payloads, updates card activity and preserves physical spell
collections and detached effect-list aliases. Requests preserve labels, decision
purpose, controller mapping and foreign caster ownership. The same source-derived
continuation defaults generate all nested frames. Request/step generation limits,
Wish overflow and multiple spellbook owners are guarded before pop or state edits;
multiple-owner source ValueError fidelity remains part of the broader raw exception
work. Inert dense templates import; reached unbound dense lifecycle instructions
reject before pop.

Validation: 382 Rust workspace tests pass with only the older all-corpus
exhaustive restoration gate filtered. All 177 boundaries in 47 lifecycle scenes
are covered by the new bounded restore test. Seven native guard/live-field/
isolation regressions and nine source contracts pass. Independent release-mode
parity matches complete states, requests, events and model identity in all 47
scenes / 177 operations. Full CLI replay verifies 11,744 live scenes / 74,472
operations plus the unchanged 6,592-scene canonical corpus. Strict Clippy,
Rustfmt, Python formatting/lint, whitespace, frozen-source and lock checks pass.
Live index SHA-256 `c5876ee3b9af6a69d6f06d556d77549d2906804c03238babb113aadda61aa470`;
semantic SHA-256 `64b2e9fd11bee8ce3e6b1cf8a7f268cc25b48e42b796e38e725371ced4e279db`.

This is lifecycle support, not completion of the spell bodies: the primary-body
guard remains explicit for unported spells. Reviewed ordinary primary coverage
stays 109/596 and complete spell support stays 0/22. The source-reviewed spell-body
inventory records all 22 IDs, the ordinary/spell Shield namespace collision,
state-dependent builders (Create Undead, Midas Touch, Dimension Door, Sunburst,
Wish), and remaining primitive/filter dependencies for the next port. Full remaining
card/passive/spell/filter bodies and world/phase/round consumers, raw exception
fidelity, dense bridge retirement, make/unmake, canonical hashing/evaluation,
iterative team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel
search, integration/strength gates and 1,000 complete two-game-turn simulations/
second remain required. Frozen Python, live services and Windows backup are
unchanged; work remains uncommitted. Engine/search completion stays false.


### Native spell bodies and reusable spell predicates (2026-11-03)

All 22 frozen-source spell bodies now execute through CastSpellStep and copied
PerformCardActionStep paths. A separate source-pinned spell recipe namespace
preserves the intentional spell Shield/Brogan Shield ID collision. Five dynamic
builders bind live caster/team, physical spellbook state, spell counters and
computed stats: Create Undead, Midas Touch, Dimension Door, Sunburst and Wish.
Prepared menus and outside-spell counts read authoritative physical-card state
overrides. Negative attack values remain legal and unclamped.

Reusable native dependencies include CardsInContainerFilter (visible cards and
played-card holes), nested CountMatchFilter (unit/hex candidates and optional
tokens), CheckUnitFiltersStep, spawn-team/battle-zone hex predicates and both
legacy unit-first and lane-bound hex-first minion respawning. Respawn preserves
per-kind zone capacity, lane supply, source ordering, stored hex aliases, stale
input handling and placement events. Source virtual-tile behavior is preserved:
an unfiltered ad-hoc respawn can accept an off-map hex; printed Create Undead
filters reject it. CountMatch's temporary origin overlay preserves existing
non-null context and deletes an explicit null only when actually evaluated,
before writing the instruction's output. Observation is isolated per cloned worker.

Independent optimized parity passes 304 scenes / 6,113 operations, comparing
complete states, requests, events and object identity. Coverage includes all 22
spells via cast/copy and first/last/skip/invalid choices, dynamic spell counts,
negative attacks, foreign casters, immunity consumers, Familiar pool reuse,
third-Wish victory/fizzle ordering, filter visibility and null cleanup, and
respawn capacity/invalid/stale/Tide behavior. Six native tests cover generation
limits, live operands, shared placement containers and private worker clones;
22 Python source contracts pass (13 spell-body and nine spellbook contracts).
The source scenarios are registered in the main corpus, now 12,048 scenes /
80,585 operations. All 391 Rust workspace tests pass, with only the older
all-corpus exhaustive restore gate filtered; the new bounded restoration test
checks all 6,113 spell boundaries and private root state. Full CLI replay also
passes the unchanged 6,592-scene / 27,145-operation canonical corpus. Strict
Clippy, Rustfmt, Python formatting/lint, whitespace, frozen-source and lockfile
checks pass. Results are saved in `rust/artifacts/spell-bodies-final-validation.json`.
Live index SHA-256 `56049402c87387c5e33e20ca42ebc8978abf8b95b40e45162ef631e708e77edd`;
semantic SHA-256 `c2e81ce1b0ccceff4c6ce8d8f8343db8da2f6560c9e494f23939c97d2586fb0e`.

Reviewed ordinary primary coverage stays 109/596 (487 remaining). Ordinary
Gydion spell-access cards and the Archwizard passive are separate remaining
work. General card/passive/filter bodies, world/phase/round closure, raw exception
fidelity, dense bridge retirement, make/unmake, canonical hashing/evaluation,
iterative team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel
search, integration/strength gates and 1,000 complete two-game-turn simulations
per second remain required. Frozen Python, live services and the Windows backup
are unchanged. Engine/search completion remains false; work is uncommitted.


### Gydion ordinary spell access and Archwizard preparation (2026-11-03)

Seventeen ordinary Gydion primary faces now use reviewed source-pinned recipes:
Cantrip, all 15 spell-school access cards and Prepare Spells. School access keeps
its exact allowed printed spell IDs and binds the actual performer as caster,
including foreign copied-card actions while Gydion retains the physical card and
spellbook. Preparation executes passive discovery before returning spent spells;
the existing Archwizard continuation is now verified through ordinary text,
resolve-card and primary-action entry points. Accepting the optional passive casts
Wish instead of preparing; declining, a spent Wish or the wrong actor prepares
normally. Wish may replace preparation even when no spell has been spent.
Third-Wish victory is checked after the nested cast resolves or fizzles.

Archwizard eligibility and preparation read authoritative physical spell states,
including live overrides between steps. Optional passive prompts now check request
generation exhaustion before popping the offer; an existing prompt keeps its token
when new generation is exhausted. Six native regressions cover these guards,
current physical Wish state, request reuse, cloned casting and preparation isolation.

The recipe catalog now distinguishes its 64-recipe reviewed dense closure from
17 authoritative spellbook recipes. The canonical dense trace generator uses
that explicit closure; ordinary object generators cover all 81 recipes. The
updated source matrix adds 153 generic construction/reperformance cases and
expands 17 existing copied menus without dropping their earlier submissions.
The dense spellbook bridge remains a required part of the unfinished engine.

Independent optimized parity passes all 508 Gydion scenes / 10,150 operations, with
complete state, request, event and object-identity equality. Scenarios exercise
all 17 cards through text/action/primary/foreign-copy entry points with first,
last, skip and invalid choices; spent/missing books; and Archwizard acceptance,
decline, eligibility, no-spent-spell replacement, nested victory and failure.
Dedicated bounded restoration checks all 9,631 full-flow boundaries. Expanded
generic restoration covers 153 new construction/reperformance cases and all 18
Gydion copied menus, with root isolation. The full 398-test Rust workspace run
passes, with only the older all-corpus exhaustive restore gate filtered. All
three affected restore tests and full CLI replay were rerun after the generic
matrix expansion. Nine Gydion behavior/partition contracts, 13 catalog/radius
contracts and nine existing ordinary-body source contracts pass. Full replay covers
12,539 live scenes / 90,556 operations and the unchanged 6,592 canonical scenes /
27,145 operations. Strict Clippy, Rustfmt, Python formatting/lint, whitespace,
frozen-source and lockfile checks pass. Results and the refreshed remaining-card
inventory are saved in `rust/artifacts/gydion-primary-final-validation.json` and
`rust/artifacts/remaining-primary-inventory.json`.

Reviewed ordinary primary coverage is now 81 custom recipes + 45 base no-op faces
= 126/596. The remaining 470 comprise 467 registered bodies and three unregistered
source IDs; spell bodies stay 22/22. General remaining card/passive/filter bodies,
world/phase/round closure, raw exception fidelity, dense bridge retirement,
make/unmake, canonical hashing/evaluation, iterative team-aware minimax/alpha-beta,
hidden/simultaneous planning, parallel search, integration/strength gates and
1,000 complete two-game-turn simulations per second remain required. Frozen
Python, live services and the Windows backup are unchanged. Work is uncommitted;
engine/search completion remains false.

Live index SHA-256 `5be7dd86817d87bed50e4db06d31403eb7b771fa6606d76643e551e267f989d7`; semantic SHA-256 `1f12e3cac1595a164f50ace447a03ef169e55a2a69268203b22dcaa4de73bfb3`.


### Misa primary bodies and geometric queries (2026-11-03)

All 18 Misa primary faces are implemented: 16 pure recipes and two live-state
builders for Swoop In and Watch How I Soar. The schema-3 builder catalog keeps
pure recipes separate from explicit state-dependent variants. Ultimate eligibility
reads the actual performer's live level and non-null ultimate, including foreign
copied actions; it does not infer eligibility from the physical card owner or a
particular ultimate ID. The dense recipe closure remains 64; these authoritative
builders do not establish dense bridge completion.

Shared native filters now implement relative distance, between-hex geometry and
hex adjacency to a context target. Relative distance preserves disconnected
infinity comparisons; between-hex tests use geometric segments independent of
topology; adjacency retains map-presence checks and directional effect isolation.
CheckDistance uses live literal/key precedence and raw entity locations, without
falling back to the actor when an endpoint is missing. Inactive dense templates
can import, but a reached live distance query is guarded before popping its step.

The source corpus adds 389 Misa flows, 122 geometry cases and 180 generic
construction/reperformance/copied-menu cases: 691 cases / 8,598 operations.
Cases include both ultimate orderings, foreign performers, surviving ranged
defenders, fixed movement limits, nullable/zero/negative radius and legal negative
attacks. Eighteen existing copied-menu cases are regenerated with the expanded
source readiness.
The full live corpus is 13,212 cases / 98,954 operations; the canonical dense
corpus remains 6,592 / 27,145.

Source export supports opt-in bounded parallel workers, preserves original case
order and limits queued work. Serial and parallel exports are compared for full
values, aliases and digest equality. The default exporter remains serial.

A native regression exposed a hex selector popping its step before detecting
request-counter exhaustion. The request-capacity check now runs in preflight,
while existing request tokens remain reusable at the counter limit.

Reviewed ordinary primary coverage is 99 custom bodies + 45 base no-op faces =
144/596. Remaining primary faces: 449 registered bodies and three unregistered
source IDs. Spell bodies remain 22/22. General passive/defense/filter and world
closure, exception fidelity, dense bridge retirement, make/unmake, hashing,
evaluation, iterative team-aware search, hidden/simultaneous planning, parallel
search, integration/strength and the 1,000 complete two-game-turn simulations/sec
gate remain required. Engine and search completion remain false.

Validation: 409 Rust workspace tests pass with one older exhaustive restoration
gate filtered. All nine Misa native tests pass again after strengthening the
request-token reuse regression. All 71 Python source contracts pass, including
serial/parallel source-export equality. The final rebuilt release matches all
691 source cases / 8,598 operations. Strict Clippy, Rustfmt, Black/Ruff on all
16 affected Python files and frozen-source/lock checks pass.
See [the validation record](../../../rust/artifacts/misa-primary-final-validation.json).
Live services and the Windows backup remain unchanged; work is uncommitted.


### Wasp primary bodies and conditional query steps (2026-11-03)

Fourteen Wasp primary faces now use source-pinned pure recipes. They include
Magnetic Dagger's displacement restrictions, Charged/Thunder Boomerang targeting,
Shock/Electrocute/Electroblast post-attack discards, Telekinesis/Mass Telekinesis,
Lift Up/Control Gravity/Center of Mass orbits, Kinetic Repulse/Blast collision
handling and Static Barrier. The existing independent repeat-root allocator is
retained: source validation creates independent initial and repeat-template
steps, confirmed by physical-identity source contracts.

Native CheckUnitType and CombineBooleanContext steps read live arena fields and
store explicit boolean False. CheckUnitType keeps literal/key precedence,
normalizes piece IDs to their owning heroes, coerces scalar IDs as Python does,
and treats any truthy non-hero identity as MINION, including unknown IDs. Boolean
combination follows Python truthiness for containers; every operation other than
uppercase OR uses AND. Inactive dense query templates can import, but reaching
either step without authoritative bindings is guarded before the stack is popped.
Unit selectors now bind straight-line and inverse straight-line predicates to the
existing shared topology-aware geometry implementation.

The dedicated source matrix contains 334 complete Wasp flows and 94 direct
queries (428 cases / 7,971 operations). Another 144 generic construction,
re-performance and copied-menu cases bring the independently compared increment
to 572 cases / 8,455 operations. Eighteen Wasp copied menus are regenerated with
the expanded source readiness. A bounded restoration test exports/rebuilds every
Wasp boundary, compares state/request/event/identity results and checks that worker
mutations leave the root private. Misa and Wasp share that restoration helper.

The live corpus grows to 13,766 cases / 107,205 operations. The canonical dense
corpus remains 6,592 / 27,145, with its separate 64-recipe closure. Reviewed
ordinary primary coverage is 113 custom bodies + 45 base no-op faces = 158/596;
438 remain (435 registered and three unregistered source IDs). Spell bodies
remain 22/22. General passive/defense/filter and world closure, exception fidelity,
dense bridge retirement, make/unmake, hashing/evaluation, iterative team-aware
search, hidden/simultaneous planning, parallel search, integration/strength and
1,000 complete two-game-turn simulations/sec remain required. Engine/search
completion remains false.

Validation: 416 Rust workspace tests pass, including all 101 restoration
integration tests, with one older all-corpus exhaustive restoration gate filtered.
All 84 Python source contracts pass. The final release matches all 572 Wasp
source cases / 8,455 operations. Strict Clippy, Rustfmt, Black/Ruff and frozen
source/lock checks pass. See [the validation record](../../../rust/artifacts/wasp-primary-final-validation.json).
Live services and the preserved Windows checkout remain unchanged.


### Min primary bodies, deferred card text and permanent items (2026-11-03)

Fourteen Min primary faces are implemented: three mine cards, Smoke Bomb, five
stances, Fast as Lightning, two grenades, Inner Strength and Perfect Self.
Thirteen use pure source-pinned recipes. Perfect Self has explicit conditional
prototypes; execution chooses from the actual performer's current retired Tier II
deck cards and binds their ordered IDs. Re-performance rechecks the deck after
conversion, so its menu disappears when no eligible retired card remains.

ConvertCardToItem reads the live card key and hero operand, chooses the first
matching physical deck card, increments its item stat and emits ITEM_GAINED.
It preserves the source's unrestricted repeated conversion behavior. Card state
assignment clears temporary initiative exactly as Python does, retaining other
flags and aliases. Shared item dictionaries update every affected native hero
cache. No card is removed from its existing collections.

ApplyAfterAttackCardText performs its red-card lookup at execution time: played
cards before discards, using current color so facedown cards are skipped. It
stops on the first red card even if that card has no registered effect. The native
ordinary-registry builder uses the selected red card's stats and source binding,
propagates the actual defender into custom target channels, strips through the
first attack and queues the remaining roots without replacing the outer action
context. Generation preflight counts only that returned suffix. Late discards,
foreign copied performers and an attack-only empty suffix have source coverage.

The source exporter now normalizes step labels throughout finishing continuations
as well as repeat templates, keeping grenade catalogs deterministic. Complete
flows expire and detonate grenades through the actual end-turn scheduler,
including empty-hand defeats and token/effect removal. Inert place_token templates
can import on the older dense path; reaching one is guarded before popping the
step. This fixes unrelated dense sessions that otherwise rejected the expanded
cold catalog. It does not establish dense token execution.

Source parity covers 507 cases / 8,357 operations. A dedicated restore gate
covers 363 Min cases / 7,873 boundaries, with the expanded generic construction,
re-performance and copied-menu gates retained. There are 14,255 live cases /
115,358 operations; the canonical dense corpus remains 6,592 / 27,145.

Reviewed primary coverage is 127 custom bodies + 45 base no-op faces = 172/596.
Remaining primary faces: 421 registered bodies and three unregistered source IDs.
Spell bodies remain 22/22. Dynamic Flurry/Cloak rebuilding, other passive/defense
and filter bodies, world closure, raw exception fidelity, dense bridge retirement,
make/unmake, hashing/evaluation, iterative team-aware search, hidden/simultaneous
planning, parallel search, integration/strength and 1,000 complete two-game-turn
simulations/sec remain required. Engine/search completion remains false.

Validation: all 425 Rust workspace tests pass, including 102 restoration
integration tests and complete live/canonical CLI replays; one older all-corpus
exhaustive restoration gate is filtered. All 103 Python source contracts pass.
Independent release parity matches all 507 Min cases / 8,357 operations. Strict
Clippy, Rustfmt, Black/Ruff and frozen-source/lock checks pass. See the
[validation record](../../../rust/artifacts/min-primary-final-validation.json).
Live services and the preserved Windows checkout remain unchanged.


### Dynamic passive rebuilding and raw primary attribution (2026-11-03)

Flurry of Blows now calls the native equivalent of raw `build_steps` for an
explicitly selected ordinary registry entry, using the actual current, played
or discarded physical card and current computed stats. Cold metadata is derived
by comparing both real source APIs for all 127 reviewed builders and refusing
any difference beyond proven card-attribution paths with identical model/alias
topology. This avoids duplicating entire instruction catalogs or guessing which
fields the source binder visits. Flurry preserves the source's root-only attack
exclusion, mandatory flag, defender-or-victim fallback and recursion wrapper.

Cloak and Daggers rebuilds attributed primary attack/skill text from the current
card, honors repeat prevention and adds exclusions only to the root types the
source changes. The shared spell-access dispatcher continues to read the
physical card's effect ID even under an explicit registry override. Both methods
retain the outer action context, facedown behavior and unclamped negative attacks.
Generation limits are checked before popping the accepted offer; restoration
and cloned-worker isolation remain required.

An empty-zone arena lane check is now a no-op when its live lane operand is null.
Actual lane pushes, nonempty-zone checks and round battle/cleanup remain guarded
before pop. Differential coverage for this increment is 176 cases / 652 operations,
including full repeated combat. Primary-face coverage remains 172/596; spells
remain 22/22. Other passive/defense/filter bodies, world closure, exception fidelity,
dense bridge retirement, make/unmake, hashing/evaluation, team-aware iterative
search, hidden/simultaneous planning, parallel search, strength/integration and
1,000 complete two-game-turn simulations/sec remain required. Engine/search
completion stays false. No live services or frozen Python sources were changed.

Validation: 277 core unit tests, the new bounded restoration gate and all three
full CLI replay tests pass. The live corpus is 14,431 cases / 116,010 operations;
the canonical corpus stays 6,592 / 27,145. All 60 affected source contracts pass.
The final release matches all 176 incremental cases / 652 operations. Other
existing integration suites were not rerun for this increment. Strict Clippy,
Rustfmt, Black/Ruff, reproducible catalog and frozen-source/lock checks pass.
See the [validation record](../../../rust/artifacts/dynamic-passive-final-validation.json).
Changes remain uncommitted. Next world-closure work must bridge actual lane
checks/pushes and battle/cleanup into the authoritative arena without weakening
alias, event or restoration fidelity. The full engine/search goal remains active.


### Authoritative arena battle and lane-push execution (2026-11-03)

The existing native battle rules now expose ordered typed mutations, including
wave flips, zone changes, raw minion removal/spawn, token unlinking and pending
consumption. The arena adapter reads live step operands, wave/zone dictionaries
and tie priority, previews rule resolution on a private native world, validates
continuation capacity, then applies writes in source order. It does not rebuild
mutable containers from a final state: even a same-hex respawn must replace its
location Hex with the original spawn-point Hex. Source spawn indices include
skipped non-minion points. Token removal replaces only the active-effect list,
retaining the previous list and models through external aliases.

Five step families execute through this path: CheckLanePush, LanePush,
MinionBattle, ChooseMinionRemoval and ResolveDisplacement. Generated child steps
use frozen source templates; raw wipe/spawn helpers emit no placement/removal
events. Multi-lane simultaneous flips, victory precedence and tie remedies reuse
the source-verified native battle rules. Forced casualties retain pending input;
unforced choices consume it. Displacement preserves valid submitted Hex objects
and occurrence-specific equal origins, respects live coin priority and uses the
default lane for heroes/pieces without a lane field. Numeric casualty selections
follow the source truthiness and scalar-string conversion.

Coverage: 267 source cases / 1,446 operations, including every supplied map and
retained aliases at each boundary. Four native regressions cover pre-pop generation
failure, generation-free no-ops, live wave reads and live forced casualty counts.
The bounded restore gate checks every new operation against a freshly restored
session while preserving the private root. The main corpus is 14,698 cases /
117,456 operations; the canonical dense corpus is unchanged at 6,592 / 27,145.

Next phase boundary: EndPhaseCleanup must preserve source ordering for the second
THIS_ROUND expiry sweep, marker returns, stale-effect cleanup, card retrieval and
card flags, transient token removal, mandatory level purchases and ultimate unlock
hooks, pity gold, pending upgrade/item reveals and planning position snapshots.
Then close upgrades/reset and remaining phase consumers. General card/passive/
defense/filter coverage, raw exception fidelity, dense bridge retirement,
make/unmake, hashing/evaluation, team-aware iterative search, hidden/simultaneous
planning, parallel search, strength/integration and 1,000 complete two-game-turn
simulations/sec remain required. Primary-face coverage stays 172/596; spells stay
22/22. Frozen Python and live services are unchanged. Engine/search completion
remains false; this work remains uncommitted.

Validation: all 281 core unit tests, the new 267-case / 1,446-boundary restoration
gate and all three complete CLI replays pass. All 18 battle/dynamic-passive source
contracts pass. Independent final-release parity matches all 267 incremental
cases, including every supplied map. Other existing integration suites were not
rerun for this increment. Strict Clippy, Rustfmt, Black/Ruff and unchanged frozen
source/lock checks pass. See the
[validation record](../../../rust/artifacts/arena-battle-final-validation.json).
Live index SHA-256 `67e5c56ba993774183710efb259d874bb029b48676909b923cc11b3267d01162`;
semantic SHA-256 `8f1fc9f63a99f760661b5a294a147d7a77ef17360a1be297011d051724c585cb`.


### Authoritative end-of-round cleanup transaction (2026-11-03)

EndPhaseCleanup now executes against a private identity-preserving native session/
arena transaction. The source step is staged as popped, then cleanup applies its
real ordering: discard the second THIS_ROUND sweep's finishing payloads, return
markers in place, replace stale effects, expire links for played/current cards,
retrieve physical cards, clear transient pooled tokens, purchase mandatory levels
and run the two frozen unlock hooks. A successful candidate commits its arena and
world together; generation failure cannot leak card, effect, gold or stack writes.
No source model graph is reconstructed from a final-state diff.

Retrieval retains list objects, handles physical model equality and state-assignment
initiative clearing, and leaves untouched hand-card flags alone. Active/passive
discard-linked effects follow the source behavior. Token supply order and the live
physical persistence flag control removals. Ready for War emits its silver swap
before gold retrieval events; Unbound Fury sets physical override flags. Pending
and reveal maps update in place, while individual item reveals and the planning
position snapshot are shallow copies. A new planning round can commit and uncommit
after retrieving the previous current card.

Level-up snapshots now import through the authoritative arena and round-rule
metadata is available for their hooks. ResolveUpgrades/ RoundReset consumers remain
explicit pre-pop guards until their arena implementations are ported. The cleanup
transaction currently clones mutable session/arena storage while sharing immutable
programs; optimize it as part of make/unmake and throughput work rather than
treating this as a performance milestone.

Coverage is 95 source cases / 192 operations, including the full duration/card-link
matrix, live token flags, negative gold, multi-level purchases, sparse items and
both unlock hooks. Four native regressions cover transaction atomicity, ignored
finishing payloads at generation exhaustion, live persistence overrides and the
restorable upgrade guard. The main corpus is 14,793 / 117,648; canonical coverage
stays 6,592 / 27,145. Nine cleanup source contracts specify identity and ordering.
Primary coverage stays 172/596 and spell bodies 22/22. Next: implement upgrade
choices/validation and round reset with retained card/list/item aliases, then
remaining phase closure. All other card/passive/defense/filter, exception-fidelity,
dense bridge retirement, make/unmake, hashing/evaluation, iterative team-aware
search, hidden/simultaneous planning, parallel search, strength/integration and
1,000 complete two-game-turn simulations/sec requirements remain active.
Frozen Python and live services are unchanged; engine/search completion remains
false.

Final verification: 285 core tests, the cleanup restoration gate (95 cases /
192 operations), three full CLI replay tests, 27 affected source contracts and
independent optimized parity pass. Strict Clippy, formatting, whitespace and
frozen-source/lock checks pass. Other existing integration suites were not rerun.
The interrupted initial CLI invocation was rerun in full and passed. See
`rust/artifacts/arena-cleanup-final-validation.json` for scope and corpus hashes.


### Authoritative arena upgrade choices and round reset (2026-11-03)

ResolveUpgrades now reads live physical collections and pending counts. Single
choices and ordered all-remaining batches use the same native tier eligibility
rules as dense execution. Invalid submitted batches reject before token/input or
world edits; invalid imported pending choices clear and reissue like the source.
A falsey empty pending-input dictionary remains attached. Chosen cards append to
the existing hand, previous cards retire, paired cards become items, and all
lifecycle assignments clear initiative overrides while retaining other flags.
Sparse item increments preserve dictionary identity and refresh every hero cache
when an item map is shared. Pending counts decrement/delete in place. Completion
clears the existing reveal map and schedules the real RoundReset continuation.

RoundReset increments round, resets turn, replaces the position snapshot with a
shallow dictionary copy, enters planning, and clears the defeated list in place.
Snapshot Hex objects stay shared. Both consumers stage against private mutable
session/arena clones and share immutable programs. Generation failure cannot leak
partial card, item, pending/reveal or stack writes. This is not make/unmake and
establishes no search-throughput result. Source partial-error fidelity remains
separate unfinished work.

The new differential corpus contains 107 cases / 653 operations across Arien,
Min, Gydion, Takahide, Ursafar, Misa and Wasp; both upgrade sides; one/three/six
single or batch choices; invalid input; physical flags; shared item maps; imported
pending inputs; reset aliases; and complete multihero cleanup-to-planning flow.
Nine explicit Python contracts cover ordering/identity and tier validation. Six
native regressions cover generation guards, issued-token reuse, invalid-submit
atomicity, independent worker clones, shared caches and reset without generation.
The dedicated restoration gate checks every new operation boundary; the cleanup
restoration gate also exercises the updated upgrade continuation behavior.

The live corpus is 14,900 cases / 118,301 operations; canonical dense coverage
remains 6,592 / 27,145. Primary-face/spell coverage remains 172/596 and 22/22.
Remaining card/passive/defense/filter bodies, general phase/game closure, raw
exception fidelity, dense bridge retirement, make/unmake, hashing/evaluation,
team-aware iterative minimax/alpha-beta, hidden/simultaneous planning, parallel
search, strength/integration and 1,000 complete two-game-turn simulations/sec
remain required. Frozen Python and live services are unchanged. Engine/search
completion remains false.

The phase audit identified guarded `ResolveCard`, `FlowStep::Respawn` and
`ParkedPhase` consumers. Hero respawn is implemented in the following increment;
the already-existing live card adapter and remaining mappings are audited below
before round/game closure is claimed.

Validation: all 291 core tests, the new upgrade/reset restoration gate (107 cases /
653 operations), the affected cleanup restoration gate (95 / 192), and all three
full CLI replay tests pass. Independent release parity passes all 107 cases.
All 36 affected source contracts and 28 existing Python upgrade-rule tests pass.
Strict Clippy, Rustfmt, Black/Ruff, whitespace and frozen-source/lock checks pass.
Other existing integration suites and the older exhaustive all-corpus restoration
gate were not rerun for this increment.
Results: `rust/artifacts/arena-upgrades-final-validation.json`.
Work remains uncommitted; engine/search completion remains false.


### Authoritative hero respawn (2026-11-03)

RespawnHero now reaches the live arena consumer rather than importing as a parked
phase step. It reads the current hero_id operand, returns immediately for unknown
or present heroes, and handles PASS before spawn discovery. Pending input and
request/step aliases remain intact on completion. Nonempty dict selections are
validated/coerced through the existing Hex parser; malformed/illegal dicts reopen
the hex prompt. An empty dict, immutable Hex model or unrelated scalar reopens the
initial respawn prompt, matching source isinstance/truthiness behavior.

Marked hero spawn points use actor-aware obstacle checks, with the source token
exception. The shared dense candidate helper now also uses those checks. If all
marked points are blocked, fallback BFS intentionally uses generic obstacles,
can pass through blocked spaces and returns nearest candidates in the first
usable spawn zone; it may offer a space denied by an actor-specific effect.
Static barriers do not affect an offboard actor with no position.

A selected dict creates a fresh immutable Hex instead of reusing a spawn or
selection object. Placement keeps location/tile maps and shared snapshots intact.
A spawn token is removed first, replacing its linked-effect list as the source
does and emitting exactly one respawn TokenRemoved event; direct placement emits
no extra unit-placement event. A token on terrain is not cleared. Multipiece
heroes place their first supplied piece in stable numeric supply order, even
when misc-entity registration order is reversed. The owner hero stays unplaced.

Request-capacity failure is checked before pop or world writes. A private mutable
session/arena staging clone keeps token/effect/placement changes isolated, shares
immutable programs, and does not establish make/unmake or performance. Raw
partial-error fidelity remains unfinished.

Coverage: 123 source cases / 417 operations, including both teams across all six
supplied maps; first/last/PASS choices; no-spawn/no-space cases; terrain/token/
effect interactions; fallback and duplicate spawns; live hero operands; partial
multipiece presence and supply order; numeric/extra-key coordinate coercion;
malformed/empty/model pending selections; and retained pending, step, tile, Hex,
effect-list, location and snapshot aliases. Ten source contracts and six native
regressions specify ordering, guard atomicity, request reuse and root/worker
isolation. The dedicated restore gate covers every operation boundary.

The live corpus is 15,023 cases / 118,718 operations; canonical dense coverage
stays 6,592 / 27,145. Primary/spell coverage remains 172/596 and 22/22. Next audit:
ResolveCard and remaining ParkedPhase mappings, complete ordinary rounds and game
termination. Remaining card/passive/defense/filter bodies, raw exception fidelity,
dense bridge retirement, make/unmake, hashing/evaluation, team-aware iterative
minimax/alpha-beta, hidden/simultaneous planning, parallel search, strength and
integration, and 1,000 complete two-game-turn simulations/sec remain required.
Engine/search completion remains false. Frozen Python and live services are
unchanged.

ResolveCard verification requirements, derived from the frozen source:

- Read the live hero/current physical card and board presence; write current_card_id
  before option generation, including on invalid input.
- Preserve primary/secondary order, DEFENSE exclusion, DEFENSE_SKILL-to-SKILL
  primary dispatch, action-validator card context and FAST_TRAVEL safe-zone gating.
  Display computed values while passing the source base values to standard child
  primitives; preserve legal negative values.
- Reuse source-pinned ActionBindings recipes where their constructor shape matches.
  Queue acting-piece choice, BEFORE_ACTION and the specific action passive before
  primary pre-movement/pre-discard/text or secondary primitives. Retain after-attack
  attribution and all after-action checks in source order.
- Prove dynamic physical-card overrides, invalid/repeated inputs, context aliases,
  generation guards and complete action-to-confirm/finalize/next-actor flows against
  Python before claiming ordinary round closure.

Validation: all 297 core tests, the 123-case / 417-operation hero-respawn
restoration gate, and all three full CLI replay tests pass. Independent release
parity matches every new case. All 61 affected Python source-contract and existing
hero/piece/round-rule tests pass. Strict Clippy, Rustfmt, Black/Ruff, whitespace,
frozen-source and lock checks pass. Other existing integration suites and the
older exhaustive all-corpus restoration gate were not rerun for this increment.
Results: `rust/artifacts/hero-respawn-final-validation.json`.
Work remains uncommitted. Engine/search completion remains false.


### Phase-closure audit and bounded complete-game verification (2026-11-03)

The runtime audit corrected an outdated TODO: ResolveCard already imports as
LiveCardAction and uses ActionBindings. The remaining dense ResolveCard rejection
is a pre-pop invariant guard, not evidence of a missing live consumer. A native
regression compiles every supported source-pinned instruction prototype and rejects
any parked/dense-card residue. This does not claim support for every unreviewed
source GameStep or card body.

Existing real-handler turn/round scenarios now expose a read-only capture hook.
The new live family expands their full advances to one-step boundaries and retains
stack, current-card, card-container, item, pending/reveal, effect, position and
snapshot aliases. Coverage is 191 scenarios / 1,819 operations through planning,
revelation/actor handoff, action menus and dispatch, respawn, confirm/finalize,
battles, cleanup, upgrades and round reset, including all supplied-map turn flows.
The existing action adapter and primitives already match these complete flows.

Whole-game verification originally exposed a test-harness memory problem: full
snapshot collection grew past 5 GB in one source worker. Source iter_reference and
the CLI stack_trace_stream command now expose one operation record at a time.
The native engine stays in one session for the entire game, including round and
planning boundaries. Transport packets carry request identity and monotonically
ordered sequence numbers, followed by one completion/count packet or terminal
error. A fallible sink stops replay on output failure. Collected replay behavior
is retained as a compatibility wrapper; no server/frontend contract changed.

Golden data uses lossless set/delete/splice snapshot deltas in immutable chunks
of at most 64 records. The Rust fixture verifier lazily loads each chunk, rebuilds
the complete expected snapshot, and compares state, request, events and identity
at every operation. Chunk boundaries do not reset the native game. Offline v2->v3
array compaction verifies the SHA-256 of every reconstructed complete record stream;
no field, intermediate operation or failing choice is discarded. Source replay and
fresh native JSON can explicitly normalize in place, avoiding redundant frame
copies while retaining the default non-mutating API.

The deterministic test policy prefers available attacks and movement toward combat,
with varied remaining legal choices and upgrades. Both complete games start from
ordinary GameSetup rosters (Arien/Brogan vs Tigerclaw/Sabina) on Forgotten Island
and Magma and Ice. Generation fixes PYTHONHASHSEED=0 for stable source option order.
This policy creates rule coverage; it is not the Rust search implementation or a
strength benchmark. Complete-game fixtures remain a separate compact corpus;
the main live corpus is 15,214 cases / 120,537 operations and canonical dense
coverage remains 6,592 / 27,145.

Primary/spell coverage remains 172/596 and 22/22. The next implementation expansion
should port genuinely missing bodies (Hanu is an important default-roster gap),
including any required co-movement, initiative, journey-return/control primitives
and filters. General card/passive/defense/filter closure, raw exception fidelity,
dense bridge retirement, make/unmake, hashing/evaluation, team-aware iterative
minimax/alpha-beta, hidden/simultaneous planning, parallel search, integration/
strength and 1,000 complete two-game-turn simulations/sec remain required.
Engine/search completion remains false. Frozen Python and live services are
unchanged; work remains uncommitted.

Final validation: 298 core tests, the new 191-case / 1,819-operation restoration
gate, seven regular CLI tests plus the separate compact full-game replay gate,
and 98 affected Python tests pass. The nine streaming contracts pass again after
final decoder/resource cleanup. Independent release parity passes all 191 phase
scenarios and both complete games. The games match every state, request, event
and identity record through game over: 5,410 operations to round 14 on Forgotten
Island and 1,649 operations to round 5 on Magma and Ice. Full regular replay also
passes the 15,214-case / 120,537-operation live corpus and unchanged 6,592-case /
27,145-operation canonical corpus. Strict Clippy, Rustfmt, Black/Ruff, whitespace
and frozen-source/lock checks pass. Other existing integration suites and the
older exhaustive all-corpus restoration gate were not rerun.

The completed source verifier peaked at 214,708 KiB (about 210 MiB). Lossless
array compaction reduced the same 7,059-record golden delta payload from
705,282,827 to 21,448,692 bytes, preserving both complete-record stream hashes.
An explicitly offline cleanup reclaimed 707,497,011 bytes from only this turn's
exact superseded files after replacement and release verification. Current
fixtures, historical baseline cases, active caches, frozen Python and live services
remain intact. These are verification/storage improvements, not search throughput
measurements. The one-time v2 converter is retained only as an ignored audit
artifact; maintained tools generate/read the current v3 compact format.

Work remains uncommitted. Primary/spell coverage remains 172/596 and 22/22;
engine/search completion remains false.
Results: `rust/artifacts/phase-closure-and-stream-final-validation.json`.


#### Hanu prerequisite: current-card status predicates

Implemented native unresolved/resolved card filtering in typed and live-object
unit paths, with rejection in hex paths. Source contracts cover all current-card
lifecycles and face orientations, absent cards, and resolved turn-slot indexing.
The maintained exporter includes 37 scenarios / 222 operations and a dedicated
all-boundary restoration gate. Primary coverage remains 172/596; Hanu's remaining
co-movement, initiative override and delayed journey/control primitives are still
required before admitting his complete family. No search-throughput claim follows
from this prerequisite.


#### Hanu prerequisite: coordinated movement

Implemented the mirrored destination filter, co-directional drag sequencer and
its token-walking dependency in authoritative live adapters. Source parity covers
280 scenarios / 1,020 operations, with independent release parity and restoration
at every boundary. Four native regressions check generation guards, private live
filter/destination operands and the explicit unsupported non-straight raw call.
Generic handler gates apply to token walking before its step body is invoked.

Following the full-disk interruption, the 22,135 indexed arena/canonical/live
fixtures passed case-content and semantic-index integrity checks using compressed
storage. The recovered development footprint was 6.07 GiB before the release
rebuild. Continue with initiative override and journey/action-control scheduling,
then admit Hanu's reviewed primary recipes. Primary/spell counts remain 172/596
and 22/22; the full engine/search/performance requirements remain unchanged.


#### Hanu prerequisite: initiative override

Ported `SetCardInitiativeStep` for the admitted current rules. The arena physical
card and native card cache receive the same override; printed definitions and
active-effect containers remain unchanged. State transitions already clear the
override, including repeated non-UNRESOLVED assignments. Source differential
coverage is 52 scenarios / 118 operations, covering aliases, owner-resolved piece
IDs, zero/negative/large integer values, discard transitions and initiative item
bonuses during actor selection. The all-boundary restoration gate and 305 core
unit tests pass, along with 38 affected Python contracts and strict Clippy.

Next: action-control and journey-return scheduling, delegated co-movement input
flows, then reviewed Hanu primary recipes. Full raw exception fidelity, remaining
roster/defense/passive/filter closure and the original search/performance goals
remain required; this does not change primary/spell coverage (172/596, 22/22).


#### Hanu prerequisite: action-control scheduling

Implemented `ScheduleActionControlStep` using a shared manager-creation path.
Existing CreateEffectStep scopes remain live references; internal manager calls
receive fresh literal scopes and avoid context-card/action inference. Collection
validation, entity counters, registration and card flags use the same machinery.
Unbound manager finishing programs are explicitly rejected pending the Journey
port, rather than silently discarded.

Source parity passes 61 new scenarios / 302 operations and 64 affected existing
creation scenarios / 258 operations. The new source flows cover controller
ownership during complete HOLD/confirm turns, forced windows, explicit other
players, stale card guards, repeat scheduling, colliding IDs and round cleanup.
Three native regressions cover entity-counter failure, live level/ultimate gates,
step-counter independence and private scopes/retained lists.

Next: bind manager-created Journey finishing programs with their actual model
identities, then port journey return scheduling and delegated co-movement card
flows before admitting Hanu's primary recipes. Primary/spell counts and the full
engine/search/performance requirements remain unchanged.


#### Hanu prerequisite: Journey return scheduling

Implemented ScheduleJourneyReturnStep through shared literal manager creation.
Manager finishing programs now retain actual bound step/model identities;
CreateEffectStep continues to copy its templates. The delayed payload freezes
literal performer/target IDs, so clearing turn context cannot redirect the return.
The optional move retains source MovementPathFilter and ObstacleFilter models.
Deduplication happens before generated-program allocation; aggregate entity and
step limits are checked before either half mutates state.

Python parity passes 77 scenarios / 321 operations. All-boundary restoration
passes for these cases and the affected 125 generic/control cases (560 operations).
312 core unit tests and 60 affected Python contracts pass. Source, requests,
events and identity are compared; this is not a speed benchmark.

Next: delegated co-movement input flows and reviewed Hanu primary recipes.
Primary/spell counts remain 172/596 and 22/22. All original engine, raw-exception,
compact-state, make/unmake, hashing, evaluation, search and throughput requirements
remain open. See rust/artifacts/journey-final-validation.json for scoped evidence.


#### Hanu primary recipes and cross-root source aliases

Admitted all 18 reviewed Hanu primary builders. Delegated distance selection
already used the correct context-selected decision owner; full card-flow tests
now prove the return to Hanu for direction selection and equal displacement of
both movers. Journeys exercise natural turn cleanup, and Hurry Up exercises
initiative override plus level-eight action control.

Trusted Sidekick and Pile On exposed shared filter models across separate root
steps. Recipes now record backward model/container imports. Grouped native
allocation restores these identities for ordinary, copied, reperformed, deferred
and dynamic passive bodies, while per-worker arena state remains private. Shared
executable roots still require their own generation-ID policy and are explicitly
rejected by the source freezer.

Source/native parity passes 471 primary scenarios / 7,902 operations and 16
focused Flurry/Cloak rebuild scenarios / 32 operations. An additional 18 zero/absent/negative-range scenes add 405 operations. The maintained
exporter contains all 505 scenes / 8,339 operations. Primary coverage increases to 190/596 (145 custom bodies,
including three stateful Misa/Min builders, plus 45 base no-op faces); spell
coverage remains 22/22. Full roster/defense/passive/filter closure, raw exception
fidelity, dense retirement and all original optimized-state/search/performance
requirements remain open. No throughput claim follows from this expansion.

Final verification is recorded in rust/artifacts/hanu-primary-final-validation.json.


Hanu edge-case closure also exposed source `range or 1` semantics that the
previous placeholder substitution lost at zero. Both catalogs now audit and
freeze range-default operands independently of radius-default operands. Native
binding uses the fallback only at zero, preserving negative values. Range-driven
physical alias changes are explicitly rejected during source freezing. The dense
importer accepts inert Hanu instructions and guards reached ones before popping,
so expanding arena coverage does not break unrelated dense imports.


Next verification/implementation priorities after this family:

- [x] Replay a complete natural Hanu/Arien versus Brogan/Tigerclaw game through the
  frozen source and native arena; primary-family traces do not prove that scope.
- [x] Finish reviewed primary-body catalog coverage for all 596 hero card faces
  and native casting and copied-card coverage for all 22 spells.
- [ ] Verify current defense, passive, filter and world interactions against the
  frozen Python engine across the full roster. Catalog coverage alone does not
  prove those interactions or complete-game parity.
- [x] Establish a native two-turn simulation benchmark with imports, protocol
  JSON and snapshot export outside the timed region. Report Rust cloning and
  advancement separately and use Python only to establish the expected result.
- [ ] Keep the original compact-state/make-unmake, hashing/evaluation, team-aware
  iterative search, hidden/simultaneous planning, parallelism, integration and
  1,000 complete two-game-turn simulations/second requirements intact.


#### Hanu complete-game verification

Two seeded source-policy games now reach GAME_OVER with state/request/event and
physical-identity parity at every operation: Forgotten Island seed 41 (5,475
operations, round 13 turn 4) and Magma and Ice seed 43 (2,090 operations, round 6
turn 1). The separate Hanu policy sometimes chooses offered primary skills; the
baseline policy and its two existing games remain intact. The recorder now emits
encountered-step/event statistics and bounded progress, with roster-specific
output defaults. Source verification peaked at 148,672 KiB RSS; this measures the
verifier, not engine-search memory or throughput.

The new compact corpus index is complete-games-hanu.json, SHA-256
`6f9743e3a6c1b14f8cadeb527e8ca130f9770feea51c91ad6d05afddc4c9c56a`.
Full payloads remain ignored compressed cache entries. The native test runner
shares one complete-game replay helper between the two indexes; validation scope
and final results are in rust/artifacts/hanu-complete-games-final-validation.json.

Primary/spell counts remain 190/596 and 22/22. No Python engine/pin, lab or bot
service changes are involved. All remaining engine and search requirements,
including a fair benchmark and the original throughput gate, remain open.


#### Matched two-turn simulation baseline

Added a prepared-command native benchmark endpoint and a matched Python driver.
Six windows come from the verified Hanu games: opening, round boundary and combat
on both maps. Consecutive one-step diagnostic advances are replaced by normal
advance-until-decision calls. Their copied/coalesced source endpoints are checked
against the original atomic game trace before timing. Native timed execution
checks full endpoint values/bindings, event/request counts and unchanged roots.
Import, decoding, serialization and comparison stay outside the timers. Copy,
advancement, cleanup and end-to-end costs are reported separately.

Python deepcopy can reorder Zone.hexes, a declared set. Only that serialized set
is canonicalized; executable simulations are unchanged, ordered lists remain
ordered, and identity paths into that set are guarded explicitly. Automatic GC
stays enabled, with an untimed full collection before each sample.

On one i5-10400F CPU under WSL, five interleaved samples of 32 simulations per
language per window measure 1.95–3.77x Rust speedup: 41.1–84.0 Rust versus
16.7–34.2 Python complete two-turn simulations/second. Root copies take roughly
0.15–0.18 ms in Rust versus 5.77–7.08 ms in Python. Advancement consumes 97–98.5%
of Rust time. These serial replay results do not establish search throughput,
parallel scaling, full-roster performance or the original 1,000 simulations/s gate.

Next: profile advancement and replace its expensive work from evidence. Full
compact-state/make-unmake, canonical hashing/evaluation, team-aware iterative
search, hidden/simultaneous planning, parallel search, roster/rules closure and
integration/strength requirements remain intact. Primary/spell counts remain
190/596 and 22/22. See rust/artifacts/simulation-benchmark-final-validation.json.


#### Dense object-protection membership

A user-space CPU profile of 1,500 two-turn round-boundary simulations found
14.8% of sampled self time in the arena reachability walk and 9.5% in
ObjectId hash-set growth, alongside substantial generic hashing cost. The
retained profile resolves ELF file offsets using the executable load-segment
delta; generic hash labels may be merged by LTO and are not independently
attributed to symbol-table work.

Object protection now uses the dense arena IDs as bitmap indices and keeps a
compact list of visited handles for copied-step membership transfer. The exact
root set, step-payload exclusions, graph edges, refresh points and mutation
guards are unchanged. No reachability cache or lazy invalidation is introduced.
Focused tests cover word boundaries, duplicate IDs, private copies, shared
references, cycles, payload exclusions and root replacement.

The same six source-verified windows, single CPU, five interleaved samples and
32 iterations now measure 83.5–149.5 Rust versus 16.7–34.0 Python simulations/s:
3.97–7.99× Python, and 1.76–2.22× the prior Rust baseline. All endpoint, alias,
request/event-count and root-privacy checks pass. Original and optimized raw
measurements remain separate; see rust/artifacts/simulation-benchmark-dense-protection.json.

This is still fixed-decision serial replay. Remaining roster/rule/exception
closure, compact-state/make-unmake, canonical hashing/evaluation, team-aware
iterative search, hidden/simultaneous planning, parallel search, integration and
strength gates remain required, as does 1,000 complete two-global-turn simulations/s.

Regression verification passes all 319 core tests, three object-identity tests,
the 505-case / 8,339-operation Hanu restoration gate and all 11 CLI tests. CLI
coverage includes all 16,226 live cases / 130,859 operations, 6,592 canonical
cases / 27,145 operations and four complete games / 14,624 operations. Strict
Clippy, Rustfmt, whitespace and frozen Python checks pass. Other workspace
integration tests and the older exhaustive all-corpus restoration gate were not
rerun. See rust/artifacts/dense-protection-final-validation.json.


#### Protection traversal follow-up

The optimized-build profile contains 3,425 self samples: 25.0% in the protection
walk, 44.2% in unresolved libc code. Library samples remain unattributed; they do
not prove allocator/caller cost. Added scripts/rust_oracle/native_profile.py to
verify worker build identity, translate executable ELF file offsets and retain
LTO symbol aliases. Its two guard tests pass, including nonzero segment deltas,
symbol gaps, shared aliases and mismatched build/path identities; a real old
profile is also rejected against the current worker.

The next small refinement resolves exclusion keys to interned IDs once per walk
and checks exclusions only for object references. Scalar fields cannot add
graph edges; equally named fields on non-step containers still traverse. All
319 core tests pass before an equivalent Clippy condition cleanup. Strict lint
and formatting pass. All six matched benchmark endpoints and alias/count/root
checks pass, with 87.5–150.6 simulations/s (4.10–8.13× Python). The small
cross-run improvement over the dense bitmap alone is not a firm speedup claim.
Full CLI corpus regression passes all 11 tests, covering all 16,226 live cases /
130,859 operations, 6,592 canonical cases / 27,145 operations and four complete
games / 14,624 operations. The three protection-specific tests also pass after
the equivalent condition cleanup, and the frozen source remains unchanged.
Results are in rust/artifacts/protection-fields-final-validation.json; the retained
full replay log is rust/artifacts/protection-fields-cli-tests.log. The original engine,
roster, search, integration and performance requirements remain open.


#### Xargatha primary recipes and cached-tile adjacency

Added CountAdjacentEnemiesStep as an authoritative live query. It reads the
actor's direct location and board dictionary tile occupants, matching the frozen
source rather than deriving occupancy from current location records. Hero pieces
count individually; an unlocated multi-piece owner does not acquire an acting
piece position. Missing actors/units/locations return zero. Arithmetic subtracts,
clamps the count at zero and then multiplies, preserving negative multipliers.
Unrepresentable i64 results remain an explicit unsupported-rule guard before
popping or writing; this does not claim arbitrary-precision/raw-exception closure.

All 18 Xargatha primary faces now have reviewed recipes (16 newly admitted).
Reviewed primary coverage is 206/596: 158 pure builders, three stateful builders
and 45 inherited base no-op faces. Spell coverage remains 22/22. Independent
iterate-worker parity passes 482 scenes / 7,850 operations, including copies,
reperformance, optional/invalid choices, negative attacks, range/radius defaults,
heavy-minion immunity, real round-end delayed defeat, cached occupancy drift and
Rapid Thrusts recounting after its first kill. All 107 affected source contracts and all 321 core tests pass, including the two
new native counter tests and expanded dense guard test. Full-family restoration and independent release replay pass all 482 cases and
7,850 operations. All 11 CLI tests pass for the Xargatha milestone. The family corpus SHA-256 is
`a2f2a426a78d5c870731a2fd10f64f2434ea31312586c3f5142a09daef4ac457`.

Full roster/defense/passive/filter/world and exception closure, dense retirement,
compact state/make-unmake, canonical hashing/evaluation, team-aware iterative
minimax/alpha-beta, hidden/simultaneous planning, parallel search, integration and
strength validation, and 1,000 complete two-global-turn simulations/s remain
required. No Python source/pin, NN runtime or live service changes are involved.


#### Bain bounty targeting

Bain's Dead or Alive, Hand Crossbow and Hunter Seeker are now admitted to the
native primary catalog. Marker filtering reads the marker registry by key and
matches the exact target ID. Source cases cover aliases across marker keys,
markers in supply, placement and removal, exact Razzle piece targets, minions,
and both attack orders. All 85 scenarios match Python across 2,441 operations
with state, request, event and identity parity. Reviewed primary coverage is
209/596 at this milestone. All 321 core tests, the boundary restoration check,
release replay and all 11 CLI tests passed.


#### Shared line of sight and Bain line attacks

The native sight filter supports hex, unit and token selections. It resolves a
literal origin, a context origin or the acting hero and uses the board position
of the acting piece when applicable. The scan checks intermediate tiles for map
gaps and applies the requested unit, Petrify-aware terrain and actor-specific
obstacle rules. It walks the path lazily and excludes the destination from
blocker checks.

Light Crossbow, Heavy Crossbow, Arbalest and Get Over Here now have native
primary recipes. The initial iterate replay matched the source across all 181
scenarios and 2,366 operations. All 321 core tests, the 2,366-boundary restoration check and the release replay
pass with the final lazy scan. The expanded CLI suite is running. Reviewed
primary coverage is 213/596. Cutter's Grappling Bolt
also needs its movement step.


#### Grappling Bolt pull

Cutter's Grappling Bolt now selects a visible obstacle and queues a straight-line
move to the hex before that obstacle. The pull resolves the current acting
piece, checks topology and leaves movement validation to MoveUnitStep. The
iterate replay matches Python across 33 scenarios and 353 operations covering
terrain, units, tokens, adjacent anchors, topology splits and copied cards.
All 321 core tests, boundary restoration and release replay pass. Reviewed
primary coverage is 214/596.


#### Walk the Plank and zone changes

The native query reads the recorded hex and the unit's direct position. It
returns true when their zone IDs differ and null otherwise. The lookup
uses the tile's cached zone when present and falls back to the first zone
containing the hex for missing tiles.

Walk the Plank now uses this query between the push and forced discard or
defeat. The iterate replay matches Python across 20 cases and 402 operations,
including no-op pushes, unchanged zones, an empty defender hand and the minion
branch. All 321 core tests, boundary restoration and release replay pass. The
combined CLI regression for both Cutter cards passed all 11 tests. Reviewed primary
coverage is 215/596.


#### Cutter primary deck

The remaining fifteen custom Cutter primary builders are frozen from Python.
The direction filter handles literal, actor and recorded-hex origins and
checks topology before accepting candidates on the same ray. The solo victory
step reads live gold and queues the existing individual game-over step.

The charge attack uses a separate checked operand for primary attack plus two.
The freezer verifies both damage operands with a probe distinct from the radius
placeholder. The source traces exercise a negative charge attack, a retreat
after defeating the target and a victory after gaining coins.

All 18 Cutter primary faces are accounted for. The focused iterate run matches
Python across 320 scenarios and 5,871 operations. The release replay, boundary
restoration and all 321 core tests pass. The expanded CLI regression is running.
Reviewed primary coverage is 230/596.


#### Arena undo journal

Speculative arena scopes record an object's or physical card override row's
previous value on its first write in that scope. Nested scopes track their own
first writes. Rollback restores those records in reverse order, discards
temporary objects and card instances, and restores the symbol table.
Scopes can nest and unwind safely. Worker forks inherit the current arena
values and start a separate journal. The 323 core tests and all three aggregate
tests pass. Speculative replay matches Python for 49 operations across 12
aggregate cases and restores each starting arena envelope.

Live session scopes carry the arena journal through staged respawn, cleanup,
and upgrade commits. Copied step program bindings and protection entries are
undone individually. Session frames, planning state, counters, and the typed
world projection still use a value checkpoint. The checkpoint shares the fixed
card, entity, and hero lookup tables and shares card history until it changes.
Physical card and hero rows use first-write undo at the typed world boundary.
Staged world commits carry the journal, and ordinary worker clones start an
independent journal. Spatial undo restores location order, symbol indexes, and tile occupants.
The compact occupancy bitmap and other mutable world state still use a
checkpoint. Incremental undo for those components and a search fingerprint
remain open.

Snorri's Inscribe the Runes brings reviewed primary coverage to 231/596 faces.
Its source-derived live recipe matches Python over seven card-text operations
and 23 normal card-action operations, including ordered batches and auto-fill.

#### Sampled release rollout baseline

A release run on one i5-10400F CPU under WSL used 128 repetitions per
position, game seed 29, sample seed 39, a two-global-turn horizon, first-legal
continuation and the lab-attack-range-v1 evaluator. Eight sampled openings and
midround positions covered Forgotten Island and Magma and Ice with level-one
and upgraded rosters. Rust matched Python's final state, decision count and
evaluator in all eight. Rates were 705–877 Rust and 32–51 Python rollouts/s.
The four unsampled controls ran at 515–898 Rust and 30–46 Python rollouts/s.

This baseline repeats a fixed sample per position. It measures matched
diagnostic rollouts. The diverse-policy, latency, strength and 1,000
rollouts/s gates remain open. The release executable was built from
9f7d080f plus the working changes and has SHA-256
4f3c1f629804bb45b67bd1885a4e5e29cad803807179b00a06b2343fe2ae738a.
The lab evaluator source hash is
c89203f8bfbefec63e13c3426c42cdba27fc401b0884efb38b3adb1eaac753b3.

The timed sampling wrapper (observation, hand reconstruction, installation
and rollback) accounts for 4.4–5.9% across the same eight release positions.
The rerun matched Python on all eight and measured 729–870 Rust rollouts/s.
A separate release profile of 4,000 sampled Magma and Ice openings captured
4,378 user-space self samples. The largest named Rust entries were
run_objects at 7.4% and ObjectArena::field at 5.5%. Card-list traversal and
collection synchronization also appear among the leading entries. libc
accounts for 25.7%, grouped at library level. These are self samples and
cannot attribute that library time to a caller. The local profile and request
are retained under the WSL benchmark cache. Prioritize rule execution and
collection synchronization over further sampling optimizations for this workload.

#### Reuse unchanged card collections

Card projection synchronization compares the live arena list against the
typed row and reuses unchanged vectors. It still inspects every live handle,
including aliased lists, and refreshes auras. Unchanged hero rows also avoid
an undo-journal write. All 348 core tests passed.

On one pinned CPU, five alternating pairs of 512 release rollouts per
binary and position showed median gains of 11.4–23.9%. Sampled openings and
upgraded midround positions rose from 767–864 to 885–1,051 rollouts/s.
Unsampled round boundaries rose from 520–564 to 579–663 rollouts/s.
Decision counts and lab evaluator values matched between binaries in all
six positions. This first-legal workload establishes a speedup at identical
settings. The full diverse-position throughput target remains open.

#### Dense board coordinate lookup

A round-end profile from Magma and Ice recorded 5,550 user-space self
samples. Hex hashing accounted for 5.9% and BoardGeometry::obstacle for
3.8%. Compact boards now use a flat axial lookup bounded to four cells per
tile. Sparse boards keep the hash lookup. Tile IDs, holes and duplicate
validation are preserved. All four geometry tests and 283,752 pinned Python
movement destination comparisons passed.

The same six-position paired release benchmark improved round-end throughput
from 637 to 769 rollouts/s on Forgotten Island and from 580 to 678 on Magma
and Ice. Sampled positions changed by -2.3% to +9.6%, with matching evaluator
values and decision counts. The round-end positions remain below the 1,000
rollouts/s target.

A follow-up tried adopting the field vector directly when building fresh arena
dictionaries. All seven focused arena tests passed. The same paired release
workload returned changes between -4.8% and +4.7%, with round-end changes of
-1.6% and +1.7%. The optimization was reverted because it offered no consistent
benefit. The raw paired rates are in the local WSL benchmark cache.

#### Scoped planning worker scaling

The release worker benchmark used four sampled worlds, four rows and replies
per world, one search depth, and a two-global-turn continuation. Five
interleaved repetitions per configuration on four physical i5-10400F cores
compared one, two and four workers. The four opening and midround positions
on Forgotten Island and Magma and Ice returned identical commitments,
scores, coverage and node counts at every worker count. Python verified
the winning plans. Median request latency including transport ranged from
350–429 ms with one worker, 247–294 ms with two and 214–244 ms with four.
Four workers were 1.63–1.92x faster than one. The generous test budget let
all rows finish. Shared budget behavior across multiple live games remains
unmeasured.

#### Single validation per native root import

NativeSession validates the incoming envelope once. Its private arena,
planning, and world constructors consume the validated input or its derived
projection. Standalone public constructors still validate their own inputs.
Sixteen alternating import-and-evaluate requests per binary and opening
measured 146.5 to 124.6 ms on Forgotten Island and 159.2 to 129.6 ms on Magma
and Ice with the iterate build. Both binaries returned identical evaluations.
Sampled and upgraded rollout state parity passed across all six maps.

A borrowed typed-world projection avoided cloning the complete source
envelope. It passed all six maps' sampled and upgraded rollout checks, but
the same paired opening benchmark moved from 125.0 to 128.8 ms on
Forgotten Island and 127.1 to 124.2 ms on Magma and Ice. The variant was
reverted because the measured workload showed no consistent latency benefit.

#### Rotating hidden-world rollout samples

The release benchmark can rotate through up to eight reproducible seeds and
check the native state, node count and evaluator against Python for each.
On one pinned i5-10400F CPU under WSL, 1,000 two-turn first-legal rollouts per
position rotated seeds 39–42. Forgotten Island's opening ran at 1,021 Rust
versus 53 Python rollouts/s. The upgraded Magma and Ice turn-two position
ran at 1,093 Rust versus 33 Python rollouts/s. Both matched the Python end
states, decision counts and lab evaluator across all four samples. Root
import and warmup were outside the timed loop. These are generated
positions and a diagnostic policy. The original recorded search workload
and broader coverage gate remain open.

#### Round-end rollout coverage

The release benchmark now checks seeded hidden-hand rollouts across round
closure as well as at turns one and two. On one pinned WSL CPU, the current
build ran 500 two-turn rollouts per position across all six bundled maps with
the Arien/Brogan versus Tigerclaw/Sabina roster. Sampled rates were 960–1,098/s
on turn one, 1,065–1,155/s on turn two, and 681–808/s on turn four. All 72
sampled end states matched the untimed Python rule replay. Unsampled checks
matched at both horizons for all six openings and round ends.

Sampling and restoring the hidden world accounted for 3.5–4.0% of turn-four
time. Profile round closure and advancement before optimizing the sampler for
this workload. The 1,000/s gate remains open across the full target corpus.

A 6,000-rollout Forgotten Island perf sample found arena object-field vector
copying at 5.6% of round-end CPU samples and object-field vector drops at 2.5%.
Cleanup and round transitions stage full arena copies to preserve atomicity.
The profile showed where copies occur in aggregate. The installed perf build
could not resolve the sampled caller stacks.

An arena-wide copy-on-write experiment passed all 351 core tests. Across
three alternating 750-rollout batches per build, median round-end throughput
rose from 765 to 843/s on Forgotten Island and 722 to 802/s on Magma and Ice.
Opening throughput fell from 1,100 to 1,036/s and from 965 to 915/s. The
experiment was reverted. Scope the next attempt to staged transition copies
so ordinary steps keep their current fast path.

#### Rust-selected replay with a second roster

The replay driver accepts a dedicated four-hero roster with
`--rust-replay-turns 6 --replay-roster Emmitt Min Misa Wasp`.
At the default game seed 29, five turns pass on Forgotten Island and
Magma and Ice in both policy assignments (235 decisions total, with matching
final states). At six turns, Forgotten Island diverges at trace index 65
(round 2, turn 2) with the candidate on the first team. Python offers Misa
an optional one-space pre-action movement prompt, while the Rust trace
submits CONFIRM. The divergence also reproduces on the pre-copy-on-write
release worker. Inspect the native action continuation and Misa's movement
effect before attributing the difference to either one.

The no-upgrade cleanup branch advanced the arena and round state but left the
combat round stale. Importing the same action prompt reconstructed the combat
clock, which explained why the isolated replay produced Misa's movement prompt.
Cleanup now advances both clocks, with a regression assertion for the typed
combat clock. Seven focused cleanup tests passed. The rebuilt iterate worker
matched Python through twelve turns across all six maps and both policy
assignments, validating 1,749 Rust decisions and all twelve final states.

The offline verifier now accepts the worker's full 64-turn horizon and labels
the endpoint as a turn boundary or game over. Nine 2v2 rosters cover all 33
standard heroes through twelve turns on Forgotten Island and Magma and Ice,
with both policy assignments. Gydion/Xargatha versus Cutter/Hanu also passed
64-turn replays on both maps and in both assignments, covering 3,094 Rust
decisions and matching final states at round 17, turn 1. These matches reached
the turn cap. Coverage of the hero roster here establishes that each hero
participated in accepted-choice replay. Card and terminal-state coverage are
tracked separately.

The offline verifier's `--replay-terminal` mode reconstructs both recorded
Hanu late-game positions four input decisions before their reference endings.
Rust selects the remaining decisions with both policy assignments. Python
validates and replays those decisions and checks the entire terminal state,
team winner and individual winner fields. All sixteen decisions passed.
Forgotten Island ended in a Blue win and Magma and Ice in a Red win.

#### Current recorded two-turn throughput

The current release worker matched Python's untimed reference endpoints for
all six fixed Hanu replay windows, with identical clone and rollback results
and unchanged roots. Five samples of 500 iterations per mode on one WSL CPU
produced these median Rust simulations per second.

| Window | Clone | Rollback |
| --- | ---: | ---: |
| Forgotten Island opening | 960 | 1,143 |
| Forgotten Island round boundary | 719 | 788 |
| Forgotten Island combat | 852 | 983 |
| Magma and Ice opening | 848 | 1,049 |
| Magma and Ice round boundary | 386 | 404 |
| Magma and Ice combat | 801 | 970 |

Magma and Ice's round boundary is the slowest recorded case. It advances
through 26 input requests and 20 events, versus 22 requests and 14 events on
Forgotten Island. Advancement accounted for roughly 2.45 ms of its 2.48 ms
rollback run, so profile this exact trace before attempting another arena
layout change. The recorded throughput gate remains open.

#### Recorded round-boundary profile

A pinned 999 Hz user-CPU profile of 3,000 rollback simulations per recorded
round boundary collected 3,890 samples on Forgotten Island and 7,416 on Magma
and Ice. Arena field-vector clone and drop routines accounted for 9.1% and
20.7% of samples respectively. BoundObjects cloning itself accounted for
1.0% and 3.6%. The profiles measure self time, and the unresolved libc samples
remain unattributed.

The reference instruction stream contains one cleanup and no upgrade or reset
steps on Forgotten Island. Magma and Ice contains one cleanup, two upgrade
steps, one reset and one defeat. Each successful Rust cleanup, upgrade or reset
currently stages a full arena copy. Prioritize that upgrade/reset path in the
transaction work, and use the recorded Magma and Ice window as the benchmark.

Upgrade resolution and round reset now use a nested arena journal instead of
cloning the arena. The typed session remains staged. On success the arena
undo entries are retained for any enclosing rollback. On error the arena and
copied bindings are restored. Request publication is inside the transaction.
All 351 core tests passed. Three alternating 500-replay release batches per
build measured a median 410 to 567 simulations/s for the recorded Magma and
Ice round boundary (+38%). Both builds matched Python's endpoint, counts
and root invariants on the two openings and round boundaries. The other
three medians moved by at most 3%. Cleanup and respawn still use staged
arena copies, and the 1,000/s recorded throughput gate remains open.

Cleanup and hero respawn now share the upgrade/reset transaction, removing
the last full-arena staging helper. The typed session is still staged to
preserve atomicity. Three alternating 500-replay release batches per build
measured median rollback throughput of 814 to 999/s on Forgotten Island's
round boundary and 565 to 648/s on Magma and Ice. Opening medians changed by
less than 2%. The paired endpoints, counts and root invariants matched the
reference. All 351 core tests passed. The rebuilt release worker also passed
the 16 terminal decisions and all 1,749 decisions in the six-map Emmitt/Min
versus Misa/Wasp replay. The recorded throughput gate remains open.

A fresh 5,000-replay profile after the arena changes put field-vector clone
and drop at 8.5% of 7,734 Magma and Ice samples, down from 20.7%. Hex lookup
was 5.4%, obstacle evaluation 2.7%, and the reachability result loop 2.5%.
Movement traversal now checks obstacle and token passability only on the
first topologically connected approach to each tile, while retaining the
edge-specific connectivity checks. It also borrows the actor fallback slice.
All 17 focused movement tests and paired recorded endpoint checks passed.
Median rates across four paired positions moved by less than 2%, so this
change does not establish a throughput gain. Magma and Ice remains below
the recorded throughput gate.

Stateful movement now carries tile indices through the queue and uses the
board's precomputed adjacency and indexed obstacle bits. Off-board origins
still resolve coordinates directly, and topology checks retain their original
order and coordinate pairs. All 17 movement tests, four geometry tests and
72 seeded rollout endpoint checks passed. Three alternating release batches
per build moved the Magma and Ice round-boundary median from 655 to 671/s.
The other three paired medians varied by about 2% in either direction, so
the result establishes only a small gain on the slow recorded position.

A single-pass live guard read and borrowed context-value experiment passed
all 351 core tests and the paired recorded endpoint checks. It changed all
four measured medians by less than 2% and moved the Magma and Ice boundary
from 665 to 655/s. The experiment was reverted. The result points away from
guard field access as a near-term throughput improvement.

#### Sampling at a live resolution prompt

A resolution-root sample must preserve the already revealed current and extra
cards, their physical handles, the active actor, and the root request token.
The planning installer removes hidden commitments and rebuilds hand and
lifecycle rows. Calling that installer unchanged during resolution could
invalidate references held by active effects, nested steps or the pending
request. Extract the common physical-card update from the planning installer
and give resolution its own validation and installation path.

Start with prompts whose payload and continuation bindings are independent of
the hidden hand. Preserve visible card handles, and use the observed current
reveals and public upgrade evidence as constraints on the sampled remainder.
Require the sampled world to present the same root owner, request token and
offered choices. Prompts that expose or retain private physical cards need
explicit observation and rebinding rules before they can use this path.
Exercise same-seed resolution choices with two roots that differ only in
unobserved hands, including a defense continuation before the next planning
boundary. Check exact root restoration on interruption and after scoring.

The experimental offline worker accepts `resolution_sample_seed` on
`search_resolution`. It samples one world at an ordinary, uncontrolled
action, defense or initiative prompt, preserves revealed current
cards and the issued options, and restores the root after scoring. The root
sample can be combined with a turn rollout and an independent
`planning_sample_seed` for later planning epochs. Public upgrade evidence
can constrain both samplers.
A same-seed test with an unobserved card substitution preserved the result,
depth, and node count. Python accepted the Rust-selected SKILL on a live
Forgotten Island action prompt with the original request token.

Worker smoke checks with root seed 39, future planning seed 71 and a
two-turn horizon completed depth one on Forgotten Island and Magma and Ice
with 38 and 40 nodes respectively. Python accepted both Rust selections
against their original request tokens.

A level-two Forgotten Island position also validated sampling directly at
a team initiative tie. The imported tie-break frame retained its request
token and team eligibility, and Python accepted the Rust-selected actor.

The first supported active-effect family at a sampled resolution root is
pre-action movement without finishing steps. The source and any linked card
must be in revealed played or discarded slots. A live Forgotten Island
position with Misa's Focus active passed the selected-action check in Python.

Run the maintained verifier with `rollout_parity.py --sampled-resolution
--all-maps --worker <worker>`. It validates the Rust-selected opening
action and upgraded team tie on all six maps. The current iterate worker
completed all twelve two-turn searches at depth one with 38–42 nodes each.
Python checked the submitted choices against the original requests.

The maintained verifier also runs the Forgotten Island Focus case at seed 29.
With the same two-turn settings, it completed depth one in 107 nodes and
Python accepted ATTACK. The Focus case uses this fixed map and seed because
other opening continuations can leave additional effect families active.

Resolution search also accepts an optional `resolution_sample_worlds` count
from one to eight with `resolution_sample_seed`. Each root move is evaluated
in the same sampled worlds. The search compares sums of exact per-world
scores, shares a node and time budget, and only publishes an aggregate score
after a move has completed across every world. The original one-world search
remains the default. Use `--sample-count 3` with the sampled resolution verifier
to validate the multi-world selection against the original Python request.

The three-world verifier completed depth one on all six maps, covering six
opening actions, six upgraded initiative ties and the Forgotten Island Focus
case. Python accepted all 13 selected inputs. The ordinary prompts used
108–120 search nodes, and Focus used 306.

#### Roster-wide replay smoke checks

Run the offline verifier with --roster-sweep-turns 12 to play nine first-legal
Rust matchups covering all 33 registered heroes. The default maps are Forgotten
Island and Magma and Ice. Add --all-maps for the full bundled map list.
Python validates only the selected decisions and the final state of each replay.
Seed 29 passed all 18 default cases and 2,580 decisions through round four.
These fixed-policy walks exercise ordinary match interactions across the roster.
Other policies, longer games and broader interaction coverage remain open.

Seed 31 passed the 24-turn sweep across all six maps. Python validated 15,862
selected Rust decisions and all 54 endpoints at round seven, turn one. None
of these fixed-policy replays had reached game over by that cutoff.

Add --sweep-policy sampled to exercise a seeded selection of legal planning
commitments and resolution inputs. This includes optional second cards when
the hero can play them. The seed controls the diagnostic policy as well as
game setup. Both teams use that policy during this rule-validation sweep.

The seed-37 sampled sweep passed all 54 map and roster cases through 12 turns.
Python accepted all 7,590 selected decisions and matched every final state.

The seed-37 sampled sweep also reached round 17 on the two reference maps.
Python validated 13,449 selections and all 18 endpoints, including a focused
rerun of the Wasp matchup after correcting the verifier to skip automatic
passes. The 64-turn cutoff was reached before game over in every case.

The sweep also accepts --sweep-team-size and --sweep-game-type. All 34
1v1 quick games on the two reference maps matched Python over 13,506 decisions
through 64 turns. All 30 4v4 games across six maps matched over 5,599 decisions
through eight turns. Both sweeps reached their turn caps. Separate replay from
recorded late-game positions validated all 16 selected terminal decisions and
the winners on both reference maps. The preceding CLI suite passed all 12 tests.

#### Parallel scoring across sampled resolution worlds

The verifier matched serial and parallel replies across 13 prompts on six maps,
including the selected move, score, depth and node count. Python accepted every
selection. A preliminary iterate-build run alternated one and two workers for
four worlds, eight requests per mode per prompt. On two pinned WSL CPUs, the
median of per-position medians was 175.50 ms serial and 175.22 ms parallel.
The exhaustive fixture suite was active on the same host. This shallow workload
shows no meaningful end-to-end speedup. Serial remains the default.

#### Recorded sampled rollout throughput

The provisional paired release run compared the current rollback changes with
the pre-occupancy-journal build. All six recorded replay endpoints matched.
Rollback replay throughput changed by -5.8% to +3.8% across the six positions.
The Magma and Ice round boundary moved from 624 to 626 replays/s.

Full first-legal rollouts from those recorded positions rotated four seeds for
1,000 iterations each and used the lab attack-range evaluator. All 24 states,
decision counts and evaluator checks matched the untimed Python validation.
Rust achieved 545–746 full rollouts/s. Magma and Ice round-boundary throughput
was 578/s. Sampling and restoration accounted for 4.9–6.4%. The exhaustive
fixture suite was active on the host, so repeat timing under idle conditions.
The 1,000/s gate remains open.

A 5,000-rollout profile of the recorded Magma and Ice round boundary mapped
7,900 CPU samples to the release binary and shared libraries. Interpreter
execution accounted for 9.3% of samples, arena field lookup 6.7% and literal
materialization 3.0%. Combined card-collection synchronization was 3.7%.
Unresolved libc frames accounted for 20.8%. These are self-time shares.

Scored offline resolution now uses two sampled root worlds and applies forced
choices directly. After separating 65 forced inputs, the eight-turn replays
reported 57 fallbacks. Allowing attack-immunity effects with revealed source
and linked cards and no finishing program lowered that count to 47, with all
344 selected decisions still matching Python. The remaining diagnostic reports
identify action restrictions, upgrades and movement destination prompts.

Sampling now admits public action restrictions with revealed source bindings
and ordinary movement selectors followed by a live move. The eight-turn
replays still matched all 344 decisions and endpoints, with fallbacks down
from 47 to 36. A three-world search at a Forgotten Island movement prompt
matched serial and parallel results, and Python accepted the selected hex.

Allowing public placement prevention and minion protection reduced the two
reference maps to 14 fallbacks over their eight-turn replays. The six-map run
matched all 12 replay endpoints and checked every selected decision against
Python. Unsupported prompts now include mandatory upgrades and several
card-specific movement, placement and repeat prompts.

The root field index passed focused arena tests and all six paired replay
endpoints. Its rollback replay rates ranged from -3.8% to +4.0% while the
workspace suite was active. The experiment was removed because the paired
results showed no consistent speed benefit.

The arena now records first writes with reusable bitmaps for object and physical
card slots. Six alternating 1,000-rollout release batches on Magma and Ice moved
the median from 541 to 565/s (+4.6%), with matching sample counts and evaluations.
The workspace suite was active on the host, so this is a provisional result.

The sampled-resolution verifier also checked a 73-destination Fast Travel
prompt on Forgotten Island. The three-world search visited 4,122 nodes with
matching serial and parallel replies. Python accepted the selected destination.
Fast Travel sampling requires any referenced source card to be a revealed
current or extra card of the decision owner.

The corrected full workspace run for commit d86f4d28 completed with 530 passing
tests and no failures. It includes the exhaustive 17,347-case restoration run.

Cleanup, upgrades and respawn now use nested typed-row journals in the same
transaction as the arena. The remaining session fields retain a value snapshot.
All 356 current core tests and the nested session rollback integration passed.
Six paired replay endpoints matched. Round-boundary rollback medians improved
from 831 to 904/s on Forgotten Island and 773 to 816/s on Magma and Ice. The
other four positions ranged from -3.6% to +5.6%. Full sampled timing is separate.

After the exhaustive suite completed, a four-seed, 1,000-iteration recorded
sampled rollout run matched Python states, decision counts and lab evaluator
values in all 24 cases. Rates ranged from 734 to 1,093/s. Forgotten Island
round-boundary throughput was 844/s and Magma and Ice was 761/s. The latter
remains below the 1,000/s acceptance target.

Typed card and hero journals now reuse first-write bitmaps as well as the arena
journal. All 356 core tests passed. Focused nested transition verification and
paired release measurements are pending.

The typed-row bitmaps passed nested rollback verification. Four alternating
release batches per workload matched all six paired replay endpoints. The
bitmap change moved rollback rates by -3.4% to +1.9%. The full sampled release
run on this build matched all 24 Python state, count and evaluation checks.
Rates were 860–1,183/s. The round boundaries ran at 932/s on Forgotten Island
and 875/s on Magma and Ice. The throughput gate remains open.

The incremental fingerprint now covers ordered entity positions and tile
occupants. Spatial rollback restores the saved fingerprint and checks it against
a full recomputation in debug builds. The existing nested spatial test checks
both changed and restored positions. All 356 core tests passed. Card, effect,
continuation and context keys are still required before enabling cache reuse.

The dense-world diagnostic fingerprint now includes physical card rows,
including their lifecycle, equality metadata and typed dynamic metadata.
Card updates invalidate only affected rows. Nested rollback invalidates the
restored rows even when a branch already computed its fingerprint. The
recorded simulation benchmark checks the combined spatial, hero and card
fingerprint after each replay batch. This key is scoped to a native root and
its symbol table. Effects, pending continuations and execution context are
still required for a search cache key.

The diagnostic key also covers the ordered typed active-effect rows,
including compiled behavior, mutable counters and native arena bindings.
Effects are hashed when the diagnostic is read. The existing restore
verification still checks the full envelope, including finishing steps stored
in the object arena. The typed key does not yet cover those arena continuations,
execution context or all other mutable world fields, so it remains diagnostic.

The rollback benchmark now also verifies a separate typed execution
fingerprint. It covers the stack and frame overrides, current request,
request and step generation counters, context, arena payloads and physical
overrides, and step-to-program bindings. The shared pending-step fixture
checks that submitting a choice changes the fingerprint and that rollback
restores it. The diagnostic scans arena storage and uses root-local IDs.
A transposition key still needs complete mutable state coverage, a definition
of equivalence across branches, and measurement of lookup cost and hit rate.

Sampled multi-world resolution search now uses a bounded immediate-scoring
pass before game-turn rollouts. It publishes a provisional score only after
the same candidate has been evaluated in every sampled world. Warmup nodes
count against the shared search budget. A terminal or horizon-reaching score
is retained for that candidate and world. Completed rollout depths still take
precedence. This improves the fallback when the first rollout cannot finish.

Multi-world resolution now visits the best warmup move first, then starts each
deeper iteration with the previous completed winner. Equal aggregate scores
still favor the earlier move in the original legal order. The 13 Rust search
tests passed. At a five-second budget with three sampled worlds, one and two
workers returned identical replies for 15 prompts across all six maps. Python
accepted all 15 Rust selections. Every prompt completed the requested first
depth with a two-turn rollout score, including the 73-destination Fast Travel
case. This run verifies decision legality and worker agreement on the sampled
prompts.

The sampled-resolution verifier can now exercise interrupted requests with
`--verify-resolution-deadline`. It validates every returned move against
the original Python prompt and reports setup deadlines separately from
scored and unscored search fallbacks. Timing is measured at the caller and
includes JSON and transport overhead.

With an iterate worker, a 200 ms internal budget, three worlds, one and two
workers, and three repetitions per prompt, 83 of 90 requests completed
depth one. All six Fast Travel interruptions returned a scored candidate
that Python accepted. Their end-to-end maxima were 210.74 ms and 209.99 ms
for one and two workers respectively. The first cold request expired during
setup and returned no move in 242.07 ms. The strict five-second verification
still matched replies across worker counts and all 15 selected moves were
legal. Cold preparation and caller-visible deadline accounting remain open.

The offline worker now accepts an idempotent `prepare_search` request to
initialize immutable rule catalogs and compiled primary programs before
timed decisions. The verifier's `--prepare-search` option reports preparation
separately. It took 151.53 ms on the iterate build. On the same 90-request
six-map workload, 84 searches completed depth one and six Fast Travel searches
returned scored deadline fallbacks. Python accepted all 90 moves. The first
prepared decision took at most 53.48 ms across its single-worker repetitions.
The worst Fast Travel reply took 210.10 ms end to end with a 200 ms internal
budget. Caller-side deadline accounting remains open.

The offline worker caller can now supply a monotonic decision deadline. It
deducts envelope serialization time and an explicit transport reserve before
sending Rust its remaining budget, and retires a stalled worker at the caller
deadline. The sampled-resolution verifier enables this path with
`--resolution-transport-reserve-ms`. In the prepared six-map run with a 200 ms
caller budget and a 15 ms reserve, all 90 replies were within budget and all
90 returned moves were legal. Six Fast Travel replies hit the internal search
deadline and returned scored fallbacks. The slowest reply took 191.56 ms.
The measurement starts with a prepared envelope and stops before oracle
validation. Preparation, live game export, other workloads and real-time
scheduling guarantees are outside that measurement.

An inline single-choice buffer removed temporary allocations in fixed-policy
resolution. Three alternating release runs per variant used 1,000 rollouts
per recorded workload and four sampled worlds. Every run matched the Python
endpoints, decision counts and evaluator checks. Median rates changed by
-2.2% to +1.5% across six workloads. The experiment was removed because the
changes did not show a consistent throughput gain. In the same measurements
the baseline medians were 887/s and 797/s at the Forgotten Island and Magma
and Ice round boundaries. The 1,000/s throughput gate remains open.

A build-ID-checked release profile of the Magma and Ice round boundary
collected 6,606 CPU samples. Interpreter execution accounted for 7.27% of
self time, arena field lookup 6.39%, literal materialization 2.59%, symbol
lookup 2.53%, and card collection synchronization and its comparison closure
4.15% combined. The unresolved libc share was 22.28%.

An arena experiment recorded individual field edits instead of cloning a
whole object on its first field write. It passed nested rollback, including
a committed child scope with a whole-dictionary clear. Three paired release
runs matched Python endpoints, node counts and evaluator values. Median
throughput changed by -1.68% to +1.90%, so the field-journal experiment was
removed. The Magma and Ice round-boundary median moved from 837 to 853/s.

Arena cycle validation now borrows existing inputs and visits object links
only. Leaf objects need no traversal allocation. Read-only hot paths can also
borrow fields instead of cloning values. Three alternating release runs per
variant matched all Python endpoints, node counts and evaluator checks for
four seeds on each of the six workloads. Median throughput improved by
6.0–10.0%. Forgotten Island's round boundary rose from 920 to 995/s, and
Magma and Ice's from 845 to 897/s. Five arena unit tests and the workspace
check passed. The remaining sub-1,000/s workloads keep the throughput gate open.

The recorded two-turn benchmark now supports one to four independent workers.
Each worker owns its session and restores it after each rollout. All results
match the serial per-seed warmup and the verifier checks the sampled states,
decision counts and evaluations against Python outside the timer. Timing
includes worker startup and all rollout work, with root import, cloning and
post-run validation outside the timer. Worker-time sums are reported
separately from elapsed wall time.

On the i5-10400F WSL host, the release benchmark ran 3,001 iterations per
position with four rotating seeds. One worker delivered 878–1,228 rollouts/s,
two delivered 1,669–2,379/s, and four delivered 3,061–4,400/s across all six
positions. Magma and Ice's round boundary measured 918, 1,669 and 3,158/s.
The sampled throughput target is demonstrated on this corpus with two
workers. This measures independent fixed-policy rollouts. The serial
throughput gap, broader corpus and adversarial search gates remain open.
On the separate 5,000-iteration Magma and Ice round-boundary run, peak
process RSS was 140,828 KiB with one worker, 142,636 KiB with two and
145,712 KiB with four. The four per-seed results were identical.

The roster verifier now supports `--sweep-policy search`. Both teams use
native sampled planning and resolution search, with the CLI decision budget.
Python validates Rust's selections and endpoints. The verifier checks winner
fields whenever a game reaches game over and prints fallback counts by team.

A seed-29, 100 ms, four-turn quick-game sweep covered all 33 registered heroes
in 18 matches on Forgotten Island and Magma and Ice. All 863 recorded Rust
decisions and all endpoints matched Python. Each match reached its turn cap.
There were 445 search fallbacks across per-hero planning and resolution
decisions. The most actionable gaps include public effect handling
(MovementZone, LosBlocker, MovementAura, AreaStat and StaticBarrier), card-specific
prompts, and ordinary-hand sampling in some rosters. These are search coverage
gaps even though fallback moves and rule execution matched in this run.

Sampled resolution now admits MovementZone and LosBlocker effects with
revealed source and linked cards and no finishing program. The guard checks
the live arena's finishing lists because the dense world projection omits
them. Three sampler tests passed, including the arena finishing regression.
The same 18-match sweep again matched all 863 Python decision checks and
endpoints. The iterate run logged 439 fallbacks against 445 in the earlier
release run. MovementZone and LosBlocker no longer appeared among the
reported leading reasons. The builds use different profiles, so the count
difference is descriptive.

The hidden-hand sampler now uses the printed starting pools for level-one
Dodger, Min and Snorri when their public card evidence and upgrade checkpoints
agree. Their nonstandard upgrade paths remain outside this reconstruction.
Six loadout tests passed. The same 18-match, 100 ms iterate sweep reduced
fallbacks from 439 to 333. All 863 decisions and final states matched Python.
The sampled matches reached their four-turn caps.

The sampled-resolution effect guard now also accepts public area-stat effects,
movement auras, static barriers and petrification when their source and linked
cards are revealed and the arena has no finishing program. Three focused
sampler tests passed. On the same 18-match iterate sweep, fallbacks decreased
from 333 to 291. All 861 recorded decisions and endpoints matched Python.
Some effects are still rejected because their source or linked card is
outside the supported public zones. Other remaining gaps are prompt-specific.

Sampled resolution accepts the fixed color chooser and eight reviewed
two-option numeric selectors used by printed card modes. Other numeric
requests still require provenance review. The three sampler tests passed.
The same 18-match iterate sweep reached all turn caps, matching 864 selected
decisions and every final state against Python. It recorded 288 fallbacks,
compared with 291 before the change. Some similarly labeled prompts are
outside the supported selectors or still have unsupported effect provenance.

Resolution sampling now treats an effect's revealed current or extra card as
public provenance. Those physical slots already remain fixed across sampled
worlds. Planning retains its previous provenance rules. Four focused sampler
tests passed. The same 18-match iterate sweep matched all 861 selected
decisions and final states against Python and recorded 285 fallbacks, down
from 288. Other effect provenance fallbacks remain.

Resolution sampling now accepts cardless passive effects anchored to a
faceup, placed Smoke Bomb, Ice or Barrier token for the supported
line-of-sight and area-stat behaviors. The anchor must match the effect
scope. The existing finishing-step guard still applies. Five focused
sampler tests passed, including a Smoke Bomb fixture that rejects a
facedown anchor. The same 18-match iterate sweep recorded 274 fallbacks,
down from 285. Python accepted all 860 selected decisions and matched all
endpoints.

Resolution sampling recognizes the acting viewer's own private hand and
current slots when they supply an effect card. Other players' private
cards are excluded. The five focused sampler tests passed, including the
viewer boundary. Python accepted all 863 selections and matched all
endpoints in the same 18-match sweep. It recorded 287 fallbacks, versus
274 in the previous run, so this did not reduce the aggregate count.
The new diagnostics identify missing source cards for the reported
line-of-sight and area-stat effects. Their public token anchoring needs
further investigation.

Faceup public token effects now also survive sampled planning inside a
resolution rollout. The token's live arena row must have an empty finishing
list in both phases. Nine planning-command tests passed, including the
faceup/facedown check in both phases. The same 18-match iterate sweep
recorded 212 fallbacks, down from 287. Python accepted all 862 selected
decisions and matched every endpoint. Missing source-card errors for the
token effects no longer appeared among the reported leading reasons.

Sampled resolution now accepts simple public token and turret placement
selectors with range and occupancy filters when the next instruction
consumes the selected hex. The Smoke Bomb fixture exercises the new gate.
Nine planning-command tests passed. The same 18-match iterate sweep
recorded 203 fallbacks, down from 212, and Python matched all 862 decisions
and final states. Multi-token Glitch placement still uses fallback.

Sampled resolution now accepts Glitch batch placement when the current
selector and all batch slots use public range and empty-space filters.
The selected hex must feed the matching token placement instruction.
The recorded Unstable Timeline prompt and all ten planning-command tests
passed. The same 18-match iterate sweep matched 865 decisions and every
final state against Python, with 201 fallbacks. The subsequent Glitch
target selection prompts still require separate review.
