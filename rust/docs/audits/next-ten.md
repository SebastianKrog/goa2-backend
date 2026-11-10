# Ten locally actionable cleanup tasks

Completed on `bots-upstream`, 10 November 2026, after checklist commit `ea1e2b0a`.
The [structured results](next-ten.json) preserve commands, counts and limits.
The checklist now has **10 completed and 74 open tasks**.

| Task | Result and maintained evidence |
| --- | --- |
| C01-01 | [Bidirectional extraction audit](extraction-dependencies.md), [machine graph](dependencies.json), required external inputs/consumers in `extraction-manifest.json`; 38 literal embeds/modules and two local Cargo dependencies resolve. Exclusions and symlink/recursive extraction guards are enforced. |
| C01-02 | [Four portable baselines](../../tests/baselines/README.md): manual scoped 2D protocol, bots, clocks, rollback, upgrades, restart and replay. Manifest admits source/content identities; drivers assert commands, scoped views, ordered events and revisions. Recorded trace contains 473 allowlisted observations. |
| C02-05 | [Retained Linux x86_64 / Rust 1.99.1 policy](../platform-and-packaging.md); no additional platform/MSRV support claimed. Python tooling remains the separately pinned 3.11 environment. |
| C03-06 | [Five benchmark input classes](../../benchmarks/inputs.json) distinguish full-game replay, generated workloads, arena starts, profiles and bounded contract seeds, with identity/generator/consumer/retention metadata. |
| C03-07 | [Error/guard classification](error-guards.json) distinguishes intentional rejection, porting limits and reference defects; source consumers, regressions and removal conditions are recorded. |
| C03-08 | [All 29 milestone records](material-inventory.json) retain raw hashes, last-change commits, original recorded compiler/scope and consumers; unknown historical compiler metadata is explicit. |
| C03-09 | [Retain-only material decision](material-ownership.md): 29 repository report files inventoried, no identical groups, no deletion. Unique information is preserved regardless of structural scan results. |
| C03-10 | [Cache identity audit](cache-identity.md) and enforced smoke oracle/schema, admitted full-index hash/count checks. Tests preserve ordered maps/lists, nulls, physical-instance distinctions, repeated aliases and full-width integers. |
| C03-11 | [Retained workspace/catalog placement](../platform-and-packaging.md); all three crates inherit `publish = false`. Public source hosting is the packaging target subject to the separate rights/payload decisions. |
| C04-06 | [Nine executable gate recipes](../verification-gates.md) with explicit inputs, pinned bridge hashes, owners/triggers and nonzero Cargo-test minimums. Missing inputs cannot turn into a successful no-op. |

## Verification

All **18 tooling tests passed**, both in the checkout and a disposable extraction
without the original Git database. All four portable service scenarios passed in
both trees; the extracted run emitted 474 observations. Poll counts and timings
vary, while driver predicates and the scoped projection define the contract.
The baseline generator uses temporary SQLite/processes and retains no credentials,
private checkpoints or replay bodies. Actual manual 2D rendering/reconnect evidence
remains the earlier frontend `a3c5e0a` walkthrough; this batch adds portable protocol
baselines rather than claiming a new browser run.

Quick data verification passed **572/572**, including in the extracted tree. All
nine gate definitions validated there. The setup gate ran its one explicitly
ignored native test and compared **28 opening states and identity graphs**.
The pinned-source wrapper admitted `simulation_bench --help` and
`rollout_parity --help` from the existing verified oracle archive; neither workload
was executed. Formatting, strict Python lint and diff whitespace checks passed.
Native production/gameplay sources were unchanged; earlier native/service-suite
counts remain separately dated evidence, not newly executed results.

Full preflight still exits 1: **23,955 indexed cases and 12 monolithic payloads
are missing**, with zero admitted mismatches. No fixture family, index or oracle
pin was replaced. The original milestone/error metadata remains byte-identical.

## Selection and remaining gates

After C01-01, these were the next nine tasks whose decisions, source inspection,
synthetic inputs and tooling were available locally. Rights/final-payload approval
(C01-03/04), real distribution/acquisition routes (C02-01–04), absent corpora and
complete registry acquisition/rights metadata (C03-01–05), and full/view acceptance
(C04-01–05) remain open. Running setup alone does not close C04-05.

The expensive-command task closes recipe definitions, not remote CI enforcement,
release scheduling, benchmark/strength results or hosted verification. These
remain their own checklist gates. Final public rights, destination, named owners,
history review and release acceptance are unchanged; nothing was published.
