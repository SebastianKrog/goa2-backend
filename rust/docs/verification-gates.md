# Explicit expensive verification commands

The versioned [gate catalog](../verification-gates.json) supplies concrete argv,
required file/directory/environment inputs, owners, triggers and minimum test
counts. Its bridge hashes pin command defaults/workload definitions to reviewed
source. It is distinct from a successful result or available artifact store.

From the repository root:

```sh
python3 rust/tools/gates.py validate
python3 rust/tools/gates.py list
python3 rust/tools/gates.py show setup-parity
python3 rust/tools/gates.py run setup-parity --report /absolute/path/setup-result.json
```

`validate`, `list` and `show` never execute a gate. `run` checks explicit inputs
and executes argv directly, without shell expansion. Source/runtime, full case
preflight and test minimums still apply. Missing data and zero-match Cargo test
filters fail. Reports contain status/counts rather than copying environment
credentials or raw checkpoints. Caller-supplied input/output paths must be
absolute so temporary pinned-source roots do not change their meaning.

| Gate | Inputs | Owner / trigger |
| --- | --- | --- |
| `setup-parity` | `GOA2_GAME_CONTENT`, `GOA2_GAME_SETUP_CASES` | Engine; setup/content changes, weekly reference run, release |
| `view-parity` | `GOA2_GAME_VIEW_CASES` | Engine/service; view/privacy changes, weekly reference run, release |
| `timed-bot-native` | `GOA2_GAME_CONTENT` | Service/search; clock/bot/budget changes, release |
| `full-differential` | `GOA2_FIXTURE_CACHE` | Engine; engine/worker/fixture changes, weekly run, release |
| `restore-exhaustive` | `GOA2_FIXTURE_CACHE` | Engine; codec/arena/restore changes, weekly run, release |
| `simulation-benchmark` | Exact oracle source, complete-game cache, release search binary, explicit report path | Search; hot-path changes/release, manual fixed-work run |
| `strength-baseline` | Exact oracle source and release search binary | Search; policy changes/release, manual matched evaluation |
| `hosted-local-contract` | Compatible Python executable/source, native game binary/content | Service/release; provisioning/draft changes and explicit pre-promotion check |
| `hosted-staging-readonly` | Explicit frontend/Rust origins, untimed game ID, packaged manifest; verifier credentials via environment | Release; explicitly selected deployed integration before promotion |

The exact executable/environment names and argv are in each catalog entry.
`GOA2_SERVICE_SOURCE` is the compatible service's Python source directory, used
as `PYTHONPATH`; its environment must include the service's locked FastAPI/httpx
requirements. It is not the native runtime or the pinned gameplay oracle.
The staging recipe omits canary-creation flags. Provisioning/deployed writes
require their separately selected workflow and destination.

Full/restore jobs already have change-selection and weekly/manual CI definitions.
This catalog names the other required release/manual/scheduled invocations; it
does not claim the full release matrix or hosted schedule is enforced remotely.
CI selection/release enforcement and periodic cold acquisition remain C04-07/08.
No benchmark, strength experiment or hosted request is run by defining recipes.
Record content/compiler/profile/hardware/seed/policy and measured units for actual
performance runs; throughput and deadline-limited quality remain separate.
