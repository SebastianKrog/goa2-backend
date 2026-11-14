# Private Rust cleanup handoff

Use the history-free export identified by its source map. The map records
each retained file's origin and SHA-256. The original baseline and extraction
plan commits are recorded there. This handoff describes the verified export.
Edits in the working checkout require a fresh export before they can be
included in a later build.

| Artifact | Identity |
| --- | --- |
| Standalone export, 1,213 files | Source map SHA-256 `5a193b0bf0f4e1b6be7ba3738ca5f14ec9f2add735031cffbde0d03a16e1996a` |
| Indexed fixture pack, 24,197 payloads | Archive SHA-256 `114fbf212334497877049b9434f70a51ecbe35c9ec5a50f377b688c38ec2611d`, index SHA-256 `cd66fdd8a4b602dd00a9d1cff108c8974e0ff0f47ae7542c61dfff24d6fc7106` |
| Full game content, 33 heroes and six maps | SHA-256 `4b7520f058d68dda93efb15837d0b657332a1a3132d37148c14a9bb2968ff651` |
| Pinned Python oracle source archive | SHA-256 `863351ef82d3027424da1e9d3bf04be4fae4e5c809759d84785fc50cb487945f` |
| Linux x86-64 game-server package | SHA-256 `dfbb21845a1403f12cb26ee3336cb9da4c03c3e3f0c68af918f86a5857aef636` |

In the operator's private cleanup archive, the export is in
`standalone-fixture-cleanup-final-check/`, the fixture pack is in
`verified-seekable-deduplicated/`, and the Linux package is
`standalone-fixture-cleanup-private-release/package-1.tar.gz`. The package
contains the full game content. The oracle tar is retained separately in
the private verification cache. Keep the pack's archive, index and manifest
together.
Use the [content guide](content.md) to install and verify the full-content
pack before starting the native server. See [verification evidence](cleanup-evidence.json)
for the compiler, binary and test-log identities. The private package was tested
from an empty home, cache and database. A wrong checksum was rejected before
installation, and the installed native server reported the pinned content
and binary hashes on loopback.

## Install and check the private Linux package

From the standalone export root, set these paths to the verified archive and
a new scratch directory. Use the archive hash listed above as the trust anchor
before extracting it.

```sh
set -eu
export PACKAGE=/absolute/path/to/package-1.tar.gz
export SCRATCH=/absolute/path/to/new-scratch-directory
printf '%s  %s\n' dfbb21845a1403f12cb26ee3336cb9da4c03c3e3f0c68af918f86a5857aef636 "$PACKAGE" | sha256sum -c -
mkdir "$SCRATCH"
tar -xzf "$PACKAGE" -C "$SCRATCH"
(cd "$SCRATCH" && sha256sum -c SHA256SUMS)
python3 rust/tools/content.py "$SCRATCH/live-game-content.json" \
  --sha256 4b7520f058d68dda93efb15837d0b657332a1a3132d37148c14a9bb2968ff651 \
  --output "$SCRATCH/installed.json"
"$SCRATCH/goa2-game-server" --check-content "$SCRATCH/installed.json"
```

To run the same native startup check, start the server with fresh local state.
Pick an unused loopback port if 3299 is occupied.

```sh
env -i HOME="$SCRATCH" PATH=/usr/bin:/bin \
  GOA2_GAME_CONTENT="$SCRATCH/installed.json" \
  GOA2_GAME_DATABASE="$SCRATCH/game.sqlite" GOA2_GAME_BIND=127.0.0.1:3299 \
  "$SCRATCH/goa2-game-server"
```

In a second terminal, run `curl -fsS http://127.0.0.1:3299/health`.
Expect rules version 9, the installed content hash, 33 heroes, six maps and
binary SHA-256 `90114ef9b4a6dbc1c0ce09dcc8f6d04cf1242f7391b8af302514b47e2b452f9f`.
Stop the disposable server with Ctrl+C.

## Verify and regenerate fixtures

From the standalone export root, point the fixture reader at the private pack.

```sh
export GOA2_FIXTURE_PACK=/absolute/path/to/fixture-pack
python3 rust/tools/fixtures.py verify --full
```

Expect 24,197 successful payload checks, plus the four complete-game and 230
snapshot-chunk checks. Follow [fixture archive instructions](fixture-archives.md)
to rebuild the pack from the admitted reference tar or regenerate individual
families. Regeneration uses the pinned Python oracle, a disposable output
directory and the native worker for complete-game fixtures. Compare regenerated
payloads and indexes to the committed registry before adopting them.

## Deferred verification and release follow-up

| Work | Responsible role | Acceptance |
| --- | --- | --- |
| Historical and standalone verification | Engine and search maintainers | Verify every corrected checkpoint and compare the same four fixed-work searches between adjacent revisions. Record semantic differences and timings with source and input hashes. Verify all 17,347 restore cases against the exact standalone export. |
| Contributor acquisition and CI | Engine and release maintainers | Choose private artifact access and verify a fresh runner can acquire the exact inputs, run the required jobs, and fail clearly on missing or wrong inputs. |
| Source publication and Python oracle release | Repository owner and release maintainers | Choose the source destination and history policy, approve the payload and rights, name maintainers, and record the oracle release on the Python fork when authorized. |
| Game-server hosting | Service maintainer | Choose the host and compatible frontend, then verify staging routing, ingress, backup/restore and rollback before deployment. |

The Hetzner verification workspace is private and is used for historical
checkpoint testing. Completed runs and log checksums are recorded in
[cleanup evidence](cleanup-evidence.json). A successful history run triggers
the fixed-work matrix of 12 sources, each with four 4,096-node workloads
and three repetitions. When that matrix completes, Hetzner runs all eight
restore shards against the exact 1,213-file standalone export listed above.
Track broader release decisions in the
[cleanup checklist](cleanup-checklist.md).
