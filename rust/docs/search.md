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

Maintained authoritative bot calls use `goa2_core::api::recommend_bot`. Worker-only
session/search/arena imports use the explicitly experimental facade described in
[API boundaries](audits/api-boundaries.md); this does not stabilize arena slots or
private fingerprints. The [worker examples](worker-protocol-examples.json) are
schema contracts, distinct from admitted gameplay/throughput inputs.
