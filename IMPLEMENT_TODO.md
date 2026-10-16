# Implementation TODO: live player history

## Goal and agreed behavior

Allow players and spectators to browse earlier board states during a live game,
including after disconnecting or reconnecting. Keep the complete audit replay,
but exclude abandoned attempts and correction operations from player browsing.

- The client buffers the latest 30 complete historical views and fetches missing
  or older positions from the server.
- Keep sending complete player-scoped live views. Changing live delivery to
  patches is a separate optimization.
- Store server history as periodic snapshots plus JSON Patches. New history
  reads must not require engine re-simulation.
- Capture information visible to each viewer at that historical moment,
  including their own private cards. Never serve omniscient history to players.
- Collapse each planning phase into one replaceable position. After planning,
  capture each accepted decision and its deterministic processing, rather than
  each atomic engine step.
- Browsing is read-only and does not pause the match or its clocks.

## 1. History model and corrections

Maintain two separate histories:

- **Audit history:** the existing append-only decision log plus immutable
  captured view records. Preserve corrections and abandoned attempts.
- **Player history:** an ordered index referencing captured states currently
  valid for browsing.

Each captured record has an immutable, monotonically increasing `record_id`.
Each player position references a record and includes its phase, round, turn,
timestamp, and associated raw decision count. Never renumber or reuse IDs.
Use the record ID as the public `position_id`; replacements receive a new ID.

Maintain a monotonic `revision` for index changes and an `epoch` that increases
whenever existing player positions are replaced or removed.

- [ ] Capture an initial position for games created through REST and draft.
- [ ] During each planning phase, maintain one position representing its latest
      state. Replace it after commitments, passes, second-card choices,
      starting-position changes, and takebacks.
- [ ] Preserve the final pre-reveal planning position when resolution begins.
      For the decision that triggers revelation, capture its pre-decision state
      as the final planning position and its post-decision state as the first
      resolution position. Do not manufacture a post-commit, pre-reveal state
      that the engine never exposed.
- [ ] After planning, append a position after every accepted decision and its
      deterministic processing, including automatic timeout decisions.
- [ ] Capture non-decision `advance` operations only if they change the
      canonical historical view. Do not append duplicates for broadcasts,
      reconnects, readiness changes, pointers, pings, or clock ticks.
- [ ] Action rollback: remove positions after the exact engine rollback anchor
      and refresh the restored head. Do not append an undo browsing step.
- [ ] Planning takeback: replace the current planning position. This preserves
      other players' commitments without retaining the abandoned commitment.
- [ ] Table rewind: restore the player index associated with the targeted raw
      decision prefix and refresh its head. Do not append a rewind browsing step.
- [ ] Retain internal index edits by raw decision prefix so nested table rewinds
      reproduce the correct branch, including targets omitted from browsing.
- [ ] Preserve existing admin replay, override history, event history, and
      shared replay behavior.

Rollback anchors must follow actual `GameSession` snapshot creation and
invalidation, rather than turn numbers or actor changes. The current engine
re-anchors after foreign responses and hidden-information boundaries.

- [ ] Expose an explicit rollback-anchor generation identifier on `GameSession`.
      Change it whenever a snapshot is newly created or invalidated; keep it
      across repeated restores to the same anchor.
- [ ] Persist the identifier with the rollback snapshot and map it to a captured
      history position. Clear stale mappings when the engine invalidates them.
- [ ] Restoring or re-anchoring must never prune another player's valid response
      or cross a hidden-information boundary that the engine protects.

## 2. Capture and canonical views

Add a shared history service used by REST actions, WebSocket actions, automatic
timeouts, starting-position changes, and applied overrides. Do not use broadcast
or result logging as the sole capture hook: some mutations have no SessionResult,
and a timeout batch can contain multiple decisions before one broadcast.

- [ ] Capture after each successful individual decision under `game.lock`, after
      deterministic processing and before persistence or broadcast.
- [ ] Capture each intermediate result in a multi-decision timeout batch,
      including automatic commit followed by automatic finish-planning.
- [ ] Associate captures with raw decision counts, excluding clock telemetry,
      so existing replay and rewind indices retain their meaning.
- [ ] Build projections for every seated hero and one spectator, including
      disconnected seats. Reuse the prebuilt public board during projection.
- [ ] Apply visibility before calculating patches. Never call the omniscient
      replay `state_body()` to serve player history.
- [ ] Retain historical `view` and `winner`. Omit actionable input requests,
      tokens, connection metadata, and client timing fields.
- [ ] Set historical `view.clock` to `null`; retain `time_control`. Historical
      clock playback is outside v1 and must not display a ticking live clock.
- [ ] Deep-copy/freeze canonical values so later live mutations cannot modify
      cached or stored historical projections.
- [ ] Reuse captured canonical projections for live broadcast construction where
      practical, without removing live clocks or inputs from live messages.

## 3. Storage, durability, and recovery

Use the existing `jsonpatch` dependency and immutable compressed sidecar records
beneath the game persistence directory. Keep the committed history manifest in
the existing atomically replaced game save. Do not rewrite historical view
payloads on every move.

- [ ] Store an initial full keyframe and another every 30 captured records.
- [ ] Between keyframes, store forward RFC 6902 patches against the immediately
      preceding audit record, with an explicit base record ID.
- [ ] Compress each immutable record with gzip. Store all viewer projections
      together so compression can reduce repeated public content.
- [ ] Reconstruct a selected viewer from its nearest keyframe with at most 29
      patches. Keep only the latest projections and a bounded read cache in RAM.
- [ ] Do not persist reverse patches in v1: clients cache complete views, and
      server reads reconstruct forward from bounded keyframe chains.
- [ ] Extend saves with the committed record index, active player index,
      revision, epoch, raw-prefix index edit metadata, and rollback-anchor map.
- [ ] Write new immutable files through temporary files and atomic replacement,
      then atomically save game state and the manifest referencing those files.
      Ensure files are flushed before publishing the referencing save.
- [ ] On restart, accept only records referenced by the saved manifest; remove
      orphan files left by interrupted operations. Restore history before timers.
- [ ] If a record write fails, publish no reference to that record. Mark a
      history gap, log the failure, and continue live gameplay. Start a fresh
      keyframe on the next successful capture.
- [ ] If the game save fails, retain pending records in memory for the next save;
      restart remains anchored to the last successful game save, matching the
      restored game state. Do not infer committed history from directory contents.
- [ ] Validate missing/corrupt files and patch bases on read. Report unavailable
      ranges rather than returning a wrong board. Attempt replay repair off-loop
      when compatibility permits.
- [ ] With persistence disabled, retain equivalent in-memory history for tests
      and ephemeral games; do not promise recovery across server restart.
- [ ] Remove sidecars when their game save is removed. Keep existing independent
      replay-log retention unchanged. Use only server-generated, validated paths.

## 4. Authenticated API and live protocol

Use the existing game-scoped bearer authentication. Players receive only their
projection; spectators receive the public projection. Never accept a requested
viewer identity or `reveal_all` flag from the client.

| Endpoint | Behavior |
| --- | --- |
| `GET /games/{game_id}/history` | Page of active position metadata, revision, epoch, total, and availability. Default latest 30; support `before_position_id`; maximum 100. Return entries chronologically. |
| `GET /games/{game_id}/history/state?position_id=...` | Complete historical view, winner, and position metadata. |
| `GET /games/{game_id}/history/frames?from_position_id=...&limit=30` | Keyframe at the requested active position plus forward patches for up to 30 consecutive active positions, with each position's metadata. |

- [ ] Accept an expected revision for index and grouped reads; return `409` if
      the index changed and the request must restart.
- [ ] Return `410` for superseded positions and `404` for unknown positions.
      Describe unavailable ranges explicitly in the history metadata.
- [ ] Build client-facing patch groups exclusively from active positions.
      Internal audit reconstruction may cross abandoned records, but never send
      those records or intermediate patches to the player.
- [ ] Snapshot read metadata under `game.lock`, read and reconstruct outside the
      lock, then verify the requested positions and epoch remain valid before
      returning. Grouped reads must also validate their expected revision.
- [ ] Use private, non-shared caching headers. Key reconstructed-view caches by
      game, viewer, and immutable record ID; bound them to 128 views process-wide.
- [ ] Add optional `history` metadata to REST game views and WebSocket
      `STATE_UPDATE`: revision, epoch, latest position ID, its active predecessor,
      active count, and availability. Return a position ID only when the supplied
      live view corresponds to that position after historical normalization.
- [ ] Distinguish updates with no new history position from new captured states,
      so readiness and timer updates do not generate duplicate client history.
- [ ] Update response models and `docs/CLIENT_INTEGRATION_GUIDE.md` together,
      including pagination, correction invalidation, gaps, and example payloads.

## 5. Client buffer and browsing UI

Implement frontend integration in the companion frontend repository. Its live
socket handler currently replaces `view` from each full `STATE_UPDATE`; events
drive animations and logging, rather than reconstructing the game.

- [ ] Add history API types and a `useGameHistory` hook. Cache at most 30 complete
      historical views by immutable position ID.
- [ ] Buffer eligible live views with history IDs, normalizing them using the
      same historical rules, including a null clock. Never mutate the live view.
- [ ] If a position's predecessor is missing, fetch that group when browsing
      reaches the gap. One received message need not equal one backend decision.
- [ ] On epoch change, clear history caches and refresh the active index.
      Append-only revisions preserve existing cached positions.
- [ ] Fetch older groups on demand. Retain the displayed position and closest
      neighbors while evicting distant entries to maintain the 30-view bound.
- [ ] Cancel or ignore responses for old epochs, games, or viewers; clear caches
      on seat changes. A `409` refreshes the index; a `410` drops the stale entry.
- [ ] Keep the newest live state separate from the displayed historical state.
      Continue receiving live updates while browsing.
- [ ] Add a History entry point, previous/next controls, position details, and
      Return to live. Reuse existing replay controls where appropriate.
- [ ] Disable gameplay submissions, selections, pings, and live interaction
      controls while browsing. Keep connection and live-turn notifications.
- [ ] If the displayed position is superseded, return to live with a brief
      explanation. Do not automatically play incoming live animations on the
      historical board.
- [ ] Scope caches to the authenticated viewer and do not persist private view
      data in shared browser storage.

## 6. Existing games and backfill

- [ ] When restoring an older save, capture its current state immediately as a
      baseline, then capture new decisions normally.
- [ ] Backfill earlier states from a consistent replay prefix in a worker process.
      Reuse the same capture and correction/index rules; never run expensive
      reconstruction under the live game lock.
- [ ] Tag baseline/new captures with their raw decision counts. Merge the
      backfilled prefix with live capture metadata atomically; retry the merge
      if a correction changed the epoch while reconstruction was running.
- [ ] Build anchor and prefix metadata during backfill so later table rewinds
      into the reconstructed range use the correct effective player history.
- [ ] Mark reconstructed ranges as such. Recorded rules and current engine
      compatibility limit backfill fidelity; historical clocks remain absent.
- [ ] If reconstruction fails, keep the current baseline and subsequent captures,
      expose the available range, and log the failure. Do not block live play.
- [ ] Preserve captured history across engine updates; only backfill/repair uses
      engine simulation. Include a history storage format version for future
      migrations independent of game rules versions.

## 7. Validation and acceptance

- [ ] Disconnect and reconnect a player, including a server restart; fetch earlier
      states without relying on any browser-retained history.
- [ ] Verify every keyframe/patch chain equals the canonical captured view for
      all viewers; test keyframe boundaries, empty patches, removals, and arrays.
- [ ] Test visibility for hands, commitments, guesses, reveals, tokens, upgrades,
      and spectators, both before and after public information changes.
- [ ] Test interleaved planning commitments, takebacks, repeated takebacks, and
      two-card heroes; only one current planning position survives.
- [ ] Test rollbacks at real segment anchors, including foreign responses,
      hidden reveals, repeated rollback, and restart with an active anchor.
- [ ] Test table rewinds and nested rewinds, including targets in planning,
      previously superseded segments, and reconstructed history.
- [ ] Test every capture entry point, particularly automatic commit plus
      finish-planning in one timeout batch, starting-position changes, and
      overrides that produce no SessionResult.
- [ ] Verify client-facing groups do not expose abandoned states via their
      keyframes, patches, labels, or metadata.
- [ ] Exercise stale revisions, superseded/unknown IDs, read/correction races,
      interrupted writes, failed saves, orphan files, missing records, and repair.
- [ ] Test client cache misses, eviction, epoch changes, late responses, viewer
      switches, and receiving live updates while browsing.
- [ ] Run the complete backend server suite and frontend tests, type checks,
      and production build. Add focused engine tests for anchor lifecycle.
- [ ] Benchmark a long ten-player game: record p50/p95 capture latency, cold and
      warm reads, save growth, memory, and compressed disk usage. Target less
      than 10 ms additional p95 decision latency on the deployment host; reuse
      board/projection work and tune compression/cache sizes before enabling by
      default if the target is missed.

Acceptance: a fresh client can browse all available active history; captured
ranges require no engine replay; browsing never changes or pauses live play;
abandoned attempts stay out of player browsing; audit history remains intact;
and private information remains scoped to the viewer at the historical moment.
