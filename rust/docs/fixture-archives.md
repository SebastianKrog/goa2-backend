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

The installer streams downloads and tar members to staging, checking the
archive checksum before extraction. Local archives use a hardlink when available
and are checked again before cache publication. Other local files and HTTPS
downloads use a separate staged copy. It rejects paths outside the cache,
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
A local full-corpus round trip packed and installed 24,197 files from a
private archive into a fresh cache using the standalone source extraction.
The archive checksum, file counts and verification reports are recorded in
[cleanup evidence](cleanup-evidence.json). Configured remote CI remains an
acceptance gate.

## Planned parity corpus consolidation

The current private frozen corpus remains the reference during migration. The
replacement must retain every named scenario and its rule, request, event-order,
object identity and restore assertions. We can distribute and test it privately.
Public publication and game-server deployment are separate milestones.

### Pack and deduplicate

1. Create a versioned, deterministic seekable zstd pack in test-index order. Use
   4 MiB frames and a bounded 128 MiB decoded-frame cache as the starting
   configuration. Store the fixture lookup index, original logical hashes and
   pinned oracle identity alongside the pack. Use the pack directly in both
   Rust and Python verification without a full unpack. Keep the original
   importer available during migration.
2. Preserve the name and provenance of every case. Store one payload for cases
   that differ only in their top-level name. Reconstruct the original named
   bytes for hash verification. Verify that names do not affect execution,
   normalization, selection or coverage accounting before sharing a replay.
   Use one verified replay per equivalent group and report the named cases it
   covers.
3. For distinct variations, share an immutable starting checkpoint or common
   operation prefix where their inputs truly match. Branch with private state
   for each variation and keep its expected results. Store repeated expected
   state as exact deltas where that reduces materialization work, following the
   existing complete-game traces. Use larger games where roster size or
   interactions are part of the rule under test.

The Python and native Linux test readers support a private pack directory through
`GOA2_FIXTURE_PACK`. It expects `fixtures-4MiB.tar.zst` and
`fixtures.index.json` and uses zstd for decompression. Python loads the Linux `libzstd.so.1` runtime, while Rust uses a test-only decoder dependency. With the
variable set, `python3 rust/tools/fixtures.py verify --full` reads external
payloads directly from the pack and still verifies them against the frozen
fixture registry. The native tests use the same variable for external
fixtures and continue to load committed indexes from the checkout.

Build a private pack from the admitted legacy tar using Python 3.14:

```sh
python3.14 rust/tools/fixture_seekable.py /absolute/path/reference.tar \
  --sha256 ARCHIVE_SHA256 --output /absolute/path/fixture-pack
```

The builder checks the source tar checksum, verifies the output against the
frozen registry, and records the pack, index and oracle identities in
`manifest.json` before publishing the output directory locally.

Pass `--deduplicate` to build the schema 2 index with name-only aliases.
Both local builds passed the full registry check over 24,197 fixtures.
The deduplicated build stores 1,818 fewer payloads and is 100.60 MB plus
a 4.84 MB index. The plain build is 110.63 MB plus a 4.25 MB index.
To verify exact distribution bytes, pass `--pack-manifest-sha256` with
`verify --full` and an independently pinned manifest checksum. The normal
`tools/check.py full` and `tools/check.py restore` commands accept the same
option and verify the pack before starting Cargo.

### Reduce repeated verification

- Restore shards traverse contiguous fixture ranges so the workers can reuse
  decoded frames once the pack reader lands. The partition test verifies that
  shard ranges are disjoint and complete and retain their prior case counts.
  Prefer matching the test order to the archive and sharing a bounded cache
  before adding concurrency.
- The experimental native differential path enables name-only replay sharing
  with `GOA2_SHARE_IDENTICAL_TRACES=1`. Each payload is still read, and
  only traces whose leading name field matches the fixture index and whose
  remaining bytes are identical can share a replay. The representative is
  parsed and replayed before any duplicate can reuse its result. The legacy
  runner remains the default until the side-by-side coverage gate passes.
- Compile common starting checkpoints once when reuse preserves aliases,
  request generations, rule state and worker isolation. Replay common prefixes
  once only where all relevant inputs and expectations agree. Run independent
  branches from the shared boundary.
- Deduplicate exhaustive restore work only when the full restorable state,
  pending request and generation state, alias structure, next operation, and
  required assertions agree. Preserve a coverage map from original named
  cases and restore boundaries to the checks that cover them. Keep frozen
  oracle replays as a deep verification gate.

Run the paired comparison on Linux with the pinned pack manifest. Supply
the installed frozen gzip cache to measure storage changes separately from
replay sharing. The legacy mode reads external payloads only from that cache,
even when a checkout also contains local copies. The comparison checks names against the tracked indexes and
exact bytes against the pack index in every mode.

```sh
python3 rust/tools/check_shared_replay.py \
  --pack /absolute/path/fixture-pack --manifest-sha256 MANIFEST_SHA256 \
  --legacy-cache /absolute/path/installed-reference --measure-rss \
  --output /absolute/path/new-benchmark-directory
```

The runner saves each completed comparison to `results.json`. After an
interruption, rerun unfinished restore shards in a new output directory
with `--gate restore --restore-shards 8 --restore-shard INDEX`. Repeat
`--restore-shard` for multiple shards and use the same pack, profile and
worker settings when comparing the results.

The three-mode comparison passed for the two fallback cases and all 6,592
canonical traces. Canonical tests took 448.89 s from gzip, 445.92 s from the
pack with independent replays, and 451.80 s with shared replays. Sharing avoided
91 canonical replays. All three live modes passed 17,347 named cases and
150,142 operation checks. Live tests took 2,464.13 s from gzip, 2,548.74 s
from the pack with independent replays, and 2,301.59 s with shared replays.
Sharing avoided 1,727 replays. Both packed modes decoded the same number
of frames. The gzip files were on the WSL filesystem and the pack was on
the Windows E drive. These timings exclude archive acquisition and Cargo
startup. The runner requested two workers, and the WSL service limited the
native tests to one effective worker. Combined canonical and live test time
was 48.6 minutes from gzip and 45.9 minutes with sharing. That is about 6%
less time than gzip, short of the 2x target. Sharing avoids complete identical
replays while distinct traces still run every operation.

All eight restore shards matched across the three modes, covering 17,347
named cases and 150,142 restore boundaries. Combined test time was
75.9 minutes from gzip, 76.9 minutes from the pack with independent
replays, and 69.6 minutes with sharing. The shared run executed 15,621
replays and 131,059 boundary checks, with identical coverage reported
for exact duplicates. Both packed modes decoded the same frames.
Sharing remains opt-in after these measurements.

A sample of 192 canonical traces found no repeated starting snapshots or
operation prefixes. In 192 object-execution traces, 180 remained after
name-only deduplication. Those contained 13 repeated starting snapshots and
39 repeated prefixes across 1,064 operations. The sample used sorted JSON
keys to count candidates. Safe checkpoint sharing still requires verifying
that any order and identity the engine observes are preserved. The same live
sample has 86 candidate repeats among 884 later restore boundaries when the
previous record, next operation, and resulting record all match with their
order preserved. The initial boundary repeats 13 times across 180 traces.
These records do not capture all native transport and restore state. Check
candidate keys against the full exported native envelope and pending request
generation before sharing the assertion. The native envelope carries
definitions, object bindings, board keys, and resume counters. The restore
test reattaches the last issued request separately, so a candidate key must
include that request and token as well. A root-relative execution fingerprint
alone omits the world state and does not establish equivalence across sessions.

A separate restore cost is the deep clone of each exported envelope before
compilation. The test needs the original for exact state, board-key, and
binding comparisons. The compiler reads the input and retains it behind
an `Arc`. A shared-input compiler entry point could move that allocation
outside the call, letting the test retain a reference to the same immutable
envelope. The restored session must still compile fresh mutable state,
re-export it, and apply the next operation independently. The paired
experiment matched all 271 cases and 3,650 boundaries on two shards. Times
were 18.10 versus 17.98 s and 73.77 versus 72.34 s, with the order reversed
on the second shard. The small gain did not justify adding a public
shared-input compile method.

`GOA2_SHARE_RESTORE_STARTS=1` enables an experimental cache of identical
compiled initial envelopes. Each worker keeps at most four compiled starts,
and envelopes larger than 4 MiB skip the cache. Each replay gets its own
clone. The isolation test passed, and the first 136-case comparison matched
coverage in both modes. It recorded no hits after exact-trace sharing, so
the cache stays opt-in. A second shard reused 76 compiled starts and took
202.96 s with the cache versus 184.28 s without it. The full eight-shard run matched all
17,347 named cases and 150,142 restore boundaries. It reused
2,092 compiled starts, including at least 543 across distinct traces.


Use the fixture registry to regenerate and verify families in a new directory.
Omit `--family` to run every registered generator and supply `--worker` for
the complete-game fixtures. An all-family run computes cache identities from
the fixtures it just generated. To regenerate cache identities separately in a
standalone checkout, pass the directory containing the top-level source fixtures
with `--identity-source`.

```sh
python3 rust/tools/regenerate_fixtures.py --python /absolute/path/oracle-python \
  --family states --family object-arena --output /absolute/path/new-fixtures
```

All 22 registered families reproduced their admitted hashes. The
object-execution check verified all 17,347 named payloads and rebuilt the
index byte-for-byte. See [cleanup evidence](cleanup-evidence.json).

### Acceptance

- Round-trip all 24,197 original fixture files by logical name and decoded
  SHA-256. Reject missing, corrupt or mismatched frames and indexes. Verify
  the coverage map against both the old and new test runners. Preserve
  complete-game, error, event-order and physical-instance alias coverage.
- Run both runners side by side. Match stable assertion IDs keyed by case,
  operation or restore boundary, and assertion kind. Compare expected and
  observed semantic records and require each old assertion to map to a
  verified new assertion. A shared check must account for every original ID
  it covers. Inject failures in representative groups and assert both runners
  detect them, including cases that differ by event order or alias identity.
- Make every retained fixture family regeneratable from pinned inputs,
  including reference errors. Run the registry's generator commands in a
  clean directory and verify case IDs, expected outcomes and admitted hashes.
  Diagnose any historical mismatch before replacing or repinning a fixture. The
  object-execution generator accepts `--workers 2` before `export` to
  parallelize the source replay with bounded in-flight snapshots. The
  registry uses the serial default so it also works on smaller machines.
  Historical export selects the frozen case names before replay and uses the
  original action selections in copied-card menu scenarios. It restores the
  historical serialization order of zones with identical hex membership and
  reproduces process-local step labels. The generator verifies the complete
  frozen hash of each case, including ordered events and explicit alias
  records, then checks the index byte-for-byte.
- Compare the old and new runners on the same machine, oracle, compiler,
  corpus and worker limits. Report artifact and index size, bytes decoded,
  cache misses, fixture load time, replay and restore wall time, peak memory,
  named cases, unique replays and verified restore boundaries. Record cold
  acquisition separately from repeat test runs.
- Target at least a 2x improvement in end-to-end full parity time and measure
  exhaustive restore separately. The read-only experiment is a baseline,
  and any shortfall must be explained before switching the default runner.

Replaying the same 17,347 restore lookups against the index-ordered layout
with a 128 MiB cache reduced modeled frame misses from 20,239 to 5,131 at
4 MiB, from 4,994 to 648 at 32 MiB, and from 1,287 to 168 at 128 MiB.
These are access counts, independent of engine and decoder speed.

The index-ordered experiment used no trained dictionary. Its 4 MiB pack was
110.63 MB and replayed differential reads in 13.0 s and restore reads in 38.4 s.
The 32 MiB and 128 MiB packs were 27.45 MB and 17.62 MB, with differential
reads of 30.5 s and 34.5 s and restore reads of 188.3 s and 201.9 s.
Sizes exclude the shared index. All variants round-tripped the frozen payloads.
Masking only the top-level case name revealed 986 duplicate groups with 1,818
extra payloads and 2.62 GB of redundant decoded bytes. Those counts establish a
storage opportunity. Execution equivalence still needs to be verified.
