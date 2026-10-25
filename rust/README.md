# Native search engine

Work in progress, following [the implementation plan](../docs/superpowers/plans/2026-10-23-rust-search-engine.md).
This workspace executes the supported rule closure through ordinary full rounds.
It does not yet support general complete games or select bot actions.
The live Python backend and lab are unchanged.

## Build and test in WSL

The toolchain is pinned in `rust-toolchain.toml`; dependency versions are locked
in `Cargo.lock`. Install Rust with rustup and a C linker (`gcc` on Ubuntu), then:

```sh
cd ~/projects/goa2-backend-bots
sh rust/cargo-wsl.sh test --locked --workspace
sh rust/cargo-wsl.sh clippy --locked --workspace --all-targets -- -D warnings
sh rust/cargo-wsl.sh fmt --all -- --check
sh rust/cargo-wsl.sh build --locked --release --workspace
```

The wrapper keeps build artifacts on the Linux filesystem at
`$HOME/.cache/goa2-rust/target`; override `CARGO_TARGET_DIR` when needed. Source
files stay in the bot checkout. Plain `cargo` also works on other supported hosts.
Keep the source checkout on the Linux filesystem as well as the build outputs;
using `/mnt/c` for source can trigger WSL's Cargo performance notice.
The active checkout on this machine is `/home/sebastian/projects/goa2-backend-bots`.
The Windows checkout is retained as a committed backup and frozen Python runtime.
No neural model, training runtime, server startup or lab restart is required.

For faster full-corpus iteration, run the optimized test binaries in WSL:

```sh
sh rust/cargo-wsl.sh test --locked --release --workspace -- --test-threads=2
```

## Python reference and differential checks

The oracle is **00becf4a68615afa410f8eabe0c841809af0427b**, the legality-optimized
Python revision. `oracle-manifest.json` identifies every tracked Python engine,
search and data source by SHA-256, plus the exact Python, Pydantic/core and
jsonpatch versions. The content identity covers source files and that runtime.
Export and verification reject changed/new source files or runtime drift. Do not regenerate the manifest to accept a different oracle
without an explicit reference update.

The frozen Windows Python runtime can validate the Linux checkout over WSL's
file share. From PowerShell, set these paths to the preserved runtime and active
checkout (do not edit the Windows backup):

```powershell
$oraclePython = 'C:/Users/Sebastian/Documents/GitHub/goa2-backend-bots/.venv/Scripts/python.exe'
$engineRoot = '\\wsl.localhost\Ubuntu\home\sebastian\projects\goa2-backend-bots'
& $oraclePython "$engineRoot/scripts/rust_oracle/oracle.py" verify-movement -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
& $oraclePython "$engineRoot/scripts/rust_oracle/state_codec.py" verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
& $oraclePython "$engineRoot/scripts/rust_oracle/stack_traces.py" verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
& $oraclePython "$engineRoot/scripts/rust_oracle/model_traces.py" verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
& $oraclePython "$engineRoot/scripts/rust_oracle/planning_traces.py" verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
& $oraclePython "$engineRoot/scripts/rust_oracle/action_traces.py" verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
```

Replace the Linux executable path if using another user or target directory.
Prefer the verified Linux-local reference environment to avoid repeated file-share
reads. On this machine it is `$HOME/.cache/goa2-rust/oracle-venv/bin/python`:

```sh
cd ~/projects/goa2-backend-bots
oracle_python="$HOME/.cache/goa2-rust/oracle-venv/bin/python"
worker="$HOME/.cache/goa2-rust/target/release/goa2-search-cli"
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/stack_traces.py verify -- "$worker"
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" -m pytest -q tests/rust_oracle tests/engine/test_effect_manager.py
```

The environment pins CPython 3.11.17, Pydantic 2.13.4, pydantic_core 2.46.4
and jsonpatch 1.33. Exporters verify those versions and all frozen source hashes
before generating reference data. Do not substitute the system Python. Neural
model packages are unnecessary. The preserved Windows runtime remains available
for independent cross-platform comparisons.

To regenerate fixtures from the *unchanged* frozen source, using the same PowerShell variables:

```powershell
& $oraclePython "$engineRoot/scripts/rust_oracle/oracle.py" export-movement
& $oraclePython "$engineRoot/scripts/rust_oracle/state_codec.py" export
& $oraclePython "$engineRoot/scripts/rust_oracle/stack_traces.py" export
& $oraclePython "$engineRoot/scripts/rust_oracle/model_traces.py" export
& $oraclePython "$engineRoot/scripts/rust_oracle/planning_traces.py" export
& $oraclePython "$engineRoot/scripts/rust_oracle/action_traces.py" export
```

`cargo test` consumes the committed fixtures directly. Worker verification also
checks the persistent JSON-lines protocol and corpus/source fingerprints.
Movement disagreements are saved under ignored `rust/artifacts/` for diagnosis.

## Current coverage

| Area | Implemented and verified | Still required |
| --- | --- | --- |
| Reference | Frozen source manifest; independent Python BFS comparisons; fingerprinted corpora | Complete decision/event trace export, replay and minimization; full-search baseline runner |
| State boundary | Semantic round trips; typed effect storage and lifecycle; dense entities/physical cards; typed hero collections and ordinary card-model mutations; native positions/occupancy; ordered planning buffers and revelation knowledge | Token/effect lifecycle and remaining rule-relevant mutable state |
| Geometry | Checked cube/hex IDs; dynamic bitsets; terrain/occupancy; batched movement; effect-aware walking paths, dynamic obstacles and passable tokens; compiled range, obstacle, movement-path and straight-line predicates | Effect-aware push paths, targeting and remaining spatial-filter consumers |
| Execution | Typed native interpreter; typed registers/continuations; context, actor, number/confirmation, coin, ForEach, two-card retrieval, acting-piece binding, unit/token selection, MoveUnit, hex selection, movement and Fast Travel sequence instructions; ordinary combat/defeat/removal/game-over, after-action minion return, lane push/minion battle/displacement, confirmation/finalization/respawn/initiative handoff and AttackSequence orchestration; generated-frame restore; real trace parity | Full phase/combat/effect consumers and remaining card text; aggregate context arena; make/unmake and hashing |
| Stats/card model | Item/marker/active-effect stats, scoped immunity/topology, minion defense, masked card-stat recipes, Metamorphosis, Unlimited Firepower and conditional Big Sodding Gun stat auras; play/unplay/discard/resolve/swap/retrieve; physical alias export | Illusion-minion equivalence, ambiguous duplicate-print model equality, remaining phase-specific effect hooks |
| Card action rules | Typed menus/recipes, action/repeat restrictions, movement caps and Witching Hour; ordinary secondary actions; 38 reviewed primary effects; passive scan/offer/use, Big Sodding Gun and ordinary/granted Wild Hunt; unregistered-effect fallback; repeated unit/token selection and Clear; native placement/swap/recording, effect-aware movement sequences and bounded repeats | Remaining primary/passive effects, general active-effect execution consumers, spell faces and phase-specific effects |
| Search/integration | No live integration or advertised search capability | Evaluation, iterative team-aware search, hidden/simultaneous planning, parallelism, optional lab adapter |

The JSON state envelope is an outer boundary format, **not** the eventual native
simulation representation. Importing a saved continuation losslessly does not
claim its steps can execute. Native transition and search capabilities remain
false. The separate `stack_trace` diagnostic operation executes only the declared
native rule closure. Unsupported phases/effects/instructions reject the
whole trace; it does not call Python to finish individual branches.

The geometry primitive reproduces the **bare-board** Python movement contract,
including virtual off-map terrain. Stateful topology has different off-map
behavior. Corpus generation compares Python's optimized stateful movement with
its own original stateful BFS separately, and verifies Rust against both sets of
verdicts. The stateful traversal never crosses a missing map space, even with
pass-through permission. It never substitutes one contract for the other. Final occupancy permission and permission to pass through obstacles
remain distinct flags. Query results are not cached across mutations.

## Recorded validation, 2026-10-24

- 3,451 Python engine/domain/legality/oracle-tool tests passed; NN/training tests excluded.
- 112 native unit/integration tests passed in WSL, including all frozen corpora;
  strict Clippy and locked release compilation pass.
- 68 selected Python retrieval/Sabina/oracle tests passed
  after the latest addition; NN tests remain excluded.
- Native regression corpus: 141,876 bare-board plus 141,876 stateful destination
  verdicts across six map files and three generated boards; three complete
  semantic-state/alias round trips.
- 2,152 native/Python transition traces compare 8,817 operations, including invalid
  choices, strict stale-request handling, actor overrides, Razzle piece ownership,
  optional skips, mandatory aborts, paused template copies, occupancy changes,
  forced movement, terrain crossings, straight-line checks, ordered destination
  choices, malformed hex inputs, movement macro expansion and two-card retrieval
  (both choices, invalid inputs, restored requests and physical aliases), plus
  unit/token targeting, implicit immunity/self filters, lane-specific heavy
  support, protected tokens, prompt-owner overrides and piece choice followed
  by movement, plus full Fast Travel macro/relocation traces on all maps and
  generated edge states. Piece-addressed retrieval and prompt ownership use the same
  native owner lookup, including missing-owner handling. Only typed
  ephemeral request IDs and generated step labels are normalized; semantic IDs
  and state fields remain. Python frame labels use retained object identities,
  avoiding accidental merges when Python's raw step labels collide.
- 16,482 scalar/card-stat queries and 37 Hero model operations match Python,
  including Razzle's shared markers, unplaced pieces, minion defense, sparse
  initiative overrides, spell rejection, turn-slot gaps and partial model errors.
- 12 planning/revelation traces compare 77 operations against real Python phase
  functions: final-commit handoff, ordered initiative ties, respawn queues, item
  and override initiative, pass slots, request cleanup, public-reveal deduplication,
  restored buffers and Emmitt second-card takebacks/retrieval queues. They stop
  before draining the resolution stack and do not claim session/action parity.
- 11 isolated tie-step traces compare 19 operations against the real Python
  step: team-priority selection, invalid retries, exact prompts, actor/owner
  changes, off-board/multi-piece respawn queues and delayed coin flips. The
  generated card actions are queued, not executed by these diagnostics.
- 30 affected Python planning/Emmitt/tie/oracle tests passed after this addition.
- 38 isolated card-action cases compare 1,828 menu/selection operations over
  all 33 registered heroes' ordinary decks, hidden cards, item bonuses, blocked
  Fast Travel and off-board heroes. Checks include exact request metadata,
  context updates and every generated child record in execution order. Python
  verifies that ResolveCardStep changes no other persistent state; native rule
  generation takes an immutable world reference. Children are not executed by
  this corpus, so it does not prove card-effect/combat/session parity.
- 176 ordered safe-zone queries and 60 direct zone lookups match Python across
  all six map files and seven generated cases, including tile metadata and
  overlapping fallback zones. Lookup uses a concrete tile's recorded zone first;
  without a tile it returns the first matching zone's own ID. Cloned workers
  recompute safety from their own locations.
- 22 affected Python action/Fast Travel/piece/oracle tests passed.
- 37 affected Python immunity/filter/piece/oracle tests passed after unit
  targeting was added. Native workers keep acting-piece bindings private and
  recompute heavy immunity from each worker's current support positions.
- 31 affected Python Fast Travel/filter/piece/oracle tests passed. Fast Travel
  workers relocate independently; a full registered game can export a compact
  native envelope and restore its paused request with physical card alias IDs
  intact. `NativeSession::export_envelope` is a boundary operation, not a
  per-node simulation path.
- Native unit coverage includes invalid references/content/rules versions,
  unsupported execution closure, mutable occupancy, boards above 512 tiles,
  worker isolation, bounded step execution, deadline/cancellation and restored
  request identity. Immutable programs/source records are shared across workers;
  registers and continuation frames are private. JSON and context insertion order
  are preserved through import/export and register updates.

The initial protocol verification took about 0.49 seconds including WSL process
startup, JSON transport and comparisons. That is a correctness smoke measurement,
not a full-simulation speed claim. The two-turn throughput and playing-strength
gates in the plan remain unmeasured and incomplete.
Frozen instruction/primary catalogs are source-validated and parsed once per
worker process; mutable session programs clone only their required records.
A 12-pair check of one root's import plus protocol round trip measured medians
of 198.66 ms before caching and 113.41 ms after. It performed no game advancement.


## Next rule-port work

The interpreter runs ordinary action handoffs through confirmation, finalization,
minion return/lane checks, initiative ties, respawn and the next planning boundary.
Planning and execution share actor/phase state and actor selection logic.
Round cleanup, ordered upgrades and typed planning commands are implemented.
Continue with active effects,
remaining targeting filters and the passive/defense effect registry. Two-card
retrieval remains a typed interpreter instruction. Action rules return typed
recipes preserving passive and basic-action hooks. The interpreter drains ordinary secondary actions through
piece choice, passive checks, movement, attack, Fast Travel, Clear and Hold.
Small typed bindings share immutable child programs and preserve generated
continuations across export/restore; request encoding is shared with the isolated
diagnostics. Remaining registered primary bodies and eligible unported passives
reject explicitly. Spell faces,
active-effect restrictions and unported action/stat aura hooks remain explicit unsupported
closures. Fast Travel now executes its native destination scan, optional prompt and
relocation sequence, including zone-change-only travel and legality rechecks.
Its relocation emits UNIT_PLACED and does not traverse a path or trigger mines.
Action/topology effects and unported aura hooks remain unsupported for this closure.
Primary card text now executes Noble Blade, Arcane Whirlpool, Liquid Leap,
Magical Current, Stranger Tide, Onslaught, Hit and Run, Leaping Strike, Poisoned
Dagger and Poisoned Dart, plus Rogue Wave, Tidal Blast, Blink Strike, Mad Dash,
Brutal Jab, Mighty Punch and Savage Kick. Reviewed pure builders are frozen through the public
Python effect API with explicit hero/card/stat operands; Rust compiles these
once and binds typed values during execution. Unregistered effects follow
Python's primitive fallback, while unported registered bodies fail explicitly.
Placement, swapping and hex recording preserve occupancy, location order and
unit/token events. Spawn exclusions use real tile spawn metadata on all maps.
The 182 new traces include zero/negative ranges, item stats, optional moves,
full action wrappers and instruction boundaries. Restoration checks preserve
stats computed at effect creation, even after later marker/state changes.
State-dependent builders need explicit native logic; probing one state and
freezing its result is not a valid port.
PushUnit and ComputeHex now execute natively. Pushes preserve early no-op paths,
collision/trimming behavior, zero-distance events, victim context and AFTER_PUSH
ordering; token pushes use their separate event and skip that passive hook.
Mine-triggered and effect-dependent pushes remain unsupported. Unit predicates
support nested OR and empty-behind-space checks; adjacency tags are presence
checks, so immune neighboring units still count. Card-ranged recipe operands
are explicitly detected, compiled and frozen into exported continuations.


Ordinary ResolveCombat, DefeatUnit, RemoveUnit and TriggerGameOver instructions
now execute natively. The 70 combat traces cover blocking and defense flags,
minion support, hero/minion rewards, assists, markers, multi-piece defeats,
turn-slot completion, annihilation and registered physical-card aliases. Shared
life/unresolved/defeated state prevents planning from retaining stale defeat
results. Generated defeat/removal/game-over frames survive compact export and
restore; worker copies retain private rewards, counters, markers and occupancy.
Defense windows now offer exact native card/pass prompts, including Initiative
blocks, primary-defense restrictions, item/marker values and piece ownership.
The 223 additional Python traces cover all 33 registered heroes' starting hands,
card choices, invalid/stale choices and discard orchestration. Native DiscardCard
updates the shared public reveal knowledge, turn discard log and context, then
queues the real AFTER_CARD_DISCARD hook before BEFORE_DEFENSE. Generated bindings
restore between those steps; the original action type is restored after combat.
Passive trigger/configuration metadata comes from the pinned Python registry;
eligible unported passives and remaining primary defense/on-block text reject explicitly.
AttackSequence now expands to typed target selection, reaction, temporary
actor binding, before-action/pre-action hooks, defense text, combat, on-block
handling and action restoration. Optional attacks delay that expansion until
selection succeeds; preselected and missing targets preserve Python's behavior.
Effective attack/range stats and bonuses are computed at execution time, including
bound pieces. The basic-attack flag uses the performed card, with owner-specific
and global lookup order preserved. CheckTargetDefeated consumes per-attack log
marks and can restrict matching defeats to the current actor's owning hero.
The 49 new attack traces exercise these paths, including target menus on all maps.
Native defense bodies now execute Aspiring Duelist, Stop Projectiles, Perfect
Getaway, Close Call and Narrow Escape through their real actor-wrapped child
steps. Reflect/Deflect Projectiles also execute their ranged-defense flags and
optional on-block enemy selection followed by forced discard. Runtime range,
attacker exclusion and temporary defender ownership survive exported continuations. The frozen registry
identifies inherited empty on-block hooks, so these are distinguished from
unported custom bodies. Shared PlaceMarker/RemoveMarker instructions preserve
singleton registry entries, events, AFTER_PLACE_MARKER ordering, piece ownership
and copied-action substitutions. Marker changes immediately affect later stats.
Ninety-seven real-handler traces include atomic instruction snapshots and upgrades
explicitly made faceup before their effects are tested. Worker and export/restore
checks cover marker mutations and temporary defense actors.
A further 112 real-handler traces cover card selection across all 33 heroes,
physical container/alias order, visibility/property filters, selection metadata,
forced-discard ownership and full Reflect/Deflect Projectiles continuations.
Generic CARD predicate closures and non-string context identity coercions remain
unsupported. Forced discards support creator-owned discard shields, scoped
immunity and premature card-linked effect expiry as described below.
Remaining passive/defense bodies, protection/expiry/bounty active effects and
full card-action/phase integration remain unported. Remaining active-effect
combat roots reject explicitly; this is not full combat or game simulation support.

The isolated planning diagnostic still stops at queued continuations and rejects
an all-pass turn-end transition. Executing sessions advance ordinary turns,
auto-pass empty hands, execute end-phase work and complete cleanup/upgrades into
the next planning round. Movement macros use runtime item, marker and supported
card-aura stats, preserving actor/piece ownership and effective ranges through
prompts, filters and restored continuations. Metamorphosis freezes obstacle
passage when a movement action starts; ordinary effect-side MoveUnit steps do
not gain that permission. Active effects and remaining custom aura hooks are
still guarded in these action bodies. Automatic mine detection, choices and triggers are supported as described below;
blast discards support the bounded shield/immunity/expiry closure below.

Physical references stay independent in all state boundaries. Lifecycle model
operations currently reject ambiguous duplicate printed IDs before mutation,
until Python's value/private-field model equality is ported; they do not guess
which copy Python would remove. Standard registered decks have distinct printed
IDs. This is an explicit remaining coverage gate, not a full-model support claim. Replace scalar-only
`ForEach`/record handling with a mutable aggregate arena before advertising that
context closure. Current guards also reject non-ASCII integer coercion and
integers outside i64, rather than silently returning an incorrect legality
verdict. Unsupported closure must trigger whole-root fallback during integration.

Native UNIT and UNIT_OR_TOKEN selection now runs in the interpreter, with
ordered entity candidates, target-position checks and implicit self/immunity
filters. Compiled unit predicates include team, unit/token/minion type, identity
exclusion, range, captured IDs and hero-piece ownership. Ordinary heavy immunity
uses the target's lane Battle Zone and friendly minion roster; token protection
uses the owner's actual team. ChooseActingPiece and SetActingPiece update the
mutable world and exported continuation state. Effect-driven targeting,
remaining compositional predicates and non-string captured-ID coercion are still
unsupported. These are partial rule closures, not full combat support.

The completion/performance checklist in the plan is unchanged and remains open.

Oracle serialization is deliberately order-sensitive. Transport never sorts
semantic dictionaries; secondary-action ordering is part of immutable definition
identity. Corpus fingerprints preserve object order as well as array order.
Frozen continuation defaults are in `tests/fixtures/instruction-templates.json`;
regenerate with `scripts/rust_oracle/instruction_templates.py` only against the
unchanged reference. Native compilation checks their reference content identity.

Native trace tests load one indexed full-state case at a time. Unit tests
share the small case index and deserialize only the requested input envelope.
`stack-traces.json` is a schema-2 index; complete content-addressed cases
live in `tests/fixtures/stack-traces/`. Python verifies
each case SHA-256 and its indexed name/path, as well as the index fingerprint.
The full corpus is not embedded into Rust test binaries. This avoids multiplying
fixture memory across parallel tests without dropping any state/request/event
comparison. Neural-network and model-training tests are excluded.

`advance_prefix` is an offline stack-trace diagnostic operation. It stops after
a specified number of atomic instructions and retains their events, matching
Python's existing `process_stack(stop_before_step=...)` boundary. It does not
change the real execution budget or count partial traces as search simulations.
Only steps declaring `source_card_id` receive card linkage during export, matching
Python's effect-card binding API.


After-action minion returns execute natively with effect-aware movement/topology: each
minion uses its own lane's Battle Zone, favored-team/ID ordering, ordered shortest
walking destinations, and placement fallback when no walking path exists.
Impossible returns are marked processed so later minions still run. Generated
move/place/continuation frames preserve literal destinations, path length,
mandatory flags and processed IDs across export/restore. Immutable prompt data
is shared between cloned workers. Traversal uses the minion's obstacle and
topology perspective, including passable tokens; destination legality uses the
current actor for displacement restrictions (falling back to the minion for a
missing/empty actor). Movement caps use geometric distance for candidate
selection, matching Python, while the generated move retains shortest path
length. Placement fallback checks connectivity and placement prevention. Rejected
destinations do not stop the breadth-first search. Uncompiled card aura
restrictions remain explicit errors.
Thirty-six reference traces cover all six maps, detours, obstacles, disconnected
spaces, invalid/retried team choices, missing lanes/zones/teams, processed IDs,
cross-lane continuation after failure and instruction boundaries. Positive cases
assert actual relocation into the correct home zone. The 62 affected Python
rules/oracle tests pass; NN tests are excluded. General effect coverage and search are still required; ordinary full-round
progression and planning commands are covered below.


Minion battle and lane pushes are native, including cleanup-phase battle roots.
Ordinary minions are removed before heavy minions, with team choices when the
casualty is not forced; counts include the registered One Man Army ultimate.
Single-lane last-wave victories and multi-lane simultaneous throne/final-wave
comparisons follow distinct reference rules. Multi-lane targets are computed
before mutation; tie remedies retain the old Battle Zones and flipped counters.
Direct spawns run before blocked spawn assignment so no minion is double-booked.
Removable tokens clear without inventing placement/removal events. Blocked spawn
choices use the favored team, origin hexes (unit IDs for duplicate origins),
lane-specific nearest placements and exact request context/pending-input state.

Seventy-six real-handler cases add 232 operations, covering all maps, explicit
and missing targets, wave counters, simultaneous victories/ties, spawn supply,
terrain/tokens/blocked points, heavy eligibility, malformed choices and cleanup
battles, including empty-zone sentinel behavior. Positive fixture assertions check intended victories, tie remedies,
actual spawned supply and One Man Army casualties. Restoration covers every
instruction and prompt; cloned workers share immutable programs/map data while
keeping wave counters, zones, occupancy and outcomes private. Active-effect battle
exclusions, token-linked cleanup during pushes and topology-aware spawn displacement
are now implemented (see the effect-aware battle update below). Ordinary round
orchestration is covered below; remaining effect closures and search remain unfinished.


Action handoff now executes ConfirmResolution, FinalizeHeroTurn, FindNextActor,
ResolveTieBreaker, RespawnHero and AdvanceTurn. Finalization resolves the correct
physical card/turn slot, resets played/ultimate passive uses, clears transient
actor/piece state and applies the revealed-guess lifetime rule. Turn advancement
records complete ordered position snapshots, clears discard history only at the
correct boundary and handles automatic empty-hand passes. Confirmation respects
hidden-event/foreign-input rollback flags. Mandatory failures preserve pending
combat or confirmation/finalization boundaries instead of discarding progression.

Fifty-three real-handler traces compare 368 operations, including complete
four-hero handoffs to planning on every map, actual movement/snapshot changes,
initiative items/masking/ties, invalid confirmation/respawn inputs, token-clearing
and fallback respawns, hero-piece ownership, guess lifetimes, passive counters,
turn slots and explicit end-phase queue snapshots. Round cleanup is implemented
below. Ordinary resolved cards no longer block stats by default:
the frozen registry records actual stat/movement/action-prevention hook overrides
(currently four effects), and lifecycle changes refresh that native flag. The
registry-method identity has a Python authenticity test. Active effects, remaining
card/spell bodies, search and its performance gates remain
unfinished. No complete-game/search capability is advertised.


Round cleanup now retrieves physical card aliases, resets markers and card flags,
removes temporary pooled tokens, purchases all affordable levels, issues pity
coins only when appropriate and records item reveal snapshots. Ordered upgrade
options preserve lowest-tier/color rules; single or complete batches are checked
against a private preview before accepting input. Applying upgrades retires the
previous card, puts the selected face in hand and gives the opposite face's item.
Both frozen ultimate-unlock hooks are implemented: Ursafar's enraged flags and
Takahide's silver/gold deck changes with their exact events. Round reset retains
the reference distinction between direct cleanup and the post-upgrade path.

Typed commit/uncommit/pass/finish commands drive the same native planning buffers
and continuation programs as imported stacks. Full hold-based rounds on all six
maps and two consecutive rounds match the Python phase API. Twelve game traces
add 1,077 operations; 138 cleanup/upgrade traces add 446 operations, including
both upgrade choices for every registered hero, invalid batches and unlock hooks.
Restoration is checked before and after upgrade input and at every instruction;
cloned workers keep cards, items, levels, pending choices and snapshots private.
These scenarios establish the supported ordinary round closure, not arbitrary
card-effect coverage or search throughput. Active effects, unported hooks,
remaining cards/spells, make/unmake, hashing, evaluation and search remain open.

Fixture export uses atomic case replacement, including disk-full retries. Merging
verified immutable cases uses same-volume hard links when available, falling back
to a copy. It removes orphaned temporary links before retrying so source snapshots
cannot be truncated. No semantic state is pruned from the comparisons.

The full native test suite matches 1,959 cases and 8,449 operations. The added
35 conditional-aura/primary/adjacency cases (77 operations) also match through
the separate WSL release-worker protocol, as do all 16,482 model queries and
37 lifecycle operations. Canonical trace fingerprint:
`3bf6d7714895eb4f51ca1e5399ebcb70ba44f886ed8c96c5fd5b79ee0b0e82a8`.


Native card-aura support now includes Metamorphosis and Unlimited Firepower.
Both follow the real active-card selection rules: level-eight face-up ultimates
and resolved face-up played cards, preserving repeated physical aliases.
Metamorphosis counts distinct adjacent enemies across a multi-piece owner's
positions for initiative, while movement uses the bound piece when present.
Trinkets gains range/radius only where the original stat pipeline applies them;
melee card range remains fixed. No immunity filter is invented for the count.

Movement actions freeze Metamorphosis's obstacle permission in typed child-frame
bindings shared with movement-path and straight-line filters. Export writes the
actual frozen flags into generated steps, so restoration and later aura/range
changes cannot silently change an action already started. Worker clones share
program/definition data while keeping locations and aura counts independent.
Ordinary effect movement does not inherit movement-action permission.

Thirty-one added stat cases cover both auras, all maps, activation/face/lifecycle
rules, item/marker stacking, missing positions, tokens, duplicate aliases and
multi-piece aggregation. Fourteen real-handler traces add 49 operations and
verify actual obstacle crossing, failed ordinary moves, ranged kills and changed
initiative handoff. Registered hook specifications have an authenticity test.
Big Sodding Gun's stat hook and after-push passive are native; Witching Hour remains guarded.


Conditional stats now resolve the performed card from live typed registers, using
owner-specific lookup, global lookup and turn-card fallback in reference order.
An explicit card-stat request overrides that implicit context. Menus, attack
expansion, fallback primary logic, reaction values, movement stats and basic-attack
metadata use the same resolver. No mutable performing-card cache is introduced.
Big Sodding Gun's +2 Attack/+2 Range applies only to a basic card whose declared
primary action is Attack, including the reference's facedown preview behavior.

Point Blank Shot is the eighteenth reviewed native primary recipe. Its post-attack
adjacency check uses direct piece positions and the plain-topology contract,
including literal/key precedence and falsy short-circuiting. Positive traces
cover kills at the aura-extended range and an adjacent blocked target being pushed.
Restoration and worker-isolation tests change the performed card between attacks
and distinguish explicit card stats from live implicit context. Its eligible
push passive now runs through the native provider/offer/use pipeline. Witching Hour,
active effects, non-string identity coercions and conditional spell-face stats remain unported.

### Mandatory discard or defeat

The native `force_discard_or_defeat` instruction expands to the existing typed
card-selection/discard or defeat programs. A victim with cards receives the
mandatory discard prompt; an empty hand queues defeat with literal, context,
or current-actor kill credit in that order. Missing, falsy, nonhero and off-board
victims are no-ops. Owner-level multi-piece presence and piece prompt ownership
follow the frozen engine, including its owner-level defeat/removal behavior.
Generated prompt and defeat bindings survive export/restore at every instruction.

Twenty new reference traces compare 37 complete operations, including three
instruction-prefix records. The full corpus now has 1,959 traces / 8,449 operations.
Truthy non-string victim/killer coercion and active-effect immunity remain explicit
unsupported closures. Big Sodding Gun now uses this primitive through its native after-push hook.


### Passive providers, offers and usage

Native scanning preserves played-card order and then the level-eight ultimate,
including repeated offers when the same physical card occupies both sources.
Face/lifecycle/usage checks read current card state; immutable config metadata and
prompts come from the frozen registry. Unsupported eligible providers reject
explicitly. Effect-backed Wild Hunt providers are supported as described below;
other eligible unported provider bodies remain guarded.

Offers re-resolve their card from the current owner's played/ultimate slots.
Optional offers preserve the exact confirmation prompt and card context; YES
executes the body, other responses decline. Face-down, removed and unknown-effect
cards are no-ops. A body that emits steps queues usage marking after those steps.
Marking follows the reference's current actor, including explicit-owner quirks,
and increments the actual physical card rather than a copied counter.

Big Sodding Gun's after-push body forces enemy heroes to discard or be defeated.
Friendly/minion/missing targets emit no body and consume no usage; an enemy who
has left the board still emits a no-op discard-or-defeat step and then marks usage,
as in Python. Normal, repeated, blocked and real Point Blank Shot pushes have
complete state/request/event parity. Runtime advancement uses typed frames and
registers without Python calls or JSON instruction construction. Cyclic
passive/discard programs compile once at session scope.

Thirty-five added reference traces compare 116 operations. Native tests restore
every instruction, branch private usage/context, recheck waiting optional offers,
and verify that unported providers remain guarded. The full corpus is now
1,959 traces / 8,449 operations at that milestone. Subsequent provider/Wild Hunt
coverage is described below. Remaining passive bodies and truthy non-string
victim coercion remain unported; global game/search
capabilities remain false.


### Native resume identity

`NativeSession::export_envelope` includes native-only transport history outside
`state`, so completed requests and generated frames keep their identities after
restore even when no pending frame remains:

```json
"native_resume": {"version": 1, "request_high_water": 17, "step_high_water": 42}
```

Keep this block when resuming a native session. Fresh Python oracle imports omit
it; their initial lower bound comes from live continuation identities, including
generated template prefixes. Native restore rejects a future resume version,
malformed counters, counters preceding live frame IDs and exhausted counters.
Materialized game state, physical aliases and reference trace fingerprints are
unaffected. The JSON-lines `roundtrip_state` boundary preserves the block.

Request strings must match the issued ID exactly. Padded or signed numeric
aliases cannot answer a canonical generated request; a literal imported ID is
still accepted when it is the exact pending ID. These IDs and counters belong
to each session lineage, with private copies in cloned search workers.


### Tactical minion skills and bounded repeats

The reviewed primary catalog now contains 29 effects / 99 top-level recipe steps.
Sabina gains Back to Back, Listen Up, Roger Roger, Ready and Waiting, Troop
Movement, Marching Orders, Path to Victory, Close Support, Covering Fire,
Shootout and Bullet Hell. Their real handlers swap minions, move them repeatedly,
force enemy discards/defeats and remove minions after a targeting-time adjacency
check. Removal retains its separate no-reward behavior. Other Sabina primaries
remain unported; this does not claim complete hero support.

Native repeat instructions preserve optional YES/NO/SKIP decisions, limits,
partially completed and nested repeats, copied template state and lane-push checks
before each repeat. Repeat validation preserves Python's scalar AND values.
The shared template-copy path also serves ForEach. Active repeat-prevention
is now native; aggregate context aliases remain explicitly unsupported.

Added 127 tactical primary traces / 1,001 operations, 27 repeat/validation traces /
97 operations and 11 ordinary Point Blank Shot traces / 64 operations. Positive
reference cases assert real swaps, repeated movement, discard logs and minion
removals. Native tests restore every instruction in the new primary/repeat
continuations, exercise a real boundary lane push, and keep nested counters
private between search workers.


### Entity queries and conditional card actions

Count and CollectUnits use the same ordered explicit/implicit unit predicates as
selection. They omit SelectStep's intrinsic targeting validation: an off-board
actor can still count otherwise matching units. Count skips immunity by default;
CollectUnits respects it by default. Hex counting uses ordered board candidates.
Other target kinds retain the frozen implementation's empty-candidate behavior.
The handler's central skip check preserves the previous output for a skipped
query, matching actual process_stack behavior.

PlayedCardFilter reads the current game turn's resolved slot plus the live current
card, through the physical card table. It checks action/color together on each
card and uses facedown masking. With neither criterion, a facedown card still
counts as a card, as in Python. Piece candidates resolve the owner's card state
but remain distinct candidates. Shared one-based turn-slot lookup avoids using a
hero's resolved-turn counter as the game turn.

Context conditions preserve True/None results and Python's numeric/non-numeric
rules. Float/integer comparisons remain exact at large i64 thresholds rather
than rounding thresholds to f64; 882 reference comparisons cover all operators,
unknown operators, booleans, nonnumeric values, fractions and boundary values.
Nonfinite numeric inputs remain explicitly unsupported.

Quickdraw, Gunslinger, Dead Shot, Shield Bash and Counterattack now execute as
reviewed primary recipes, bringing the catalog to 34 effects / 118 top-level
steps. Their conditional bonuses and discard/defeat behavior match hidden,
current, resolved, old-slot and piece-target cases, including preexisting bonus
context. Added 176 complete traces / 693 operations; ordinary primary coverage
now has 473 cases. Restore tests cover generated query continuations, and worker
tests change card visibility, game turn and occupancy without shared mutation.


## Ordinary retrieval and premature card expiry

RetrieveCardStep now returns selected played, discarded or current cards to the
hand, or retrieves the discard pile in its original order. Sparse played slots,
physical aliases, piece actors, current-card clearing, face/initiative resets,
unchanged discard history and exact retrieval event metadata match Python.
Missing or non-string card selections remain no-ops. Ordinary retrieval events
remain distinct from Alternative Timelines' kept-card metadata.

Retrieval and discard share premature-expiry handling. The reference manager
looks up the first card globally by printed ID after moving the card; the native
engine preserves that lookup order, including independent printed duplicates on
another hero. It clears the base active flag while retaining enraged overrides.
Active-effect expiry and repeated-physical bulk retrieval with partial-failure
semantics remain explicitly unsupported before mutation.

Steady Advance, Unwavering Resolve, Devoted Followers and Fresh Converts bring
the reviewed primary catalog to 38 effects / 136 top-level steps. Added 129
complete traces / 301 operations: 31 direct retrieval/discard cases, 54 targeted
support-skill cases and 44 ordinary primary cases. Native regressions cover
instruction-boundary restoration, private worker collections/active flags and
rejection before mutation. The ordinary primary matrix now has 517 cases.


Validation: the locked WSL release suite passed all 105 native tests, including
all 2,088 traces / 8,750 operations (CLI corpus comparison: 117.47 seconds).
The 68 relevant Python tests passed; strict Clippy and formatting checks passed.
This timing includes corpus I/O and comparison and is not search throughput.


## Active-effect lifecycle

`effects.rs` imports complete effect payloads as typed immutable definitions,
shared by worker clones. Activation flags and the ordered instance collection
remain private to each worker. World export preserves all payload fields,
including finishing instructions and passive grants; no JSON is used by native
lifecycle operations.

Implemented activation/deactivation by card and by effect ID, temporal activity
queries, collect/remove natural turn expiry, duration expiry, premature removal
by card/source/ID and stale cleanup. Card-bound source expiry follows the
physical card owner, including extra-turn cards; token bindings survive source
and duration sweeps. Natural expiry returns ordered finishing payloads;
after-cards-played triggers fizzle at generic expiry. Premature removal never
returns finishing payloads. Card flags follow effect existence, with the frozen
global printed-ID lookup and enraged overrides preserved.

The `effect_trace` diagnostic compares 41 Python EffectManager cases / 286
operations, including every effect category, all durations, dormant/cardless
activity, token/empty bindings, physical owner versus performer, repeated IDs,
shared card links, lookup collisions, restore and finishing order. Native tests
also check shared immutable definitions and private worker activation/removal.
These checks cover lifecycle, not behavior of each effect category. CreateEffectStep
orchestration and token-linked removal are implemented below. Other behavioral
consumers and scheduling/execution of finishing payloads still need ports; live
session guards and search capability flags stay in place.


Latest lifecycle validation: 107 native tests pass in the locked WSL release
suite, including all 2,088 transition traces / 8,750 operations (CLI comparison:
119.75 seconds), plus 41 lifecycle cases / 286 operations. The release worker
matches the new lifecycle corpus, and 59 relevant Python tests pass. Strict
Clippy, Rust fmt, Black, Ruff and whitespace checks pass. Existing services and
NN tests were untouched.


## Effect creation

EffectCreateSpec compiles a validated, fully expanded effect template at the
boundary. Native creation shares its immutable definition and allocates only
private instance state. Card/type/affects deduplication includes dormant rows and
is a pure no-op: no timestamps, payload, charges, counter or card flag refresh.
Cardless and empty-card effects are not deduplicated. Normalization anchors
implicit multi-piece origins to the acting piece unless the scope is global or
has a fixed hex; static barriers normalize their separate anchor independently.

Existing non-unresolved cards make their effects immediately active. Attack
origins snapshot basic-card identity from the actual performed card, including
nested owner/extra-card lookup and hidden cards. Entity IDs/counters and creation
timestamps survive export/restore and remain private to worker branches.
Counter exhaustion rejects before world mutation, while deduplication still
works with an exhausted counter.

Added 82 creation cases / 284 operations. The combined lifecycle/creation corpus
now contains 123 cases / 570 operations and matches the release worker exactly.
The locked WSL release suite passes all 109 tests, including 2,088 full transition
traces / 8,750 operations (CLI comparison 172.67 seconds). All 59 selected Python
manager/oracle tests, strict Clippy, Rust fmt, Black and Ruff checks pass.
CreateEffectStep bindings, behavioral consumers, finishing-payload scheduling,
full search and performance gates remain unfinished; no live services changed.


Latest verification after migrating the committed checkout to WSL: all 112
native tests pass, including 2,152 transition traces / 8,817 operations and the
123-case effect lifecycle/creation corpus. Strict Clippy and Rust fmt pass.
CreateEffectStep binds dynamic state and freezes finishing colors, with exact
creation events and restore/private-worker regressions. Gameplay/input consumers
of active effects remain guarded; search and the full engine are unfinished.


## Token-bound effect removal

RemoveTokenStep now removes effects linked through token_id, source_id or the
scope's origin_id when the token is on the board. It removes matching instances,
including inactive/expired rows, without removing unrelated rows with the same
effect ID. An off-board token is a no-op; its effects remain. The event reports
the actual number removed. Card flags and finishing payloads are untouched, as
in the frozen reference. A barrier-only anchor is not a removal link.

Creation/removal/context/log resolution sequences can be exported and restored
with active effects. Unported gameplay, phase and input consumers remain
guarded; the supported effect-aware closure is described below. This does not
enable general search or live integration.


Latest token-removal verification: all 114 native tests pass, including all
2,203 transition cases / 8,870 operations (full CLI comparison 103.98 seconds).
The 60 focused Python tests and strict Clippy/formatting checks pass. No search
throughput claim follows from these correctness timings.


## Active-effect stat consumers and topology

The native stat pipeline implements positive item doubling, scoped area stat
modifiers, basic-action bonuses and conditional modifiers evaluated after all
unconditional sources, in effect order. Shared rules implement temporal activity,
all scope shapes and relational filters, heavy/action/attack immunity, and reality
splits/isolation. Multi-piece initiative aggregates each effect once across
on-board pieces; non-initiative queries follow the acting piece. Positional
card-aura counts and minion-defense calculations use the same topology rules.

`effect_stat_traces.py` freezes 136 Python states / 29,103 scalar, card-stat and
minion-defense queries. Verify them with the Linux reference environment:

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/effect_stat_traces.py verify -- "$worker"
```

CheckAdjacencyStep now uses those native rules in actual resolution, with 17
full handler traces / 18 operations covering directionality, immunity and restore.
The original Python topology loop intentionally ignores topology duration/active
flags; action immunity checks its explicit flag, while attack immunity also checks
temporal activity. The port preserves these differences.

A radius-immunity/reality-split cycle can raise RecursionError in the unchanged
Python reference. `effect-errors.json` records this case; Rust rejects recursive
scope resolution explicitly rather than returning a fabricated result. Remaining
targeting/combat/phase effect consumers and finishing scheduling remain gated.
Illusion-minion defense equivalence is still explicitly unsupported. No full-game
or search capability is advertised by these correctness checks.


Latest verification: 119 native tests pass, including all 2,220 handler cases /
8,888 operations and 28,675 new stat queries. Full CLI comparison took 128.00
seconds; this is a boundary correctness workload, not native search throughput.
All 60 focused Python tests and strict lint/format checks pass.


## Displacement restrictions

`NativeWorld::can_displace` implements the reference's can_be_moved,
can_be_pushed, can_be_swapped and can_be_placed validators. It checks explicit
placement-prevention effects, timing, spatial/relational scope, immunity and
actor relationship flags, and returns the first blocking effect/source/reason.
A batch compiles the world once; native checks allocate no JSON or Python state.
These checks cover displacement restrictions, not walking paths or full action
availability. Effect-aware push paths and unported passive-effect consumers
remain separate gates.

PlaceUnitStep and SwapUnitsStep now call this validator during real resolution.
They preserve no-op/mandatory-abort behavior, validate both swap participants
before changing occupancy, and retain token/unit event semantics. The simple
SetActorStep can compose with these steps; origin positions are read live after
movement and after continuation restore. Empty actors and the swap's literal
system fallback follow the frozen reference.

`displacement-validation.json` contains 159 states / 85,701 complete validation
verdicts; `displacement_effect_traces.py` also supplies 318 full handler cases /
636 operations in the main transition corpus. Includes every effect type, all
scope/relationship pairs, flags/timing, token/piece/heavy/owner targets, immune
units, duplicate effect IDs, empty/unknown actors, both swap participants and
live/fixed/piece origins. Verify the batch diagnostic with:

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/displacement_effect_traces.py verify -- "$worker"
```

The diagnostic operation is `displacement_legality`. Full game/search capability
flags remain false. Other unported gameplay and finishing consumers remain guarded.


Latest displacement verification: all 124 native tests pass, including 2,538
handler cases / 9,524 operations, the 85,701 displacement verdicts and existing
stat/model/geometry/planning coverage. Full CLI comparison took 110.49 seconds;
this is correctness-boundary work, not search throughput. All 87 selected Python
oracle/rules tests and strict lint/format checks pass.


## Action restrictions, repeats and movement caps

Native action legality now evaluates active-effect restrictions (including
replacement-action prerequisites), explicit card-color exceptions, scopes,
immunity and actor relationship flags. Repeat prevention uses the same live
scope rules. Movement checks combine optional action permission with raw board
presence and the tightest applicable movement cap. Multi-piece owners retain
bound-piece identity and unbound any-piece permission, as in the reference.

Negative attack values are legal by default; there is no general zero clamp or
negative-attack prohibition. Witching Hour is a specific card restriction:
within its radius, enemy attacks below zero are blocked. Its unlocked/resolved
card discovery, radius/stat modifiers, topology and action-immunity exemptions
are native. Any general Python prohibition must be recorded as a reference bug,
not copied into Rust. None appeared in this slice's ordinary negative cases.

`action_rule_traces.py` freezes 198 states / 306,816 full legality verdicts and
31 menu cases / 1,169 complete menu/action-recipe operations. It also generates
175 real repeat-validation handler cases / 350 operations in the canonical
transition corpus. These cover current actor changes, scalar context chaining,
live source movement and per-instruction continuation restoration. Search workers
share immutable definitions while reading their private live effect origins.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/action_rule_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/action_rule_traces.py verify -- "$worker"
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/action_rule_traces.py export-transitions
```

The batched diagnostic is `action_legality`. Action-menu/recipe diagnostics now
support effects; actual general action execution with effects remains guarded
until its remaining targeting, passive and phase consumers are ported. Real
ValidateRepeatStep execution is enabled in the bounded effect-aware resolution
closure. General engine/search capability flags remain false.

The stat corpus now includes 136 states / 29,103 queries. New cases exposed and
fixed an origin-zone lookup error: an existing tile's explicit zone ID (including
null) takes precedence over fallback zone membership at both ends of a scope.


Latest action-rule verification: 128 native tests pass, including all 2,713
handler cases / 9,874 operations (full CLI comparison 104.10 seconds), plus
306,816 action/movement/repeat verdicts, 85,701 displacement verdicts and 29,103
effect-stat queries. All 87 focused Python oracle/rules tests and strict lint,
formatting and whitespace checks pass. These timings measure correctness-boundary
work, not search throughput. Full gameplay, search and its performance gate
remain unfinished.


## Effect-aware terrain and walking

`terrain.rs` supplies typed actor-specific obstacles, static barrier boundaries,
empty-hex denial, petrified occupied terrain and passable-token queries. It
preserves source/actor teams, active timing, immunity, live origins and the
reference's distinct barrier/empty-area scope contracts. Out-of-map tile lookups
are terrain, matching Python's virtual tile behavior. Supply-only token lookup
preserves the entity-table-first fallback and moved-token mine restrictions.

Stateful path/reachability batches perform one native traversal per query,
respecting both each traversed edge and visibility from the original starting
hex across reality splits/isolation. Crossing obstacles and accepting an
obstacle as the final space remain separate flags. Passable tokens can be
traversed without being valid landing spaces. Results do not survive mutation.

Actual MoveUnitStep now uses displacement prevention, action availability,
movement caps, effect-aware path checks and actor-specific straight/crossed
obstacles. Mandatory prevention aborts while optional prevention continues;
ordinary path failure remains a no-op. Movement events retain the requested
range. Active-effect roots can execute these bounded direct moves and restore
between instructions. Unported targeting/card selections and other effect
consumers remain separate gates.

`terrain-validation.json` freezes 196 source states / 42,728 queries (183,848
scalar verdicts including destination vectors). `terrain_traces.py` supplies
756 real movement traces / 1,512 operations, including successful moves,
mandatory/optional failures, action caps, split-crossing attempts, barriers,
straight paths and events. Native tests also verify worker-private moving
origins, a corridor through a passable token and per-instruction restoration.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/terrain_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/terrain_traces.py verify -- "$worker"
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/terrain_traces.py export-transitions
```

The worker diagnostic is `terrain`. Automatic movement over passable tokens,
including supply-only mines, is supported by the mine integration described below.
Eligible unported passive bodies and dynamically created blast-effect closures
remain gated. Standalone terrain/path queries do not execute discard payloads. General game/search capability flags remain false.


Latest terrain/movement verification: all 134 locked native workspace tests
pass, including 3,469 full handler traces / 11,386 operations (complete CLI
comparison 112.36 seconds), the 42,728 new terrain/path queries and all prior
rule/model/stat/planning corpora. All 166 focused Python oracle/rules tests,
strict Clippy, formatting/lint and patch whitespace checks pass. No full-game
or search-throughput claim follows from these boundary correctness timings.


## Effect-aware movement sequences and controlled prompts

MoveSequenceStep now freezes the effective movement range and obstacle-crossing
permission at expansion, combining computed stats, scoped movement caps, card
auras and movement-aura zones. Generated destination selection and movement
retain those values across export/restore. MoveUnit still rechecks displacement
and action/cap permission when it executes. Preselected destinations, full-distance
and straight movement, optional skips and invalid/retried choices match Python.

Compiled hex filters now use topology for range, adjacency and straight-line
checks, actor-relative obstacles, moved-unit-aware token traversal and live
source positions. Explicit geometric ranges remain physical spacing checks;
not-in-a-straight-line negates the complete topology-aware condition. Spawn
adjacency includes the Tide of Darkness battle-zone exception, whose source rule
uses the unlocked ultimate's static effect ID even when facedown. Pure filter
scans do not require mine-trigger execution. Movement sequences use the automatic
mine handling described below, with effect-aware blast discard chains still gated.

The common request boundary supports CONTROL_NEXT_ACTION. Only a matching actor,
resolution owner, original turn card and explicit active control effect redirect
the recipient. Source order and the source's direct active-flag check are
preserved. Choices/legality and decision metadata retain the original actor;
`controlled_hero_id` identifies that actor, while the request owner is the
controller. Restore reconstructs the same request identity and ownership.
Bounded effect-aware roots support HEX/NUMBER selections and confirmation
prompts; unit/card and other unported consumers remain separately gated.
Fast Travel filters with effects reject the whole root before execution.

`hex-filters.json` contains 201 states / 19,497 scans / 1,208,717 verdicts.
`movement_sequence_traces.py` contributes 1,045 complete handler cases / 5,347
operations, including 64 controlled-prompt cases / 352 operations. Export asserts
real movement and optional skip behavior. Native tests cover instruction restore,
aura permission frozen before its source moves, private controllers and original
actor metadata, and early unported-filter rejection.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/movement_sequence_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/movement_sequence_traces.py verify -- "$worker"
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/movement_sequence_traces.py export-transitions
```

The filter diagnostic is `hex_filters`. General game/search capabilities remain
false. Eligible effect-backed passive bodies, remaining gameplay/finishing consumers, card/spell
rules, aggregate aliases and the full iterative search/performance work remain
unfinished.


Latest sequence/control verification: all 142 locked native workspace tests
pass, including 4,514 complete transition cases / 16,733 operations (full CLI
comparison 161.39 seconds), 1,208,717 new filter verdicts and all prior corpora.
All 290 focused Python oracle/rules/effect tests and strict lint, formatting and
whitespace checks pass. These are correctness-boundary timings, not search
throughput; full engine/search requirements remain unfinished.


## Mine paths, standalone choices and explicit triggers

The native mine BFS uses parent links and shared immutable mine sets, avoiding
path copies on every edge. It preserves Python's destination, option and path
order, actor obstacles, per-edge topology, friendly ownership and passable-token
supply fallback. Mine identities have stable entity/supply ranks independent of
symbol interning. Results are local to the current world; none survive mutation.
The worker diagnostic is `mine_paths`.

Standalone MinePathChoiceStep supports automatic single-path selection,
multiple-path prompts, invalid/retried choices, custom context keys and restore.
TriggerMineStep preserves explicit queue order, token/effect removal, events,
rollback flags and blast-forced discard chains. Duplicate or off-board blast
identities retain the reference's additional discard behavior. Known dud-only
queues do not compile unreachable blast consumers. Blast queues support ordinary
active effects, forced-discard immunity, shield choices and premature linked-card
expiry. Eligible unported passive bodies and dynamically created effect closures
remain explicit gates. Automatic MoveUnit detection and choice/requeue
integration are described in the following section.

Three reference defects are reproducible in `mine-reference-errors.json`: implicit
multi-mine set iteration changes queue order across Python hash seeds, and the
generic selection helper converts numeric option IDs to integers that the mine
choice step rejects. A literal destination is also lost when MoveUnit generates a
mine choice, allowing movement without triggering a mine. This source defect is
recorded and preserved as an explicit compatibility case; Python is unchanged.
Tests submit the actual string IDs. Choice-only fixtures
normalize declared mathematical mine sets and mine-location sets; explicit
trigger arrays, events, option order and paths are never sorted.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/mine_traces.py verify -- "$worker"
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/mine_traces.py verify-errors
```


Latest mine verification: all 150 locked release workspace tests pass, including
4,631 complete transition cases / 17,261 operations. Canonical corpus SHA-256:
6fa1afe326791dadc91163e24909172ef7986129c01abd505e5f9db5471211be.
The full CLI comparison took 154.63 seconds; this measures correctness-boundary
work, not search throughput. All 307 focused Python oracle/rules/effect tests,
strict Clippy, Python lint and formatting checks pass. Automatic movement/mine
integration and the full engine/search requirements remain unfinished.


## Automatic mine handling during movement

MoveUnit now detects enemy mine supply, evaluates straight-line crossings or
mine-aware path alternatives, queues a choice when required, then repeats the
validated move with its step-scoped `mine_path_prechosen` flag. The flag stays on
that copy and cannot suppress mine detection on a later move. Generated copies
have independent runtime identities; imported fixture labels remain explicit.

Movement writes the compatibility context keys and queues an actual TriggerMine
with literal victim/mine snapshots. Those bindings and frozen sequence ranges
survive instruction-level export/restore. Compiler linking includes deployed
potential blast IDs, explicit prechosen queues and possible token placements;
it avoids unused off-board roster blast recipes for ordinary dud movement.
Runtime checks remain before relocation if a new blast requires an uncompiled
consumer. Source token-pool ownership/passability detection is kept separate
from entity-first path traversal. Straight traversal retains source line order;
flexible path sets use stable native identity order. Implicit multi-mine order
is a recorded Python hash-order defect; it is not treated as an ordered oracle.

`mine_movement_traces.py` adds 66 real-handler cases / 396 operations covering
forced and sequence movement, retries/numeric rejection, friendly/unowned and
supply-only tokens, pieces, controlled prompts, explicit prechosen queues,
straight multi-mine order, a second movement in the same turn and the literal
destination source defect. Implicit multi-mine event ordering is not normalized
away: these movement cases use at most one detected enemy mine per flexible
path, while multi-mine straight/explicit trigger cases compare exact event order.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/mine_movement_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/mine_movement_traces.py verify -- "$worker"
```

Eligible effect-backed passive bodies and dynamically created blast-effect
closures remain mine gates.
The full engine/search scope and throughput requirements remain unfinished.


Automatic movement verification: all 153 locked release workspace tests pass,
including 4,697 full transition cases / 17,657 operations. Canonical SHA-256:
afb2455e6068ae5a945acbda59570931cba40ff6031dc4eb649f10f486b5bac3.
Full CLI comparison took 151.26 seconds; this is correctness-boundary work,
not search throughput. All 307 focused Python oracle/rules/effect tests pass,
including reproduction of the three mine reference defects. Strict Rust/Python
lint, formatting and whitespace checks pass. No NN tests, frozen Python source
changes, live restarts, commits or pushes. Full engine/search requirements and
effect-aware blast discard closure remain unfinished.


## Effect-aware forced discards and shield choices

Forced discards now check scoped actor immunity before examining the hand. An
eligible hand remains required: a shield alone does not force a discard. Active
DISCARD_SHIELD rows are matched by their creator and printed source-card ID,
using the direct active flag. Played/current cards retain container order and
are deduplicated by printed ID; shield discovery does not substitute temporal
scope or geometry checks. Ordinary CARD selection can restrict played choices
to these shields, including a current-turn shield.

When a basic/nonbasic filter is present, the generated selection snapshots the
eligible hand IDs plus all shield IDs and drops the basic predicate for shield
choices. That allowed list, container mode and owner survive export/restore.
Unfiltered prompts continue to read the live hand. Discarding a shield can find
it in played/current even when the nominal source is HAND. Public discard
history/context is recorded and card-linked effects expire prematurely, with
no finishing steps; physical card flags and worker-private aliases stay correct.

Blast-mine chains use this same closure with existing active effects. Compiler
linking avoids unreachable passive offer/combat bodies when a trigger has no
eligible offers. Provider scans use source creator/active/card/trigger/usage
and seen-card rules; irrelevant grants no longer blanket-reject a discard.
Eligible unported ordinary/effect-backed passive bodies still reject explicitly,
with compiler preflight before a supported initial discard chain mutates. Blast
roots that can create effects dynamically remain gated; full active-effect
reaction windows and remaining combat/phase consumers are not yet advertised.

`discard_effect_traces.py` contributes 103 real handler cases / 680 operations:
hand versus shield, filtered discards, current/facedown/duplicate shields,
inactive/old-active/off-scope creators, multiple shields, empty hand, immunity,
linked expiry, irrelevant passive grants and movement/direct blast chains.
All full state/request/event records match the release worker. Native tests add
instruction restoration, selected-shield restoration, allowed-hand snapshots,
independent shield/hand branches and explicit eligible-provider rejection.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/discard_effect_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/discard_effect_traces.py verify -- "$worker"
```


Effect-aware discard verification: all 158 locked release workspace tests pass,
including 4,800 full transition cases / 18,337 operations. Canonical SHA-256:
5983b21fe85936de3500fa767da69b7c1b4cde880f6f35e98b2a64449025b656.
Full CLI comparison took 180.31 seconds; this is correctness-boundary execution,
not search throughput. All 353 focused Python oracle/rules/effect tests pass,
including the real Mrak shield tests. Strict Rust/Python lint, formatting and
patch whitespace checks pass. Frozen Python sources, their pin and live services
are unchanged. No NN tests, commits or pushes. The full engine/search objective,
eligible unported passive bodies and dynamic blast-effect closure remain open.


## Ordinary and granted Wild Hunt passives

Passive offers now retain an optional active-effect provider ID. Provider scans
preserve creator, direct active flag, nonempty grant/card, global card lookup,
usage and played/ultimate seen-card rules. Execution re-resolves all provider
conditions before each offer attempt; a removed, inactive or changed provider
cannot execute an accepted stale prompt. Copied passives use their creator's
hero and the actual foreign carrier card's identity/name, even if that card is
facedown. Provider usage is typed mutable effect state, independent of the
physical card counter, and survives export/restore. Explicit marking matches
provider ID/card even for inactive rows or missing physical carrier cards, as
in Python. A provider removed during its body is not marked afterward.

Body programs are compiled once and shared across workers, keyed by effect and
owner. Only reachable trigger families are linked, avoiding unnecessary combat
or movement programs in pure usage-marker roots. Wild Hunt's recipe comes from
the frozen registry: select an exact two-space straight-line destination, then
perform an effect-side MoveUnit with a destination condition. It does not become
a MOVEMENT action or inherit movement-action bonuses. Decline, optional skip,
invalid/retried hexes, movement caps, on-board mines and continuation ownership
match the reference. Ordinary Wild Hunt uses its physical card counter; an
effect-granted copy uses its provider counter. Other eligible passive bodies
remain explicit gates. Effect-aware Big Sodding Gun is covered by the later
combat milestone below.

`passive_provider_traces.py` freezes 202 real handler cases / 1,267 operations:
ordinary/direct/scanned grants, foreign/facedown carriers, missing/changed grants,
active/scope/usage flags, seen-card deduplication, no actor/off-board owners,
optional decisions, exact moves, caps, mines, provider removal during the body,
and direct usage markers. Native tests add instruction restoration, live offer
invalidation, private counters/shared immutable programs and restored usage.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/passive_provider_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/passive_provider_traces.py verify -- "$worker"
```

The full game/search capability flags remain false. Remaining passive/card/spell
bodies, complete active-effect phase/combat/finishing consumers and the full
search/integration/performance requirements are still open.


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


## Effect-aware combat, minion protection and defeat bounties

ResolveCombat keeps signed attacks, including negative values, and uses the
existing defense comparison. Defeat now expires effects by the actual physical
card owner (or creator for unbound effects), preserving token-bound rows, without
executing premature finishing steps. Minion protection runs before defeat logs,
markers, source expiry or rewards. Mandatory totem sacrifice takes precedence:
it removes the token and linked effects and prevents the defeat entirely.
Card-discard protection offers the protector's current matching hand; accepting
records defeat/kill coins while retaining the minion. Invalid/declined/stale
choices try the next provider. Its generated discard follows ordinary linked
expiry and passive handling. The reference deliberately does not pay bounty
coins on an accepted card save; an exhausted protection chain does pay them.

Minion-defeat bounties use subject_id or creator as beneficiary, their live
position and topology-aware radius, and enemy-team eligibility. Each payout
updates typed worker-private max_value state and emits its ordered event.
Recreating a deduplicated card-bound effect does not refill spent coins.
Protection continuations retain literal victim/killer/tried IDs across restore;
retries use fresh default checker frames. Active-effect empty-stack game-over
states can restore. Big Sodding Gun's granted discard-or-defeat body is now
linked, including carrier expiry during a victim defeat and provider marking.
ForceDiscardOrDefeat remains hand-only; ordinary ForceDiscard shield choices do
not silently expand its contract.

`combat_effect_traces.py` adds 132 real-handler cases / 542 operations, including
all minion types, sacrifice priority/invalid anchors, multiple/tried protectors,
raw and imported pending inputs, missing/expired/scope/type/color conditions,
source versus foreign-card expiry, token-bound effects, negative attacks,
immunity, topology, bounty exhaustion, dynamic effect creation/deduplication,
terminal states and granted Gun. Full native state/request/ordered-event records
match Python. Subset SHA-256:
71639d8299f0ef59aea9f77f805a88337b772d3dc22df24102bf3d36db0ef7bc.
Native tests restore instruction boundaries, recheck a removed waiting provider,
and isolate bounty counters between workers. Explicit Python regression tests
assert negative attacks compare without a general zero clamp or prohibition.
No negative-attack reference defect was found in these cases.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/combat_effect_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/combat_effect_traces.py verify -- "$worker"
```

The later reaction milestone below adds illusion-minion defense. Full
active-effect AttackSequence/reaction/phase/finishing behavior, other card and
passive/spell bodies, aggregate context aliases, make/unmake, hashing, evaluator,
iterative team-aware search, hidden planning, parallelism, validated integration,
strength and 1,000 complete two-game-turn simulations/sec remain required.
These correctness checks do not establish search throughput or full-game support.


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


## Illusion defense and effect-aware reaction windows

Minion defense now takes the live actor explicitly. Illusion equivalence uses
only a provider's direct active flag and its source's canonical hero identity,
matching copied abilities and piece actors. It deliberately ignores temporal
expiry, area and subject fields. All on-board Illusion tokens count as that
team's melee minions in the connected first ring, regardless of token owner;
second-ring Illusions do not contribute. Ordinary melee/heavy/ranged minion
rules and topology remain unchanged. Combat, reaction menus and offline stat
diagnostics all pass the same actor rather than caching an equivalence team.

Reaction windows append eligible active played/current shield cards after the
hand, preserve order/deduplication, compute stats for the actual offered physical
card and use the existing hand/played/current discard path. A shield whose
provider expires while input is pending is rejected and the window reopens.
Initiative reactions use Initiative bases/bonuses. Discard expiry, ordinary and
granted passive checks, defense flags, minion-modifier caching, actor restoration,
marker side effects, and the supported primary-defense bodies remain native.

Defense-body dependency linking no longer compiles projectile targeting for
every defense-text instruction. On-block links are added only when a projectile
card can reach the body: bounded active-effect roots inspect hand, discard,
played and current containers; ordinary phase roots include future deck upgrades.
Discarded cards remain candidates when restoring an on-block continuation.
The subsequent targeting milestone below enables these projectile bodies.
Missing body links remain explicit errors rather than silent empty effects.

`reaction_effect_traces.py` contributes 200 real-handler cases / 1,425 operations:
actor/source/piece identity, inactive/old-active/out-of-scope providers, duplicate
providers, source/token positions, team-independent Illusions, the first two rings,
target kinds, topology, shield choices/invalid input/current/facedown/duplicate
cards, expiry, all 33 hero decks with Defense and Initiative stat effects, and
complete Aspiring Duelist, Stop Projectiles and Bain bounty-defense chains.
Every state/request/ordered-event record matches Python. Subset SHA-256:
23f95fc058c7e4fbecf1ee3a3c360a350f967841b0fd292fa98614f6d96f82c8.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/reaction_effect_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/reaction_effect_traces.py verify -- "$worker"
```

Full effect-aware targeting/AttackSequence/projectile reactions and remaining
card/passive/spell/phase/finishing semantics remain required, alongside aggregate
context aliases, make/unmake, hashing, evaluator, iterative team-aware search,
hidden planning, parallelism, validated integration, strength gates and 1,000
complete two-game-turn simulations/sec. Full engine/search flags remain false.


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


## Effect-aware targeting, pre-actions and projectile attack execution

Single UNIT selection now enumerates actual units first, then equivalent
Illusion tokens in location order. UNIT_OR_TOKEN keeps its mixed location order.
Multi-select, Count and Collect intentionally retain their source pools, which
omit equivalent tokens in UNIT mode. Team/minion-type filters still recognize
an equivalent Illusion whenever the chosen pool contains it. Shared illusion
rules use live actor/source ownership, not a cached team or the token's owner.

Intrinsic targeting implements source LOS blockers: only strictly interior
points on straight hex-axis segments block. It uses live/fixed origins, validator
activation rules, the actor's immunity and source actor restrictions, without
inventing scope/range/target-immunity checks. Units run this validation; tokens
skip it in both selector modes. Override-player targeting uses that actor for
intrinsic LOS, while predicates still use the current actor. Predicate immunity
now includes action immunity, temporal attack immunity and the performed card's
basic status. Placement/movement filters use existing displacement validation.
Range predicates retain origin/candidate identities for topology immunity, and
context-adjacency uses source topology. Falsy actors finish multi-select as in
Python. Unsupported filter kinds remain explicit compilation errors.

Pre-action movement is no longer a no-op: its exact selection/move recipe is
exported from ResolvePreActionMovementStep, then bound to typed hero/range
continuations. First eligible source provider, active/temporal rules, zero/null
fallback to one, optional choice, signed ranges, non-action movement and dynamic
prompt plurality match Python. Pre-action disruptors retain processed effect IDs
in the same requeued frame, re-evaluate live scope/immunity/hand/presence, consume
matching IDs and refresh source-card activation before forcing a discard. Empty
hands either retain the effect or cause source-attributed defeat. Premature
consumption does not execute finishing steps; defender defeat can continue the
attacker's chain without aborting it. All fields restore as ordinary source step
fields, including queued movement children and processed IDs.

These consumers enable the existing attack expansion and Reflect/Deflect
Projectiles bodies with active effects. Complete integration traces cover
pre-defense movement, leaving a disruptor radius, consumed linked effects,
multiple disruptors, empty-hand defeat and mine-triggered discards during defense.
Unported card/passive bodies, filter kinds and phase/finishing consumers still
reject explicitly; full-game/search flags remain false.

`targeting.json` contains 66 source states / 29,780 complete verdicts and ordered
single/multi/Count/Collect scans. SHA-256:
a12410da81f268fdce8c63b7138c0ad51861714e82a823e415449dea2853e4e3.
`targeting_effect_traces.py` adds 482 complete handler cases / 2,902 operations,
SHA-256 69dc87592a7d2680f03dd5ac295e2953a1ab450a3059aac129400888dde4119e.
Both match native full results, not sampled booleans. Existing projectile cases
are rerun with effects through the real Python handler and registered cards;
pre-action recipes are frozen from source APIs rather than Rust expectations.

```sh
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/targeting_rules.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/targeting_rules.py verify -- "$worker"
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/targeting_effect_traces.py export
PYTHONPATH=scripts/rust_oracle:src "$oracle_python" scripts/rust_oracle/targeting_effect_traces.py verify -- "$worker"
```

Remaining requirements include all remaining card/passive/spell/filter bodies,
complete phase/dynamic-effect/finishing behavior, aggregate context aliases,
make/unmake, hashing, evaluator, iterative team-aware minimax/alpha-beta, hidden
and simultaneous planning, parallelism, validated integration, strength gates
and 1,000 complete two-game-turn simulations/sec. Current checks establish
correctness of the covered boundaries, not full-game support or throughput.


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

The arena binds natural/premature effect removal, physical-card active flags,
copy-time scalar snapshots and original stored finishing-step handles. Removed
effect models and their pending steps remain reachable through retained aliases.
Turn expiry exposes a read-only collector and a remove-now helper. After-reveal
trigger scheduling removes by effect type, queues only due payloads with fresh
actor/abort wrappers, and preserves shared original steps. Runtime generation
uses typed recipes rather than JSON. Detached selection options are read live.

The live corpus has 142 source scenarios / 428 operations, SHA-256
`42fe944b2089a9c6da719abe66b76c5bfbca02651a4784c6be565eba118e5c39`.
Its tests cover identity, restoration, worker isolation, stale/cross-round triggers,
repeated IDs/objects and mandatory aborts. The frozen source and both earlier
corpora are unchanged. All 202 locked release workspace Rust tests and 791
focused non-NN Python rules/oracle tests pass. The final CLI replay took 204.52
seconds; that measures correctness/serialization replay, not search throughput.
The release worker independently matches both identity corpora. Strict Clippy,
Rust fmt, Black/Ruff, whitespace and frozen-source integrity checks pass.

Automatic arena turn/round/revelation orchestration, the remaining world and
rule bindings, make/unmake, hashing, evaluation, search, integration/strength gates
and 1,000 complete two-game-turn simulations/sec remain unfinished. Full
transition/search capabilities remain false. Negative attacks remain legal.
See the [implementation plan](../docs/superpowers/plans/2026-10-23-rust-search-engine.md#live-effect-lifecycle-and-after-reveal-scheduling)
for removal-order details and remaining requirements. No NN work, live services,
commits or pushes are changed.


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
