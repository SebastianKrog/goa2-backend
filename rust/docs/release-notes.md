# Internal cleanup candidate notes

Candidate source `ed1597be` on `bots-upstream`, built with Rust 1.99.1 in release
profile for Linux x86_64. [Provenance](release-provenance.json) records exact
binary/content/archive hashes, compiler, schemas, checks and outstanding gates.
This is preparation for review; no release promotion, publication or deployment
has occurred. The packaged content is the seven-hero synthetic test pack, not a
full-roster product release. Checksums establish integrity, not publisher identity.

This candidate upgrades rusqlite 0.37.0 to 0.41.1 and bundled SQLite to 3.54.1,
resolving RUSTSEC-2026-0459 / CVE-2026-119375. A checked unsigned SQL adapter
preserves the earlier signed INTEGER limits and rejects out-of-range/negative
values; full-width seeds and timestamps remain in JSON. There is no intended
rules, oracle, content, command or wire-version change. The maintained scan has
no advisory waivers or license exceptions; workspace/content rights remain open.

New maintained tools cover native cold acquisition/build, admitted historical
checkpoint/replay/SQLite readers, matching-backup binary rollback, dependency
review and fixed-work maintenance measurements. The synthetic archive was
produced by the actual pre-cleanup binary; v1/v2 rows are explicitly constructed
migration intermediates, not separately released historical binaries.

Validation passes 402 native tests (64 explicit ignores), 44 tooling tests,
15 process scenarios, four historical schema rows, future-database rejection and
one actual old/new binary rollback using the matching original backup. A fresh
Cargo home and empty target also pass native build, quality, 402 tests and the
compiled example. Full preflight still lacks 23,955 indexed cases and 12
monolithic payloads. These local results do not establish full parity, remote CI
success or final C12 acceptance.

Before upgrading a real service, follow [operations](operations.md): make a
consistent SQLite backup using the backup API, retain the exact approved secure
binary/content/configuration, and verify the backup on a disposable copy with
links, revisions, receipts, reconstructed replay and share metadata. Schema 0–3
migrates transactionally to 3. Future schema 4 rejects unchanged. For rollback,
stop the service and restore the pre-upgrade backup with its matching approved
secure binary/content; do not point an older unguarded binary at the newer live
database. The obsolete pre-cleanup producer is used only as a trusted-data test
instrument: its vulnerable SQLite is not a production rollback recommendation.
Retain post-upgrade data separately for a deliberately reviewed reconciliation.

Release remains gated on independent artifact distribution/full suites, rights,
license/attribution, named ownership/security/destination, authenticity, hosted
checks and the owner's final payload decision. See [readiness](readiness.md).
