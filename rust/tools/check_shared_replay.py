#!/usr/bin/env python3
"""Run paired native fixture gates and compare their named operation coverage."""

import argparse
import json
import os
import re
import subprocess
import time
from pathlib import Path

from compare_trace_coverage import compare
from fixtures import WORKSPACE, verify_pack_identity

TRACE_TESTS = {
    "canonical": "every_native_transition_matches_python_full_state_request_and_event_order",
    "live": "live_shared_steps_and_context_match_python_values_requests_and_identity",
    "fallback": "primary_text_fallback_movement_and_attack_match_python",
}
RESTORE_TEST = "every_shared_execution_boundary_restores_values_aliases_and_request_generations"


def commands(gates, profile, shards, selected_shards=None):
    common = ["cargo", "test", "--locked", "--profile", profile]
    harness = ["--", "--ignored", "--exact", "--nocapture"]
    for gate in gates:
        if gate == "restore":
            for shard in selected_shards or range(1, shards + 1):
                yield (
                    f"restore-{shard}-of-{shards}",
                    [
                        *common,
                        "-p",
                        "goa2-core",
                        "--features",
                        "full-parity",
                        "--test",
                        "object_execution",
                        RESTORE_TEST,
                        *harness,
                    ],
                    {"GOA2_RESTORE_SHARD": f"{shard}/{shards}"},
                )
        else:
            yield (
                gate,
                [
                    *common,
                    "-p",
                    "goa2-search-cli",
                    "--bin",
                    "goa2-search-cli",
                    "trace_tests::" + TRACE_TESTS[gate],
                    *harness,
                ],
                {},
            )


def expected_cases(gate, packed_fixtures):
    filename = "stack-traces.json" if gate == "canonical" else "object-execution.json"
    cases = json.loads((WORKSPACE / "tests/fixtures" / filename).read_text())["cases"]
    label = filename
    if gate == "fallback":
        names = {f"object-execution-primary-text-fallback-{kind}" for kind in ("attack", "movement")}
        cases = [row for row in cases if row["name"] in names]
        if {row["name"] for row in cases} != names:
            raise ValueError("Fallback cases missing from fixture index")
    if gate.startswith("restore-"):
        _, shard, _, shards = gate.split("-")
        shard, shards = int(shard), int(shards)
        size, extra = divmod(len(cases), shards)
        start = (shard - 1) * size + min(shard - 1, extra)
        cases = cases[start : start + size + (shard <= extra)]
        label = gate
    expected = {(label, row["name"]): packed_fixtures[row["path"]][2] for row in cases}
    if len(expected) != len(cases):
        raise ValueError(f"Duplicate case names in {filename}")
    return expected


def run(args):
    verify_pack_identity(args.pack, args.manifest_sha256)
    packed_fixtures = json.loads((args.pack / "fixtures.index.json").read_text())["fixtures"]
    if args.legacy_cache is not None and not args.legacy_cache.is_dir():
        raise ValueError(f"Legacy fixture cache is missing: {args.legacy_cache}")
    args.output.mkdir()
    env = {
        **os.environ,
        "GOA2_FIXTURE_PACK": str(args.pack.resolve()),
        "GOA2_TRACE_COVERAGE": "1",
        "GOA2_ORACLE_PROGRESS": "1",
        "GOA2_ORACLE_WORKERS": str(args.workers),
    }
    env.pop("GOA2_SHARE_IDENTICAL_TRACES", None)
    env.pop("GOA2_SHARE_RESTORE_STARTS", None)
    env.pop("GOA2_FIXTURE_CACHE", None)
    env.pop("GOA2_FIXTURE_CACHE_ONLY", None)
    report = {
        "manifest_sha256": args.manifest_sha256,
        "profile": args.profile,
        "workers": args.workers,
        "includes_legacy_cache": args.legacy_cache is not None,
        "gates": [],
    }
    for gate, command, extra in commands(
        args.gate, args.profile, args.restore_shards, args.restore_shard
    ):
        logs = {}
        timings = {}
        test_timings = {}
        peak_rss = {}
        decode = {}
        modes = ("legacy", "baseline", "shared") if args.legacy_cache else ("baseline", "shared")
        for mode in modes:
            path = args.output / f"{gate}-{mode}.log"
            mode_env = {**env, **extra}
            if mode == "shared":
                mode_env["GOA2_SHARE_IDENTICAL_TRACES"] = "1"
            if mode == "legacy":
                mode_env.pop("GOA2_FIXTURE_PACK")
                mode_env["GOA2_FIXTURE_CACHE"] = str(args.legacy_cache.resolve())
                mode_env["GOA2_FIXTURE_CACHE_ONLY"] = "1"
            measured = command
            if args.measure_rss:
                rss_path = args.output.resolve() / f"{gate}-{mode}.rss"
                measured = ["/usr/bin/time", "-f", "%M", "-o", str(rss_path), *command]
            print(f"Running {gate} {mode}", flush=True)
            with path.open("x") as log:
                start = time.monotonic()
                subprocess.run(
                    measured,
                    cwd=WORKSPACE,
                    env=mode_env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    check=True,
                )
            timings[mode] = round(time.monotonic() - start, 3)
            if args.measure_rss:
                peak_rss[mode] = int(rss_path.read_text().strip())
            durations = []
            with path.open() as log:
                for line in log:
                    match = re.match(
                        r"test result: ok\. 1 passed;.*finished in ([0-9.]+)s", line
                    )
                    if match:
                        durations.append(match.group(1))
                    _, marker, encoded = line.partition("fixture-pack-stats ")
                    if marker:
                        if mode in decode:
                            raise ValueError(f"Duplicate fixture pack statistics: {path}")
                        decode[mode] = json.loads(encoded)
            if mode != "legacy" and mode not in decode:
                raise ValueError(f"Missing fixture pack statistics: {path}")
            if len(durations) != 1:
                raise ValueError(f"Missing or ambiguous native test duration: {path}")
            test_timings[mode] = float(durations[0])
            logs[mode] = path
        expected = expected_cases(gate, packed_fixtures)
        coverage = compare(logs["baseline"], logs["shared"], expected)
        if args.legacy_cache is not None:
            compare(logs["legacy"], logs["baseline"], expected)
        row = {
            "gate": gate,
            "coverage": coverage,
            "command_seconds": timings,
            "test_seconds": test_timings,
            "peak_rss_kib": peak_rss,
            "pack_decode": decode,
        }
        report["gates"].append(row)
        stage = args.output / "results.tmp"
        stage.write_text(json.dumps(report, indent=2) + chr(10))
        stage.replace(args.output / "results.json")
        print(json.dumps(row), flush=True)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--legacy-cache", type=Path, help="Also run against the original gzip cache")
    parser.add_argument(
        "--measure-rss", action="store_true", help="Measure command peak RSS with GNU time"
    )
    parser.add_argument("--output", type=Path, required=True, help="New report directory")
    parser.add_argument("--gate", action="append", choices=[*TRACE_TESTS, "restore"])
    parser.add_argument(
        "--restore-shards", type=int, choices=range(1, 129), default=8, metavar="1..128"
    )
    parser.add_argument(
        "--restore-shard", type=int, action="append", metavar="INDEX",
        help="Run a specific restore shard (repeatable, defaults to all shards)",
    )
    parser.add_argument("--workers", type=int, choices=range(1, 9), default=2)
    parser.add_argument("--profile", choices=["dev", "iterate", "release"], default="iterate")
    args = parser.parse_args()
    args.gate = list(dict.fromkeys(args.gate or ["canonical", "live", "fallback", "restore"]))
    if args.restore_shard:
        if "restore" not in args.gate:
            parser.error("--restore-shard requires the restore gate")
        if any(not 1 <= shard <= args.restore_shards for shard in args.restore_shard):
            parser.error("--restore-shard must be between 1 and --restore-shards")
        args.restore_shard = sorted(set(args.restore_shard))
    run(args)


if __name__ == "__main__":
    main()
