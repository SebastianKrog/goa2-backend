# Python oracle fixtures

Generated offline by `scripts/rust_oracle/`; source identity is pinned in
`rust/oracle-manifest.json`. The corpus SHA-256 covers compact JSON in its original object/array order,
excluding its own `corpus_sha256` field. Object insertion order is semantic for
candidate/action generation and must not be sorted away. The oracle identity
also pins Python and validation-library versions. Never derive expected values from Rust output.

- `movement.json`: all six map files plus three seeded boards; occupied and
  terrain spaces, all four movement-flag combinations, zero/one/four-step range,
  mapped/off-map origins and destinations. Expected verdicts come from Python's
  original bare-board BFS and its separate stateful BFS. Both contracts have
  141,876 destination verdicts. Export also verifies optimized stateful Python
  checks against their independent stateful BFS.
- `states.json`: full semantic states plus alias-preserving envelopes. Includes
  token supply, spells, Razzle pieces, independent physical copies sharing a
  printed card ID, a changed scenario definition, planning aliases and a paused
  selection step. Its fixture-local step identity is assigned explicitly.

- `stack-traces.json`: schema-2 index of 6,592 real Python handler traces in
  `stack-traces/<case-sha256>.json`, recording full state,
  request and ordered events after 24,473 advance/input/prefix/planning operations. Includes paused
  template copies, stale/malformed choices, optional and mandatory paths, context
  conditions, multi-piece prompt ownership, real MoveUnitStep occupancy/event
  transitions, blocked and straight paths, zero-distance movement and map gaps. Imported fixture step labels are explicit; typed ephemeral request IDs and
  generated-step labels are normalized without changing the live engine objects.
  Retrieval cases use real Emmitt revelation state and test both card choices,
  malformed input, imported requests, no-op conditions and physical aliases.
  Unit/piece cases exercise implicit self/immunity filters, heavy support by
  lane, protected tokens, unit/token/structure distinctions, owner overrides,
  optional and mandatory failures, physical piece actors and binding followed
  by movement. The helper in `scripts/rust_oracle/unit_traces.py` uses the
  actual Python handler and registered rule/filter classes.
  `travel_traces.py` adds 40 Fast Travel traces: all maps, cross-terrain
  relocation, optional/invalid choices, zone changes, bound pieces, restored
  continuations and destinations becoming unsafe after selection.
  `combat_traces.py` adds 70 real-handler traces for ordinary combat, defeat,
  removal and game-over consequences. Covers block flags/bonuses, minion support,
  all hero reward levels, assists, marker return, piece-owner identity, repeated
  defeats, life clamping, annihilation ordering and real card aliases/turn slots.
  These traces do not replace full attack or active-effect coverage.
  `reaction_traces.py` adds 223 real-handler traces covering defense prompts and
  card choices for all 33 registered heroes, Initiative blocks, defense flags,
  invalid inputs, restored requests, card/pass combat paths and ordinary discard
  orchestration. Public knowledge, discard history, action stacks and generated
  passive-hook ordering are included in the complete state comparisons.
  Remaining defense/on-block text and unported eligible passive bodies remain
  separate coverage gates. Native tests stream cases rather than embedding the
  full corpus or loading all expected snapshots in every parallel worker.
  `attack_traces.py` adds 49 real-handler traces for complete attack expansion,
  optional target freshness, preselected/missing targets, multiple attacks,
  per-attack defeat marks, piece actors/victims, runtime stats, performed-card
  identity (including owner-only extra cards) and target menus across all maps.
  Attack orchestration runs through ordinary pass/combat paths; actual eligible
  defense/passive effect bodies remain separate gates.
  `defense_effect_traces.py` adds 97 real-handler traces for the ported defense
  bodies, all three singleton markers, copied-action policy and immediate stat
  changes. Atomic prefixes use Python's existing stop-before-step hook and retain
  events, generated children and temporary actors after each instruction.
  Upgraded defense cards are explicitly faceup in positive effect cases; facedown
  no-op behavior is tested separately. Custom unported on-block bodies stay gated.
  `card_selection_traces.py` adds 112 real-handler traces for CARD selection
  across all 33 heroes, ordered containers/aliases, visibility and property
  filters, selection metadata, auto/optional/mandatory choices, snapshotted
  forced-discard victims and full Reflect/Deflect Projectiles flows. Includes
  range items, attacker exclusion, piece victims, invalid input and intermediate
  instruction export/restore before selection and discard.
  `action_execution_traces.py` adds 156 real-handler traces draining ordinary
  secondary actions for all 33 registered heroes, including repeated unit/token
  selection, Clear removal, acting-piece choice, invalid choices, item stats,
  facedown cards and exported continuations. Mrak's registered upgrades exercise
  secondary attacks against heroes/minions and item attack/range bonuses; those
  cases assert that the intended target is actually defeated. Direct multi-select cases cover
  minimum/maximum counts, early finish, duplicates, saved choices and skips.
  Token removal preserves supply entities and records its actual event order.
  These traces do not claim primary card-text or turn-end phase support.
  `primary_traces.py` supplies 517 real-handler traces for 38 reviewed primary
  effects, primitive fallback, all-map teleports, placement/swap/hex recording,
  item and zero/negative stats, piece targets and per-instruction continuations.
  Positive attack cases assert actual defeats; primitive cases distinguish
  occupied tiles, terrain, self-placement, missing entities, virtual boards,
  unplaced tokens, literal/key precedence and location order after swapping.
  `push_traces.py` adds 49 direct traces for pushes, computed hexes, recursive
  OR targets and adjacency/behind-space predicates. Covers blocked/crossed/trimmed
  paths, board edges, zero/negative/coerced distances, literal/key precedence,
  token/piece targets, no-op/failure paths and queued passive ordering.
  `minion_return_traces.py` adds 36 after-action return traces on all six maps
  and generated boards: ordered shortest paths, placement fallback, team choices,
  processed IDs and lane-specific continuation after an impossible return.
  Positive cases assert actual movement into the minion's own Battle Zone.
  `battle_traces.py` adds 76 lane-push/battle/displacement traces (232 operations):
  all maps, simultaneous throne/final-wave outcomes and tie remedies, ordered
  casualty choices, heavy eligibility, registered One Man Army, two-pass spawn
  assignment, token/terrain/blocked spawns, duplicate origins, exact request
  context and retained invalid pending input. Battle roots use CLEANUP phase.
  `turn_traces.py` adds 53 action-handoff/boundary/respawn traces (368 operations):
  full four-hero resolution to planning on every map, moved-position snapshots,
  confirmation and mandatory-abort boundaries, initiative ties/items/masking,
  guess lifetimes, passive-use resets, hero-piece ownership, empty-hand auto-pass,
  respawn retries/tokens/fallbacks and explicit end-phase queue snapshots. The
  latter stop before round cleanup; they are boundary snapshots.
  `round_traces.py` adds 138 cleanup/upgrade traces (446 operations), including
  ordered retrieval/marker/token cleanup, cumulative leveling, both ultimate
  unlock hooks, all 33 heroes' two upgrade choices, atomic invalid-batch retries,
  reveal snapshots and next-round reset on all six maps.
  `game_traces.py` adds 12 planning-to-round traces (1,077 operations). Real
  commit/uncommit/pass/finish calls drive hold-based rounds on every map, two
  consecutive rounds, invalid planning, Emmitt two-card planning and empty-hand
  passes. These do not cover arbitrary card bodies or search.
  `aura_traces.py` adds 14 aura gameplay traces (49 operations), including
  real movement actions through obstacles, isolated effect moves, invalid choices,
  instruction-boundary export, ranged minion kills and changed initiative order.
  `conditional_aura_traces.py` adds 35 cases (77 operations) for conditional
  basic-attack stats, nested card ownership/fallbacks, live context changes,
  registered versus fallback primary logic, post-attack adjacency and blocked
  Point Blank Shot pushing. Twenty-three stat cases separately compare generic
  implicit-context queries with explicit per-card views, including pieces.
  Each index entry checks the case's complete content hash, ordered name and
  canonical path. The index has its own fingerprint and retains the original
  full-corpus semantic fingerprint. Migration checked every full snapshot and
  preserved dictionary/array ordering. Cases remain separate so normal source
  control and per-scenario native tests avoid a single large fixture file.

- `models.json`: 57 item/marker/card/minion-defense/aura cases with 16,482 stat
  queries and five real Hero model traces with 37 operations. Records complete
  semantic state after both successful mutations and model errors; covers sparse
  overrides, shared physical aliases, spell rejection and repeated references.
- `planning.json`: 12 real phase-function traces with 77 operations. Includes
  full-state planning mutations and queued resolution handoffs, Emmitt two-card
  planning, ordered ties, pass slots, respawn queues and restored planning buffers.
  Also includes 11 isolated ResolveTieBreakerStep traces with 19 operations,
  comparing prompt ownership/options and ordered generated continuations,
  including the delayed coin flip. The action stack is not drained; session-level
  parity remains a separate gate.
- `actions.json`: 38 isolated ResolveCardStep cases, 1,828 operations over all
  33 registered heroes' ordinary card decks and selected edge states. Compares
  exact request metadata, context changes and complete ordered generated child
  steps; the children are not executed. Python checks that no other persistent
  state changes. Also includes 176 safe-zone queries and 60 direct zone lookups across six maps
  and seven generated cases, preserving neighbor order, duplicate candidates,
  tile-zone metadata precedence and first-membership fallback.
- `instruction-templates.json`: frozen Python defaults for native-generated
  selection/movement, phase and action continuation records, spatial filters, and the
  registered two-card-planning capability set. Reference
  content identity is checked at native compilation. Export explicitly rebuilds
  Python's nested serialization unions, so subtype-specific filter and child
  instruction fields are retained.

Keep object/array order wherever it carries semantics. These boundary fixtures
have not been normalized by deleting IDs or reducing state to a public view.
They do not yet constitute complete game-transition or hidden-information tests.

- `primary-effects.json`: exact public `get_steps_with_stats` output for 34
  reviewed pure recipe builders, with reserved hero/card/stat operands.
  `primary_templates.py` exports it; a test compares it to fresh API output.
  This is a closed native recipe catalog, not a translator for arbitrary Python.

`merge_corpora` validates compatible indexed corpora and case hashes, computes
exact full-corpus semantic SHA-256 incrementally, then swaps the index atomically.
It holds one complete case at a time and leaves existing retained files intact;
no state fields or snapshots are removed to reduce memory. Verified immutable
case files are hard-linked when supported, with a copy fallback. Export writes
through temporary files and atomic replacement so an interrupted regeneration
cannot corrupt installed snapshots or other links.

The primary catalog records top-level attack steps whose ranged flag directly
reads the card. The exporter compares both boolean inputs and rejects recipe
shape changes or other ranged-dependent fields instead of freezing one branch.

The instruction catalog also records effect classes overriding stat, movement or
action-prevention aura hooks. Native card definitions compile that closed set
once; ordinary resolved cards inherit empty hooks. A Python test compares the
catalog against the exact frozen method implementations. Metamorphosis and
Unlimited Firepower now execute natively, with separate public-hook specification
checks. Big Sodding Gun's conditional stat hook and after-push passive are native;
Witching Hour remains unported and guarded.

The instruction catalog freezes the two actual ultimate-unlock hook IDs as well.
Their native implementations are tested against the real Python handler, and a
separate authenticity test checks the registered hook catalog.

Mandatory discard-or-defeat coverage adds 20 traces / 37 operations for forced
card choice, empty-hand defeat, literal/context/fallback kill credit, missing and
off-board victims, multi-piece owner/piece identities and instruction-prefix
continuations. Generation asserts the intended outcomes, including the frozen
engine's owner-level multi-piece removal behavior; Rust restoration tests check
every instruction boundary. This is primitive coverage, not passive-provider
execution coverage.


`passive_traces.py` adds 35 traces / 116 operations for ordered provider scanning,
usage/face/lifecycle rules, played/ultimate aliases, explicit-owner handling,
optional YES/NO/SKIP, missing cards, mark-used steps and real single/repeated/blocked
pushes. A real Point Blank Shot flows through a defense, push, forced discard and
usage marking. All cases include an instruction-prefix snapshot. Python generation
asserts use counts and effect outcomes; separate public-API tests verify every
frozen config plus waiting-card revalidation. Native tests restore every
instruction and keep search workers' counters/registers independent.


Native transport resume metadata is outside the frozen semantic game state.
Python-exported fixtures omit it. Native tests reproduce a completed request ID
being reused after restore, then verify that serialization preserves the issued
request/step high-water marks even with an empty stack. They also check exact
wire-ID matching, generated template prefixes and rejection of invalid, future,
regressing or exhausted resume metadata. This changes no reference corpus hash.


The tactical Sabina additions contribute 127 primary traces / 1,001 operations
for four swaps, three repeatable minion movements, two support/discard skills
and two attack/follow-up removal skills. Point Blank Shot gains its previously
missing ordinary primary matrix: 11 cases / 64 operations. These complement its
conditional-aura and real after-push traces rather than replace them.
`repeat_traces.py` adds 27 cases / 97 operations for positive/declined/skipped,
partial, nested, negative/zero/exhausted, absent/off-board/piece actors, scalar
AND validation and real lane-boundary behavior. Native every-instruction restore
tests cover the frozen repeat count and lane-push flag; worker tests verify fresh
template copies and private nested counters. Active repeat-prevention is now covered by `action_rule_traces.py`;
aggregate alias validation remains guarded.


`query_traces.py` adds 41 query/condition cases with 41 handler operations. These
include an 882-instruction comparison matrix, count/collect immunity defaults,
real token candidates, heavy support, off-board actors, owner/piece self-exclusion,
collection order after swaps, hidden/current/resolved turn-slot cards and paired
action/color criteria. Skipped queries preserve old output through the actual
handler's central gate; direct leaf resolve behavior is not substituted for it.

`conditional_primary_traces.py` adds 80 cases / 425 operations for Quickdraw,
Gunslinger, Dead Shot, Shield Bash and Counterattack. Generated assertions check
actual bonuses, discard logs and empty-hand defeats, including resolved/old slots,
face-down cards, pieces and stale bonus context. Their ordinary primary matrices
add 55 cases / 227 operations. The combined 176-case corpus matches the release
worker exactly; native tests restore each relevant instruction and isolate
visibility, slot and occupancy changes between workers.


`retrieval_traces.py` adds 31 handler traces for ordinary single/bulk retrieval
and premature expiry during discard. Includes played/current/discard aliases,
piece actors, no-op selections, ordered collections, base/enraged flags,
global printed-ID lookup and unchanged public/discard history.
`retrieval_primary_traces.py` adds 54 targeted support-skill traces for Steady
Advance, Unwavering Resolve, Devoted Followers and Fresh Converts: friendly
counts, enemy adjacency, actual retrieval/movement, optional choices, stale
context, items, invalid choices and instruction prefixes. Expected values come
from the frozen Python engine and assertions check the intended outcomes.


- `effects.json`: 123 real EffectManager lifecycle/creation traces / 570 operations,
  generated by `effect_traces.py`. Full semantic state and returned finishing
  instructions/IDs are recorded after every operation. Includes all effect
  categories and durations, dormant and cardless activation, token exemption,
  physical card owner versus performer, extra-turn cards, same-print copies,
  repeated effect IDs, empty links, shared card flags, natural expiry ordering,
  after-cards-played trigger fizzling and restore. Payload execution and CreateEffectStep binding remain separate gates.

`effect_creation_traces.py` adds 82 creation cases / 284 operations: card states,
cardless/empty links, foreign cards and same-print copies; dormant deduplication
and charge/timestamp preservation; distinct affects/type rows; expiry and
recreation; every scope shape with fixed/implicit/explicit origins; independent
static-barrier origins; basic/tiered/hidden/nested attack provenance and all effect
payload types. Reference values come from the real EffectManager.create_effect.
Fixture export replaces files atomically to preserve immutable snapshot readers.


`token_effect_traces.py` adds 51 real handler cases / 53 operations for token
removal with token_id, source_id and scope.origin_id links. Includes all effect
categories, inactive/expired rows, fixed scope hexes, independent barrier anchors,
repeated IDs, card flags, finishing-payload suppression, off-board/unknown targets,
context precedence, repeated removal and create/remove instruction boundaries.
Linux and Windows frozen runtimes produced identical cases. These cases test
removal semantics, not the gameplay consumers of the effect categories.


- `effect-stats.json`: 136 source-verified states / 29,103 stat queries from
  `effect_stat_traces.py`: every scope/relationship pair, items, markers,
  conditional ordering, performed-card selection, multi-piece aggregation,
  effect timing, heavy/action/attack immunity and topology splits/isolation.
- `effect_adjacency_traces.py`: 17 full handler cases / 18 operations in the
  main corpus, including directional isolation and immunity exceptions.
- `effect-errors.json`: one frozen reference RecursionError for a recursive
  radius-immunity/topology scope. Native execution rejects the cycle explicitly;
  it is not counted as a successful transition or a gameplay-correct result.


- `displacement-validation.json`: 159 frozen states / 85,701 full can_be_*
  results, including allowed/reason/effect IDs/modifier IDs/source. Generated and
  verified by `displacement_effect_traces.py` against the actual native worker.
- `displacement_effect_traces.py`: 318 full placement/swap handler cases / 636
  operations, including instruction prefixes, mandatory abort, live origins,
  actor switches, token and piece events, and self/unknown/off-board identities.
  These cases are part of the canonical stack-trace index.


- `action-validation.json`: 198 source-verified states / 306,816 complete action,
  repeat and movement verdicts, plus 31 full menu/recipe cases / 1,169 operations.
  Includes prerequisite precedence, explicit card colors, all effect categories,
  scope/relationship/timing matrices, negative/zero/tied movement caps, immunity,
  token/off-board/unknown actors, bound/unbound multi-piece owners, ordinary
  negative attacks and Witching Hour's card-specific restriction.
  SHA-256: bd8ceee56244fc816ca6bf24abfd67dfe885d2def968fd352299e0e56fde4c6c.
- `action_rule_traces.py`: 175 real repeat-validation handler cases / 350
  operations, now part of the canonical index. Preserve full state/events,
  scalar AND operands, current-actor changes, source movement and restore.
- Updated `effect-stats.json` SHA-256:
  f1fa19af6b73f2f6b28535f9e3641bc623f9f2206ff867ad72545c33da01ebe5.
  Explicit/missing origin tile zone IDs have separate reference cases.

- Current canonical `stack-traces.json` SHA-256:
  2959d73e4ead1d8a9c7d344f9a88706b4a848b13f04ff9f9c7cc44135bacd03b.
  This index now contains 5,002 cases / 19,604 operations.


- `terrain-validation.json`: 196 frozen states / 42,728 queries (183,848 scalar
  answers including path/reachability destination vectors), from the actual
  terrain validators and stateful movement BFS. Includes static barriers,
  empty-space denial, petrification, all scope/timing/type base variants,
  topology/actor identity, explicit empty topology lists, off-map origins and
  destinations, all movement-flag combinations, passable tokens and supply
  fallback/precedence. SHA-256:
  0c3ab5625215eccff4d9c38b347105a54de2a764ba0b1d7a545c3bfc76a094da.
- `terrain_traces.py`: 756 full MoveUnit handler cases / 1,512 operations.
  Export asserts successful ordinary moves and exact mandatory/optional abort
  outcomes. Includes split-crossing attempts, straight paths, caps and restore.
  Actual mine triggers remain a separate gate; these cases do not simulate them.
  Standalone transition subset SHA-256:
  25fb9ac6196acce721492f39e9cf37d6332950c83464d0a0c326a35e15f5c9b5.


- `hex-filters.json`: 201 states / 19,497 complete compiled-filter scans /
  1,208,717 candidate verdicts. Includes actor scope/timing/topology, geometric
  ranges, inverse straight lines, passable tokens and moved identity, live
  barriers, spawn adjacency and the unlocked/facedown Tide of Darkness rule.
  SHA-256: b19d529d1d577aab8194dbdf6dc47f199c5da07d3cd359c7a96bfc673ee932aa.
- `movement_sequence_traces.py`: 1,045 full handler traces / 5,347 operations,
  including 64 controlled-prompt cases / 352 operations. Actual Python state,
  requests and events are frozen at instruction boundaries. Ordinary movement
  and skip outcomes are asserted; invalid inputs, repeated IDs, off-board
  controllers, explicit active-flag timing, ownership mismatch and original
  actor discard metadata are included. Standalone subset SHA-256:
  2431d2c4c5214cdd092fb19e8d048920aea548550d21703136deaa10dbfa7e30.


- `mine-paths.json`: 23 states / 920 complete mine-path queries. Compare exact
  destination option/path order and mathematical mine sets. Actor-relative
  barriers, topology, supply precedence, friendly/unowned mines and worker-local
  mutation are covered. SHA-256:
  9b84beec381b4e0954665ca15c4274a565d3a30a44d44d063e9d7857cd63a4fd.
- `mine_traces.py`: 117 handler cases / 528 operations: 100 standalone choices
  and 17 explicit trigger chains. Restore every instruction; preserve explicit
  trigger/event order, duplicate/off-board blast discards, rollback flags and
  token-linked effect removal. Only choice-only cases declare unordered mine
  context keys; no ordered trigger case opts into normalization. Standalone
  subset SHA-256:
  ad63ba83bdca62de5ec504953201065fd5202e4a48421ce9904f0ee686ba2f07.
- `mine-reference-errors.json`: three source-reproduced defects: hash-dependent
  implicit mine order, numeric option-ID coercion rejected by MinePathChoice, and
  literal destination loss in generated movement mine choices.
  Nested wire metadata is preserved. SHA-256:
  db96e31f88d9bf82582f48369836ae03ab1fc7e7fcb9c86a585b2b6477908de8.
  The literal-destination bug is an explicit compatibility fixture; frozen Python
  sources are unchanged.


- `mine_movement_traces.py`: 66 full handler cases / 396 operations for automatic
  movement/mine scheduling. Exact state/request/event comparison includes
  straight crossings, mine/avoid choices, retries, discards, successive moves,
  explicit prechosen queues, supply/entity precedence, forced/piece movement,
  controlled prompts, skips and literal destination loss. Flexible paths contain
  at most one detected enemy mine; ordered multi-mine cases use straight or
  explicit queues. No event or trigger-queue order is sorted away. Subset SHA-256:
  71c6469e1b462286ffd4b4b6aa8bd67505d4b0b6f7c1e7edb7123978ff2db576.


- `discard_effect_traces.py`: 103 real-handler cases / 680 operations for
  effect-aware forced discards, shield alternatives, public discard metadata,
  linked-card effect expiry without finishing steps, and blast chains during
  movement or explicit triggering. Uses registered Mrak deck cards with physical
  aliases preserved. Includes basic/nonbasic filtering, no eligible hand,
  current/facedown/duplicate/multiple shields, direct active-flag behavior,
  immunity and irrelevant passive-provider grants. Subset SHA-256:
  94c596b194f80ede9896ad7e78107236175d6d5145f258f5d28ccc4b436d6182.
  Eligible unported effect-backed passive bodies and dynamically created blast
  effect closures remain explicit gates; these cases do not prove full combat.


- `passive_provider_traces.py`: 202 real handler cases / 1,267 operations for
  ordinary and granted Wild Hunt execution, live provider resolution and typed
  provider-local usage. Includes copied/facedown foreign cards, direct/scanned
  offers, declined/skipped/retried decisions, raw active/scope/card/creator flags,
  seen-card deduplication, unlimited usage, exact non-action movement, caps, mines,
  provider removal before marking and missing-carrier/inactive usage markers.
  Full state/request/event order is preserved. Source recipe metadata is exported
  from the actual registered passive into instruction-templates.json, with
  owner placeholders bound only at native compilation. Subset SHA-256:
  74700ceaff60f72f2d366dcf8a680bd66fd8a29a74a814a91314b854158cdea2.
  Other eligible passive bodies remain gated; the combat corpus below covers
  effect-aware Gun.


- `combat_effect_traces.py`: 132 full handler cases / 542 operations for minion
  saves, limited bounty payouts, source/card/token expiry on defeat, effect-aware
  signed combat and granted Big Sodding Gun. Includes all minion types, totem
  priority and invalid anchors, hand/color/type/scope/temporal eligibility,
  imported pending input, multiple providers and tried IDs, accepted-save versus
  fallback bounty behavior, enemy/subject/source/range/topology eligibility,
  ordered payouts, exhausted counters, dynamic creation/deduplication and active
  game-over states. Negative attacks remain legal; no general Python prohibition
  was found. Native tests restore every selected instruction boundary and check
  private counters and stale provider invalidation. Subset SHA-256:
  71639d8299f0ef59aea9f77f805a88337b772d3dc22df24102bf3d36db0ef7bc.
  Full attack/reaction and phase/finishing closures remain independent gates;
  the next increment adds illusion-minion defense.


Publishing an index retains superseded content-addressed snapshots so streaming
readers holding an older index can finish. Reclaim obsolete snapshots only in
an offline cleanup after every reader has stopped; export/merge never prunes.


Current stack index SHA-256:
890b01218bb6efe0ab8482a9f383d4e17305a1aac2456e5e9e72c81e69049659.
All 5,816 cases / 24,473 operations match the release worker, including combat,
illusion/reaction, targeting, pre-action and complete attack increments. Full state, request and ordered events
are compared; these records do not establish simulation/search throughput.


- `reaction_effect_traces.py`: 200 real-handler cases / 1,425 operations for
  actor-bound illusion defense, live hand/shield reaction choices, defending-card
  stat context, expiry/passive/actor restoration and supported primary-defense
  bodies. All 33 decks include Defense/Initiative effects. Includes copied/piece
  actors, direct versus temporal active flags, scope/subject independence,
  topology, ring distances, inactive/current/facedown/duplicate shields, invalid
  input and complete Arien/Wasp/Bain defense chains. Source assertions supplement
  full ordered snapshots. Native restoration runs every new case instruction by
  instruction. Subset SHA-256:
  23f95fc058c7e4fbecf1ee3a3c360a350f967841b0fd292fa98614f6d96f82c8.
  The following targeting increment covers active-effect projectile execution.


- `targeting.json`: 66 frozen source states / 29,780 complete LOS verdicts and
  ordered selector/multi/Count/Collect results. Includes exact blocker metadata,
  actor/target origins and immunity, source actor restrictions, direct/temporal
  activation, token validation bypass, illusion pools and type/team predicates,
  performed-card attack immunity, placement/movement restrictions, heavy support,
  topology and bound owners. SHA-256:
  a12410da81f268fdce8c63b7138c0ad51861714e82a823e415449dea2853e4e3.
- `targeting_effect_traces.py`: 482 actual handler cases / 2,902 operations for
  effect-aware selectors, overrides, multi/query differences, optional pre-action
  movement, disruptor consumption/requeue/defeat, physical card activation,
  source recipe fields, mine movement and complete projectile attack bodies.
  Includes 32 full attacks combining pre-defense moves, disruptors and mines.
  Full ordered state/request/event records are retained. SHA-256:
  69dc87592a7d2680f03dd5ac295e2953a1ab450a3059aac129400888dde4119e.


Effect-aware automatic minion return adds 238 frozen real-handler traces / 695
operations, generated by `scripts/rust_oracle/minion_return_effect_traces.py`.
They cover displacement restrictions and immunity, movement caps and action-only
flags, actor fallback, topology, terrain/barriers, and token passability. Forced
corridors verify walking through passable tokens versus connected placement around
blocking tokens. Expected results come exclusively from the frozen Python handler.
All new cases match native state, requests and events; native restoration checks
resume at every generated instruction and retain independent worker choices.


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


Effect-aware battle adds 322 frozen real-handler traces / 1,375 operations from
`scripts/rust_oracle/battle_effect_traces.py`. The source fixtures retain all 76
original battle/lane/spawn scenarios with effects and add linked-token cleanup,
159 displacement-rule states, plus cap/origin/scope/duration/topology/immunity
exclusion cases. They compare complete semantic state, requests and event order.
Source contracts pin overlapping-origin ordering and raw active-flag behavior.
Native checks restore each instruction and fork independent casualty choices.


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


Effect-aware immediate cleanup adds 216 real-handler traces / 602 operations from
`scripts/rust_oracle/round_effect_traces.py`: the 138 existing cleanup/upgrade cases
with persistent effects (maps enter at cleanup), and 78 duration/card-link/stale/
token/finishing variants. Original ordinary full-flow map cases remain intact.
Expected snapshots include exact token effect-removal counts and persistent rows
across level-up/reset. Native tests restore every instruction, keep upgrade
branches private and reject unported planning commands before mutation.


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


`object-arena.json` is a separate schema-2 index of 12 Python identity scenes /
49 mutations, generated by `scripts/rust_oracle/object_arena_traces.py`. These are
storage semantics checks, not gameplay transition traces: full values, explicit
object bindings and ordered object keys are compared. The source uses actual
GameState, registered cards, live models and shared finishing GameSteps. Native
checks include worker isolation, visible identity roundtrip, malformed bindings,
cycle rejection and frozen-model writes. The ordinary 6,592-case stack corpus is
unchanged. Arena bindings to dense worlds/registers/frames are still required.
Object arena corpus SHA-256:
d137bf6846416caf5b2dff0f537dcb31a5654261b561e9274343964126bcbe23.


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


`object-execution.json` is a separate schema-2 index of 18 real Python stack
scenarios / 41 operations from `scripts/rust_oracle/object_execution_traces.py`.
It tests live shared-step pending fields, stored effect/context references,
independent equal textual step IDs, stale/invalid/repeated inputs, auto/skip/abort,
conditions and list/model/tuple context operations. Full state, request, event,
error and identity topology are compared. Normalization changes only typed step
request fields, preserving user strings and physical step identity. Native checks
also restore every operation, fork private choices and enforce budget/scope guards.
This is closed scalar/context execution, not complete arena-backed gameplay or
scheduled finishing coverage. The 6,592-case ordinary gameplay corpus is unchanged.
Live-execution corpus SHA-256:
472b7e0d268c41e888a268d7c27abfcd55f514bb396b331a83bcbbcf407d8a31.

The alias-aware exporter rejects generic references into mutable descendants of
atomic card/token payloads until those slots are bound. Direct physical-instance
references remain supported. Source tests cover both cases, avoiding an apparent
identity-aware capture that silently loses physical-child sharing.


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


The current live identity corpus expands to 43 scenarios / 159 operations with
ForEach item references, live/shared counters, separate template deepcopy memos,
nested copies, dirty/prompting templates, list mutation/replacement and real budget
resumption. It now compares instance_bindings as well as generic object bindings,
so copied card/token identity cannot pass by equal JSON values. deepcopy_shared
records plain Hex's source copy contract; frozen subclasses remain real copies.
Both identity corpora were regenerated with this metadata.
Live corpus SHA-256:
fba6da46d7ecfc94b6ff73b0e2a4c3dc89074cc9a32e233a986859ddd3e5779a.
Storage corpus SHA-256:
1001eb3d2c8202ef1a3f9070e5983b9701a8506e69ac65a41a569c15e77361a9.
The ordinary 6,592-case / 27,145-operation corpus is unchanged. Full dense-world,
finishing and engine/search coverage still needs implementation.


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


The live identity corpus now contains 86 scenarios / 258 operations, adding
nested action-source pushes/restores, absent versus null values, shallow aliases,
shared type stacks, own/other-hero abort boundaries, copied loop aborts and actor/
controller changes. Ordered context keys are compared explicitly alongside full
state, requests/events/errors and generic/physical identity topology. Source
helper tests establish scope behavior; native checks restore every boundary and
verify private snapshot rows and ordinary/arena owner parity.
Current live corpus SHA-256:
bd7543443c99beec4716d4331383746e30a1d708c2c9eaa1ae132a9e33c6bc9f.
Full dense-world, finishing, phase/rule and search requirements remain open.


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


Live effect lifecycle and after-reveal corpus: `object-execution.json` now contains
142 cases / 428 operations (SHA-256
`42fe944b2089a9c6da719abe66b76c5bfbca02651a4784c6be565eba118e5c39`).
New cases cover natural/premature expiry, detached collection aliases, physical
card scalar-copy snapshots, removed-effect step restoration, finishing abort
boundaries, read-only turn-expiry collection, and actual after-reveal scheduling
with actor wrappers. Dedicated-trigger removal matches by effect type rather
than ID. Source-generated wrapper identities normalize by physical step object,
so reused textual IDs cannot collapse independent steps. A detached selection
case mutates its option list before requeue and must offer the updated choices.
All current and superseded content-addressed snapshots are retained for readers.
The frozen Python source/pin, storage identity corpus and canonical gameplay
corpus remain unchanged. Automatic arena turn/phase flow is not claimed complete.

Lifecycle checkpoint verification: all 202 locked release workspace Rust tests and
791 focused non-NN source tests pass. The final CLI replay of all 6,592 canonical
gameplay cases passes (204.52 seconds, not a search throughput result). The final
release worker independently verifies all 142 live identity cases / 428 operations
and all 12 storage identity cases / 49 mutations. Frozen source integrity and
strict formatting/lint/whitespace checks pass.


Turn-boundary live corpus: 170 rule-v9 scenes / 512 operations, SHA-256
`fe48993ec034e02dc8eeaed919afcebd0eb4109932d9beed449cc2dbb69f4592`.
The 28 new scenarios compare deferred/natural expiry, active flags, live expiry-ID
lists, snapshots, discard-log replacement, empty-hand and repeated all-pass
revelation, source actor wrappers and staged round-finishing order. No legacy
rules-version import was added. Unbound round consumers remain importable as
parked steps and reject before execution mutation. The frozen Python source,
canonical gameplay corpus and storage identity corpus are unchanged.

Turn-boundary verification: all 208 locked release workspace Rust tests and 797
focused non-NN Python rules/oracle tests pass (35 live-object contracts plus 762
other focused tests). The complete 6,592-case / 27,145-operation gameplay replay
passes in 200.51 seconds; this is not a search-throughput result. Independent
release-worker checks match all 170 live scenes / 512 operations and all 12 storage
scenes / 49 mutations. Strict lint/format/whitespace and frozen-source checks pass.


Revelation/initiative live corpus: 211 scenes / 598 operations, SHA-256
`be1972ce5f667f0eefb37fe270f20e0b4354ded3cfc0ecb6ac4e89be3e0382f2`.
The 41 new real-source scenes cover committed-card reveal, frozen public records,
two-card retrieval, reversed initiative, live ties, delayed coin flips, pending
input/override omission and actor scheduling. Parked card bodies remain guarded.
The source engine/pin, 12-scene storage corpus and 6,592-case canonical gameplay
corpus remain unchanged. All 214 locked release workspace Rust tests and 804 focused non-NN source tests
pass. The complete canonical gameplay replay passes (177.59 seconds, not search
throughput). The release worker independently matches both identity corpora; strict
lint/format/whitespace and frozen-source checks pass.
