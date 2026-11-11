# Database, training data, statistics and Elo plan

Status: proposed implementation plan; no database migration or data admission is
claimed complete. Written 11 November 2026. This is plan 3 of five, following
the [shared foundation](SHARED_RUST_FOUNDATION_PLAN.md) and
[client-side bot execution](CLIENT_SIDE_BOT_EXECUTION_PLAN.md).

## 1. Goals and decisions

Optimize three workloads without making one physical database serve every
access pattern:

1. Live web play: short, durable, concurrent transactions; scoped reads; restart,
   retry, clocks and controller ownership.
2. NN training: immutable, reproducible, large sequential reads of decisions,
   observations, targets and provenance.
3. Player/hero/map statistics and Elo: indexed historical queries, rebuildable
   aggregates, audited eligibility and deterministic ranking updates.

**Recommended starting architecture:** PostgreSQL for operational game storage;
a separate PostgreSQL database for statistics/rating projections; private object
storage for canonical archives and versioned columnar training datasets.
Local filesystem storage may implement the object-store interface for development.
Start without Kafka, a warehouse or a distributed data lake. Introduce a separate
columnar analytics engine only if measurements justify it.

Operational and analytics databases may initially share a managed PostgreSQL
cluster with separate roles, pools, quotas and timeouts, provided interference
tests pass. They remain separate logical databases. Shared cluster CPU/IO is
not physical resource isolation: move analytics to a separate instance before
claiming isolation under loads that violate live-service budgets.
Training workers never scan the live database directly.

Reuse important semantic structure across all stores: origin/game/lineage IDs,
current rules/content identities, participants, ordered revisions and decisions,
outcome/result versions, purpose/eligibility, artifact hashes and schema versions.
Share contracts and IDs, not one universal table of tensors, accounts and live
engine objects.

Assume cleanup is finished. The engine already runs separately from the service,
and the current service already persists checkpoints, receipts and replay in
SQLite. Keep the foundation's prepare/commit and scoped-observation boundaries.
This plan extends persistence, queryability and downstream processing; it does
not put database calls inside the Rust rules engine.

**Current games must keep working.** Preserve current-format active games,
credentials/links, decisions, revisions, clock semantics, replay, receipts and
public shares during migration. No game reset, oracle repin, new game rules or
support for discontinued historical formats is required. This documentation
extends `14ad80a4`; its source base predates the independent cleanup continuation.
Resolve source paths against the completed cleanup when implementation starts.

## 2. Data flow and authority

```text
Authenticated command / authoritative timer
  -> Rust prepare on immutable current game
  -> PostgreSQL transaction:
       revision + current checkpoint + replay operation + receipt + outbox
  -> install committed state and send scoped update

Committed outbox / verified archive export
  -> sanitized analytics facts -> statistics database -> APIs and Elo ledger
  -> restricted canonical archive -> validated dataset builder
       -> immutable Parquet shards + admission manifest -> NN trainer

Isolated test/self-play/evaluation services
  -> the same contracts with different environment/purpose/lineage
  -> separately admitted test or training artifacts; never automatic ranked input
```

The operational record is authoritative for accepted gameplay. Analytics,
rankings and datasets are derived, versioned products. Neither a reporting query,
client-uploaded trace nor training process may mutate an active game's state.

The initial implementation stores the complete **current** checkpoint and
replay start in PostgreSQL, with ordered replay operations and periodic immutable
anchors. It does not require object storage for accepting a live move.
Completed archives move to object storage asynchronously after verification.
Do not retain an expensive full checkpoint for every historical revision unless
restore/latency measurements justify it; do retain everything needed to rebuild
every promised replay boundary.

Client wire events alone are insufficient for deterministic replay. Preserve the
current private operation record, explicit accepted times, timeout entropy,
rewind/patch/pause operations, start snapshot and per-revision digests, or a
proved equivalent current representation.

## 3. Identity, purpose and lineage shared by all stores

Define a versioned canonical game record and export schema. Use stable IDs
independent of machine, directory, database row address or content compression.

| Field/group | Meaning and invariant |
| --- | --- |
| Environment/namespace | Deployment and data authority: production, staging, CI, developer, training or evaluation; server-assigned, not an arbitrary client flag |
| Origin/game ID | Globally identifiable origin plus stable game ID; preserve current public game IDs, which need not be UUID strings |
| Lineage | Root origin, parent game/revision/digest, fork/restore reason and branch ID; clones/forks receive new IDs while retaining ancestry |
| Session/run ID | Optional enclosing manual test, automated test, self-play, benchmark or evaluation run; not a replacement for a game ID |
| Current runtime identity | Rules, oracle/source, content/catalog, checkpoint/resume/replay and execution compatibility identities from the foundation |
| Participants/seats | Stable internal seat, team and hero references; account/bot identity separately; controller history over revision intervals |
| Revision/decision | Strictly ordered accepted game revision, decision/token identity, actor/operation and exact accepted time |
| Result version | Proposed/certified/retracted/corrected outcome, terminal revision, reason and result digest |
| Purpose | Ranked human play, unrated human play, analysis, manual test, automated test, self-play, model evaluation or benchmark |
| Permissions/eligibility | Versioned statistics, rating, export and training-admission decisions with reasons and provenance |
| Payload identity | Codec/schema, raw digest, byte/element limits and storage reference; raw versus compressed hashes distinguished |

Purpose is selected by trusted creation/provisioning policy. An HTTP body cannot
declare itself production, certified or TRAIN-admitted. Freeze original purpose
and provenance; record subsequent eligibility/correction decisions as audited
records rather than rewriting a test game's origin.

Keep environment, purpose, completion status, visibility and admission separate.
For example, an unrated human game can be useful for an approved training
dataset; a completed model-evaluation game must stay out of TRAIN; a training
game may be decisive without being a public ranked match. One boolean named
`is_test`, `finished` or `valid` cannot encode these distinctions.

A restore/fork does not become an independent ranked match by receiving a new
game ID. Analysis/test descendants inherit exclusion from competitive rankings.
Track a stable leakage-group identity across forks, duplicate imports, repeated
starts, seat-swapped variants and reused evaluation seeds. A new export filename,
different compression or new dataset version cannot erase ancestry.

Use the content manifest to key hero/map/card definitions:
`(content_identity, definition_id)`. Display names are not foreign keys.
Cross-version statistics require an explicit reviewed mapping; never silently
merge changed hero rules because the printed name stayed the same.
Game-local physical instances remain distinct from definition IDs.

Player accounts use stable internal identifiers, with names/profile data outside
replay payloads. Account authentication/verified seat-to-account linking is a
prerequisite for player Elo, not an assumed existing feature of bearer-token
games. Add or integrate that identity boundary before enabling the ladder;
current guest games remain playable and unrated. Seats may be guests or bots; do not infer an account from a
display name, IP address or bearer token. Bot identity includes strategy/model
and execution-placement provenance where known. Human, client-bot, server-bot
and automatic timer actions remain distinguishable through controller history.

## 4. Operational PostgreSQL structure

The following is a logical schema, not executable migration SQL. Final migrations
must define types, keys, constraints, row limits and indexes and pass D01.
Pin the PostgreSQL major/minor baseline and any extensions in the environment/
CI/deployment manifest; do not infer server compatibility from a client-driver
version or an unpinned container tag.
Every game-scoped key includes its namespace; child foreign keys include that
same scope. Prefer typed relational keys/metadata plus lossless versioned
engine bytes, rather than normalizing every interpreter field into SQL rows.

| Table/group | Important keys and contents |
| --- | --- |
| `namespaces` | Environment, authority, allowed purposes, retention class and deployment identity |
| `sessions` | Session/run ID, namespace, purpose, creator/tool identity, fixture/cohort/experiment config digest, time bounds, status and expiry |
| `game_definitions` | Admitted content/rules/runtime identity and immutable artifact references; hero/map definition inventory |
| `games` | PK `(namespace, game_id)`; purpose/session/lineage, map/mode/config identity, lifecycle, latest revision/checkpoint digest, active result version and timestamps |
| `game_seats` | PK `(namespace, game_id, seat_id)`; engine hero/seat key, team, definition, optional account/bot subject and initial assignment |
| `seat_control_history` | Seat plus effective revision/control generation; human/bot/system type, policy/model identity and authorized handoff provenance |
| `game_state` | One current checkpoint per game: revision, codec/schema, lossless bytes, digest and byte size; transactionally paired with `games` |
| `replay_anchors` | Game/revision key; immutable start/periodic checkpoint bytes and digest, retained until verified archival coverage |
| `game_journal` | Unique game/revision; operation/actor/time, ordered replay payload, previous/current digests, branch/rewind linkage and outcome transition metadata |
| `command_receipts` | Unique game/authenticated seat/action ID; canonical command identity, accepted revision and original scoped reply bytes/digest |
| `game_results` | Unique game/result version; terminal revision, winner/draw, termination reason, certification/eligibility decision, superseded version and correction provenance |
| `seat_controllers` | One current owner per delegated seat; placement, scoped session, generation and server-time lease expiry; independent of gameplay revision |
| `game_coordinators` | Game scheduling owner/generation/lease for server bots and deadlines; separate from player-side controller ownership |
| `access_grants` | Revocable scoped player/spectator/controller grants; verifier/token metadata and expiry, isolated from game/analytics payloads |
| `replay_shares` | Public-share token verifier/reference, scope, generation, immutable baked-projection reference, revocation and current metadata |
| `archive_catalog` | Game/revision coverage, archive schema/digests, private/public class, object version/size, verified state and retention dependencies |
| `outbox` and deliveries | Unique event ID, game/revision or result version, event kind, restricted payload/reference and per-destination delivery/claim state |
| `eligibility_audit` / tombstones | Statistics/rating/training decisions, consent/retention changes, invalidations and deletion propagation |

Account identity, authentication and private tokens do not belong in every
checkpoint, replay or training shard. Store token verifiers for authentication;
encrypt any credential material that must remain retrievable for current
authorized creation/link-retry semantics. Keep encryption keys outside SQL and
test their backup/rotation/restore path. Do not invalidate current links merely
to simplify the import.

Use `BIGINT` for bounded revisions/counters with explicit limits. Preserve
existing full-width seeds/timestamps inside their lossless payloads; use checked
`NUMERIC`/byte encodings where a separately indexed field requires unsigned
64-bit range. No cast may silently narrow values. Use UTC database timestamps
for operational times and preserve the exact engine millisecond representation
needed by replay.

Store authoritative checkpoints/operations in `BYTEA` or an equivalent
lossless encoding with explicit schema. JSONB may reorder dictionary keys and
is not automatically a valid engine round-trip. JSONB is appropriate for
bounded non-authoritative metadata after its semantics are specified.
Ordering, null versus absent, physical aliases and raw/semantic digests must
survive storage exactly as required by the current runtime.

### Initial indexes and queries

Start with these access paths; justify additions with measured query plans:

- Game PK; active games by namespace/status and keyset cursor.
- Seat/account game history by namespace/account/created time/game ID.
- Journal and anchors by game/revision; latest anchor at or before revision.
- Receipt unique key and accepted revision lookup.
- Active controller/coordinator generation and lease-expiry work queues.
- Undelivered outbox work by destination/state/next-attempt time.
- Session/purpose/expiry indexes for bounded cleanup, plus lineage ancestry.
- Verified archive/result lookup by game and immutable version.

Use partial indexes for active/unfinished work where helpful. Avoid indexing
every JSON path or neural feature. Paginate histories by stable keyset rather
than unbounded OFFSET. Do not partition tables before retained volume/query
evidence warrants it; choosing partition keys must not break required uniqueness
or foreign-key semantics.

Load games on demand into a bounded service cache. The database remains the
source of truth; startup must not deserialize every historical game or fetch
every private archive. Expired clocks/recovery and owned scheduling work need
a bounded indexed scan. An evicted active game remains resumable from its
current checkpoint and pinned rules/content.

## 5. Transactional gameplay and multiple service instances

Keep the foundation's acceptance contract. A typical write path is:

1. Authenticate and scope the operation. Reconcile an existing action ID before
   treating an accepted retry's old revision as a fresh stale command.
2. Load/pin the current revision and prepare on an immutable copy outside a
   database lock. Search/evaluation never occurs while holding a SQL lock.
3. In a short transaction, lock the game/control rows in a consistent order.
   Recheck source revision/digest, permission/controller generation and
   authoritative time. Stale candidates are discarded/reprepared; they are not
   applied against whatever newer state happens to be in memory.
4. Persist checkpoint/revision, complete replay operation, receipt, result change
   if any, and outbox event atomically. A fresh accepted state-changing command
   advances the game revision exactly once. Validation failures do not create
   successful receipts or partial journals.
5. Commit, then update the in-memory copy and publish scoped acknowledgements/
   notifications. On uncertain outcome, recover by durable action ID/revision.

Use row-level locking or tested conditional revision updates at READ COMMITTED
with the needed constraints; do not assume an in-process mutex protects another
service instance. Serialize all state-changing paths, including timeout, rewind,
patch/consensus, restoration/replacement and approved game correction.

Get acceptance/lease time from one authoritative deployment clock source.
Recommended multi-instance policy is database-evaluated time in the short
transaction, recorded as the explicit engine time. Avoid long-transaction
start timestamps masquerading as the actual acceptance time. Deadline
schedulers may estimate wake-up locally, but acceptance checks authoritative
expiry again; lock contention or client clock skew cannot extend a deadline.
Document downtime behavior and test timed-game migration/restart.

For server bots/deadlines, use a renewable per-game coordinator lease with
monotonic fencing generation. Its expiry/renewal is independent of gameplay
revision. Every scheduled mutation verifies its generation at acceptance.
Player-side bot ownership uses the separate seat fence from plan 2; moving a
seat to a local client stops its server search without disabling normal
authoritative deadlines. A lease alone without a fence cannot prevent a late
previous owner committing.

Multiple API instances may serve a game, but each acceptance reads/checks the
durable head. Cross-instance updates invalidate stale caches and notify local
WebSockets. Initially PostgreSQL LISTEN/NOTIFY may supply wake-up hints; durable
outbox/cursors and periodic resync supply correctness. Lost notifications are
not lost gameplay. A fan-out process or broker can replace hints later without
changing command semantics.

On commit success followed by process death, memory/broadcast failure or lost
reply, the receipt and head recover the accepted action once. A transaction
failure leaves all authoritative records unchanged. Timeouts/retries must be
bounded, with backoff and explicit database-unavailable responses. Do not keep
accepting divergent in-memory commands when durable storage is unavailable.

## 6. Ordered change feed, archives and derived data

Write an outbox entry in the same transaction as the state/result it describes.
Delivery is at least once, with idempotent consumers, not a claim of global
exactly-once transport. Use a per-game ordered revision/result version and a
unique event ID. Different games may process concurrently.

Claim work with bounded leases/conditional state changes. Each destination
acknowledges only after its own durable sink transaction or immutable manifest
publication. Commit destination facts and its processed-event record together;
a crash before source acknowledgement can redeliver harmlessly.
Handle out-of-order delivery by predecessor checks/reload or bounded buffering.

**Do not use MAX(sequence_id) as proof that every lower transaction committed.**
PostgreSQL sequence allocation can be ahead of commit order. Use acknowledged
per-event/per-game state, or a properly implemented commit/CDC watermark.
Backfills and incremental processing must share a documented consistent snapshot
plus catch-up protocol, with no missing in-flight commits.

Outbox payloads crossing to analytics contain only approved facts/references.
A statistics worker does not receive private checkpoints or token tables.
A restricted archiver/dataset builder may read authorized private records through
its own role; public notifications and generic logging never include them.

### Archive publication

Build a bounded immutable archive from a consistent game/revision range.
Include the exact current runtime/content identity, start/anchor, ordered replay
operations, digests, purpose/lineage and certified-result coverage.
Keep private archives separate from baked public share projections.

Stage objects, verify raw/compressed size and hashes and replay coverage, then
publish an immutable catalog/manifest reference. Object storage is not in the
gameplay transaction. Failed upload/catalog publication leaves live rows intact;
orphan staging objects are garbage-collected after a grace period.
Delete/archive database history only after every required dependency, active
share and retained dataset has a valid policy-approved replacement/reference.

Exporter/analytics outages must not block normal moves while durable capacity
remains available. Monitor backlog age/bytes, retries, dead-letter causes and
disk headroom. Define capacity/backpressure alarms before pruning anything;
never silently discard replay/receipts or acknowledge non-durable moves to
keep a full disk apparently healthy.

A corrected/reopened game emits a new result version and downstream invalidation.
Do not rewrite earlier journal history. A rewind records its source/target and
marks abandoned trajectory branches. Analytics counts the active certified
result; training follows its admitted branch policy and does not attach the new
winner to every abandoned branch automatically.

## 7. NN training storage and admission

Use a registry/metadata database plus immutable object storage. Store raw
verified game archives separately from derived training features/targets.
Use Parquet shards (Arrow-compatible nested/columnar records) for high-throughput
training reads; retain lossless replay payloads/digests outside those normalized
feature tables. A Parquet projection is not automatically an exact checkpoint
serialization.

Initial training-side structure:

| Dataset entity | Contents and invariant |
| --- | --- |
| Source game registry | Origin/game/result version, authoritative archive digest, lineage/leakage group, purpose, completion/quality and usage/consent decision |
| Decision records | Game/branch/revision, root viewer/seat/team, decision kind, observation/feature schema, typed selected action and stable legal-candidate mapping |
| Observation/feature shards | Only information permitted at that decision; ordered/masked entity/card/hex features; unknown masks and geometry identity |
| Target shards | Versioned value/policy targets, boundary/horizon, outcome/censoring, weighting and provenance; explicitly separate from inference inputs |
| Search evidence | Optional policy/model/seed/budgets, visits/values/completed work/stop reason and trust source; absence is valid |
| Dataset versions | Immutable source/result/admission manifest, split mapping, transforms/extractor code, schema, shard counts/hashes and validation report |
| Training runs/model registry | Dataset version, initialization/ancestor identity, hyperparameters/software, run outcome and model artifact/evaluation references |

Partition shards by dataset/schema/split and enough coarse provenance to support
bounded reads. Avoid one tiny file per decision or one enormous whole-corpus
archive. Choose shard size/row-group/compression from measured worker IO and
memory. Stream game-by-game and shard-by-shard; no permanent full-corpus
extraction or whole-corpus RAM cache. Producers write to ignored external
storage; commit generators, schemas, manifests/indexes and small synthetic cases.

### Admission and trust

A completed game is not automatically a training example. A trusted builder
validates replay/content integrity, eligibility/usage, decision coverage,
result version, termination reason and sample provenance before admission.

Gameplay commands and outcomes accepted by the server are authoritative
gameplay evidence. Client-reported search visits, values, policy probabilities,
features, model names or NN outputs are **untrusted telemetry**. Never use those
as verified policy targets because the move itself was legal. Either generate
targets in a controlled reproducible run with provenance, or mark them untrusted
and exclude them from target admission. Human actions can be admitted as
imitation labels under their own declared recipe; that is not a search-policy
target or proof of best play.

Reconstruct observations with the exact viewer and knowledge at each decision.
Finished private replay projections may expose future/revealed secrets and
must not become policy input directly. Prevent future cards, hidden commitments,
final-state features and outcome labels from leaking into inference columns.
Privileged target/critic data, if later used by the NN plan, has a separate
schema and explicit access/usage contract. Any optional player/rating/history
features use point-in-time data available at that decision, not post-game Elo
or aggregates rebuilt using future matches.

Strip names, credentials and unnecessary account identifiers from trainer-facing
shards. Use dataset game/seat references and a restricted source/lineage mapping
for audit and retraction. The trusted registry retains consistent group/split
membership across dataset versions; pseudonymizing or rekeying exports must not
permit a duplicate start to cross into a held-out split.

Distinguish decisive normal play, rules-defined draws, timeout/adjudicated
results, resignation, censored turn limits, interrupted runs, exceptions and
corrupted replay. Do not encode every non-win as a draw/loss or every stopped
search as a completed rollout. Dataset recipes declare included termination
classes and masks/weights; default reference training admission uses verified,
complete normal games. Other classes need an explicit validated recipe.

### Splits, duplicates and reproducibility

Assign TRAIN/validation/test/evaluation membership by a versioned registry and
stable leakage-group key before row generation. Keep all decisions, forks,
duplicate imports, seat-swapped repeats and related seed/start families together.
Use group-level deterministic assignment, not a random row split or a fresh
game ID. Validation/test/arena/promotion seeds are never automatically admitted
to TRAIN, even if the same game was also exported as self-play.

A dataset manifest freezes exact source versions and admissions, extractor/
feature/target identities, split algorithm/configuration, ordering, weights,
row counts, file hashes and accepted exclusion reasons. Rebuilding the same
manifest must reproduce semantic rows/counts/hashes even if physical compression
or storage paths differ. Record ancestor-model/data provenance; do not claim a
held-out set was never seen by inherited weights without that evidence.

Training reads a fixed admitted dataset snapshot, not a moving live-game query.
New games appear only in a new dataset version. A corrected result, consent
change or tombstone invalidates affected source/admission records and produces
a replacement/retraction manifest; it cannot silently change already referenced
shards. Downstream run/model records retain the affected lineage for review.

The database plan delivers storage, extraction and admission contracts plus
synthetic end-to-end tests. Actual NN architecture, training targets, optimization,
training runs and model promotion remain plan 4/full NN implementation work.

## 8. Statistics structure and definitions

Build a sanitized analytics read model with a separate role/database and
per-projector schema/version/cursor. It must be rebuildable from approved
canonical records, not from user-uploaded winner claims or scraped UI logs.

| Fact/dimension | Grain and use |
| --- | --- |
| `fact_games` | One active certified game result; map/mode/rules/season, purpose, duration, termination, team size and eligibility |
| `fact_seats` | One game/result/seat appearance; team, hero definition, player/bot subject and controller intervals |
| `fact_player_games` | One game/result/account/team contribution; deduplicates a person controlling multiple seats |
| `fact_decisions` | Optional sanitized revision/decision measures; only approved visible metrics, not raw hidden command JSON |
| Hero/map/config dimensions | Versioned catalog definitions and reviewed cross-version mappings |
| Aggregate tables | Player histories, hero/map/matchup cohorts and time-bucketed measures with source/result-version provenance |
| Projection state | Processed event IDs, current source version, schema/transform version, rebuild run and lag/coverage |

Controller changes are recorded over time. Do not label an entire game human
or one particular bot model based only on the seat's final owner. Mark mixed/
assisted/automatic play according to a declared statistics definition.
Account merge/rename changes display/identity mappings, not original game
provenance; it requires a rebuildable audited correction.

Define each metric's denominator and unit:

- Player games/wins/losses/draws count distinct eligible games, not the number
  of heroes that account controlled. Keep completion/abandonment/timeouts separate.
- Hero win rate counts eligible hero appearances with their team's result;
  document duplicate/same-hero opposing appearances and stratify by current
  rules, mode, map, roster size and human/bot cohort.
- Map statistics include games, side win/draw rates and mode/roster distribution.
  A global map win rate without a defined side/participant denominator is not
  meaningful.
- Matchups/synergies expose sample size and uncertainty; observed correlations
  are not causal balance conclusions or a replacement for player Elo.
- Action/turn/clock durations exclude or label pauses, replay rewinds,
  inactivity and synthetic/frozen clocks; wall time is not thinking time.

Support account histories, hero/map summaries and leaderboards with indexed,
bounded queries and keyset pagination. Filter purpose/eligibility through
approved views/queries by default; a caller cannot accidentally include CI or
self-play by omitting a UI checkbox. Public aggregation must not reveal private
active decisions or unrestricted replay state. Apply account/game visibility,
usage policy and any necessary small-cohort suppression before publication.

A correction applies the difference or replaces old facts transactionally,
keyed by origin/game/result version and projector version. A duplicate or old
message cannot add another win. Rebuild into a new version, compare totals,
then switch the read-model pointer; never expose a half-rebuilt leaderboard.

Stats are eventually consistent. APIs report source coverage/version and
`as_of`/lag where relevant. A temporary stats outage must not interrupt live
moves, and stale aggregates must not be presented as the latest committed game.
Start with PostgreSQL aggregates/indexes. Consider a read replica/columnar
warehouse only after realistic queries and concurrent-load measurements.

## 9. Elo ranking and result certification

Keep Elo as a separate, versioned, reproducible ledger with explicit pools.
Hero/map statistics are not silently converted into hero/map Elo.
A ladder pool identifies season, rules/content compatibility, mode/roster rules,
rating policy version and allowed participant class.

### Initial ranked policy

Propose a conservative first player ladder: two opposing equal-sized teams,
one distinct authenticated ranked account per seat, no bots or seat takeover,
fresh eligible production ranked games from an allowed setup, and no cheats,
analysis restore, test lineage or disallowed corrections. All other current
games still work and may have unrated statistics. Guest/multi-seat/bot/mixed
games require a separate future pool/policy rather than contaminating player Elo.

Seal ranked intent, pool/policy, roster and setup eligibility before play.
Record later disqualification reasons. Server-known controller type is not
proof that a human never consulted external software; terms, account verification
and abuse review are distinct policy decisions. Client telemetry cannot certify
a competitive result. Suspicious/unsupported cases may be quarantined while
gameplay remains intact.

The rules engine supplies terminal result and reason. A certification workflow
checks result/head consistency and ranked eligibility. Map all allowed victory/
draw/adjudication reasons explicitly; an unknown winner type or arbitrary timeout
report does not invent a ranked win. A reopened/corrected game retracts its prior
certification and triggers the defined recomputation path.

### Proposed `team-elo-v1`

For the initial equal-team, distinct-player pool, use each team's arithmetic
mean of pre-game player ratings. Define:

```text
R_A = mean(pre-game ratings on team A)
R_B = mean(pre-game ratings on team B)
E_A = 1 / (1 + 10 ^ ((R_B - R_A) / 400))
E_B = 1 - E_A
S_A = 1 for A win, 0.5 for draw, 0 for A loss
S_B = 1 - S_A
new_rating_i = old_rating_i + K * (S_team(i) - E_team(i))
```

Proposed initial values are 1500 and fixed K=32 per player, versioned in the
policy rather than hardcoded forever. Equal team sizes and equal K make the
sum of team updates symmetric. Do not apply this formula unchanged to unequal
teams, multiple seats per account or free-for-all outcomes. Define a new policy
before supporting those formats in a rated pool.

Specify numeric precision, exponent evaluation/rounding and display rounding.
Preserve authoritative internal precision; repeatedly rounding display integers
must not change later updates. Provide golden calculation vectors so a rebuild
on another worker produces the same ledger.

Initial values/K/eligible termination and map rules need a recorded product
decision before the first ranked release; this proposal makes that decision
reviewable. Avoid inactivity decay, placement modifiers or hero-dependent K
until a separately versioned policy and tests justify them.

### Ledger and recomputation

Use `rating_pools`, `rating_policy_versions`, `rating_events`,
`rating_changes` and `rating_current`/snapshots. One event binds pool,
game/result version and certified outcome to ordered pre/post player ratings,
expected scores, K/policy and numerical recipe. Its uniqueness prevents duplicate
delivery or two workers updating the same game twice.

Give each rated game a stable key (pool, origin, game ID) and assign its order
slot on first certification. Corrections/result versions reuse that same slot;
they never append a second match. A retracted/ineligible game leaves an empty
slot until a valid replacement certification exists. Keep the full event audit,
but each active ledger generation has at most one effective contribution per
rated game and one change per participant in that contribution.

Persist this deterministic per-pool order once, including tie/late-arrival rules;
do not reorder history on every wall-clock query. Serialize pool application or lock subjects/pool in a
fixed order and atomically append all participants' changes plus the processed
event/current ratings. Key materialized changes by pool, ledger generation,
rated-game slot and participant so a rebuild retains old generations without
colliding with or double-counting the active one. Do not update one team's
ratings then calculate the
other team's expectation from already updated numbers.

If an earlier result is corrected, reverse-and-continue is not generally enough:
later Elo expectations depended on it. Recompute the affected pool suffix from
an audited snapshot using corrected certifications and the same recorded order;
publish a new ledger generation atomically. Catch up/buffer new rating events
against a declared certification watermark and fence the pointer switch so a
concurrent result cannot disappear between rebuild and activation. Retain the
previous generation for audit. Late certified games append at their recorded
order; retroactive ordering requires an explicit rebuild.

Current leaderboard reads only a complete active generation. While a correction
invalidates that generation, mark the pool rebuilding/provisional or withhold
its competitive view; do not present the invalidated ratings as freshly certified.
Ratings may lag game completion while certification/projection runs. Show provisional, quarantined
and retracted results clearly. Tests must cover same-result redelivery, concurrent
workers, draw/equal/unequal strengths, correction after later games, duplicate
account attempts and clean full rebuild equality.

## 10. Testing sessions and environment isolation

Testing sessions include human exploratory sessions as well as automated tests,
benchmarks, self-play and model evaluation. They use the same game/command
contracts with explicit environment, purpose, lineage and policy.

| Session class | Default statistics | Human Elo | TRAIN admission |
| --- | --- | --- | --- |
| Production ranked human | Eligible certified competitive facts | Only after certification | Separate usage/recipe admission, not implied by ranked play |
| Production unrated human | Separate eligible unrated cohort | No | Only through explicit approved recipe/usage policy |
| Manual test / analysis / restored fork | Internal/test or private analysis only | No | No by default; intentional curated examples need a separate labelled recipe |
| CI / integration / benchmark | Test reports only | No | No |
| Controlled self-play | Separate model/run metrics | No human ladder | Only with admitted generation/split/completion provenance |
| Validation / model test / arena / promotion | Separate evaluation metrics | No human ladder | Never automatically; held-out groups remain excluded |

Purpose inheritance and database constraints prevent a test/analysis fork from
being relabelled ranked at completion. Curating a synthetic regression or
training example creates an audited derived artifact, not a falsified production
match. A completed benchmark with fixed clocks is not a real duration statistic.

### Physical and logical separation

Use separate production versus test database instances/credentials and object
buckets or equivalently enforced storage authorities. Production services never
accept a caller-supplied database URL/namespace. Test services must be unable to
connect to/write production with their credentials. A mutable test flag in a
production row is not sufficient isolation.

Within development/CI, create a disposable database/schema and object prefix per
run/worker, with unique names, scoped roles and expiry metadata. All game
child keys retain namespace FKs; test session cleanup cannot cascade across
namespaces. Production analytics exposes only approved-purpose views, and export
roles are explicit. Where row-level policies/session scope are used, verify
transaction-local scope and pooled-connection reset; no prior request's access
context may leak into the next one.

Sanitized/synthetic fixtures are the default. Copying a production backup into
a broadly accessible test environment is not an acceptable fixture shortcut.
A restore drill involving restricted production data requires its own approved
access boundary and de-identification/retention process.

### Database and pipeline tests

- Use real PostgreSQL in required integration/CI tests for transactions, indexes,
  locking, SQL migrations, leases and multiple service instances. SQLite or
  an in-memory fake cannot establish PostgreSQL conformance.
- Keep fast engine tests independent of SQL. Use a fault-injecting storage adapter
  for prepared-transition semantics and actual PostgreSQL for final guarantees.
  SQLite may remain an explicit lightweight development adapter only if both
  adapters' intended contracts are maintained; no obligation to retain it as a
  production backend after current-data migration.
- Test empty-schema creation, supported current-schema upgrades and current
  SQLite-to-PostgreSQL import. Do not build a historical migration promise.
- Inject process death and failures before/after commit, lost acknowledgement,
  duplicate/out-of-order outbox delivery, object-store outage, schema mismatch,
  expired scheduler/controller fences and clock expiry.
- Run parallel CI sessions, replay duplicate imports, fork live/test fixtures and
  attempt cross-namespace joins/claims/export/rating admission. Fail closed.
- Feed identical synthetic canonical results through live export, statistics,
  ratings and dataset admission. Check denominators, deduplication, source
  coverage, split leakage and deterministic rebuilds end to end.
- Treat a test run's genuine terminal games separately from failed/interrupted
  harness cases. Test assertions passing do not certify the generated game's
  ranked/training eligibility.

Frozen/injected logical game clocks are allowed only by the isolated test/run
policy and are recorded in its replay/manifest. They do not change operational
lease-expiry time or let a production client choose acceptance time. Verify
synthetic clock controls cannot be enabled through ordinary production routes.

Session cleanup is an explicit idempotent job with namespace/run ownership,
expiry and bounded batches. Keep failed-run evidence long enough for diagnosis;
delete only after policy/dependency checks. Report leaked sessions/objects/leases
and verify cleanup cannot touch current player games or admitted datasets.
Training experiments retain manifests/lineage even when ephemeral games are
removed. Never delete unique fixture coverage to meet a disk budget.

## 11. Privacy, access, retention and recovery

Apply least-privilege roles: live service, migrations, archiver, sanitized
analytics consumer, dataset builder, trainer and read-only reporting are distinct.
No trainer or public stats API gets the live service credential or token table.
Separate live player views, private replay, public share and training usage
authorization. Finishing a game does not automatically make private replay,
account information or training use public/permitted.

Use transport encryption, managed encrypted storage/backups and external key
management where sensitive retrievable fields require it. Audit sensitive access
and redact credentials, private payloads and secret RNG state from logs.
Expose only authorized aggregate/player data; archive access uses scoped
authorization rather than publicly guessable object paths.

Define retention by class before release: active saves/receipts; finished private
history; public shares; sanitized analytics/ratings; test sessions; raw training
sources; admitted datasets; run/model provenance; backups. Specify concrete
durations, owners and recovery objectives in the deployment record, not invented
promises in this plan. Receipt retention must cover the accepted retry guarantee.

Delete/anonymize through audited tombstones and dependency-aware processing.
Propagate to source catalogs, derived facts, dataset admissions, caches and
eligible model/run records as policy requires. Preserve nonidentifying integrity/
ledger provenance only where permitted. An immutable shard is reproducible,
not exempt from deletion: revoke access/retract manifests and rebuild/delete
affected artifacts under policy. Define backup expiry and tombstone replay so
restoring an old backup does not resurrect revoked access or erased subjects.
Do not promise removal of influence from already trained models without a
separately supported model/data policy.

Use PostgreSQL consistent backups plus tested WAL/PITR if selected for production.
Object versions/manifests and encryption keys have their own backups. Record a
recovery point where SQL archive references and verified objects agree.
Restore live games, scoped links/credentials, receipts, clocks, journals and
controller/coordinator fencing on an isolated service. Rebuild analytics and
ratings, and reconcile export/outbox progress after restore.

Rating/statistics/dataset backups improve recovery time but are not a substitute
for canonical records and deterministic rebuild procedures. A database schema
rollback uses the matching backup/runtime; do not point old SQL code at newer
schemas and hope unchanged columns are sufficient.

## 12. Current-game migration and deployment

Prefer a controlled current-state cutover over a long-lived dual-write system:

1. Freeze the current-format cohort and runtime/content identities. Inventory
   active/finished games, receipts, replay starts/entries, shares, access grants
   and any existing participant/session metadata.
2. Build PostgreSQL schema/importer and rehearse on disposable current-format
   data. Map unknown account/purpose provenance conservatively: preserve the game,
   but do not invent ranked accounts, certify old winners or auto-admit training.
3. Stop new writes briefly under a documented maintenance policy. Obtain a
   consistent SQLite backup through the current backup API; include WAL safely.
   Never copy only the main SQLite file while writes continue.
4. Import lossless blobs and all current IDs/revisions/credentials/receipts.
   Verify counts, byte/semantic digests, replay reconstruction, scoped shares and
   continued commands using the same engine. Record omissions explicitly;
   an active game or receipt omission blocks cutover.
5. Switch one authoritative writer to PostgreSQL. Resume timers/controllers
   using documented downtime semantics and new fences; no silent extra time,
   lost timeout operation or duplicate bot coordinator. Keep SQLite read-only
   as the rollback source, not a competing writer.
6. Verify real current clients/reconnect and downstream outbox delivery. Monitor
   errors, lag, database load, content mismatches and clock behavior.
7. If rollback is necessary after accepting PostgreSQL-only moves, reconcile
   those accepted commands into a proved compatible current backup or retain
   PostgreSQL until recovery. Simply restoring pre-cutover SQLite would lose
   accepted gameplay and is not a safe rollback.
8. Retire the import/old backend path only after the rollback window and
   dependency retention requirements are satisfied.

No historical schema support is needed beyond the selected current input.
Current data migration is still mandatory. Database-only migrations must not
silently transform engine rules/content or old model artifacts; plan 5 will
define the separate rules/oracle/Python update procedure.

## 13. Implementation packages and acceptance

All packages are open. Owners are implementation roles to assign.

| ID | Owner | Deliverable | Depends on | Complete when |
| --- | --- | --- | --- | --- |
| D01 | Service/data | Shared identities/schema, purpose/eligibility, current cohort, query/workload and initial capacity/recovery policy | Foundation contracts | Versioned logical/physical schemas and reviewable policy decisions; real PostgreSQL test harness |
| D02 | Service/storage | PostgreSQL adapter, current checkpoint/journal/receipt transaction and on-demand game cache | D01, foundation transition conformance | Empty/current schema tests, live command/restart/retry and byte/identity preservation pass |
| D03 | Service | Database clock, coordinator and client-controller fences, multi-instance cache/update behavior | D02, client ownership contract | Two real instances cannot double-apply timer/bot/client work; lease renewal does not create gameplay revisions |
| D04 | Service/data | Outbox/delivery, archive verification/publication and backfill protocol | D02 | Crash/reorder/duplicate/object-outage tests lose no acknowledged data or source coverage |
| D05 | Analytics | Sanitized facts/dimensions, scoped queries, aggregates and corrections/rebuilds | D01, D04 | Exact reference denominators, eligibility, lag reporting and deterministic rebuilds |
| D06 | Service/analytics | Ranked policy/certification, team-Elo ledger, pools/seasons and correction generations | D03–D05 | Golden vectors, duplicate/concurrent/correction tests and full rebuild equality |
| D07 | ML/data | Canonical training exports, registry, Parquet builder, split/admission/invalidation contracts | D01, D04 | Streaming synthetic dataset build/rebuild, no inference-label leakage or held-out/test contamination |
| D08 | Service/test/data | Testing sessions, namespace roles, parallel integration harness and expiry/cleanup | D01–D07 as integrated | Every purpose/denial path tested; test credentials cannot reach production |
| D09 | Operations/privacy | Retention/tombstones, SQL/object/key backup and isolated restore/rebuild drills | D02, D04–D08 | Concrete recovery objectives/policies and measured dependency-consistent restore evidence |
| D10 | Release/service/data | Current SQLite migration/cutover, combined-load and real-client acceptance, documentation | D02–D09 | All gates below pass; current games continue and production decisions are recorded |

Deliver D02 first for reliable live storage; D04 then enables analytics and
training independently. D05/D06 and D07 can progress independently with matching
schemas/admission policy. D08 is cross-cutting from the first migration, not a
last-minute test flag. No actual NN training is required for D07.

| Gate | Evaluation | Required result |
| --- | --- | --- |
| Q01 Schema/identity | All keys/FKs/scopes, unsigned values, lossless blobs, current definitions and incompatible schema cases | No silent narrowing, order/alias loss, cross-namespace reference or unsupported-identity admission |
| Q02 Live integrity | Real PostgreSQL current-game play, restart, receipts, scoped views/events/shares, rollback and concurrent writes | Exact required current semantics; one durable revision/receipt/journal per accepted operation |
| Q03 Fencing/clock | Two service instances, old/new client and scheduler owners, lease expiry/renewal, clock contention and process death | No double owner mutation, clock extension or stale candidate acceptance |
| Q04 Feed/archive | Every failure boundary, sequence commit-order inversion, duplicate/out-of-order/backfill, unavailable object store and corrupt archives | No missed committed events, duplicate sink effects or premature source deletion; recoverable backlog |
| Q05 Statistics | Hand-counted synthetic player/hero/map cohorts, multi-seat/mixed control, mirrored lineups, incomplete games and result correction | Correct units/denominators and purpose filters; clean rebuild matches incremental facts/aggregates |
| Q06 Elo | Golden calculations, equal/opposite teams, draws, duplicates, quarantines, late results, earlier correction and rebuild | Exact policy-bound ledger, one active result contribution and atomic complete leaderboard generations |
| Q07 Training | Admitted synthetic games, corrupted/untrusted traces, unknown/future information, repeated seeds/forks and result retractions | Reproducible shards/manifests, valid split groups, no forbidden data or unverified targets |
| Q08 Testing isolation | Real parallel CI/manual/evaluation sessions, forged purpose, accidental production URL, pooled scope reuse and cleanup race | No production access, ranked contamination or TRAIN leakage; bounded owned cleanup and retained evidence |
| Q09 Recovery/privacy | SQL/object/key restore, revoked subjects/shares, tombstone replay and downstream catch-up | Current games/permissions/retries survive; deletion policy is not undone; derived stores rebuild |
| Q10 Migration | Consistent current SQLite import and cutover with timed/untimed live clients, post-cutover failure and rollback rehearsal | Counts/digests/behavior match; no active game, credential, accepted action or time semantics lost |
| Q11 Combined workload | Declared live concurrency plus archive export, stats/rating queries and external training reads; EXPLAIN/lock/WAL/backlog measurements | Recorded live latency/error/resource budgets pass; bounded query/index/export behavior and no trainer scans on live SQL |
| Q12 Release evidence | Fresh install/migrations, current-content full required correctness tiers, permissions, backup/policy documentation and real disposable deployment | All selected tests have nonzero counts and no missing/failed inputs; unsupported workflows are explicit |

Before D01 closes, declare the representative workload (games, concurrent seats/
commands, save/replay bytes, finished history, dashboard queries, export rate
and dataset size), live latency/error budgets, retention/capacity assumptions and
target hardware. Do not invent results or compare an empty test database to a
populated production one.

Measure p50/p95/p99 only with adequate recorded sample counts; include raw
latencies/errors, lock waits, IO/CPU/memory, WAL/write amplification, index size,
connection pools, cache miss/load, outbox lag and archive/training throughput.
Use fixed replay workloads/content identity and run live-only versus combined
load on the same setup. Query plans must cover both active and historical scale.
If shared-cluster analytics harms live budgets, separate its physical resources
or reduce bounded load before accepting Q11.

Completion requires schemas/migrations, executable commands, source/runtime/data
identities, expected/executed/missing/failed counts, raw reports and the recorded
product decisions for ranked eligibility, usage, retention and recovery.
Writing this plan does not certify ranked play or authorize production migration.

## 14. Handoffs and references

Use the [plan index](PLANS_INDEX.md) for dependencies and the [neural plan](NEURAL_SEARCH_IMPLEMENTATION_PLAN.md)
for consumers of admitted datasets. NN development may use a conforming local
manifest/shard backend; production database rollout is not a research prerequisite.

The [dedicated rule audit plan](RULE_VERSION_AUDIT_IMPLEMENTATION_PLAN.md) owns
compact private trace hashes, cross-engine profiles and target-start recipes.
A target checks recorded evidence in one replay. Preserve comparison coverage
separately from admission of a fully executed new derived trajectory; neither
automatically changes original results or Elo. No per-decision full snapshot
or repeated original-engine replay is required.

References: [shared foundation](SHARED_RUST_FOUNDATION_PLAN.md),
[client execution and ownership](CLIENT_SIDE_BOT_EXECUTION_PLAN.md),
[current persistence](../rust/crates/goa2-game-server/src/store.rs),
[current replay operations](../rust/crates/goa2-game-server/src/replay.rs),
[current scoped history](../rust/crates/goa2-game-server/src/history.rs),
[service orchestration](../rust/crates/goa2-game-server/src/game.rs) and
[current client integration guide](CLIENT_INTEGRATION_GUIDE.md).
These paths identify reviewed responsibilities, not a claim that this proposed
PostgreSQL/analytics/training schema or ranking system is already implemented.
