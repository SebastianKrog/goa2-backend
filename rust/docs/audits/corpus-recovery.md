# Recovered full corpus and independent source rehearsal

Verified on 11 November 2026 after Arena v2 merge 565b467b.
The [structured evidence](corpus-recovery.json) records counts, commands and
artifact identities.

The original private WSL cache passes every admitted fingerprint. Verification
covers 6,592 canonical traces, 17,347 live cases, 12 object-arena cases, four
complete games and 12 monolithic payloads. The complete-game verifier now also
checks all 230 referenced snapshot chunks, their hashes and record counts, and
their agreement with each game's operation count. Missing or corrupt chunks
fail preflight. The original case/index/oracle identities were retained.

This closes C03-01, C03-02, C03-03 and C04-02. The separate setup/view run closes
C04-05 with 28 opening states and 65 player/spectator projections. These are
artifact and projection checks. Full differential execution and exhaustive
restoration have their own pending results.

## Reproduction and resources

From the Rust workspace, using the original admitted payload directory when
it is outside the checkout:

~~~sh
export GOA2_FIXTURE_CACHE=/absolute/path/to/admitted/payloads
python3 tools/fixtures.py verify --full
python3 tools/fixtures.py verify --full --family complete-games --family complete-games-hanu
~~~

Case, snapshot-chunk and monolithic-payload counts are separate. The first
full scan used the pre-extension verifier; the second command verifies its
additional chunk coverage. The full native tier repeats current preflight
before executing the differential tests.

The local scan used a transient systemd user scope with MemoryMax=1G and
MemorySwapMax=0. The queued full/restore pipeline uses MemoryMax=3G,
MemorySwapMax=0, CPUQuota=200%, and one Cargo, test and oracle worker.
Results and streamed child output are retained in a private persistent cache.
A reboot can interrupt computation; only a recorded successful exit closes a gate.

## Generator and acquisition checks

The registry now names concrete source files, current exporter hashes, exact
wrapper argv, runtime/rules identity and concrete consumers for the supported
generators. Commands use an absolute disposable OUTPUT and, for streamed
complete games, an explicit native WORKER. The frozen-source wrapper exposes
these exporters. Their 18 distinct help interfaces passed in an extraction
without .git or sibling checkouts.

The full pack and setup corpus were regenerated there using a verified 5,079,040
byte oracle archive and the pinned Python runtime. Both hashes matched the
original admitted content/setup inputs. The native service loaded all 33 heroes
and six maps with an empty process environment. The source archive checksum is
recorded in the structured evidence.

Generator commands are investigation recipes. Compare regenerated output with
the admitted fingerprints before installation. An exporter update cannot
authorize a changed frozen case. The historical effect-error capture has no
current standalone exporter and remains explicitly identified as such.

The private rehearsal does not supply a public distribution URL, rights
approval or a successful remote CI run. Full corpus acquisition on an
independent contributor machine remains C02/C03/C04 release work.
