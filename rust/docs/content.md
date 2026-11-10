# Content and frozen reference source

Production catalogs live in `content/catalogs/`. Their bytes were relocated
without changing schema, oracle, rules identity or JSON insertion order.
`python3 tools/fixtures.py verify` checks them and the synthetic test pack.
The test pack selects seven heroes and one map from the prepared content export;
`content/smoke-manifest.json` records the full source-pack hash and selection.
It contains definitions only, not a player game or credentials.

A full service needs an admitted version-1 pack. Acquire it with an independently
trusted SHA256 and install it atomically:

```sh
python3 tools/content.py /path/to/live-game-content.json --sha256 EXPECTED_SHA256 --output target/live-game-content.json
```

HTTPS release URLs are supported too. Checksums detect byte changes; they do not
establish publisher authenticity. No public full-content download URL or
redistribution approval is established yet. Do not invent one. Validate the
installed pack with the built server's `--check-content` command.

## Oracle without original Git history

The frozen revision and every source-file/runtime hash live in
`oracle-manifest.json`. Keep the pin unchanged. Maintainers with the original
source objects can make a portable artifact:

```sh
python3 tools/oracle_source.py --git-repo /path/to/original/backend --output /path/to/oracle-source.tar
```

The exporter validates every admitted file before writing. Supply that artifact
in an extracted tree:

```sh
export GOA2_ORACLE_SOURCE=/path/to/oracle-source.tar
../.venv-oracle/bin/python ../scripts/rust_oracle/run_pinned.py live_game_content --help
```

The wrapper checks file hashes and the exact file set, rejects duplicates, links,
unsafe members and oversized source payloads, and extracts to a temporary root.
The existing runtime/source check still runs before generation. Current bridge
scripts are copied into the isolated tree; current Python gameplay is not used.
A missing historical commit now has an acquisition error rather than requiring
a silent repin. Export commands and fixture generators remain optional tools.
Full fixture publication still requires rights and provenance review.
