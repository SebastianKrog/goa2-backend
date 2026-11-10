# Async work, locks and panic-boundary review

C08-05/C11-01, 10 November 2026. Source review spans HTTP/WS, orchestration,
SQLite, replay/share work, bot/deadline/vote workers and core import/codec/content.
[Boundary inventory](boundaries.json) lists production panic/unwrap/expect sites
with their exact locations/context and separately counts test assertions. A
`#[cfg(test)]` import does not hide the production code below it; a tooling
regression protects that distinction. Textual categories are review inputs, not
a proof of whole-program panic freedom or permission to remove checks.

## Blocking and cancellation boundaries

| Boundary | Ownership, bounded work and failure behavior |
| --- | --- |
| HTTP/WS commands | Auth/field parsing precedes blocking orchestration. Per-game `current` guards cover candidate construction, SQLite journal/receipt commit and publication; rejected durable writes leave the published revision unchanged. |
| SQLite | Store serializes its connection. SQL, migrations, online backup and replay reads run in blocking work, with one database ownership lock. Lock order is game/current → voting where needed → store; store does not call back to acquire a game mutex. |
| Replay/shares | Archive byte/revision/hash gates precede reconstruction. Share baking uses a separately limited permit, does expensive replay/projection outside publication locks, then checks the candidate game revision before publishing durable metadata/groups. |
| Bots | Two global search permits; immutable snapshot/revision captured, revision rechecked after permit, bounded native search in `spawn_blocking`, cancellation flag on state/retire/shutdown changes, awaited completion releases the permit. Stale submission conflicts remain rejected. Deadline fallback remains explicitly legal, not a successful unsupported search. |
| Deadlines/vote expiry | Recompute from the latest snapshot after update wakeups; clock actions and persistence use blocking work. Shutdown/retire watch guards are dropped before awaits. Restart restores interrupted clock holds through an explicit journaled cancellation. |
| Shutdown | Stop accepting writes, notify sockets (1012), finish already admitted durable work and cancel bounded workers. Existing process tests block a transaction during SIGINT and verify accepted receipts/replay after reopening. This is cooperative shutdown, not preemption of an instruction or filesystem call. |
| Slow clients | Existing send timeout/backpressure and lag recovery remain; scoped current state and pending vote synchronization are used on reconnect. |

### Concrete executor-blocking fix

Bot/deadline loops previously called synchronous `Game::snapshot` directly on
Tokio executor threads. Another command holds that mutex through SQLite commit;
even a snapshot pointer clone could therefore stall the executor. These loops now
use `snapshot_async`, which obtains the same immutable snapshot through the
blocking pool. No rule progression or lock/commit boundary moved.

Socket voter registration and pending-vote reads can contend with persisted
voting commands too. They now run in blocking work. Socket handling has an outer
registration/cleanup boundary so normal and early-return disconnect cleanup also
drops the voter guard in the blocking pool. The inner handler retains existing
message parsing, ordering, scoping and shutdown behavior. Map lookups/presence
accounting that do not encompass storage/rules work remain short guarded accesses.

`contended_snapshot_does_not_block_the_async_executor` uses a single-threaded Tokio
runtime and an explicitly held mutex. An async timer must run and release the
holder while snapshot work waits; the old direct wait would stall that runtime.
The actual process suites verify bots, clocks, votes, restart, failure atomicity,
signals, WS delivery and WAL backup after the fix. No busy-loop sleep is added to
production scheduling. SQLite/OS calls and a native instruction remain
cooperatively bounded rather than instantly cancellable; cold/load and final
full-data acceptance remain independent tasks.

## Panic assumptions and trust distinctions

| Surface / sites | Review and retained guard |
| --- | --- |
| Codec/reference/object paths | External envelope fields, instance/definition indices, path kinds, overlapping bindings and schema/oracle/rules disagreements use checked deserialization/validation and return errors. New bounded corrupt-reference/binding cases exercise this boundary. JSON order/null/alias semantics are retained. |
| Setup content's minion type unwrap | Spawn is appended only under `spawn.minion_type.is_some()` and the immutable source is not changed before consumption. Retain this guarded invariant; malformed required pack shapes/JSON are rejected. Pack files and template model contents remain trusted, hash-admitted rules data, not authenticated merely by an oracle string. |
| Embedded hero catalogue expect | Compile-time `include_str!` of checked-in generated metadata; no HTTP input supplies that string. Developer/content validation remains required. No missing catalogue defaults or fallback identity. |
| Voting patch object expect | Serialization of a typed `PatchCommand` object, not an arbitrary client `Value`; parser/variant validation precedes it. The two voting `unreachable!` matches cover variants already separated by the outer match. |
| Replay `unreachable!` | The outer exhaustive operation match handles command/timeout/patch/unstick/rewind; only the three pause variants reach the inner match. New archive mutations test range, hash, bot roster, future schema and rewind overflow before restoration. |
| Worker `unreachable!` | The outer trace-operation match restricts the inner match to commit/uncommit/pass/finish. Unknown operation strings reject; malformed/future worker packets do not execute or poison retained board state. |
| Core native invariant sites | Interned symbols/private typed IDs, validated object bindings, compiled instructions and checked game-phase state supply the preconditions. The machine list preserves each assumption for later module work. Raw arena/session APIs are experimental trusted Rust boundaries; arbitrary manufactured internal state is not an authenticated input API. |
| Numeric helpers | Digit first/last unwraps follow nonempty checks; scientific-float splitting parses Rust's own finite numeric formatting. Hex deserialization checks integer type, cube sum and coordinate headroom before geometry work. Existing accepted coordinate sequences are retained. |
| Tests | Assertions and fixture unwraps remain test-only and separately counted. Their failures cannot be reclassified as supported runtime fallbacks. |

Existing limits remain: per-advance instruction cap `min(max_steps, 1000)`, bounded
search deadlines/node budgets, aggregate recursion depth checks, coordinate range,
HTTP body limits and replay byte/revision/share-size limits. The worker is an
explicit local JSON-lines automation boundary with its caller watchdog; it is not
an unauthenticated arbitrary-size Internet JSON endpoint. No universal input-size
or hard-preemption guarantee is inferred from `ExecutionBudget`.

No input-facing panic was normalized into a successful result. Native/property
and process checks protect actual rejected-input nonmutation/isolation, not an
assertion that all possible trusted-state corruption is recoverable. Full corpus,
load/latency measurements and release acceptance are still reported separately.
