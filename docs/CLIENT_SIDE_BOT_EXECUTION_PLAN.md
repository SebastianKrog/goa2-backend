# Client-side bot execution plan

Status: proposed implementation plan; no client implementation or release is
claimed complete. Written 11 November 2026. This is plan 2 of four, following
the [shared Rust foundation](SHARED_RUST_FOUNDATION_PLAN.md).

Deliver in three stages: **native Rust client**, then **browser WebAssembly**,
then **client-side neural evaluation after the full NN implementation is ready**.
Each stage has its own completion gate. A later stage does not block shipping
an accepted earlier one.

## 1. Goal, baseline and scope

Let players supply the compute for bot decisions while the live service remains
authoritative over rules, hidden information, permissions, revisions and clocks.
A native program or browser worker receives an authorized observation, searches
locally and submits an ordinary game command. The server continues to execute,
validate and persist that command; moving search does not make a game peer-to-peer.

Assume cleanup is finished. Consume the foundation's observation, knowledge,
decision/command, content identity, hypothetical-world, feature/evaluator and
budget contracts. F02–F04, F06–F07 and the relevant acceptance evidence must be
available before claiming a playable local bot. Do not wrap a private checkpoint
download or the existing full-session bot entry point and call it client search.

**Current games must keep working.** Attach or change controller only through
an authorized handoff at the current revision. Keep existing game identity,
pending prompts, physical card bindings, histories, clocks, retries and replay.
No reset, new rules version or silent content conversion is part of this plan.
Obsolete client builds, historical formats and previous model generations need
not remain supported. Current-format in-progress games and current human
clients remain acceptance cases.

The planning branch extends the foundation documentation at `3451404d`;
its remote source base precedes the separate cleanup continuation. Source
responsibilities below describe the reviewed code, not proof that the proposed
client APIs exist. Reconcile implementation paths with completed cleanup.

### Existing behavior and required extension

The service already owns a bounded in-process bot loop, token authentication,
REST/WebSocket delivery, command acceptance and persistence. The reviewed bot
specification selects the Rust server bot; it does not yet specify remote
execution ownership. Its current recommendation code starts from `LiveGame`.

Extend execution placement and ownership independently of bot strategy:

| Setting | Meaning |
| --- | --- |
| Execution placement | Manual, server bot, or delegated client bot; persisted with the seat |
| Strategy/evaluator | Identified search policy and heuristic/NN evaluator; reusable across hosts |
| Controller session | Current authorized client instance and fenced ownership generation |
| Failure policy | Explicit behavior when that controller is absent, disconnected or expired |

A client is untrusted. It may run different search code, alter its local clock
or send any proposed legal command. The server enforces information access,
ownership, legality and time; a reported binary/model name is not remote
attestation or a guarantee of equal playing strength.

### In scope

- Bot-seat delegation, controller handoff and real command transport.
- Native installation, local compute controls, reconnect and diagnostic behavior.
- Browser worker integration using the same Rust rules/search library.
- Later loading and running approved GoA2 neural evaluators locally.
- Current-game, privacy, reliability, performance and release evidence per stage.

Production database replacement belongs to plan 3. Neural architecture,
training, self-play, feature optimization and model promotion belong to plan 4
and its full NN implementation. Rule/oracle/Python update procedure belongs to
the later plan 5. This plan neither starts training nor changes game rules.

## 2. Shared execution architecture

Keep one strategy implementation and one command encoder across native, WASM
and server-side test/reference adapters. Do not implement a second rules engine
or search algorithm in JavaScript.

```text
Authoritative service
  authenticate + controller ownership
  -> scoped observation + decision context + public content identity
       -> Rust observation-only agent
          (native process / WASM worker; later heuristic or NN evaluator)
       -> proposed typed command + action ID + revision/token + controller generation
  validate + execute + durable acceptance
  -> receipt + scoped update
```

The following separation applies in every stage:

| Component | Responsibility |
| --- | --- |
| Shared Rust agent | Observation reconstruction, constrained hidden-world sampling, candidates, bounded search and evaluator calls |
| Native host | Connection/auth, secure credential handling, IO, local monotonic deadlines, resource scheduling and process lifecycle |
| Browser host | Connection/auth in the application, capability checks, worker lifecycle, watchdog and user controls |
| WASM worker | Bounded agent jobs over authorized inputs; no privileged server data or authority |
| Service | Ownership fence, scoped projection, legal commands, clocks, receipts, persistence and broadcast |
| Artifact publisher | Admitted content/client/model manifests, integrity and authenticity, supported-platform releases |

State is per game, seat, decision and controller generation. One process may
host several explicitly assigned bot seats, but each keeps separate
observations, histories and knowledge. Do not pool teammates' or opponents'
private data merely because they run on the same computer. If an owner explicitly
delegates opposing seats to one computer, that computer necessarily receives both
credentials/observations; software separation is not protection against its
operator. Make that trust decision visible instead of claiming host-side secrecy.

## 3. Delegation, ownership and failure policy

### Authorization

Add an explicit host/owner-authorized way to assign an existing or newly
created bot seat to a client. A spectator, arbitrary player token or model
artifact cannot appoint controllers for another seat. Preserve current
consent/host rules when a human seat changes to bot control.

Use a short-lived pairing capability or equivalent authenticated handoff to
obtain a revocable, game-and-seat-scoped controller credential. Bind it to the
intended service origin. Its permissions cover scoped observation, controller
renewal/status and allowed gameplay submissions. It cannot restore checkpoints,
use staff/Lab routes, reveal unrelated credentials or administer other seats.
Do not bundle server provisioning/staff secrets with the client.

Specify this as a proposed capability and add it through the current
application authorization model; do not assume a general account system already
exists. If a pairing link is used, redact it from logs and make replay/expiry
behavior explicit. Credentials never enter the search seed, observation digest,
model files or telemetry.

### Single active controller and fencing

Persist execution placement and a monotonically changing controller generation.
Claim, renew, revoke and switch placement through serialized conditional updates.
Keep lease renewals/status separate from the gameplay revision and decision token:
a heartbeat must not invalidate a valid search or append a fake gameplay event.
A takeover/revocation changes controller generation and fences pending work even
when the gameplay revision has not changed. A renewable lease uses server time. A repeated claim operation has its own
idempotency ID and cannot produce extra controllers.

The authoritative command path validates both the current generation and
lease/session authorization when accepting a new command. This fences:

- two native processes or browser tabs trying the same seat;
- an old socket submitting after native/browser/manual takeover;
- a server bot already searching when its seat moves to client execution;
- an expired client returning after the server or another client has taken over;
- process restart and visually identical state after rollback/replacement.

Do not rely on a UI flag, process mutex, browser leader election or socket close
alone. Stop obsolete local work as well as rejecting its result. A completed
command's receipt remains idempotent if its acknowledgement was lost during
handoff; a revoked identity does not regain permissions by fetching that receipt.
Specify authorization and receipt lookup order without executing the command twice.

Use the foundation's current transactional store to persist the minimum
controller metadata now. Restart must not resurrect two owners. Either restore
a still-valid lease safely or invalidate its generation and require reclaim;
record the chosen policy and test old-session rejection. This does not require
waiting for a new production database or implementing multi-instance ownership.

For a team-scoped decision, each seat keeps its own scope. The server's current
decision token/revision and atomic acceptance select the first valid authorized
response; other pending searches cancel. A client never assumes authority to
inspect every potential responder's private state.

### Disconnect policy

Default for delegated execution: **no expensive server search runs for that
seat**. On disconnect, stop client submissions until transport is synchronized.
A lease may survive a brief reconnect grace interval, but it does not pause the
game or extend a clock. Expiry fences further submissions.

If the client does not return, expose the missing controller to the owner.
The default stops bot automation and retains the game's existing clock-expiry/
inactivity behavior. An untimed game waits for authorized reconnect or manual
takeover; it does not invent a timeout loss or silently restart server compute.

Allow an explicit alternative: after a configured lease expiry, switch to
the normal server bot or an authorized manual controller. That transition
changes generation before any new work is accepted. Show the selected policy
and any added server compute. Never have local and server bots race as an
undocumented reliability mechanism.

## 4. Connection and decision lifecycle

Define the versioned bot protocol in the current client integration guide.
Reuse existing transport/commands where suitable; add only missing bot
observation, claim/renew/release/status and current-capability messages.
Endpoint names below are responsibilities, not already implemented routes.

| Exchange | Required content/behavior |
| --- | --- |
| Discover/handshake | Service identity, current bot protocol/content/feature requirements, supported placement and bounded capability data |
| Pair/claim | Authorized seat, expected controller generation, claim action ID, revocable credential/session, server-time lease bounds |
| Sync observation | Current revision and decision token, addressed legal descriptors, permitted knowledge and authoritative clock information |
| Renew/release | Credential/session, fenced generation and bounded renewal interval; release cannot revoke a newer owner |
| Submit gameplay | Current typed command, stable action ID, expected revision/decision token and controller generation |
| Receipt/resync | Accepted/replayed/conflict result and current scoped snapshot; no private state or staff diagnostics |
| Capability change | Explicit mismatch/model-unavailable/upgrade-required outcome; no automatic game reset |

Use HTTPS/WSS for hosted execution, verified certificates and bounded
timeouts/frame sizes. Local plaintext is allowed only for explicitly selected
loopback development. Never forward bearer credentials to a redirected host,
content CDN or artifact/model URL. Browser WebSocket authentication needs a
documented secure mechanism, such as an existing authenticated flow or a
short-lived single-use ticket; do not assume browsers can set arbitrary
Authorization headers or put long-lived bearer secrets in URLs.

At runtime:

1. Pair and claim an authorized seat; verify current compatibility and obtain
   exact admitted public content. Fetch static content once and cache by digest.
2. Synchronize observation and visible knowledge. Treat WebSocket messages as
   updates, not as proof that no messages were missed; after a gap or reconnect,
   obtain an authoritative scoped snapshot/knowledge cursor.
3. If no action is addressed to this seat, wait without searching. On an
   addressed decision, create a job bound to game/seat, revision, token,
   controller generation, observation digest, content/evaluator identity and
   explicit local sampling seed.
4. Choose a local monotonic budget from configured limits and authoritative
   remaining time, subtracting a documented network/scheduling safety margin.
   Estimate elapsed/RTT conservatively; client wall-clock skew cannot extend
   the server deadline. Very short windows use only the admitted legal
   fallback or wait for existing timeout behavior.
5. Run the shared observation-only agent. New revisions, handoff, cancellation,
   terminal state and lease invalidation cancel/discard obsolete jobs.
   Heartbeats and network handling must remain responsive during native search.
6. Recheck that the job still matches the latest decision and controller;
   submit one command through the ordinary acceptance path. The server performs
   its own final checks even if the local recheck passed.
7. On a lost acknowledgement, preserve the original action ID and command.
   Retry/reconcile that operation; never issue a new ID just because a request
   timed out. On a genuinely stale unaccepted command, discard it, resync and
   create a new job.
8. After an accepted planning action, resync before the next slot/decision.
   Do not blindly submit a cached sequence. Release/cancel on shutdown, or let
   expiry safely handle an interrupted process.

The client state machine should distinguish pairing, ready/waiting, searching,
submission pending, reconnect/resync, ownership lost, incompatible, stopped
and finished. A terminal game stops search and regular lease renewals; if an
authorized correction reopens it, require a fresh synchronized decision and
valid ownership before restarting. Bound queues, heartbeat rates and retries;
use jittered backoff without consuming a timed seat's clock through busy polling.

## 5. Stage 1 — native Rust client

### Deliverable and implementation

Add a small host executable, provisionally `goa2-bot-client`, depending on the
supported Rust agent/facade. A separate host crate is justified by network,
credential and OS dependencies; the engine remains usable without them.
Do not depend on SQL, the Python engine, an installed game server or an ML
framework to play with the initial weighted evaluator.

Start with a command-line program and a simple pairing/status flow. A GUI or
desktop shell is optional later. The player must be able to:

- connect/pair to a known game and see the exact seats delegated to this program;
- start/stop local bot execution and observe connection/ownership status;
- limit threads, memory and thinking time, including an overall process cap;
- select an admitted heuristic configuration and see fallback/stop reasons;
- reconnect, release control or request the normal authorized takeover flow;
- export redacted diagnostic evidence explicitly.

Supply credentials through interactive pairing/stdin or a protected local
mechanism. Do not encourage shell arguments containing tokens. Keep secrets
in memory by default; optional persistence uses the OS credential store or
an explicit protected-file policy tested on the supported platform.
Persisting an uncertain command for crash recovery, if needed, is bounded
and protected; it is not a private checkpoint cache.

Use a dedicated compute worker/pool so connection, cancellation and lease
renewal do not block behind a search. Serial execution is the reference.
Multiple seats share an overall compute cap fairly; do not oversubscribe
one search per core per seat. One outstanding gameplay submission per seat
is the default. Resource exhaustion must produce a bounded failure/fallback,
not an unbounded allocation or a second server search.

Load only admitted public content from verified manifests. The client may
cache immutable content and later weights separately from observations.
It does not upload content or checkpoints to change rules on the service.
Unsupported current identities stop with an actionable upgrade/content error.

### Packaging and support

Proposed first-release acceptance targets are Linux x86_64 and Windows x86_64;
test installation and actual gameplay on both before advertising those builds.
macOS arm64 is a subsequent native target, accepted only with its own tested
artifact. If target scope changes before implementation, record the concrete
supported matrix and rationale rather than silently shipping an untested
cross-compile. This target work is additional to cleanup's Linux baseline.

Provide pinned/locked release builds, dependency and license review, binary
digests, source/build provenance and a documented publisher-authenticity
mechanism for downloads. Verify installation without Rust/Python development
tools. Updates are explicit and occur between jobs; current-game mismatch
requires a compatible client/content artifact, not conversion of the game.
Do not run an arbitrary downloaded executable based solely on a checksum.
Document uninstall, credential removal and how ownership expires if the
program is closed. No autostart/system service is required for stage 1.

### Stage 1 acceptance

| Gate | Test | Complete when |
| --- | --- | --- |
| N01 Ownership/auth | Unauthorized claim, wrong seat/game, duplicate claim, two processes, lease expiry, release race, manual/server takeover and service restart | Only the current authorized generation can execute a fresh command; old jobs cannot retake control |
| N02 Current gameplay | Play native bot versus human and bot, including existing current-format saved games; all supported phases/modes/decision families and foundation special cases | Commands are accepted through normal APIs; state/event/receipt/replay/privacy contracts match the current baseline |
| N03 Reliability | Lost/out-of-order/duplicate updates, disconnect during search/submission, lost acknowledgement, client crash, server restart and new second-card prompt | Resync preserves knowledge and idempotency; no duplicate action, stale continuation or clock extension |
| N04 Scope/privacy | Paired hidden-state fixtures, per-seat histories, teammate/team prompts, diagnostic export and spectator attempts | Only permitted observations reach the program; seeds/logs/caches contain no server secrets or unrelated seat data |
| N05 Compute/resources | Fixed-work reference runs; CPU/thread/memory caps, already-cancelled and in-flight cancellation, small deadline, multi-seat contention | Deterministic serial decisions, bounded work and responsive connection/renewal; explicit fallback/stop reasons |
| N06 Offload | Same observation-only policy/configuration and seeded workload hosted server-side in a test reference versus client-side; inspect production-placement counters | Zero server search/evaluator invocations for client-owned seats while fallback is inactive; validation/persistence still run normally |
| N07 Distribution | Fresh-machine install, pairing, current-game play, stop/uninstall and artifact verification for each advertised OS | Tested release artifacts and instructions exist without requiring a developer toolchain |

A smoke demo or one opening move does not close N02. Record a coverage manifest
for the current game cohort, completed end-to-end games and mid-game cases,
including planning, upgrades, response prompts, deployment, timed/untimed play,
copied cards/spells, multi-piece heroes and simultaneous decisions.
Use the foundation's admitted synthetic cases and full current-content identity.
No real credentials or private player matches become committed fixtures.

**Exit:** N01–N07 pass with evidence. A native client can ship with heuristic
evaluation while stage 2 and all NN work remain pending.

## 6. Stage 2 — browser execution with WebAssembly

### Deliverable and integration

Compile the shared Rust agent for `wasm32-unknown-unknown` and expose a small
versioned worker API: initialize compatible content/evaluator, start/resume a
job, return progress/result, cancel/dispose. Reuse native search, reconstruction,
features, tie ordering and command encoding. JavaScript owns browser integration,
not a rules or evaluator rewrite.

Embed this in the current game frontend through an explicit local-bot control.
The page owns its authenticated connection and submission state. Pass only
authorized job input and public immutable artifacts to the worker. The worker
does not need authentication tokens or staff routes. Validate worker message
shape, job ID and all identity fields on both sides before using a result.

Show activation, current controlled seats, download/startup progress, waiting/
thinking status, stop/release, resource settings and any degraded mode.
The human UI must remain usable while a bot searches. Closing the tab, changing
origin or revoking control must not leave an assumed immortal controller.

### Cancellation and browser lifecycle

A synchronous WASM call occupies its worker's JavaScript event loop. A queued
`cancel` message cannot update a normal flag until that call returns.
Implement bounded resumable work slices and yield between slices so the worker
can receive newer state and cancellation. Check deadlines inside expensive
operations as well. Yield at explicit owned search/continuation boundaries, preserving RNG state,
work counters, hypothetical-world isolation and forced-effect progress across
resumes. Verify sliced versus unsliced fixed-work equivalence. Record and test
maximum slice/overshoot behavior; moving
one huge call off the main thread is not sufficient cancellation support.

Keep a last completed result/fallback in the host, tagged to its exact decision.
A main-thread watchdog may terminate an unresponsive worker; termination
discards all worker state and cannot produce an accepted result from the old
job. Reinitialize safely if execution continues. Never trust a late worker
message after replacement, content/model switch or controller generation change.

Browser suspension and background throttling can delay both timers and
heartbeats, so no hard real-time guarantee is claimed. Display the limitation,
resync on resume and let authoritative lease/clock rules handle absence.
Do not use a service worker as a promise of uninterrupted background thinking.
Document mobile sleep/page eviction explicitly even if desktop is the initial
supported product. Suspend/reduce compute when appropriate without presenting
that as a paused game clock.

### Capability tiers, assets and multiple tabs

A serial CPU worker without SharedArrayBuffer is the baseline. WebAssembly
SIMD and multi-threaded builds are optional measured tiers selected by feature
detection. Threads require the appropriate cross-origin isolation and shared
memory support; COOP/COEP implications for the current frontend, authentication,
embedded pages and artifact hosting must be tested. Do not require those headers
for the serial fallback. Shared-atomic cancellation is an optimization for a
threaded tier, not an assumption in the baseline.

Use a bounded WASM memory configuration and measure worker startup, stack,
initial content parsing, peak memory and message-copy costs. Stream/verify
public assets where feasible, bound caches and tolerate quota errors.
Version assets by an authenticated release manifest and content digest.
Service-worker/CDN caches may hold immutable public code/content/weights;
authenticated observations, credentials and private histories must not enter
shared caches or analytics. Handle offline, interrupted download and partial
asset updates with an explicit recoverable state.

Only one tab may own a given controller session/generation. Browser locks or
BroadcastChannel can improve UX and avoid duplicate work, but the service fence
is authoritative across tabs, origins and devices. Losing the fence stops work
even when browser coordination messages are missed.

Declare exact tested browser versions/OS combinations at release. Initial
desktop targets are current stable Chromium, Firefox and Safari on compatible
supported machines. Test the serial baseline on each before claiming broad
browser support; SIMD/threaded tiers carry separate capability evidence.
An unsupported browser retains normal human play and can suggest the native
client; it does not silently start expensive server search.

### Stage 2 acceptance

| Gate | Test | Complete when |
| --- | --- | --- |
| W01 Native/WASM parity | Same admitted observations, seeds, fixed work, integer features and heuristic evaluator on native and real browser workers | Same commands/work counts/features; any floating-point convention is specified and verified |
| W02 UI and worker lifecycle | Real gameplay, responsiveness, cancel during reconstruction/search, newer decision, worker crash/restart and main-thread watchdog | Bounded slices, no main-thread search blocking and no stale result accepted |
| W03 Browser failures | Hidden tab, sleep/resume, tab close/page eviction, offline/reconnect, cache quota and partial asset update | Correct lease/clock behavior, clear status and authoritative resync; no background-execution guarantee |
| W04 Multiple controllers | Two tabs, native plus tab, browser takeover and optional browser leader coordination failure | At most one authorized controller; stale generations and completed old workers are fenced |
| W05 Capabilities/security | Baseline without isolation/SIMD, optional SIMD/threads with isolation, worker message corruption, artifact mismatch and cache inspection | Tested capability selection and no private data in public caches/messages/logs; unsupported tiers fail safely |
| W06 Current integration | N02/N03/N04/N06 through the browser host and current frontend contract suite | Current games and human workflows work; offloaded seats still cause zero server search when fallback is inactive |
| W07 Delivery/performance | Actual hosted asset loading, declared browser matrix, startup/download/copy cost, memory, battery/CPU load and timed decisions | Reproducible raw results, usable controls and explicit tested support limits |

The foundation's acceptable WASM feasibility-blocker report is not completion
of this stage. W01–W07 require a working browser product and real-browser tests.
**Exit:** browser execution can ship with the same heuristic agent, independently
of a neural evaluator.

## 7. Stage 3 — local NN evaluation, after full NN implementation

### Hard dependency and scope

This stage is **blocked until the full GoA2 NN implementation is ready**.
Plan 4's preparation document, an evaluator interface, random weights or a mock
runtime do not satisfy that dependency. Required upstream deliverables are:

- a defined feature schema and inference-boundary/score contract;
- a real trained and evaluated model admitted for the current rules/content;
- a versioned export format and compatible inference runtime;
- golden feature/output vectors and native reference evaluation evidence;
- accuracy/strength, resource and release/promotion evidence sufficient to
  approve that model for players.

This plan adds delivery and local inference for those artifacts, first in the
native client and then in the browser client. It does not choose training
objectives or require importing Python/Torch into an ordinary client install.
The heuristic client remains usable and independently releasable.

The inspiration from Stockfish/NNUE-in-JavaScript is local CPU evaluation of
downloaded weights alongside a compiled engine, often accelerated by WASM SIMD
and incremental accumulators. It is not a requirement to use Stockfish code,
weights, chess-specific features, an existing Stockfish JS package, or a
JavaScript rewrite of our search. GoA2 may use NNUE-style sparse updates,
a hex-grid network or a graph model according to the NN plan and its evidence.
Incremental inference is required only if the chosen model/runtime supports it.

### Artifact and runtime contract

Give an approved model manifest its model/parameter digest, feature schema,
rules/content compatibility, architecture, numeric/quantization convention,
input/output dimensions, supported evaluation boundaries and required runtime
capabilities. Include download/decoded-memory limits and license/provenance.
Verify the publisher and all file digests before activation; a digest alone
does not authenticate who approved the artifact.

Use an allowlisted, data-only weight format with strict bounded parsing.
Do not execute pickle, downloaded JavaScript, arbitrary WASM or model-provided
custom operators as part of loading weights. Engine/runtime updates are a
separate trusted client release. A corrupt/unsupported/oversized model is an
explicit error, not permission to relax feature or rules checks.

Pin one evaluator identity per job, cache and score aggregation. Install new
weights atomically between decisions/jobs; cancel/discard old work and invalidate
model-dependent search values and accumulators before changing identity.
Do not mix scores from different models, quantizations or perspectives.
Keep game state, rules and command transport unchanged.

Keep inference in the compiled search hot path wherever the runtime permits:
load weights once into native/WASM memory and avoid JSON serialization or
Rust-to-JavaScript callbacks for every evaluated leaf. Measure feature/inference
cost and boundary crossings separately. A browser inference library that requires
async calls or batching needs an explicit budget/cancellation integration and
parity evidence, not an unmeasured adapter hidden behind the evaluator trait.

Start with CPU inference and the native reference implementation. Add a WASM
CPU implementation with scalar fallback and SIMD where measured. Model-specific
conversion/quantization needs its own golden-output and strength evidence;
lower precision is not assumed equivalent. GPU/WebGPU inference or large
cross-position batching is optional future work with separate latency,
numerical, capability and memory acceptance, not a prerequisite for this stage.

For NNUE-style incremental state, keep one accumulator per search branch/
sampled world as required by the model. Apply/undo, restore, cloning, effects,
summoning/defeat, upgrades and global feature changes must agree with a fresh
full feature/inference recomputation. Reinitializing a worker/model never
reuses an accumulator from another game, seat, perspective or model identity.

Neural value/policy is advisory to search. Rule-derived legal choices and
terminal outcomes remain authoritative. A heuristic fallback after an inference
failure must be explicit, tagged to its evaluator identity and started from a
valid job state without retaining incompatible neural scores. It cannot hide
missing NN coverage or be reported as successful NN execution.

### Stage 3 acceptance

| Gate | Test | Complete when |
| --- | --- | --- |
| E01 Upstream readiness | Inspect actual model/export/runtime and promotion evidence from the full NN implementation | Every hard dependency above exists with compatible identities; a mock cannot close this gate |
| E02 Native inference | Golden observations/features/outputs, real current-game search, budgets, errors and model replacement | Reference outputs and legal decisions meet the defined numerical/score contract without game/transport changes |
| E03 Browser inference | Same real artifacts and vectors in supported real browsers, scalar/SIMD paths and representative decisions | Declared tolerances/tie policy and resource caps pass; no unsupported precision/strength equivalence claim |
| E04 Incremental correctness | Full-recompute differential cases across branching, apply/undo, restore, effects and model switches, if incremental inference is used | Correct branch-local state, no cross-seat/world leakage and agreement within the admitted numeric convention |
| E05 Model delivery/failure | Corruption, incompatible schema/content, oversized payload, interrupted download, quota failure and update while thinking | No unsafe activation or stale result; atomic artifact state and explicit supported fallback |
| E06 Product evidence | Matched workloads comparing heuristic/native NN/browser NN; latency, completed work, memory/download and approved strength evaluation | Reproducible end-to-end evidence, measured useful tradeoff and a documented model/runtime release decision |

**Exit:** E01–E06 pass using a real approved model. If a model does not use
incremental inference, E04 records that fact and demonstrates branch-local
nonincremental isolation instead. NN availability or faster evaluation by itself
does not establish higher playing strength.

## 8. Implementation sequence and dependencies

All packages are open. Owners are roles to assign during implementation.

| ID | Stage / owner | Work | Depends on | Completion evidence |
| --- | --- | --- | --- | --- |
| B01 | Native / service-client | Freeze protocol, execution-placement settings, current-game cohort and native target matrix | Shared foundation contracts | Versioned specification and named acceptance cases; no change to current rules |
| B02 | Native / service | Pairing, scoped credentials, controller claims/lease/fence and explicit fallback placement | B01, current transactional store | N01 fault/race/restart evidence; server scheduler excludes client-owned seats |
| B03 | Native / client-search | Host executable, shared agent, monotonic budgets, bounded multi-seat scheduling and transport state machine | B01–B02, observation-only foundation agent | N02–N05 evidence including uncertain submission recovery |
| B04 | Native / release-client | Installation, artifact trust, diagnostics, platform verification and offload measurement | B03 | N06–N07 and complete stage-1 report |
| B05 | WASM / engine-client | Portable serial agent, resumable slices, host imports and worker protocol | Accepted native stage; foundation WASM findings | W01–W02, measured slice/cancellation behavior |
| B06 | WASM / frontend-service | Local-bot controls, auth/transport integration, tab ownership, resync and lifecycle behavior | B02, B05 | W03–W06 and current human-client regression suite |
| B07 | WASM / release-client | Asset delivery/caching, tested browsers, optional SIMD/threads and resource profiling | B05–B06 | W05/W07 and complete stage-2 report |
| B08 | NN / model-client | Admit upstream model, manifest/parser, native CPU inference and safe model switching | Full NN implementation and E01; accepted native client | E02/E05 native evidence |
| B09 | NN / model-frontend | WASM CPU/SIMD inference, numerical and accumulator tests, browser model delivery | Accepted WASM stage, B08 | E03–E05 using the actual admitted model |
| B10 | NN / search-release | End-to-end quality/resource comparison and model/runtime release record | B08–B09 | E06 and complete stage-3 report |

Client and service changes are coordinated application releases. They need not
retain deprecated controller protocols, but version mismatch must be explicit
and current games must remain resumable with the compatible build.
Do not repin the oracle or alter rules to make a client target compile.

## 9. Evidence, server savings and release checks

Use the foundation acceptance cohort and current identities. Each report names
source commits for engine/service/client/frontend, toolchain/profile, OS/
architecture/browser version, artifact digests, policy/evaluator, seeds, node/
step/world limits, timing method and expected/executed/missing/failed cases.
Retain raw results and explicit unsupported cases. No selected row closes
with zero matches or an undocumented fallback.

For deterministic comparisons, use a fixed-work, observation-only reference
agent with identical content, seeds, evaluator and policy on each host.
Do not compare an omniscient server bot with a differently informed local bot
and attribute the resulting game differences solely to offloading.
Keep deadline-limited play and strength evaluation separate from equal-work
throughput. Model quantization and parallel/nondeterministic paths have their
own declared comparisons.

For offload evidence, instrument service search/evaluator invocations and CPU
time separately from projection, normal rules execution, storage, routing and
heartbeat handling. Client-owned seats with no activated server fallback must
produce **zero server search/evaluation invocations**. The normal rules/DB work
still occurs. Measure matched-game server CPU/memory and network payload/
heartbeat costs before and after placement changes; record client CPU/memory
and browser startup/download/battery costs as well. Do not promise zero server
runtime or an arbitrary percentage reduction without measurements.
Bound/rate-limit bot observation and renewal traffic so savings are not replaced
by tight polling or duplicated reconnect work.

Performance reports should include cold-process/worker start, warmed decisions,
observation transfer/copy, reconstruction, features, inference, search,
cancellation and end-to-end submission latency. Reuse the foundation sampling
method and distinguish cold process/worker from uncontrolled OS/CDN caches.
Choose user-facing defaults from measured results on supported hardware,
including modest machines, and document deadline/lease safety margins.

Before each stage release:

- [ ] Its mandatory acceptance gates and required foundation/current-game tests
      pass with complete identities and nonzero counts.
- [ ] Current games continue through restart, control transfer and timed play;
      human play and privacy contracts still pass.
- [ ] Installed/delivered artifacts are tested on the advertised platform matrix;
      publisher verification, limitations and stop/uninstall behavior are clear.
- [ ] Service-side counters prove offloading and all fallback activation is visible.
- [ ] No private checkpoint, credential, server RNG state or unrelated seat data
      crosses the client boundary or enters public assets/caches/telemetry.
- [ ] Failure, capability mismatch and outdated-current-client behavior is
      actionable and does not reset the game.
- [ ] A stage-specific release record distinguishes implementation evidence,
      real deployment/browser verification and actual promotion decisions.

Stage 1 and stage 2 use heuristic evaluation until stage 3 is admitted.
Stage 3 remains visibly blocked on full NN readiness even if its loader and
interface stubs have already been written.

## 10. Planning sequence and handoff

This document supplies plan 2's three-stage execution path. The
[database, training data, statistics and Elo plan](DATABASE_TRAINING_STATISTICS_PLAN.md)
is now recorded. Next write plan 4 for neural-search preparation.
The neural plan must include the eventual full implementation/model-admission
deliverables needed to unblock this plan's stage 3; preparation alone is
insufficient.

After the first four planning documents are finished, remind the user to write
plan 5: the rule/oracle/Python update procedure, including current-game transition
policy. This remains the sequence reminder from the foundation, not a scheduled
job and not a prerequisite to drafting the remaining plans.

Implementation references: [shared foundation](SHARED_RUST_FOUNDATION_PLAN.md),
[current bot coordinator](../rust/crates/goa2-game-server/src/bots.rs),
[current transport](../rust/crates/goa2-game-server/src/http.rs),
[current client integration guide](CLIENT_INTEGRATION_GUIDE.md) and
[current native bot entry point](../rust/crates/goa2-core/src/interpreter/live_game/bot.rs).
These paths identify reviewed responsibilities; controller APIs, resumable
browser search and NN delivery described above are proposed work.
