# Extraction dependency audit

C01-01 uses `python3 rust/tools/dependency_audit.py --output REPORT.json` from
the repository root. Its [machine inventory](dependencies.json) records literal
Rust embeds/test modules, local Cargo path dependencies, Python local imports,
optional source/service/Lab imports, dynamic import sites, `__file__` root
resolution, logical data paths and the reverse input-to-consumer map.

The five extraction groups identify runtime, development, reference verification,
deployment and historical ownership. Runtime dependencies include all three
crate sources/configuration, eight frozen catalogues, oracle identity, embedded
hero/override server metadata and numbered migration SQL. Test support outside a
crate remains selected. No unresolved compile-time file/path dependency is
accepted by the audit. Registry crates/toolchain are supplied by locked Cargo
and the pinned compiler rather than copied out of a developer cache.

Python module imports are inspected through AST, without importing generators or
starting a service. Native runtime/quick tests have no Python gameplay dependency.
Optional reference exporters use the exact source/runtime artifact admitted by
`oracle-manifest.json`. Hosted adapters/replay-share and override export instead
require an explicitly compatible Python service and its locked environment; a
frozen oracle service is not assumed to implement the current hosted adapter.
FastAPI/httpx belong to that optional service environment, while ordinary oracle
and tooling packages use `tools/requirements.lock`.

The optional Lab situation importer requires the caller's `--lab`/`--python-src`
modules (`backend`, `state_model`); the optional rollout evaluator requires its
explicit `--lab-evaluator` file. Neither is copied, started or discovered as a
native dependency. The remaining dynamic test loader resolves included tool
modules, and `__import__('stack_traces')` resolves the selected bridge module.
These sites remain visible in the machine inventory rather than being treated
as unknown third-party packages.

Preserving the `rust/` prefix makes the observed `__file__` parent calculations
resolve to the extracted root. `run_pinned.py` now copies bounded committed
fixture index/bootstrap metadata into its temporary tree for corpus-backed
commands; full payloads stay in `GOA2_FIXTURE_CACHE`. It creates an empty diagnostic
output directory and never copies a whole corpus. Source, runtime and case hashes
remain required; this does not authorize a repin.

Packaging consumes the selected `rust/deploy/goa2-rust.service`. Hosted verifiers
and their shared process modules are selected together. Optional frontend,
source, full parity, arena-profile and profiling inputs are recorded with their
consumers in the extraction manifest. Missing external inputs remain gate failures.

Extraction now applies the declared exclusions to broad allowlist globs, rejects
symlinked files/parents, and refuses a destination inside its own source tree.
The explicit smoke subtree is the only exception to the broad generated-fixture
exclusion. Secret/environment/database paths cannot enter simply by being added
to a documentation directory. Tests check these boundary failures and a successful
ordinary extraction without the original Git database.
