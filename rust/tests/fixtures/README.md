# Python oracle fixtures

Generated offline by `scripts/rust_oracle/`; source identity is pinned in
`rust/oracle-manifest.json`. The corpus SHA-256 covers compact JSON in its original object/array order,
excluding its own `corpus_sha256` field. Object insertion order is semantic for
candidate/action generation and must not be sorted away. The oracle identity
also pins Python and validation-library versions. Never derive expected values from Rust output.

Verification uses bounded case workers (default 4, CPU-capped) and two test
threads in the WSL wrapper. `GOA2_ORACLE_WORKERS=1 RUST_TEST_THREADS=1` selects
serial debugging. Full cases stream; named rule/fork tests retain only selected
inputs. Full oracle scans keep every expected snapshot and exact case counts.
See [build/test commands](../../README.md#build-and-test-in-wsl) for the fast
`iterate` profile and measured resource results. The frozen files, hashes and
comparison coverage are unchanged by these test-loader optimizations.

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
  request and ordered events after 27,145 advance/input/prefix/planning operations. Includes paused
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


Live public planning corpus: 224 scenes / 614 operations, SHA-256
`3f84f5370b50f5d2daf11349bc7223659b952a39736ea45235ef5083c9ae3009`.
The 13 new pass/finish scenes cover validation/no-op boundaries, request dictionary
replacement with shared surviving rows, original pending/done containers,
repeated commands and ordinary/dual-card final-player revelation. Physical-card
commit/uncommit commands and complete action/round bodies remain guarded.
Frozen Python source and the storage/canonical gameplay corpora are unchanged.

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


Live card-planning corpus: 247 scenes / 668 operations, SHA-256
`b464e42768468bdc3b0079208aa41813baa4667ed7be007eea6995444881f279`.
The 23 added scenes cover public commit/uncommit, LIFO two-card take-back,
first-only done removal, request detachment, copied overrides, shared hand/deck
and cross-hero hands, last-player revelation and spell nonmembership. All 54 new
operations independently match full state/request/event/identity records. Frozen
source and older corpora are unchanged; complete action/round execution and
foreign copied-card equality remain open.

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


Live confirmation corpus: 279 scenes / 757 operations, SHA-256
`a16e2cfb613b83f1762991237133ec747264e1181f8790dae0789610324f377b`.
The 32 new scenes / 89 operations cover rollback flags, input/request retention,
retries, ownership/controller routing, abort boundaries, scope restoration, loop
copies and shared-step re-execution. All new scenes independently match full
source records before full replay. Unported card/finalization/respawn/round bodies
remain guarded, and the frozen source and older corpora are unchanged.

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


Live finalization/ownership corpus: 311 scenes / 800 operations, SHA-256
`a337611e6ada4b26e3769e00dd20a097894d79d69754d1f6756a3b46a18f6f97`.
The 32 new scenes / 43 operations cover card-slot/effect/counter completion,
in-place context clearing, retained aliases, raw versus owning hero IDs,
shared/repeated/duplicate instances, skipped conditions and continuation ordering.
All new scenes independently match full source records. Minion-return and other
unported consumers remain guarded; scheduling them is not full transition/search
support. Frozen source and older corpora are unchanged.

Final finalization verification: the resumed CLI checks pass both the 6,592-case
canonical replay and the 311-scene / 800-operation live replay. The rebuilt
standalone release worker independently passes `object_execution_traces.verify`
with full state/request/event/identity parity. All 42 live integration tests pass
with the streaming loader: 71,248 KiB measured peak RSS, versus an earlier
4,181,456 KiB eager-loader observation. This is verification resource use, not
search throughput. Strict Clippy, fmt, whitespace and frozen-source checks pass.
Full transition/search support remains false, and the work remains uncommitted.

Live minion-return controller: 342 scenes / 840 operations, SHA-256
`67fbede19bec0f5d87676d1bcbe5b9e34fa0b74f8e8e3fee9b1875fb71ded7c8`.
The 31 added source scenes / 40 operations cover controller expansion, team
choices/retries, retained aliases and pending values, raw processed-list copying,
walk/fallback restrictions, and no-return/skipped/repeated cases. Generated Hex
objects keep shared-deepcopy identity. All new scenes independently match source
snapshots. Relocation bodies remain guarded; these tests do not claim completed
movement or full transition/search support. Frozen source and older corpora are
unchanged. Final workspace validation is pending.

Live placement corpus: 539 scenes / 1,234 operations, SHA-256
`74e54562da1de0635f4fe2e1be770f331a16fe93bd615dd28edf13f72d07968e`.
The 197 added scenes / 394 operations cover the complete displacement effect
matrix, all six maps, preserved location/tile/Hex/snapshot identities, dictionary
normalization, ignored input, repeated/skipped steps, mandatory/foreign abort
boundaries and actual minion placement fallback. Every added scene independently
matches Python state/request/event/identity snapshots. Walking and other unported
consumers remain guarded. The older frozen corpora/source are unchanged. All 239 workspace tests, both
release corpus replays, and the independent 539-scene release-worker comparison
pass, including every live restoration boundary.


Live walking corpus: 710 scenes / 1,565 operations, semantic SHA-256
`5dd7c85aea03a83d3ed3bd050ecf76ce3a6a9986100ffc3f07934a147cb28cc3`.
The 171 new source scenes / 331 operations cover live displacement restrictions,
walking/path/straight validation, output lifecycle and both skip boundaries,
retained spatial aliases, and completed minion walking. They independently match
Python state/request/event/identity snapshots. Live mine-choice and trigger
consumers remain guarded; complete-transition/search claims remain false.
 Frozen source and prior corpora
are unchanged.

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


Live mine and forced-discard controller corpus: 867 scenes / 2,116 operations;
semantic SHA-256
`2ad558df025638e239fa07ddf32db0aaf41fa01248e7334623f2b30dfc02dc29`, index
`d204e996f6cdc395dcf24f1b4d540eabf45fff073eb93cc56acdbedc62ee851e`.
The 157 added source scenes / 551 operations cover choice retries and pending
aliases, shallow movement copies, literal queue/victim snapshots, ordered and
repeated triggers, linked-effect replacement including duplicate IDs, mutation of
old effect-list aliases, and actual shield/immunity/basic-card discard expansion.
Only explicitly declared mine-choice sets are normalized; paths, trigger queues,
events and option ordering remain exact. Card-selection and discard consumers
remain guarded; complete blast-chain/transition/search support remains false.

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


Live intrinsic CARD selection: 944 scenes / 2,506 operations. Semantic SHA-256
`40a922c1ebd9da5b93aba93b500550b60657e3525707b538d392ab9b73c49691`;
index `7b90f9e9aa4a14efcd2ad3fbdd0d17b183343f733047cc6aefe1c0a5a3cb7cff`.
The 77 added selectors / 278 operations reuse every registered hero and the
existing candidate/property/container/pending/ownership matrix. Another 112
operations extend mine and shield continuations through real card prompts and
selection. Expected records originate only from the frozen Python engine.
Nonempty generic CARD predicates and discard execution remain guarded; this
corpus does not claim complete blast, transition or search support.

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


The action constructor catalog `action-builders.json` is reproduced by
`scripts/rust_oracle/action_builders.py`. `live_card_action_traces.py` adds 632
live scenes / 7,268 boundaries for all 596 ordinary card menus, piece selection,
14 complete action lifecycles and adversarial inputs. Constructor-only menu
scenes intentionally retain unexecuted child bodies; complete flows are named
separately. Source contracts check the cold catalog, negative attacks, primary
classification and acting-piece input/retry behavior. Fast-travel child models
are captured/restorable while their execution remains guarded. Neither menu
coverage nor generated recipes imply fully playable card or spell coverage.


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


`live_travel_traces.py` retains the canonical travel worlds as physical objects,
then exports each handler boundary with aliases for steps, locations, effects and
snapshots. Its 184 cases / 917 operations include every map, optional/invalid
input, direct and preselected destinations, Razzle/minions/tokens, granting-card
exceptions and two full secondary-action lifecycles. Separate source/native tests
exercise a radius-two enemy barrier. The expanded action-builders catalog keeps
selector filter-model identities and travel constructor defaults source-derived.


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


`live_perform_primary_traces.py` exports selected-card primary re-performance with
live scope/card/step aliases: 248 scenes / 1,171 operations. It distinguishes
constructor-only reviewed-body coverage from complete flows, empty registered
builders and replacement execution. A real Movement primary (Deluge) offers Fast
Travel; Liquid Leap remains a Skill control. The cold action catalog includes
full context restoration and source exclusion-filter defaults. Boundary restore
workers remain bounded and private; source contracts independently check alias
restoration, search order, replacement menus and exclusion inheritance.


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


`live_arien_primary_traces.py` adds full text, normal-action and re-performance
flows for the seven newly reviewed Arien bodies, plus radius and behind-target
geometry cases. Expanded primary/re-performance construction brings this increment
to 200 scenes / 2,150 boundaries. The primary catalogs now contain 45 custom IDs.
`primary-builders.json` records source-audited absent-radius defaults per typed
recipe path, distinguishing zero-default scopes/filters from nullable filters.
Existing primary and re-performance restore tests expand with their catalogs;
a separate bounded test restores every Arien flow/geometry boundary.


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


### Gydion ordinary cards (2026-11-03)

Gydion coverage includes 338 full-flow scenes / 9,631 operations plus 170
generic construction/reperformance/copied-menu scenes / 519 operations:
all 17 school/preparation cards through ordinary and foreign-copy entry points,
spent/missing books, Archwizard eligibility/accept/decline, and third-Wish nested
success/failure. Complete state/request/event/identity parity and restoration at
every boundary pass, with root clones unchanged. Six native guard/live-state/
isolation tests and nine source behavior/partition contracts pass; the expanded
ordinary catalog/radius and existing primary-body contracts add 22 checks. The
398-test workspace run passes, with only the older all-corpus exhaustive restore
gate filtered. After expanding the generic matrix, all three affected restore
tests and full CLI replay pass again.

Full live corpus: 12,539 scenes / 90,556 operations; canonical: 6,592 / 27,145.
Strict lint/format and frozen-source/lock checks pass. [Validation record](../../artifacts/gydion-primary-final-validation.json).

Index SHA-256 `5be7dd86817d87bed50e4db06d31403eb7b771fa6606d76643e551e267f989d7`; semantic SHA-256 `1f12e3cac1595a164f50ace447a03ef169e55a2a69268203b22dcaa4de73bfb3`.


### Misa primary bodies and geometry (2026-11-03)

Misa adds 18 primary bodies: 16 pure recipes and two explicit live-state
builders. The schema-3 builder catalog contains 97 pure recipes plus Swoop In
and Watch How I Soar variants selected from the actual performer's ultimate and
level. Dense recipes remain 64; spell bodies remain 22/22.

The source increment covers 691 cases / 8,598 operations: 389 Misa flows,
122 geometry cases and 180 generic construction/reperformance/copied menus.
A bounded restoration test rebuilds all 511 Misa/geometry cases at each of their
8,038 operations, checking private effects, physical cards and hex identities.
The existing generic restoration matrices expand with the new card readiness.

Current live corpus: 13,212 cases / 98,954 operations; canonical dense corpus:
6,592 / 27,145. Reviewed primary coverage is 144/596 faces, with 452 remaining.
Index SHA-256 `a93a50a6c063259355fa13e637aeb3e24a43fc147e7afaf85dde728f58351af2`;
semantic SHA-256 `0269233bd91b47209e37521a1d8cf8f602066820f6105c2fed9ae59fe4afab50`.

Validation: 409 Rust workspace tests and 71 Python source contracts pass; the
older all-corpus exhaustive restoration gate was excluded. Final release parity,
strict Clippy, Rustfmt, Black/Ruff and frozen-source checks pass.
[Validation record](../../artifacts/misa-primary-final-validation.json).


### Wasp primary bodies and conditional queries (2026-11-03)

Fourteen Wasp primary bodies join the reviewed recipes, bringing primary coverage
to 158/596 faces (111 pure recipes, two state-dependent Misa builders and 45 base
no-op faces). All 22 spell bodies remain covered separately. Repeat templates
retain the source constructor's independent step/model identities.

The Wasp increment contains 572 cases / 8,455 operations: 334 action flows,
94 direct unit-type/boolean queries and 144 generic construction, re-performance
and copied-menu cases. Flows cover two/three orbit movements, repeated teleport,
physical tokens, distinct-target ranged repeats, optional skips, collision
checks, empty-hand defeat, copied performers, radius fallbacks and legal negative
attacks. Queries check explicit False, piece owners, numeric identity coercion,
unknown IDs and Python container truthiness. Unit selectors reuse the shared
straight-line/inverse straight-line geometry filters.

Current live corpus: 13,766 cases / 107,205 operations. Canonical dense corpus:
6,592 / 27,145. Index SHA-256
`c34eeb0f817e5d10198d2d122f9a97a96e5761f6685ea9fae72c133524489b8a`;
semantic SHA-256 `9ad15df4039d57f730afbb8997d43cbfd622dab0bb87ec96f6b14ef4569106b1`.

Validation: 416 Rust workspace tests and 84 Python source contracts pass. One
older all-corpus exhaustive restoration gate was excluded. Final release parity,
strict Clippy, Rustfmt, Black/Ruff and frozen-source/lock checks pass.
[Validation record](../../artifacts/wasp-primary-final-validation.json).
