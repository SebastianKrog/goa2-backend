# Native search engine

Work in progress, following [the implementation plan](../docs/superpowers/plans/2026-10-23-rust-search-engine.md).
This workspace executes the supported rule closure through ordinary full rounds.
It does not yet support general complete games or select bot actions.
The live Python backend and lab are unchanged.

## Build and test in WSL

The toolchain is pinned in `rust-toolchain.toml`; dependency versions are locked
in `Cargo.lock`. Install Rust with rustup and a C linker (`gcc` on Ubuntu), then:

```sh
cd /mnt/c/Users/Sebastian/Documents/GitHub/goa2-backend-bots
sh rust/cargo-wsl.sh test --locked --workspace
sh rust/cargo-wsl.sh clippy --locked --workspace --all-targets -- -D warnings
sh rust/cargo-wsl.sh fmt --all -- --check
sh rust/cargo-wsl.sh build --locked --release --workspace
```

The wrapper keeps build artifacts on the Linux filesystem at
`$HOME/.cache/goa2-rust/target`; override `CARGO_TARGET_DIR` when needed. Source
files stay in the bot checkout. Plain `cargo` also works on other supported hosts.
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

From PowerShell in the bot checkout, using the existing Python environment:

```powershell
.venv/Scripts/python.exe scripts/rust_oracle/oracle.py verify-movement -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
.venv/Scripts/python.exe scripts/rust_oracle/state_codec.py verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
.venv/Scripts/python.exe scripts/rust_oracle/stack_traces.py verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
.venv/Scripts/python.exe scripts/rust_oracle/model_traces.py verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
.venv/Scripts/python.exe scripts/rust_oracle/planning_traces.py verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
.venv/Scripts/python.exe scripts/rust_oracle/action_traces.py verify -- wsl.exe --exec /home/sebastian/.cache/goa2-rust/target/release/goa2-search-cli
```

Replace the Linux executable path if using another user or target directory.
On WSL with the same frozen Python/package versions installed, use `python3` and pass the
worker executable directly. The Windows Python environment is currently used
for oracle generation; Rust builds, worker execution and native tests run in WSL.

To regenerate fixtures from the *unchanged* frozen source:

```powershell
.venv/Scripts/python.exe scripts/rust_oracle/oracle.py export-movement
.venv/Scripts/python.exe scripts/rust_oracle/state_codec.py export
.venv/Scripts/python.exe scripts/rust_oracle/stack_traces.py export
.venv/Scripts/python.exe scripts/rust_oracle/model_traces.py export
.venv/Scripts/python.exe scripts/rust_oracle/planning_traces.py export
.venv/Scripts/python.exe scripts/rust_oracle/action_traces.py export
```

`cargo test` consumes the committed fixtures directly. Worker verification also
checks the persistent JSON-lines protocol and corpus/source fingerprints.
Movement disagreements are saved under ignored `rust/artifacts/` for diagnosis.

## Current coverage

| Area | Implemented and verified | Still required |
| --- | --- | --- |
| Reference | Frozen source manifest; independent Python BFS comparisons; committed corpus | Complete decision/event trace export, replay and minimization; full-search baseline runner |
| State boundary | Semantic round trips; typed effect storage and lifecycle; dense entities/physical cards; typed hero collections and ordinary card-model mutations; native positions/occupancy; ordered planning buffers and revelation knowledge | Token/effect lifecycle and remaining rule-relevant mutable state |
| Geometry | Checked cube/hex IDs; dynamic bitsets; terrain/occupancy; batched movement; compiled range, obstacle, movement-path and straight-line predicates | Effect-driven topology, targeting, stat dependencies and remaining spatial filters |
| Execution | Typed native interpreter; typed registers/continuations; context, actor, number/confirmation, coin, ForEach, two-card retrieval, acting-piece binding, unit/token selection, MoveUnit, hex selection, movement and Fast Travel sequence instructions; ordinary combat/defeat/removal/game-over, after-action minion return, lane push/minion battle/displacement, confirmation/finalization/respawn/initiative handoff and AttackSequence orchestration; generated-frame restore; real trace parity | Full phase/combat/effect rules and remaining aura/area stats; aggregate context arena; make/unmake and hashing |
| Stats/card model | Item/marker stats, minion defense, masked card-stat recipes, Metamorphosis, Unlimited Firepower and conditional Big Sodding Gun stat auras; play/unplay/discard/resolve/swap/retrieve; physical alias export | Active effects and remaining aura hooks, ambiguous duplicate-print model equality, remaining phase-specific effect hooks |
| Card action rules | Typed menus/recipes; ordinary secondary actions; 38 reviewed primary effects; passive scan/offer/use and Big Sodding Gun; unregistered-effect fallback; repeated unit/token selection and Clear; native placement/swap/recording and bounded repeats | Remaining primary/passive effects, active-effect/aura restrictions, spell faces and phase-specific effects |
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
- 109 native unit/integration tests passed in WSL, including all frozen corpora;
  strict Clippy and locked release compilation pass.
- 68 selected Python retrieval/Sabina/oracle tests passed
  after the latest addition; NN tests remain excluded.
- Native regression corpus: 141,876 bare-board plus 141,876 stateful destination
  verdicts across six map files and three generated boards; three complete
  semantic-state/alias round trips.
- 2,088 native/Python transition traces compare 8,750 operations, including invalid
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
unsupported, as do active-effect discard shields and immunity.
Remaining passive/defense bodies, protection/expiry/bounty active effects and
full card-action/phase integration remain unported. Nonempty active-effect
roots reject explicitly; this is not full combat or game simulation support.

The isolated planning diagnostic still stops at queued continuations and rejects
an all-pass turn-end transition. Executing sessions advance ordinary turns,
auto-pass empty hands, execute end-phase work and complete cleanup/upgrades into
the next planning round. Movement macros use runtime item, marker and supported
card-aura stats, preserving actor/piece ownership and effective ranges through
prompts, filters and restored continuations. Metamorphosis freezes obstacle
passage when a movement action starts; ordinary effect-side MoveUnit steps do
not gain that permission. Active effects and remaining custom aura hooks are
still guarded. Deployed passable tokens and mine triggers remain unported.

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


After-action minion returns execute natively for plain movement/topology: each
minion uses its own lane's Battle Zone, favored-team/ID ordering, ordered shortest
walking destinations, and placement fallback when no walking path exists.
Impossible returns are marked processed so later minions still run. Generated
move/place/continuation frames preserve literal destinations, path length,
mandatory flags and processed IDs across export/restore. Immutable prompt data
is shared between cloned workers. Active effects, aura restrictions and deployed
passable tokens remain explicit unsupported closures.
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
keeping wave counters, zones, occupancy and outcomes private. Active-effect
battle exclusions and topology remain unsupported. Ordinary round orchestration is covered below; remaining effect closures
and search remain unfinished.


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
explicitly, and active-effect providers remain guarded.

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
1,959 traces / 8,449 operations. Other passive bodies, active-effect providers,
and truthy non-string victim coercion remain unported; global game/search
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
effects and aggregate context aliases remain explicit unsupported closures.

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
These checks cover lifecycle, not behavior of each effect category. CreateEffectStep orchestration, behavioral consumers and scheduling/execution of
finishing payloads still need ports; live session guards and search capability flags stay in place.


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
