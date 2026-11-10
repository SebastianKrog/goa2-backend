# Admitted historical synthetic compatibility archive

`manifest.json` admits the compressed payload by raw/gzip hashes, bounded size,
content/oracle identity and actual producer source/binary/compiler identities.
The producer is pre-cleanup backend `f0fb93e1`, built from its retained Git archive
with Rust 1.99.1 and profile `iterate`. It generated disposable native games,
accepted receipts, private checkpoints/replay and a deliberately synthetic terminal
share. Generated bearer values were replaced with explicit test-only labels;
the same historical binary reopened/authenticated them before admission. There
are no real matches, players, cloud credentials or live database files here.

Only the actual producer database is historical unversioned schema 0. The v1/v2
cases are explicitly constructed numbered migration intermediates, omitting
columns unavailable at those stages. They are not claimed to come from separately
released v1/v2 binaries. Version 3 uses the same historical rows at the current
schema; it does not regenerate gameplay. Synthetic terminal shares exercise baked
storage/projection, not a completed-match parity corpus.

Run from the repository root against the current native binary:

```sh
python3 rust/tools/historical_compatibility.py verify --binary /absolute/path/goa2-game-server --content rust/content/smoke-game.json
```

The four rows check load, player/spectator scopes/links, receipt replay, exact old
checkpoint and private archive reconstruction, independent share state, commands
and ordered reply events, actual WS updates, process restart, retry and rollback.
Future database version 4 must reject without changing database bytes. Raw and
compressed payload identity must pass before opening a restored disposable copy.

For the binary rollback rehearsal, add `--producer-binary /absolute/path/producer-server`.
Its SHA must match the admitted producer artifact. This makes a backup under the
old binary, upgrades/advances with the current binary, then reopens the matching
pre-upgrade backup with the old binary and continues play. The old binary has no
future-schema guard; the rehearsal never tries it on the newer live database.
The obsolete producer's SQLite is affected by the advisory fixed in the current
candidate: it is a trusted-data compatibility instrument, not a deployment or
production rollback recommendation. Actual rollback requires an approved secure
binary and its matching consistent backup.

Regeneration is explicit and refuses an existing destination. Build exactly the
recorded producer source/toolchain/profile and pass that binary to the `generate`
action with a **new** `--fixtures` directory. Review/admit changed identities as
new evidence; never overwrite this archive to make a regression pass. Historical
source is needed only for regeneration/rebuilding the producer; ordinary reader
verification uses this archive without original Git or Python gameplay.

The bounded payload contains synthetic private engine projections and inert test
credentials on purpose. Public redistribution/payload clearance remains the
independent C01/C07 decision; admission is not a rights grant.
