# Compact search state keys

Classic bot search uses a private, binary encoding for progression guards and
search-local transition reuse. This is not a save format and contains hidden
cards; it must not be exposed to a player. Normal engine models, API responses
and checkpoints keep their existing structure.

- `CompactState` interns fixed card, hero, minion and token properties by value.
  Custom scenario definitions and changed printed properties receive distinct
  definition IDs. Catalogs are bounded and live for one search only.
- Each card and token object has one mutable record per state. Collections and
  registries contain references, preserving order, empty turn slots and separate
  objects even when they share a printed card ID.
- Card states use numeric codes, boolean flags use bitmasks, and optional card
  values are included only when non-default. Token owner and all mutable flags
  are retained. Extra subclass fields and nested metadata are included.
- Other rule data (hidden planning, effects, prompts, execution stacks, context,
  seed and counters) remains in the key. The board is handled separately by the
  transition cache, including occupancy and terrain.
- Binary hashing uses `marshal` version 2 to avoid identity-based memoization.
  Keys are private to the search; they are not portable IDs across catalogs,
  Python versions or processes. No untrusted marshal data is loaded.

Definition IDs are meaningful only alongside their search's intern table. Never
persist these fingerprints or share a transition cache across searches. Static
interning does not make mutable engine models immutable or skip checking their
changed values. Normal cards use tracked printed-field revisions; unfamiliar
card subclasses recompute their definitions to catch mutable custom containers.

Regression tests: `tests/automata/test_search_reuse.py`. They cover clone/load
parity, ordinary serialization, card and token mutations, hidden collections,
custom definitions, catalog overflow, nested references and search isolation.


## Throughput work: active target 1,000 simulations/second

The target means completed ISMCTS iterations with the existing two-game-turn
lab horizon and scoring. Cached rule work is allowed; shortening the horizon,
duplicating visit counts or replacing completed rollouts is not a speed result.

The saved Brogan reproduction at forty iterations, no deadline, three runs:

| Change | Median wall time |
| --- | ---: |
| Binary-key baseline | 4.036 s |
| Cached immutable-geometry hash and binary tile keys | 3.344 s |
| Scalar card cloning, preserving independent mutable containers | 3.221 s |
| Cost-gated transition caching | 3.024 s |
| Reused guard keys until the next state mutation | 2.855 s |
| Decision-to-decision replay and persistent tile snapshots | 2.006 s |
| Verified keys carried between stock-policy decisions | 1.643 s |
| Bounded selectors, cached Hex geometry and exact bitset movement | 1.373 s |
| Owned continuation recipes and bounded fast-travel scoring | 1.277 s |

This is about 31 completed simulations/second, **not** the target. All fixed-work
root statistics and decisions are unchanged. Raw measurements/profiles are in
the lab checkout's ignored `.layout-lab/runtime/search-1000-*` files.

Transition caching now measures operation cost before paying to build board keys
or copy saved worlds. Cheap operations use the direct engine path. Geometry keys
have cached hashes; mutable tile snapshots use bytes instead of re-hashing every
Hex on every lookup. Input-policy key reuse is limited to unmodified built-in
Heuristic/Random input methods; custom policies are re-hashed conservatively.

Private deterministic request-ID and address-label normalization experiments
increased hit counts but did not improve elapsed search time, so were removed.
The remaining major costs are full-state guard serialization, policy scoring and
rule execution. Reaching the target needs larger improvements to those paths;
small cloning/key changes alone are insufficient.


## Exact reuse and movement shortcuts

Search clones use persistent immutable tile snapshots with private writable
views. Occupancy edits invalidate snapshot keys and walkability masks. Forks
remain independent, and ordinary board serialization materializes normal tiles.
Unknown/custom tile models keep the general cloning path. Hero/card containers
remain independently copied; card definition sharing does not share mutable
metadata, secondary actions or planning buffers.

Decision-to-decision replay is limited to the unmodified heuristic agent,
untimed stock sessions, no rollback snapshots, and the existing bounded two-turn
continuation. Keys include the complete rule fingerprint, board, pending input,
session state and controlled heroes. Replays restore independent worlds, prompts,
and progression counters. They cannot bypass deadlines or advancement budgets.
Planning recipes consume every original RNG draw and accept only the same
winning card; failed variants restore the RNG before trying the real engine.
Owned planning continuation can reuse rankings while drawing a fresh tie winner.
Per-agent reentrant locks protect draw/replay sequences without adding locks to
serializable agent state. Custom policies keep the ordinary path.

Bounded range/movement/zone candidates preserve board ordering and still run
all original predicates. Ordinary movement uses integer-bitset BFS only for
stock validators/topology, no active effects, and no board token/custom tile
semantics. Other cases retain the general engine traversal. Cached immutable Hex
neighbors preserve direction order and return fresh neighbor lists.

A 200-iteration comparison with continuation reuse disabled/enabled retained
all root action visits, scores and the final decision. The full validation run
passed 3,490 domain, engine and affected classic-search regression tests.

## Action timing in the saved opening position

An instrumented 200-iteration run took 7.81 seconds. Each root action received
40 simulations; elapsed branch times include subsequent two-turn continuation:

| Root action | Total time | Mean per simulation |
| --- | ---: | ---: |
| Movement | 2.273 s | 56.8 ms |
| Fast travel | 2.191 s | 54.8 ms |
| Attack | 1.179 s | 29.5 ms |
| Hold | 1.068 s | 26.7 ms |
| Clear | 1.038 s | 25.9 ms |

Movement/fast-travel option scoring took 0.956 seconds in total. Actual
MoveUnitStep/FastTravelUnitStep resolution took 0.079 seconds. Among actual
rule steps, SelectStep took 0.620 seconds, ResolveCardStep 0.511 seconds and
ResolveCardTextStep 0.170 seconds. These are separate overlapping breakdowns,
not percentages to add together. This is an opening position with few connected
attacks; it does not establish combat cost in a crowded late-game position.
The raw capture is the lab runtime's search-1000-action-breakdown.json.
