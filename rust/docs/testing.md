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

Acquire a complete cache with the [verified archive workflow](fixture-archives.md).

```sh
export GOA2_FIXTURE_CACHE=/absolute/path/to/verified/payloads
python3 tools/fixtures.py verify --full
python3 tools/check.py full
python3 tools/check.py restore
```

Preflight verifies all selected indexes and every case hash/name one at a time,
reporting expected, executed, missing, failed and skipped counts. Complete games
also verify each referenced snapshot chunk, its raw hash and record count, and
that chunk record totals match the operation count. Chunk totals are reported
separately from case totals. Preflight never uses
the smoke fallback. Missing or changed data makes preflight fail. A family filter
must name a registered, nonempty family. Canonical currently admits 6,592 cases;
live object execution admits 17,347. The complete-game and object-arena families
retain their independent coverage. The external monolithic reference families
are also registered and verified as payloads with admitted raw-byte hashes. The original local WSL cache passes these admitted identities. Distribution to
other checkouts remains a release gate; regenerated mismatches cannot replace
frozen inputs.

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

`--suite auth`, `--suite bots`, `--suite clocks`, `--suite votes`,
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

## Bounded robustness and worker protocol

`cargo test --locked -p goa2-core --test robustness` checks corrupt references,
coordinate/catalog boundaries and prepared-candidate restore/privacy/isolation.
Worker schema examples and internal diagnostic/executor regressions are in the
normal native suite. Run the real JSON-lines rejection/correlation check from the
repository root with `python3 rust/tools/worker_contracts.py --binary /absolute/path/goa2-search-cli`.
See [versions and robustness](audits/versions-and-robustness.md) for exact scope;
these bounded cases do not substitute for full external parity or restore scans.

## Cold native, historical and dependency checks

The weekly/manual `cold-native` workflow uses `tools/cold_build.py` with new
Cargo/target directories and no original Git history; installed pinned compiler
reuse is explicit. From the repository root:

```sh
python3 rust/tools/cold_build.py --work /absolute/path/new-cold-run --report /absolute/path/new-cold-report.json
python3 rust/tools/historical_compatibility.py verify --binary /absolute/path/goa2-game-server --content rust/content/smoke-game.json
cargo install cargo-deny --version 0.20.2 --locked --root /absolute/path/scanner
python3 rust/tools/dependency_review.py --scanner /absolute/path/scanner/bin/cargo-deny --output /absolute/path/new-scan.json
```

[Cold evidence](audits/cold-native.json), [historical matrix](audits/historical-matrix.json),
[archive provenance](../tests/compatibility/README.md) and [dependency review](audits/dependency-review.json)
record exact scope. Remote workflow results remain open. Historical matrix
verification needs the admitted archive and native binary, not original history
or Python gameplay. Full frozen cases remain a separate required tier.

## Long verification jobs in WSL

Keep a foreground WSL command attached for the lifetime of jobs launched through
systemd. Services and scopes can outlive their launching shell, but systemd
services alone do not keep a WSL instance alive. See the
[Microsoft systemd guidance](https://learn.microsoft.com/en-us/windows/wsl/systemd).

A clean WSL shutdown interrupts an unfinished verification stage. Retain its
partial log and require a completed successful result before closing a gate.
The local bounded workflow uses a separate foreground keepalive, one compiler
job and explicit cgroup memory limits. The retry uses two test threads with one
case worker for full verification, then one test thread with two case workers
for exhaustive restore. Current verification was reduced from
3 GiB to 2 GiB when Windows commit headroom became small. Resource settings
belong in the run evidence rather than changing game/search semantics.

To see progress from the indexed oracle comparisons while they run, use the
same admitted fixture cache and run the trace stage with output capture disabled:

~~~sh
GOA2_ORACLE_PROGRESS=1 cargo test --locked --profile iterate -p goa2-search-cli --bin goa2-search-cli trace_tests:: -- --ignored --nocapture
~~~

The indexed comparisons print a count every 512 completed cases and at the
end of each index. The full verification tier also runs the native suite and
fixture preflight.
