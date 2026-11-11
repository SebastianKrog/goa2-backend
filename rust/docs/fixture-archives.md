# Reference fixture archives

The acquisition tool installs a complete reference corpus from a local tar
archive or HTTPS URL. Supply the archive's independently trusted SHA256.
Its contents are then checked against the checkout's frozen indexes and raw
payload hashes before the cache becomes visible.

From the Rust workspace:

~~~sh
python3 tools/fixture_archive.py install /path/to/reference.tar --sha256 EXPECTED_SHA256 --output /new/cache
export GOA2_FIXTURE_CACHE=/new/cache
python3 tools/check.py full --profile iterate
~~~

Use an HTTPS URL as the first install argument for remote acquisition.
Checksums establish byte integrity. Publisher authenticity and redistribution
permissions remain separate release decisions.

## Preparing an archive

Keep the source cache unchanged while packaging. It must contain every
registered parity case, all 12 external monolithic payloads and every referenced
complete-game snapshot chunk.

~~~sh
python3 tools/fixture_archive.py pack --source /complete/cache --output /new/reference.tar
~~~

Packaging verifies the complete source corpus first. The resulting tar contains
only admitted payloads and referenced chunks, in sorted order, with fixed
timestamps, IDs and permissions. Existing plain JSON or gzip bytes are retained.
No personal owner names, unrelated files or private databases are packaged.
The command prints the archive SHA256, file count and validation reports.

The archive excludes committed indexes and runtime catalogs. Consumers use their
checkout's committed registry/index identities. A changed or incomplete
reference bundle cannot be admitted through a warm local cache.

## Installation and retries

The installer streams the download and tar members to staging, checking the
download checksum before extraction. It rejects paths outside the cache,
symlinks, sparse files, duplicate logical files, unknown members, unreferenced
chunks and oversized inputs. Archive/download size is limited to 8 GiB and
individual compressed or expanded payloads to 256 MiB. Every payload is
validated against the new staging
directory alone. Successful validation publishes the cache by directory rename.

A failed installation leaves an existing cache untouched and removes its normal
staging directory. A process lock prevents concurrent installers from replacing
the same destination. The lock file can remain after completion; the operating
system releases its lock when the process exits, so it does not block a retry.
Existing destination directories are immutable and must be verified/reused or
given a different output path.

## CI configuration

The full-differential and restore jobs require these repository variables:

- GOA2_FIXTURE_ARCHIVE_URL for the approved HTTPS artifact.
- GOA2_FIXTURE_ARCHIVE_SHA256 for its independently trusted archive checksum.

Each job installs and validates the archive into its own runner cache, then
runs its selected verification tier. Missing configuration, download failures,
integrity failures and missing payloads fail the job.

No public artifact location or redistribution approval has been established.
The tool and synthetic roundtrip tests provide the acquisition mechanism.
A real full-corpus archive roundtrip and configured remote CI result remain
acceptance gates.
