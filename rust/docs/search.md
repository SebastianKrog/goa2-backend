# Search and performance maintenance

Native resolution search, sampled planning, greedy policies and arena automation
remain available through `NativeSession` and the JSON-lines worker. Bot
observations preserve public information boundaries and admitted upgrade history.
The worker operation inventory names the retained Python callers.

Correctness gates precede hot-path restructuring. Keep deterministic tie breaks,
RNG draws, cancellation/deadline behavior, completed-work accounting and fallback
boundaries unchanged. An unsupported rule is not a successfully scored rollout.
`legacy_fixed_followup` has reproducibility consumers; do not remove it by age.

For each performance comparison record content/oracle identity, compiler/profile,
hardware, seed, workload, policies, node/iteration limits, decision and result
identity. Use fixed work and alternating paired runs. Measure cold startup,
clone/restore memory, throughput, latency and cancellation separately. Report
strength/deadline-limited quality separately from fixed-work throughput. Rejected
or duplicate work does not count as a completed simulation.

Historical measurements remain in the archived development log with their
original scope. They are not fresh acceptance. Full parity/restore artifacts
must be available before the planned interpreter/search split or dense-path
retirement. C10/C11 follow-ups are tracked in [readiness](readiness.md).

Benchmark input ownership, identities, generators and consumers are recorded in
[`benchmarks/inputs.json`](../benchmarks/inputs.json). Test-only smoke seeds are
explicitly distinguished from full replay inputs and strength/profile recipes.
The [gate catalog](verification-gates.md) pins runnable simulation/strength
commands to reviewed bridge source; catalog validation is not a benchmark run.

Maintained authoritative bot calls use the `goa2_core::api` facade. Worker-only
session/search/arena imports use the explicitly experimental facade described in
[API boundaries](audits/api-boundaries.md); this does not stabilize arena slots or
private fingerprints. The [worker examples](worker-protocol-examples.json) are
schema contracts, distinct from admitted gameplay/throughput inputs.

The [maintenance baseline](audits/performance-baseline.md) records pinned release
artifacts, two content identities, hardware, fixed work and raw measurements for
startup, memory, clone/restore, preparation, throughput and cooperative response.
Deadline-limited response and fixed-work comparisons remain separate; no strength
claim or full refactor baseline is inferred from the opening-position benchmark.

## Arena v2

New arena profiles use selective four-turn search with the `nodes_v2`
work budget. The node cap is shared across forecasting, the short pass
and the selective pass. Reaching the cap interrupts expansion and keeps
the best completed choice. Search can also stop when it has resolved
the position or its shortlist has settled. The cap does not set the
width, rollout horizon or forecast allowance. The scorer records the
chosen caps and cutoff counts with each match.

The [calibrated node tiers](../arena/v2-node-tiers.json) approximate
end-to-end decision times on the reference WSL machine. The table uses
224 observations across six planning and ten resolution positions. The
full report is retained locally at
`$HOME/.local/share/goa2-arena-v2/calibration.json`. Planning and
resolution use separate node counts. These durations label the fixed
budgets. Match scoring uses node counts, with wall time used only by
the transport watchdog to abort a stalled worker.

Build the arena worker and create a profile from the bundled LONG
template. The template uses the one-second tier by default. Set
`--target-ms` to pin a different tier.

```sh
oracle_python="$HOME/.cache/goa2-rust/oracle-venv/bin/python"
arena_store="$HOME/.local/share/goa2-arena-v2"
sh rust/cargo-wsl.sh build --offline --locked --profile iterate -p goa2-search-cli
"$oracle_python" scripts/rust_oracle/run_pinned.py arena_profiles \
  --template rust/arena/v2-long.json --calibration rust/arena/v2-node-tiers.json \
  --target-ms 1000 --output "$arena_store/profile"
```

Use a v2-capable standard and candidate when registering and submitting
a new queue comparison. The match referee needs only the existing
rules protocol. The queue verifies each saved match against Python after
scoring. To recalibrate, run `run_pinned.py arena_calibrate` with the
worker, referee, per-phase cap sweeps and an absolute
`--midgame-corpus` pointing to `rust/tests/fixtures/complete-games-hanu.json`.

Register one generated profile per tuning batch, then submit its candidates
with the same profile ID. Change the template's tuning and holdout seeds
between batches. The tuning seed rotates the three screening maps and draws
fresh rosters. Holdout starts use a separate seed stream.

A negative or tied screen is recorded as `screened_out`. A tied screen
can be reopened with `arena_queue extend-tie JOB --evidence "REPORT_ID and observed benefit"`.
Guardrail failures are recorded as `unhealthy`, with a joint reason when
both sides exceed the fallback ceiling. Candidates that clear tuning wait
at `awaiting_confirmation`. After reviewing the batch, select up to two
with `arena_queue confirm JOB [JOB]`. Only those jobs run the held-out games.
Start the queue worker again after review if it has already drained.

## Live bot scoring

Live Rust bots use accumulated-gold scoring in planning and resolution.
After four full turns without a battle-zone shift or wave-counter change,
they add the Lane Advance bonus until lane progress resumes. The quiet-turn
clock is saved in live checkpoints and restored with rewinds. An older
checkpoint starts a fresh clock at its saved turn.
