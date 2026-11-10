# Server harness evaluation and backup rehearsal

Decision for C08-06, 10 November 2026: **retain the binary plus split real-process
contract harness; defer extracting a public server library**. Do not add a crate
or in-process test surface solely for cleanup.

## Evidence and tradeoff

The server already separates transport/auth (`http.rs`), orchestration (`game.rs`),
store/migrations, replay, shares, clocks and bots. `scripts/rust_verification/process.py`
owns disposable process launch, ephemeral localhost ports, API/WebSocket clients,
online backup and replay validation. Suites are independently selectable by the
maintained entry point. Pure Rust unit tests live beside codec/replay/storage
logic; the new serialization checks use this existing facility without adding a
production API or exposing private state.

A server library could reduce repeated startup and permit direct scheduler/storage
fault injection. It would also expose currently private `Server`/`Game`/`Store`,
router configuration and shutdown ownership, forcing a new dependency/API boundary
without a second embedding consumer. In-process calls would not verify real file
locks, WAL backup, socket authentication, graceful signal shutdown or persistence
across distinct processes. Current suites already inject transaction failures,
concurrent requests and shutdown while a database write is blocked. Their process
boundary is useful evidence, not incidental harness overhead.

The added backup suite demonstrates that the shared harness accommodates another
storage/transport contract without duplicating startup or adding production hooks.
Serialization units cover cheap direct checks; integration drivers retain actual
processes. A local full-suite run exposed a socket-startup race in the unstick verifier:
initial state arrives before the server finishes pending-vote synchronization.
A nonmutating `GET_VIEW` request/response barrier now finishes startup on both
sockets before proposing; exact proposal/result ordering assertions remain. No
sleeps, packet skipping or server-wire changes were added. This specific harness
fix supplies no justification for a new library. There is no measured startup-cost
result that justifies replacing this arrangement now. Revisit if an actual second server embedding
consumer appears, reliable measurement shows startup dominates verification, or a
specific failure cannot be tested through existing unit/process boundaries.
Evaluate that concrete boundary separately and retain process acceptance tests
even if a library is later introduced. Owner: service maintainers; named assignment
remains C07-03.

## Online backup and restoration (C08-02)

Run against a built candidate and bundled synthetic content:

```sh
python3 scripts/verify_rust_game_server.py /absolute/path/goa2-game-server rust/content/smoke-game.json --suite backup
```

The disposable scenario keeps a SQLite read transaction open while new game,
receipt, replay and share writes remain in a nonempty WAL. The service's `--backup`
command creates the consistent copy. All rows across seven retained tables,
integrity/foreign-key checks, player/spectator links, revisions, private checkpoint,
accepted retry/conflict behavior and replay reconstruction are compared after a
separate process opens the backup. The source advances after snapshot creation;
the copy retains the earlier revision. REST/WS reconnect and continued command
execution prove it can resume. Backup mode is private and an existing destination
is rejected without changing its bytes.

A separately labelled **synthetic terminal checkpoint** exercises independently
baked finished-game share metadata/groups and public routes; it is not a replay
of a missing completed-game corpus. Public frames contain no credentials or native
continuation/private checkpoint fields; unfinished-game sharing rejects. The
share survives restore independently of the active unfinished match.

This is an online backup/restoration rehearsal on disposable data, not a binary
rollback across historical versions (C08-03), an operator production procedure or
the extended hosted completed-match acceptance gate (C08-04). No real database,
remote service or original artifact is modified.
