# Shared Rust game and search foundation

Status: proposed implementation plan; no foundation work is claimed complete.
Written 11 November 2026. This is plan 1 of four: shared foundation, player-run
bot client, live-server database integration, and neural-search preparation.

## 1. Goal and assumptions

Provide one Rust rules runtime and explicit contracts that can support:

- a bot running on a player's computer, with a possible browser/WASM build;
- a live service that durably commits games and accepted commands;
- local and offline search with replaceable parametric or neural evaluators.

Assume the cleanup is complete. Use its supported facade, immutable catalogs,
test tiers, content acquisition and reproducible build workflow. This plan does
not repeat cleanup tasks or require historical formats, deprecated worker
operations, old Python models, or older client releases to remain supported.

**Current games must keep working.** At implementation start, record the exact
current engine/content identity and formats, then test games created and saved
by that baseline. Preserve their state, physical objects, pending decisions,
privacy, revisions, clocks, receipts, replay and outcomes. Do not reset games or
silently reinterpret rules. Coordinate a current-client cutover where a new
contract is needed; compatibility shims for obsolete releases are out of scope.

The remote planning base is `bots-upstream` at
`fc0ff7024238668d7c2a800845dffa89533b5b29`. Cleanup is continuing independently;
this documentation branch does not change that branch. The source review below
identifies existing responsibilities, not a claim that the remote base already
contains the completed cleanup. Reconcile the implementation map with the
finished cleanup before starting F01; the design assumes its end state.

### Existing responsibilities to retain

The native workspace already separates `goa2-core`, `goa2-game-server` and
`goa2-search-cli`. `LiveGame` prepares transitions, restores private
checkpoints and projects player-scoped views. The service authenticates actors,
owns clocks/scheduling, stores games in SQLite, records accepted-action receipts,
and publishes updates. Database work is therefore an extension of existing
persistence, not the introduction of the first database.

The current bot recommendation entry point starts from a full `LiveGame`.
Player-side search cannot simply download that session. It needs an independent,
information-safe input and a way to build valid hypothetical continuations.

The reviewed source uses envelope schema 1, rules version 9, checkpoint version
1 and private replay schema 1, with exact oracle/source identities. These are
review observations. F01 must capture the actual completed-cleanup baseline
instead of hardcoding those observations as permanent compatibility promises.

## 2. Boundaries and deliverables

Keep the rules runtime independent of HTTP/WebSockets, SQLite/PostgreSQL,
authentication tokens, OS clocks, Python and ML frameworks. Keep search policy
and feature extraction separate from authoritative command execution. Prefer
modules behind the existing facade; add crates only where dependency or target
isolation warrants them.

| Boundary | Owns | Must not own |
| --- | --- | --- |
| Core game runtime | Rules, legal decisions, command validation, prepared transitions, private checkpoints, scoped projection | Network IO, database connections, credentials, wall-clock reads, training |
| Search contract and runtime | Player observations, hypothetical worlds, bounded search, typed candidate commands, evaluator calls | Reading authoritative hidden state through a client search entry point; committing live games |
| Service/application layer | Authentication, game/seat binding, sequencing, time input, durable acceptance, notifications | A second implementation of game rules or bot evaluation |
| Storage adapter | Atomic conditional writes, checkpoints, journal/receipt persistence, loading | Advancing games, determining legality, running evaluators |
| Client adapter | Connection, authorized seat context, local budgets, submitting commands | Authority to change server state, clocks or permissions |
| Offline tools | Synthetic cases, benchmark runners, later training/evaluation pipelines | Dependencies imported by ordinary engine/server builds |

There are two deliberately different data paths:

```text
Private server state -> core projection -> player observation -> local search
                                                        -> proposed command

Authenticated command -> core prepare -> conditional durable commit
                                      -> publish committed revision -> scoped update
```

A player observation is not a persistence snapshot. A search result is not a
committed transition. An internal search key is not a portable game identity.

Implementation deliverables should include:

- versioned Rust types and serialization specifications for the contracts below;
- one projection and command-conversion implementation shared by adapters;
- an observation-only search constructor and a deterministic test policy;
- a full-recompute feature extractor and simple weighted evaluator;
- prepared-transition/storage conformance tests using current persistence;
- headless integration, privacy, determinism and performance evidence;
- a small native/WASM feasibility report for the shared library boundary.

No downloadable-client installer, production database replacement, neural model,
self-play campaign or training run is required to finish this foundation.

## 3. Current identity and compatibility contract

Create a machine-readable foundation manifest. Give each identity one owner;
reuse existing authoritative fields rather than introducing competing versions.

| Identity | Required meaning |
| --- | --- |
| Rules/content identity | Rules version, oracle revision/source digest, static pack digest and catalog identities needed to execute this game |
| Runtime compatibility | Current checkpoint/resume and replay schemas, plus an execution compatibility identifier where rules/content identity alone is insufficient |
| Observation/decision schema | Meaning of player-visible fields, decision kinds and command encoding |
| Feature schema | Feature names/order, types, normalization, masks, coordinate and team conventions |
| Evaluator identity | Evaluator type, parameter/model digest, compatible feature schema and numeric convention |
| Search configuration | Algorithm/policy version, perspective, sampling recipe/seed, horizon, budgets and deterministic tie ordering |

Separate portable semantic identity from build provenance. Compiler, target,
profile, source commit and binary digest belong in evidence and bug reports;
requiring identical compiler binaries is not a wire-compatibility mechanism.
Never serialize arena slots, pointers, intern-table IDs or private search hashes
as persistent/public identifiers.

Bind a game to its admitted rules/content identity at creation or restore.
Reject unsupported or mismatched inputs before mutation, with an actionable
error. Do not accept an arbitrary content/model file supplied by a client.
Application deployment chooses compatible artifacts; a network client cannot
select different rules for an existing game.

Current-format save/load, restart and replay are mandatory. Supporting
discontinued identities, historical migrations and old model artifacts is not.
Any required transformation of a current-format field must be explicit,
lossless, tested and deployed with its matching runtime. A silent version bump
or loss of in-progress games cannot satisfy this plan.

## 4. Player observation and knowledge

Define a serializable `PlayerObservation` owned by core projection. It contains
only facts this authenticated seat is allowed to know, plus public static
definitions identified by the content manifest. Reuse the current visibility
rules and engine-computed decision facts.

Include:

- game/seat binding supplied by the service, revision and decision identity;
- phase, round/turn, team perspective, public board/topology, units, effects,
  markers, resources and public card/reveal/upgrade information;
- the seat's permitted private information and exact addressed prompt, including
  team-scoped decisions and simultaneous decision ownership;
- observable history/knowledge sufficient to retain previously revealed facts;
- authoritative legal-action descriptors and clock/deadline information needed
  to act, with dynamic presentation timestamps separated from semantic input.

A reconnecting bot must receive the same accumulated knowledge it could retain
by staying connected. Define public reveal knowledge as a compact sufficient
summary or a bounded, resumable visible event stream. Dropping old history must
not make reconstruction silently guess facts that were publicly established.

Use explicit absent/unknown/known values and known counts. Unknown hand or
loadout slots must not become zero values or invented known cards. Teammate
information follows actual game visibility; sharing a team does not
automatically grant access to another seat's private cards.

Exclude private checkpoints, native continuations, opponent unrevealed choices,
server RNG state/secret seeds, credentials, private replay data and unrestricted
staff projections. Do not hide these fields only in the UI: they must be absent
from the bytes, diagnostics, candidate ordering and remotely visible digests.

Public physical references need a scoped stable mapping for visible objects,
distinct from printed card definitions. Do not leak hidden allocation order,
hidden counts or private intern IDs through that mapping. Keep bindings stable
through a decision and preserve distinct visible instances of equal cards.

Define an observation digest over an explicitly canonical semantic encoding.
Exclude volatile timestamps, transport authentication and hidden state. If
request handles are opaque random values, compare them by their specified
scoped mapping in paired privacy tests; their derivation must not encode secrets.

**Privacy invariant:** two authoritative states with identical information for
a seat must produce identical semantic observations, offered actions, features
and seeded client-search behavior for that seat. Freezing public time and using
the same allowed handle mapping makes this testable. This is a data
noninterference requirement, not a promise to eliminate all network timing
side channels. Information visible only to another recipient cannot influence
the local entry point.

## 5. Decisions, legal actions and command submission

Define an immutable `DecisionContext`: game, authenticated seat, fixed root
viewer, scoring team, revision, decision/request token, phase and permitted
controllers. Track decision owner separately from score perspective.
A descendant opponent/team decision must not silently change the root viewer.

Use engine-produced legal descriptors for planning and resolution. Cover the
current command lifecycle: deployment/readiness, card commitment and second
slots, uncommit/pass/finish, upgrades, optional/mandatory selections, confirmations,
team prompts and terminal/no-action states. Administrative operations, cheats,
rollback consent and pause votes remain application operations unless explicitly
included with their existing authorization and consent rules.

Keep command payloads typed. Preserve current request IDs, selection values,
optional skip semantics, ordered selections and physical-instance references.
Labels and feature indices are not commands. Convert one chosen candidate to a
current `PlayerCommand` in one shared implementation; validate it at the server
against the real game and authenticated actor.

An action list may use a bounded/lazy representation for large domains or joint
commitments. Specify its completeness and stable ordering. A search candidate
cap is policy pruning, not proof that omitted commands are illegal. Root action
availability must not depend on unrevealed opponent information. Outcomes
depending on hidden information are resolved by the rules after submission,
rather than revealed by an oracle-like preflight endpoint.

A proposed submission carries an action/idempotency ID, expected revision,
decision token and command. The service authenticates the seat independently;
never trust a body-provided actor ID. The game revision and decision token must
invalidate work across advancement, rollback, replacement and restart as
appropriate, including states that look visually identical.

Resolve idempotency before rejecting an already accepted retry for its old
revision: same ID and same authenticated command returns the stored reply;
same ID with different command content is a conflict. Fresh stale, foreign,
out-of-turn or illegal submissions change no game, receipt, replay or clock
state beyond an explicitly documented clock-expiry operation. Server time wins
over client time. A client recommendation never bypasses the normal command path.

## 6. Observation-only hypothetical worlds

This is an early feasibility gate, not an assumed property of a UI view.

Provide a constructor with inputs limited to `PlayerObservation`, admitted
public content, explicit search configuration and a local sampling seed.
It must not accept `LiveGame`, `GameCheckpoint`, a server RNG seed, or a closure
that can recover hidden state. Full-state engine/offline access remains a
separately named trusted API; player-side search never calls it.

Build a valid native decision-boundary world from known facts and constrained
unknowns. Preserve visible physical identities, collection order, public
upgrade/reveal knowledge, required effects/context and the exact root prompt.
Share immutable definitions; keep sampled hands, hypothetical commitments,
continuations, caches and RNG state private to each search/worker.

Hidden-world sampling must enforce public constraints and the seat's private
knowledge. Its version, deterministic seed and failure behavior are explicit.
Do not sample from the true server hand and then merely relabel it as hidden.
Other players' hypothetical actions may use only their modeled knowledge:
avoid strategy fusion and treating a sampled secret as something the root player
has learned. Document the algorithm's remaining approximation separately from
the privacy guarantee.

Exercise simultaneous planning, hidden upgrades, nontrivial mid-action effects,
copied cards, spells and multi-piece heroes in the reconstruction spike.
If current visible facts are insufficient, add the minimum safe public or
seat-private engine-derived facts and test their visibility. Do not substitute
a private checkpoint or accept a silently incomplete world.

Every sample must be internally valid, preserve the observed root and support
native continuations with the same rules identity. Failure to construct a world
is a typed outcome. An explicitly reported legal fallback may keep play moving,
but cannot be used to mark missing reconstruction coverage complete.
No resampling may mutate live state or progressively reveal the true world.

## 7. Prepared transitions and persistence handoff

Retain the core's prepare/commit separation. Conceptually:

```text
prepare(current_revision, authenticated_actor, command, explicit_time, budget)
  -> PreparedTransition(base_revision, candidate_state, ordered_transition)
store.commit_if_revision(game, base_revision, candidate_checkpoint,
                         journal_entry, accepted_action_receipt)
  -> CommittedRevision | RevisionConflict | StorageFailure
```

These names specify responsibilities, not a requirement to replace equivalent
cleanup APIs. A prepared transition is bound to its immutable source state and
cannot be attached to a different game/revision. Private checkpoint data stays
on the trusted path. Time-dependent engine work takes an explicit time value;
the service obtains authoritative time and performs the final expiry/revision
check within its serialized acceptance path.

A single storage transaction records the new state/revision, replay/history
data needed to reconstruct it, and the accepted-command receipt. Publish the
candidate in memory and broadcast only after successful durable acceptance.
On conflict, reload/re-evaluate instead of replaying a stale prepared candidate.
On storage failure, keep the old live state and return no successful acceptance.

Specify recovery for both uncertain outcomes: commit succeeded but reply was
lost; commit succeeded but memory installation/broadcast failed. Recover from
durable revision/receipt and resynchronize readers. Notification failure must
not cause a second game transition or undo a committed command. Durable outbox,
multi-instance leases and cross-process notification delivery are decisions for
the database plan, not requirements to invent a distributed system here.

Keep storage conformance separate from SQL implementation. Use the current
SQLite adapter and a fault-injecting test adapter to prove the contract.
Do not introduce a generic repository abstraction unless a real consumer needs
it. The later database plan decides production backend, retention, deployment,
multi-instance coordination and operational migrations.

## 8. Search state, features and replaceable evaluation

Use separate representations for authoritative state, portable observations,
search-local sampled state and evaluator features. Their ownership/lifetimes
must be explicit. A compact search representation is allowed only when it
preserves physical-object identity, aliases, collection/event order, null/unknown
semantics, pending decisions and branch isolation.

Define an evaluation seam before choosing a network. Start with a deterministic
weighted feature evaluator whose parameters live in a small validated artifact.
Document score perspective, range, units, terminal values, draw behavior and
tie ordering. Terminal outcomes are computed from rules and dominate heuristic
values; evaluators cannot determine legality or manufacture wins.

Value evaluation takes a position at a named boundary and a fixed perspective.
It must not require a particular candidate list. Policy ranking, if supplied,
takes the legal candidates separately and maps scores back to their typed
commands. Finite scores, shape checks and an explicit no-policy path are required.
Changing candidate enumeration must not change the meaning of a position value.

Specify boundary kinds rather than evaluating arbitrary interpreter frames:
root decision, subsequent decision, completed action/turn, terminal and budget
cutoff. Forced-effect draining uses the same execution budget. A partial
continuation at interruption is not silently labelled a completed boundary.
The neural-search plan chooses the search algorithm and training targets;
this foundation supplies explicit hooks and traces for the supported boundaries.

Features cover public board geometry and units, resources, known card state,
phase/decision ownership, public history and permitted private information.
Unknown masks are first-class. A sampled-world evaluator may use hypothetical
values, but must label their provenance; actual opponent secrets never enter
client features. Offline privileged labels, if later useful, are separate from
inference inputs and are not part of this implementation.

Represent geometry with the current axial/cube convention, valid-cell masks,
six ordered neighbors, off-map behavior, zones/lanes, terrain and entity
relationships. Empty cells differ from off-board cells. Position and team
normalization must be reversible for actions; use symmetries only where map
terrain, objectives and rules actually preserve them. This supports later
hex convolution or graph/NNUE features without choosing a tensor library now.

Implement full feature recomputation as the correctness reference. Optional
dirty sets/deltas and branch-local evaluator state should make later incremental
updates possible. Incremental extraction is not required for completion; if
introduced, verify it against full recomputation after moves, undo, restore,
summoning/defeat, effects, upgrades and worker cloning. A single authoritative
state change can affect global features; moved-piece-only invalidation is
insufficient. Private caches include perspective, schemas and evaluator identity.

Replacing parameter values or the evaluator implementation must not require
changing game rules, persistence schemas, command decoding or transport code.

## 9. Portable execution and bounded resources

Give search explicit node/step/depth/horizon, world-count and memory/allocation
limits, plus cancellation and a monotonic deadline supplied by the host.
Document checks, counting and overshoot granularity. Bound observation parsing,
candidate enumeration, reconstruction and feature extraction as well as search.

Keep the serial path as the deterministic reference: identical semantic input,
configuration and seed produce identical worlds, action ordering, fixed-work
statistics and chosen command. Hash-map iteration or OS thread scheduling cannot
decide ties. Parallel execution must preserve the specified ordering/reduction
or declare and test a separate reproducibility contract.

Separate fixed-work runs from deadline-limited runs. A timed result records
completed work, stop reason, whether its value is partial, and fallback use.
Cancellation must stop obsolete work and leave the live root and siblings
unchanged. Timeouts are not losses and interrupted rollouts are not completed
training examples.

Keep OS threading, filesystem/model IO and host time behind adapters. Build a
small serial `wasm32-unknown-unknown` spike and run a decision in a browser
worker using synthetic input and the same feature/evaluator contracts.
Record dependency blockers, stack/memory requirements, bundle/startup cost
and host cancellation limitations. Native and WASM fixed-work decisions and
integer features must agree; floating-point evaluators need an explicit
tolerance/tie convention.

The native client is the baseline delivery route. A measured WASM blocker is
an acceptable foundation outcome only if the host boundary is documented and
native conformance passes; a browser product claim requires the later client
plan's actual browser tests. Do not make browser thread isolation or GPU access
a prerequisite for the native foundation.

## 10. Implementation packages

All packages below are open. Owners are implementation roles, not invented
named maintainers. Each package produces evidence before downstream sign-off.

| ID | Owner | Work and deliverable | Depends on | Complete when |
| --- | --- | --- | --- | --- |
| F01 | Engine/service | Freeze current manifest, supported formats/modes/rosters and acceptance cohort; map cleaned facade and consumer boundaries | Completed cleanup assumed | Current-version matrix and exact identities reviewed; no legacy requirement or current-game reset |
| F02 | Engine/service | Typed player observation, sufficient visible knowledge, scoped IDs/digest and decision descriptors | F01 | Privacy pairs and reconnect knowledge tests pass for every current decision family |
| F03 | Engine/search | Observation-only world constructor and constrained sampling spike, including difficult continuations | F02 | No privileged input reachable; valid samples preserve known state and all required root decisions reconstruct |
| F04 | Engine/service | Shared typed command conversion, revision/decision binding, idempotency and current API adapter | F02 | Offered choices round-trip; stale/foreign/duplicate/conflicting cases have exact expected effects |
| F05 | Engine/service/storage | Prepared transition and storage acceptance conformance using current SQLite and injected failures | F01, F04 | Durable ordering and crash/retry recovery matrix passes without exposing candidate state |
| F06 | Search | Feature schema, full recompute extractor, weighted evaluator and replaceable value/policy interfaces | F01, F03, F04 | Deterministic reference tests, terminal semantics, candidate-independent value and replacement demonstration pass |
| F07 | Engine/search/client | Bounded serial execution, branch ownership, host adapters and native/WASM spike | F03, F06 | Budget/cancellation/nonmutation tests pass; native reference works and browser feasibility is measured |
| F08 | Engine/service/search | Headless integrations, current-game regression matrix, evidence report and handoff contracts | F02–F07 | All mandatory acceptance rows below pass and three follow-on plans have usable inputs |

F01–F04 form the first implementation slice. Prove observation reconstruction
before promising local-client feature coverage. F05 and the evaluator work can
then proceed independently where dependencies permit. Keep semantic rule fixes,
representation optimizations and model experiments in separate changes.

## 11. Completion evidence and acceptance

Create a reproducible acceptance report with source commit, commands/toolchain,
content/schema identities, declared fixtures, expected/executed/missing/failed
counts and raw results. Use synthetic fixtures admitted under the current
identity, including both generated games and current-format saved checkpoints.
Existing current-format test cases may be reused; do not copy real player
credentials or private live matches into public tests.

The cohort must cover each currently registered hero and map, every supported
mode and roster size, all current prompt/decision families and the special cases
below. Coverage need not be the full Cartesian product, but the manifest must
show what exercises each requirement. No empty filter, smoke-only run or
undocumented fallback closes a selected acceptance row.

| Gate | Evaluation | Required result |
| --- | --- | --- |
| A01 Dependency boundary | Build/run engine and headless search without server, SQL drivers, Python or ML frameworks; inspect dependency graph | All ordinary native consumers work through the shared contracts |
| A02 Current game continuity | Restore current cohort, issue commands, save/restart/retry, reconstruct replay and continue to comparable outcomes against the frozen baseline | Exact current state, ordered events, physical bindings and scoped results preserved; no reset/lost receipt |
| A03 Privacy and reconnect | Paired states vary opponent hands/commitments/upgrades/secret RNG while visible facts stay fixed; actor/teammate/opponent/spectator cases; reconnect after reveals | Equal semantic observation/actions/features and fixed-seed client results; no forbidden fields in payloads/diagnostics |
| A04 Action coverage | Planning/second card, readiness/deployment, upgrades, all prompts and optional skip; serialize candidate to current command and apply to authoritative fixture | All offered commands round-trip; caps are reported as pruning; hidden-dependent outcomes do not leak before play |
| A05 World reconstruction | Multiple seeded worlds for every cohort decision; copied cards/spells/effects, multi-piece heroes, simultaneous/team decisions and hidden upgrades | Engine invariants and observable roots match; hypothesis state isolated; no secret-state access or unreported fallback |
| A06 Durable command semantics | Duplicate/conflicting IDs, stale revisions/tokens, wrong seat, concurrent accepted commands, clock expiry, rollback invalidation; failures before/during/after commit | At most one transition per accepted command ID; receipt/checkpoint/journal agree; failures have specified nonmutation/recovery |
| A07 Evaluation seam | Weighted evaluator plus a second deterministic test evaluator; terminal/draw, team perspective, candidate reorder, unknown masks, geometry and parameter mismatch cases | Swappable without rules/storage changes; finite bounded scores and correct terminal priority; no candidate-dependent value |
| A08 State isolation | Clone siblings, speculative apply/discard, checkpoint restore and any incremental feature changes across cohort | Unchanged root/siblings, exact visible physical bindings/order; incremental results equal full recomputation if implemented |
| A09 Bounded execution | Node/step/world/memory limits, already-cancelled work, cancellation during each expensive phase, deadline before first score | Explicit stop/fallback semantics, bounded overshoot, no live mutation or false completed-work count |
| A10 Cross-adapter integration | Same observation-only native agent through in-process adapter, serialized in-memory transport and real disposable service connection; storage close/reopen | Same fixed-work decision and accepted command semantics without private checkpoints crossing the transport |
| A11 WASM feasibility | Compile dependency slice and attempt worker execution; compare features/decisions to native if viable | Recorded runnable proof or precise blocker/host-boundary decision; native acceptance cannot be waived |
| A12 Performance and correctness | Required quick/service/full-reference/restore tiers for changed boundaries; fixed-work benchmark and timed-response report | No correctness/identity failures or missing selected cases; measured costs and supported limitations recorded |

Measure observation projection/bytes, reconstruction/sampling, candidate
enumeration, feature extraction/evaluation, clone/restore, search throughput,
cancellation, startup and peak memory separately. Record hardware/OS/compiler/
profile, packs, states, seeds, horizon, evaluator and node/step/world counts.
Use at least five fresh-process samples for startup/memory and at least thirty
timing samples per hot-path case after warm-up; retain raw distributions and
medians/tails without claiming precise tail latency from tiny samples.

Compare against F01's same-work baseline on the same hardware/profile where
possible. Distinguish added-contract overhead from a changed search workload.
Document material regressions and any explicit accepted tradeoff before closing
F08. This plan establishes a measured baseline, not an untested simulations/sec,
Elo or latency promise. Future structural optimizations must preserve work
semantics and use this harness for before/after comparisons.

### Final checklist

- [ ] F01–F08 deliverables and A01–A12 evidence are recorded, with the stated
      WASM feasibility exception limited to A11.
- [ ] Current games and current frontend commands pass the baseline comparison;
      observations and real client search never require private checkpoints.
- [ ] Native in-process and transported consumers use the same contracts and
      produce reproducible fixed-work results.
- [ ] Current persistence passes failure/restart/retry conformance.
- [ ] The parametric evaluator is replaceable and feature correctness has a
      full-recompute reference.
- [ ] All three follow-on plans can reference these contracts without inventing
      another game state, move encoding, visibility model or evaluator meaning.

Writing or approving this document does not mark those boxes complete. F08 closes
the foundation implementation only when its evidence exists.

## 12. Follow-on plans and reminder

| Order | Plan | Foundation handoff | Reserved decisions |
| --- | --- | --- | --- |
| 1 | Shared foundation — this document | Current identity, observation/decision/command, transitions, features, budgets and harness | Implement F01–F08 |
| 2 | [Player-run bot client](CLIENT_SIDE_BOT_EXECUTION_PLAN.md) — plan written; implementation pending | Observation-only native agent, transport and host contracts | Three stages: native Rust client, browser WASM, then local neural inference gated on the full NN implementation |
| 3 | [Database, training data, statistics and Elo](DATABASE_TRAINING_STATISTICS_PLAN.md) — plan written; implementation pending | Private checkpoint and transactional acceptance/recovery contract | Operational PostgreSQL, derived statistics/rating database, immutable training datasets, purpose/lineage and isolated testing sessions |
| 4 | Neural-search preparation — pending | Features, geometry, value/policy seam, sampled worlds and measured search boundaries | Data structures/optimization, NNUE versus hex convolution/graph experiments, incremental evaluation, search/training objectives and promotion evidence |
| 5 | Rules/oracle/Python updates — reminder after plans 1–4 are written | Artifact/version identities and current-game compatibility boundary | Source-of-truth changes, rule corrections, oracle/runtime updates, regeneration, current-game transition policy and release validation |

**Reminder trigger:** after finishing the first four planning documents, remind
the user to write plan 5. This is a sequence reminder, not a scheduled job.
Do not fold that update procedure into the foundation or silently repin rules/
oracle/Python while implementing it.

Related source and prior work: [cleanup plan](RUST_ENGINE_CLEANUP_PLAN.md),
[native workspace](../rust/README.md),
[live game boundary](../rust/crates/goa2-core/src/interpreter/live_game.rs),
[scoped views](../rust/crates/goa2-core/src/interpreter/live_game/view.rs),
[current bot entry point](../rust/crates/goa2-core/src/interpreter/live_game/bot.rs),
[service orchestration](../rust/crates/goa2-game-server/src/game.rs),
[current persistence](../rust/crates/goa2-game-server/src/store.rs),
[existing AI architecture](AI_SEARCH_AND_LEARNED_RUNTIME.md) and
[private search-state lessons](BOT_SEARCH_STATE.md).
Those earlier Python/search documents are prior art, not a requirement to retain
their model artifacts, implementations or historical compatibility paths.
