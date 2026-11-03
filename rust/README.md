# Native search engine

Work in progress, following [the implementation plan](../docs/superpowers/plans/2026-10-23-rust-search-engine.md).
Two complete-game traces now match Python for Arien/Brogan versus Tigerclaw/Sabina
on Forgotten Island and Magma and Ice. General roster support and bot action
selection remain unfinished.
The live Python backend and lab are unchanged.

Current reviewed primary coverage: **172/596 card faces** (127 custom bodies,
including three state-dependent Misa/Min builders, and 45 base no-op faces). All **22/22 spell bodies** now execute through native
casting and copied-card paths, with independent Python parity across 304 scenes /
6,113 operations. These counts do not establish complete defense, passive,
world-rule or search support.
Misa’s 18 primary bodies also pass independent Python parity across 691 scenes /
8,598 operations; see the [validation record](artifacts/misa-primary-final-validation.json).
Wasp adds 14 primary bodies and two conditional query steps, verified across
572 scenes / 8,455 operations; see the [Wasp validation record](artifacts/wasp-primary-final-validation.json).
Min adds 14 primary bodies, permanent item conversion and deferred red-card text,
verified across 507 scenes / 8,357 operations. That milestone passed
425 Rust tests and 103 Python source contracts, with one older all-corpus
exhaustive restoration gate excluded. Both full CLI corpora passed.
See the [Min validation record](artifacts/min-primary-final-validation.json).
Flurry of Blows and Cloak and Daggers now rebuild reviewed ordinary bodies using
source-specific raw/attributed APIs, explicit registry overrides and root-only
target restrictions. Differential coverage includes 176 cases / 652 operations,
including actual repeated attacks, negative damage and live collection lookup.
Unsupported card bodies remain explicit guards.
See the [dynamic passive validation record](artifacts/dynamic-passive-final-validation.json).
The authoritative arena now executes lane checks/pushes, minion battle,
casualty choices and spawn displacement through typed ordered mutations.
Coverage includes 267 source cases / 1,446 operations across all supplied maps,
preserving mapping aliases, spawn Hex identity and pending-input behavior.
General game/search closure remains unfinished.
See the [arena battle validation record](artifacts/arena-battle-final-validation.json).
Round cleanup now stages an identity-preserving arena transaction: duration and
card expiry, retrieval, markers, token persistence, mandatory leveling and both
ultimate unlock hooks. Coverage includes 95 cases / 192 operations and a new
round's commit/uncommit. Transaction and search performance gates remain open.
All 285 core tests, the cleanup restoration gate and three full CLI replay tests pass;
27 affected Python source contracts and independent release parity also pass.
See the [cleanup validation record](artifacts/arena-cleanup-final-validation.json).

Upgrade choices and round reset now execute in the authoritative arena. Ordered
single/all-remaining batches are validated before mutation; physical hand/deck,
item, pending/reveal and defeated-hero containers retain source identity. Round
reset replaces only the shallow position snapshot. Differential coverage includes
107 cases / 653 operations across seven heroes, both upgrade sides, intermediate
tiers, invalid submissions, imported pending inputs and a complete multihero round.
Mutable session/arena staging is still a correctness implementation, with no
make/unmake or throughput claim. General rules, raw exception fidelity and search
completion remain open.
Validation passes 291 core tests, both affected restoration gates, three full CLI
replay tests, 36 source contracts and 28 existing upgrade-rule tests. Full replay
covers 14,900 live cases / 118,301 operations and the unchanged canonical corpus.
See the [upgrade/reset validation record](artifacts/arena-upgrades-final-validation.json).

Hero respawn now executes through live operands and retained arena maps. It
preserves PASS-before-spawn ordering, token unlinking, fresh selected Hex identity,
actor-specific marked-spawn obstacles, source fallback BFS and multipiece supply
order. Differential coverage includes 123 cases / 417 operations, both teams on
all six supplied maps, imported pending values and shared snapshots. Dense and
live candidate paths share the actor-aware obstacle fix. Card menus and action dispatch already have a live adapter. The audit below
verifies their phase integration; unported card bodies and general roster/game
coverage remain open.
All 297 core tests, the respawn restoration gate and three full CLI replays pass,
along with 61 affected Python tests and independent release parity. Full replay
covers 15,023 live cases / 118,718 operations and the unchanged canonical corpus.
See the [hero respawn validation record](artifacts/hero-respawn-final-validation.json).

The phase audit confirms that every supported source-pinned instruction prototype
reaches its live adapter; ResolveCard was already implemented. Its dense-only
rejection is now labelled as an invariant rather than an unported feature.
A new corpus covers 191 real-handler turn/round flows / 1,819 atomic operations,
including action choice, respawn, confirmation, finalization, upgrades and reset.

Whole-game diagnostics now stream source/native snapshots and retain lossless
64-operation delta chunks. The native session stays alive across the entire game;
only golden-data loading is chunked. Sparse array splices avoid repeating unchanged
binding rows. The existing collected replay API and non-mutating normalization
remain available; explicit fresh-record ownership can avoid redundant frame copies.
No neural runtime or server is involved.
Two complete games pass all 7,059 operation comparisons through game over, with
an independent release replay. The source verifier peaked at about 210 MiB;
lossless compaction reduced its golden deltas from 673 MiB to 20.5 MiB. All 298
core tests, the 191-flow restoration gate, eight CLI tests and 98 affected Python
tests pass. See the [validation record](artifacts/phase-closure-and-stream-final-validation.json).

To regenerate full-game goldens using the pinned oracle environment:

```sh
PYTHONHASHSEED=0 PYTHONPATH=scripts/rust_oracle:src \
  "$HOME/.cache/goa2-rust/oracle-venv/bin/python" -m streamed_games \
  --worker "$HOME/.cache/goa2-rust/target/release/goa2-search-cli"
```

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

For everyday rule/test iteration, use the incremental `iterate` profile. It
inherits release semantics, uses optimization level 2 and 16 codegen units, and
disables LTO. Its cache is separate from release builds:

```sh
sh rust/cargo-wsl.sh test --locked --profile iterate --workspace
```

`GOA2_ORACLE_WORKERS` controls independent CLI and live-restoration cases (1–4, default 4,
capped by available CPUs). Each worker owns one case and compares actual
snapshots one operation at a time. Every case and operation still runs; thread
panics fail the test. Test loaders buffer file reads, stream full cases, and
filter named/subset cases before loading snapshots. The WSL wrapper defaults
`RUST_TEST_THREADS` to 2; explicit environment or `--test-threads` arguments take
precedence. For fully serial debugging, set both `RUST_TEST_THREADS=1` and
`GOA2_ORACLE_WORKERS=1`. Avoid overlapping another build with a large test run.

Keep `--release` for final optimized validation and engine/search throughput
measurements; its ThinLTO and single-codegen-unit settings are unchanged. To
measure test runtime separately from compilation:

```sh
sh rust/cargo-wsl.sh test --locked --profile iterate --workspace --no-run --timings
/usr/bin/time -v sh rust/cargo-wsl.sh test --locked --profile iterate --workspace
```

Measured on the current WSL host with all 235 tests and compilation excluded:

| Workflow | Full run | Peak test RSS |
| --- | --- | --- |
| Buffered release, serial test/case execution | 7m 03s | 1.32 GiB |
| Iterate, default bounded parallelism and streaming fixtures | 3m 34s | 0.89 GiB |

The first `iterate` compilation of all test binaries took 34 seconds, including
its fresh dependency cache. Warm test-only changes rebuilt in 2–12 seconds; a
codec change rebuilt the workspace tests in 26 seconds. Recent release builds
of all test binaries took 72–82 seconds. These are local workflow measurements,
not engine/search throughput. Large fixture arrays stream case by case, named
lookups retain only needed inputs, and all full parity comparisons remain.

A follow-up pass streams the model, effect-stat, targeting and terrain fixtures
as well. The model fixture is read at runtime instead of embedding 11.9 MB in
the test binary; named terrain tests share only three selected inputs. All 11
affected tests pass, including all 57 model-stat cases, 5 card-lifecycle cases,
136 effect-stat cases, 66 targeting cases and 196 terrain cases. On this host,
with compilation excluded, this subset changed from 3.46 s / 378,904 KiB peak RSS
to 2.90 s / 64,048 KiB. These single-run figures describe the test workflow only.

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

`cargo test` consumes the small committed fixtures and the local compressed full-corpus cache. Worker verification also
checks the persistent JSON-lines protocol and corpus/source fingerprints.
Movement disagreements are saved under ignored `rust/artifacts/` for diagnosis.

## Current coverage

| Area | Implemented and verified | Still required |
| --- | --- | --- |
| Reference | Frozen source manifest; independent Python BFS comparisons; fingerprinted corpora | Complete decision/event trace export, replay and minimization; full-search baseline runner |
| State boundary | Semantic round trips; dense entities/physical cards and typed effects/hero collections; live context, step and model identities; arena/dense walking, placement and removal with shared tile caches; ordered planning buffers and revelation knowledge | Remaining authoritative token/effect/world bindings and physical-child/value-equality coverage |
| Geometry | Checked cube/hex IDs; dynamic bitsets; terrain/occupancy; batched movement; effect-aware walking paths, dynamic obstacles and passable tokens; compiled range, obstacle, movement-path and straight-line predicates | Effect-aware push paths, targeting and remaining spatial-filter consumers |
| Execution | Ordinary closed-rule movement, combat, card-action and phase traces; live context/control, planning/revelation/actor selection, confirmation/finalization, minion-return controller, walking and placement fallback, mine choices/triggers, forced-discard expansion, intrinsic CARD selection, live equality-aware discard, passive discovery/offer/body construction, rune choice and usage | Dynamic Flurry/Cloak card-body rebuilding, remaining body consumers and generic CARD predicates; remaining card/passive/spell, respawn, lane/battle/round consumers; make/unmake and hashing |
| Stats/card model | Item/marker/active-effect stats, scoped immunity/topology, minion defense, masked card-stat recipes, Metamorphosis, Unlimited Firepower and conditional Big Sodding Gun stat auras; play/unplay/discard/resolve/swap/retrieve; physical alias export | Illusion-minion equivalence, non-JSON physical metadata equality, remaining phase-specific effect hooks |
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

Final finalization verification: the resumed CLI checks pass both the 6,592-case
canonical replay and the 311-scene / 800-operation live replay. The rebuilt
standalone release worker independently passes `object_execution_traces.verify`
with full state/request/event/identity parity. All 42 live integration tests pass
with the streaming loader: 71,248 KiB measured peak RSS, versus an earlier
4,181,456 KiB eager-loader observation. This is verification resource use, not
search throughput. Strict Clippy, fmt, whitespace and frozen-source checks pass.
Full transition/search support remains false, and the work remains uncommitted.


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


### Live walking execution

Live `MoveUnitStep` now reads unit/destination operands and output keys from the
arena, applies movement/displacement/action restrictions and actor-aware paths,
validates straight paths, and records crossed-obstacle lists and success flags.
It shares placement's location/tile mutation primitive, retaining immutable Hex
identity and refreshing aliased snapshot caches. Minion-return walking completes
across restore boundaries. Handler-level skip checks run before the step body;
the body's success clearing and second skip check retain their separate order.

The live corpus is now 710 scenes / 1,565 operations, semantic SHA-256
`5dd7c85aea03a83d3ed3bd050ecf76ce3a6a9986100ffc3f07934a147cb28cc3`.
The 171 added source scenes / 331 operations independently match state, requests,
events and object identities. They cover the real displacement-effect matrix,
key/actor fallbacks, invalid paths, zero/negative-range same-position moves,
straight walking, obstacle traversal, context/dictionary destinations, shared
locations/tiles/snapshots, success-dependent conditions and completed minion
walking. Native tests additionally check private worker destinations and mine
guards before popping a step or clearing its success flag.

Live mine-choice/requeue/trigger execution remains guarded before mutation.
Card/primary/passive/spell bodies, respawn and lane/battle/round consumers, remaining
physical/dense bindings, make/unmake, canonical hashing, evaluator, iterative
team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel search,
integration/strength gates and 1,000 complete two-game-turn simulations/sec remain
required. Transition/search capabilities stay false. Frozen Python and runtime
pins are unchanged; no live lab/backend/bot was restarted. No NN work or new
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


### Live mine and forced-discard controllers

Live movement now detects mines, schedules path choices, and requeues a shallow
copy with shared child identities and its own `mine_path_prechosen` flag. Live
choices recompute paths, preserve successful pending input, retry invalid input,
and write fresh choice lists. Ordered triggers snapshot literal mine/victim
queues, remove positions and tile occupancy, replace linked-effect lists without
collapsing duplicate effect IDs, and set the rollback reanchor boundary. Old
location, tile, effect-list and pending-input aliases retain source behavior.
Blast mines schedule source-specific forced discards with literal victims.

The forced-discard controller handles unknown/empty victims, immunity, empty
hands, basic-card filters and active discard shields. It creates actual card
selection/discard steps, preserving the raw victim in both. Those two consumers
remain guarded before popping; general blast discard resolution is not complete.
Dud movement/removal and choice retries execute to completion.

The live corpus is 867 scenes / 2,116 operations, semantic SHA-256
`2ad558df025638e239fa07ddf32db0aaf41fa01248e7334623f2b30dfc02dc29`;
index SHA-256
`d204e996f6cdc395dcf24f1b4d540eabf45fff073eb93cc56acdbedc62ee851e`.
The 157 new source scenes / 551 operations independently match full
state/request/event/identity snapshots. Only declared mathematical mine-choice
sets are normalized; trigger literals, event ordering and actual paths remain
exact. Additional native tests check worker isolation, shallow copy identity,
literal queue/victim isolation, and guarded card-selection restoration.

Next bind card selection/discard consumers and their passive/expiry chains.
Remaining card/primary/passive/spell bodies, respawn and lane/battle/round consumers,
physical/dense bindings, make/unmake, canonical hashing, evaluator, iterative
team-aware minimax/alpha-beta, hidden/simultaneous planning, parallel search,
integration/strength gates and 1,000 complete two-game-turn simulations/sec remain
required. Transition/search capabilities stay false. Frozen Python/runtime pins,
negative-attack rules and live services are unchanged. No NN work or new
commit/push is included.

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


### Live intrinsic card selection

Arena-backed CARD selectors now share the ordinary typed card candidate rules.
Property lists, owner/hero precedence, container merging, masks, shields and
metadata keys are read from live step fields; referenced context values are read
directly, without serializing or copying the whole context. Candidate/option
ordering and duplicate physical entries are preserved. Successful/automatic
selection clears pending input and request IDs; invalid input remains available
while retrying; optional skip/no-candidate outcomes clear only the output key.
Piece owners normalize for prompts while raw hero references retain source meaning.

The live source matrix reuses `card_selection_traces` through an optional callback
that leaves its normal export behavior unchanged. All 77 selector scenes / 278
operations independently match Python, covering every registered hero and the
complete existing property/container/ownership/pending matrix. Existing mine and
shield continuations now include actual card prompts and selections, adding 112
operations before reaching discard execution. The combined corpus is 944 scenes /
2,506 operations, semantic SHA-256
`40a922c1ebd9da5b93aba93b500550b60657e3525707b538d392ab9b73c49691`;
index SHA-256
`7b90f9e9aa4a14efcd2ad3fbdd0d17b183343f733047cc6aefe1c0a5a3cb7cff`.
Private-worker tests check live allowed/hand lists, metadata isolation and
pre-mutation guards. The blast regression now reaches the discard guard after a
successful card selection, including restoration.

Nonempty generic CARD predicate lists remain guarded before popping; intrinsic
card property filters are supported. Discard mutation and its expiry/passive
chains remain unbound, so complete blast chains are still not advertised. Before
binding discard, preserve Python model-value equality for current/played/hand
removal, physical aliases, global public reveal and linked-effect expiry rules.


Remaining full-engine work includes those consumers/predicates, remaining
card/primary/passive/spell and respawn/lane/battle/round bodies, authoritative
physical/dense bindings and model equality, make/unmake, canonical hashing,
evaluation, iterative team-aware minimax/alpha-beta, hidden/simultaneous planning,
parallel search, integration/strength gates and 1,000 complete two-game-turn
simulations/sec. Transition/search capability remains false. Frozen Python,
locked runtime/dependencies, negative attacks and live services are unchanged.
No NN work or new commit/push is included.

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


### Gydion spell access and preparation (2026-11-03)

Seventeen ordinary Gydion primary faces now execute native school access and
preparation, including Archwizard replacement through Wish resolution. Reviewed
ordinary primary coverage is 126/596; spells remain 22/22. Independent optimized
parity passes 508 scenes / 10,150 operations. The 398-test Rust suite and 31
Gydion/catalog/ordinary-body source contracts pass; only the older exhaustive all-corpus restore gate
is filtered. The dedicated and expanded generic restore tests check every Gydion boundary.
The three affected restorations and full CLI replay pass after the generic
matrix update. Full replay covers 12,539 live scenes / 90,556 operations and the unchanged canonical corpus.
Strict lint, formatting and frozen-source checks pass. See the [implementation
plan](../docs/superpowers/plans/2026-10-23-rust-search-engine.md) and
[validation record](artifacts/gydion-primary-final-validation.json) for details.


## Development footprint and fixture storage

Use `python scripts/development_footprint.py` from the repository root to review
the bot checkout (including Git and fixtures) plus `$HOME/.cache/goa2-rust`.
The planning budget is 12 GiB; consider cleanup around 10 GiB. Review weekly during
active development and after major exports/builds. The report never deletes,
blocks generation or enforces a cap. Raise the budget deliberately if needed.

Full generated fixture payloads are ignored local `.json.gz` caches. Generators,
scenario definitions, oracle/runtime pins, expected corpus indexes/hashes and
small bootstrap/runtime catalogs remain committed. Logical `.json` paths in
indexes are unchanged; Python and native test readers transparently decompress
individual cases, with no permanent extraction or whole-corpus RAM cache.
Gzip is used per case instead of one giant ZIP so exporters can atomically
publish/merge cases and tests can access cases independently. Missing full-cache
files fail explicitly; tests never silently substitute a reduced corpus.

For a fresh checkout, the small movement/state fixtures allow:
`sh rust/cargo-wsl.sh test --locked --test oracle`.
Full native tests require regenerating/restoring the compressed full corpus.
Use the exporter commands above with the pinned oracle and runtime. Exports now
write compressed payloads directly. Existing corpora can be migrated offline:

```sh
python scripts/rust_oracle/fixture_cache.py compress --untrack
```

Migration streams and verifies every gzip against its original bytes before
removing extracted files, preserves hard-link sharing, and removes only cached
payloads from the current Git index. It never rewrites published Git history.
Run it with no corpus generators/readers active. Keep runtime catalogs as normal
JSON because the engine embeds them at compile time. The exact retained files
are listed in `fixture_cache.COMMITTED` and `rust/.gitignore`.
