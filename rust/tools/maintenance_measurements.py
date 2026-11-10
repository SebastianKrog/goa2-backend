#!/usr/bin/env python3
"""Measure explicit native workloads; process starts do not imply cold OS page caches."""

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from rust_verification.process import request, serve  # noqa: E402


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def measured_process(command, timeout=120):
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        start = time.perf_counter_ns()
        process = subprocess.Popen(command, stdout=out, stderr=err)
        while True:
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid:
                break
            if time.perf_counter_ns() - start > timeout * 10**9:
                process.kill()
                os.wait4(process.pid, 0)
                process.returncode = -9
                raise TimeoutError("Maintenance workload exceeded its explicit watchdog")
            time.sleep(0.005)
        process.returncode = os.waitstatus_to_exitcode(status)
        out.seek(0)
        err.seek(0)
        stdout = out.read()
        stderr = err.read()
        if process.returncode:
            raise RuntimeError(stderr.decode(errors="replace"))
        return {
            "wall_ns": time.perf_counter_ns() - start,
            "user_cpu_seconds": usage.ru_utime,
            "system_cpu_seconds": usage.ru_stime,
            "peak_rss_kib": usage.ru_maxrss,
            "result": json.loads(stdout),
        }


def memory(pid):
    rows = {}
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in ["VmRSS", "VmHWM"]:
            rows[key] = int(value.split()[0])
    return rows


def measure(benchmark, server, contents, repetitions):
    oracle = json.loads((ROOT / "rust/oracle-manifest.json").read_text())
    hardware = {
        "architecture": platform.machine(),
        "kernel": platform.release(),
        "logical_cpus": os.cpu_count(),
        "affinity_cpus": len(os.sched_getaffinity(0)),
        "cpu_model": next(
            (
                line.partition(":")[2].strip()
                for line in Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("model name")
            ),
            "unknown",
        ),
        "cgroup_cpu_max": (
            Path("/sys/fs/cgroup/cpu.max").read_text().strip()
            if Path("/sys/fs/cgroup/cpu.max").exists()
            else None
        ),
    }
    cases = []
    for content in contents:
        runs = [measured_process([str(benchmark), str(content)]) for _ in range(repetitions)]
        fixed = [row["result"]["fixed_search"]["result"] for row in runs]
        assert all(row == fixed[0] for row in fixed), "Fixed-work decision/work identity drift"
        startup = []
        for _ in range(repetitions):
            with tempfile.TemporaryDirectory(prefix="goa2-startup-measure-") as name:
                start = time.perf_counter_ns()
                with serve(server, content, Path(name) / "fresh.sqlite", process_handle=True) as (
                    address,
                    process,
                ):
                    ready_ns = time.perf_counter_ns() - start
                    health = request(address, "/health")
                    before = memory(process.pid)
                    setup = {
                        "red_heroes": ["Arien", "Brogan"],
                        "blue_heroes": ["Tigerclaw", "Sabina"],
                        "seed": 29,
                        "game_type": "QUICK",
                    }
                    first = time.perf_counter_ns()
                    request(address, "/games", body=setup, expect=201)
                    first_ns = time.perf_counter_ns() - first
                    warm = time.perf_counter_ns()
                    request(address, "/games", body=setup, expect=201)
                    warm_ns = time.perf_counter_ns() - warm
                    startup.append(
                        {
                            "ready_ns": ready_ns,
                            "first_game_ns": first_ns,
                            "second_game_ns": warm_ns,
                            "memory_at_ready_kib": before,
                            "memory_after_games_kib": memory(process.pid),
                            "identity": health,
                        }
                    )
        cases.append(
            {
                "id": content.name,
                "content_sha256": sha(content),
                "fresh_process_runs": runs,
                "fresh_database_server_starts": startup,
            }
        )
    catalog = json.loads((ROOT / "rust/content/manifest.json").read_text())
    return {
        "schema_version": 1,
        "workload": "maintenance-v1",
        "profile": "release (explicit documented build command)",
        "compiler": subprocess.check_output(
            ["rustc", "--version"], cwd=ROOT / "rust", text=True
        ).strip(),
        "hardware": hardware,
        "source_parent_commit": (
            subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
            if (ROOT / ".git").exists()
            else None
        ),
        "program_path": "rust/crates/goa2-core/examples/maintenance_bench.rs",
        "program_sha256": sha(ROOT / "rust/crates/goa2-core/examples/maintenance_bench.rs"),
        "benchmark_binary_sha256": sha(benchmark),
        "benchmark_binary_bytes": benchmark.stat().st_size,
        "server_binary_sha256": sha(server),
        "server_binary_bytes": server.stat().st_size,
        "oracle_revision": oracle["oracle_revision"],
        "source_sha256": oracle["source_sha256"],
        "rules_version": oracle["rules_version"],
        "catalog_manifest_sha256": sha(ROOT / "rust/content/manifest.json"),
        "catalog_manifest": catalog,
        "repetitions_per_content": repetitions,
        "units": {
            "time": "nanoseconds unless field says seconds",
            "memory": "KiB; allocator/OS high-water, not per-object size",
        },
        "cold_scope": "Fresh processes and disposable databases; OS filesystem/page caches and host load are uncontrolled. No cache drop or strength-quality claim.",
        "cases": cases,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--benchmark-binary", required=True, type=Path)
    parser.add_argument("--server-binary", required=True, type=Path)
    parser.add_argument("--content", required=True, action="append", type=Path)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError("Do not replace an admitted measurement report")
    if not 2 <= args.repetitions <= 20:
        parser.error("Use 2..20 repetitions")
    result = measure(
        args.benchmark_binary.resolve(),
        args.server_binary.resolve(),
        [p.resolve() for p in args.content],
        args.repetitions,
    )
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                "content_cases": len(result["cases"]),
                "fresh_runs": args.repetitions * len(result["cases"]),
                "output": str(args.output),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
