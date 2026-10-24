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

- `stack-traces.json`: schema-2 index of 2,088 real Python handler traces in
  `stack-traces/<case-sha256>.json`, recording full state,
  request and ordered events after 8,750 advance/input/prefix/planning operations. Includes paused
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
template copies and private nested counters. Active repeat-prevention effects
and aggregate alias validation remain guarded.


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
