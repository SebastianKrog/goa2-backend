# Frontend boundary

The reference consumer for this cleanup is frontend commit `a3c5e0a46b35a42cd6e054512ce3dbc6a61a579c`.
Its Worker proxies `BACKEND_ORIGIN`. A separate `RUST_BACKEND_ORIGIN` route is
not present in that checkout and is not assumed by this workspace.

The server exposes `/health`, `/heroes`, game creation, scoped game REST views,
commands and `/games/{id}/ws`. Each player has a bearer token; the spectator
has a distinct read-only token. Authentication and retry/stale-revision checks
remain in `goa2-game-server/src/http.rs`. WebSocket response names, input IDs,
selection values and revision semantics remain the current consumer contract.
The executable contract is `scripts/verify_rust_game_server.py` (auth suite).

The existing integration reference is retained in the original repository at
`docs/CLIENT_INTEGRATION_GUIDE.md`. The extracted native project preserves its
implemented compatibility boundary here and in [compatibility](compatibility.md),
not through a broken sibling link. Source types in `http.rs` and native
`live_game/view/` remain authoritative; additive engine facts include
`decision_context` and `battle_zone_minion_counts`.

Engine facts describe the offered decisions and computed gameplay values.
Frontend normalization may label and render those facts but must not reconstruct
hidden choices or invent rule legality. Do not change legacy fields or submissions
during cleanup. New metadata must be scoped to the authorized recipient, including
simultaneous decisions. Preserve 64-bit seed/time values, stale tokens and retries,
team inputs, generic prompts and event order. Physical minion counts are derived
per battle zone, not substituted with modified battle strength.

The manual acceptance sequence is creation in 2D (`?3d=0`), scoped REST view,
WebSocket update, input submission, reconnect and continued play. Hosted draft
promotion additionally needs an explicitly selected compatible Worker/frontend;
local proxy success is not hosted deployment evidence.
