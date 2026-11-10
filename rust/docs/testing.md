# Test tiers and fixture ownership

The [tier registry](../test-tiers.json) lists commands, scope, owners and when
checks are required. The [expensive gate catalog](verification-gates.md) gives
exact argv, required inputs and nonzero-test checks for the named gates. Run commands from the Rust workspace unless stated otherwise.

## Quick native

```sh
python3 tools/fixtures.py verify
cargo test --locked --workspace
```

This includes native unit tests, two bootstrap movement/codec tests and small
compatibility tests using bundled content. The smoke manifest records original
source commit, logical path, byte hash and semantic hash for each retained input.
Inputs were recovered from retained source history without re-exporting rules.
They are bounded test inputs, not the canonical or live differential corpora.
Gzip is deterministic; each case decompresses independently in memory.

Corpus-backed unit gates for unavailable newer payloads and exhaustive restore
scans have explicit `full-parity` ignore reasons. Corpus integration targets
require that feature. Setup/view/environment gates remain explicitly ignored.
The quick command reports ignores rather than hiding them behind a filter.

## Full differential and exhaustive restore

```sh
export GOA2_FIXTURE_CACHE=/absolute/path/to/verified/payloads
python3 tools/fixtures.py verify --full
python3 tools/check.py full
python3 tools/check.py restore
```

Preflight verifies all selected indexes and every case hash/name one at a time,
reporting expected, executed, missing, failed and skipped counts. It never uses
the smoke fallback. Missing or changed data makes preflight fail. A family filter
must name a registered, nonempty family. Canonical currently admits 6,592 cases;
live object execution admits 17,347. The complete-game and object-arena families
retain their independent coverage. The external monolithic reference families
are also registered and verified as payloads with admitted raw-byte hashes. Acquisition of the newer missing payloads is
a release blocker, not permission to regenerate changed identities.

`tests/fixtures/registry.json` classifies every retained top-level family and
records generator/provenance/retention. The committed allowlist, `.gitignore`
and registry are checked by tooling tests. Plain JSON remains supported for
local authoring; deterministic compressed payloads belong in the external cache.
Do not extract a full corpus or hold it all in memory. Do not delete a family
without a documented consumer and unique-coverage audit.

Explicit environment gates (run only with admitted data):

```sh
cargo test --locked -p goa2-core --test live_game_setup native_setup_matches_pinned_python -- --ignored --exact --nocapture
cargo test --locked -p goa2-core --test live_game_views -- --ignored --nocapture
```

Set `GOA2_GAME_CONTENT`, `GOA2_GAME_SETUP_CASES` and the view corpus variables
named by the test. An ignored gate is not a pass. Restore-sensitive changes need
the full-feature unit scans and the restore gates in `tests/object_execution.rs`,
with their specified filters/counts; see the historical fixture guide for
operation-specific commands. Do not use an empty name filter as evidence.

## Service contracts

From the repository root, after building the server:

```sh
python3 -m venv .venv-oracle
.venv-oracle/bin/python -m pip install --require-hashes -r rust/tools/requirements.lock
.venv-oracle/bin/python scripts/verify_rust_game_server.py rust/target/debug/goa2-game-server rust/content/smoke-game.json
```

`--suite auth`, `--suite bots`, `--suite clocks`, `--suite votes` and
`--suite shutdown` and `--suite backup` select independently runnable suites; repeat the option to
combine suites. Default runs all. Each suite reports expected/executed counts.
Helpers in `scripts/rust_verification/` preserve real processes, temporary
SQLite databases, concurrent requests, restart, backup, WebSockets and replay.
Never run these against a real game database.

## Optional Python tooling

The hashed tooling lock is independent of training/torch. Use CPython 3.11.17
for oracle generation; exact Pydantic/core/jsonpatch versions are mandatory.
Run `python -m pytest rust/tests/tooling tests/rust_oracle/test_fixture_cache.py`.
The full original Python engine suite remains separate; this cleanup does not
replace it or claim it ran when only native/tooling checks ran.

## Portable synthetic baselines

The [baseline set](../tests/baselines/README.md) records four disposable drivers
and their admitted inputs/observations. Run `python3 tools/baseline_games.py`
with an explicit `--binary` and `--output`. It covers manual 2D protocol, bots,
clocks, rollback, upgrades, restart and replay, and never records credentials or
private checkpoints. It is separate from full frozen parity coverage.

The [CI selection audit](audits/ci-gates.md) records conservative full/restore
selection and a complete release-candidate matrix on version tags or an explicit
manual candidate run. Input acquisition and remote execution results remain gates.
[Boundary/audience contracts](audits/compatibility-decisions.md) and the
[backup rehearsal](audits/server-harness.md) describe the small local regressions.
