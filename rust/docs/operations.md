# Service operations and persistence

Use `GOA2_GAME_CONTENT`, `GOA2_GAME_DATABASE` and `GOA2_GAME_BIND` to select
explicit local resources. `--check-content PATH` checks content and reports the
binary/content identity. Leave Lab/provision tokens unset for ordinary local
manual games. Private checkpoints, replay archives, credentials, SQLite files
and receipts do not belong in source or public artifacts.

The service owns one database via its file lock and uses SQLite WAL, FULL
synchronous writes and foreign keys. State, accepted actions and journal entries
commit together. `store/migrations/001-base.sql` defines v1; v2 adds replay
history; v3 adds share creation dates. Legacy unversioned tables are inspected
for those columns before alteration. All pending DDL, backfill and version updates
commit in one transaction. Repeated startup is idempotent; newer schema versions
are rejected. Migration tests check failure rollback and preservation of receipts,
credentials and replay/share data; real process suites check restart and backup.

Before changing binaries or schemas, make a service-supported backup:

```sh
GOA2_GAME_DATABASE=/path/to/games.sqlite target/debug/goa2-game-server --backup /path/to/new-backup.sqlite
```

The existing backup implementation takes a consistent SQLite backup, checks its
integrity, fsyncs and publishes without replacing an existing output. Do not
copy only the live main database file while ignoring WAL. Restore into disposable
data first, verify revisions, links and receipts, and then follow the deployment's
maintenance procedure. A binary rollback across a database version must restore
the matching backup unless that binary explicitly supports the newer version.

Linux packaging is retained in `scripts/package_rust_game_server.py` with
`deploy/goa2-rust.service`: deterministic archive, binary/content hashes and
identity validation. Rehearse rollback on disposable data before promotion.
Release provenance must name source commit, toolchain, architecture, content hash,
database version, test tier/counts and backup. A checksum is not a signature.
Hosted ingress/Worker and backup verification remain a separate release gate.
