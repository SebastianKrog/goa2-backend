#!/usr/bin/env python3
"""Measure repeatable four-turn fixed-node searches on pinned arena positions."""

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
MAX_CASE_BYTES = 4 * 1024 * 1024
INDEX = ROOT / "rust/tests/fixtures/complete-games-hanu.json"


def encode(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def worker_calls(worker, *requests):
    packets = [
        encode(dict(schema_version=1, id=str(i), **request))
        for i, request in enumerate(requests)
    ]
    run = subprocess.run(
        [str(worker)], input=b"\n".join(packets) + b"\n",
        capture_output=True, check=True, timeout=120,
    )
    responses = [json.loads(line) for line in run.stdout.splitlines()]
    if len(responses) != len(requests):
        raise ValueError("Worker response count differs from request count")
    for i, response in enumerate(responses):
        if response.get("id") != str(i) or not response.get("ok"):
            raise ValueError(f"Worker rejected request {i}: {response.get('error')}")
    return [response["result"] for response in responses]


def pinned_cases(cache):
    index_bytes = INDEX.read_bytes()
    registry = json.loads((INDEX.parent / "registry.json").read_bytes())
    expected = next(row["sha256"] for row in registry["families"]
                    if row["path"] == INDEX.name)
    if digest(index_bytes) != expected:
        raise ValueError("Hanu replay index does not match the fixture registry")
    for entry in json.loads(index_bytes)["cases"]:
        expected_path = Path(INDEX.stem) / (entry["case_sha256"] + ".json")
        if entry["path"] != expected_path.as_posix():
            raise ValueError("Hanu replay path does not match its identity")
        path = cache / expected_path
        source = path if path.is_file() else path.with_suffix(".json.gz")
        open_source = gzip.open if source != path else open
        with open_source(source, "rb") as stream:
            raw = stream.read(MAX_CASE_BYTES + 1)
        if len(raw) > MAX_CASE_BYTES:
            raise ValueError("Hanu replay header exceeds the workload size limit")
        case = json.loads(raw)
        if digest(encode(case)) != entry["case_sha256"] or case["name"] != entry["name"]:
            raise ValueError("Hanu replay case does not match the pinned index")
        yield entry, case["envelope"]


def search_requests(worker, envelope, max_nodes):
    start = dict(operation="arena_start", envelope=envelope, turns=4)
    planning, = worker_calls(worker, start)
    if planning["kind"] != "planning" or planning["revision"] != 0:
        raise ValueError("Expected the original planning boundary")
    submit = dict(
        operation="arena_submit", revision=planning["revision"],
        choice=dict(kind="planning", commitments=[row["fallback"] for row in planning["slots"]]),
    )
    _, response = worker_calls(worker, start, submit)
    resolution = response["position"]
    if resolution["kind"] != "resolution" or resolution["forced"] or resolution["revision"] != 1:
        raise ValueError("Expected a non-forced resolution boundary after the first commitment")
    common = dict(work_budget="nodes_v2", max_depth=4, rollout_turns=4,
                  max_nodes=max_nodes, evaluation="material_position")
    player = planning["slots"][0]["player_id"]
    yield dict(
        common, operation="search_sampled_planning", envelope=planning["envelope"],
        player_id=player, seed=29, cap=4, workers=1, greedy_card_order=True,
        bot_team=dict(mode="true_rules", bot_ids=[player], worlds=2),
        upgrade_checkpoints=planning["upgrade_checkpoints"],
    )
    yield dict(
        common, operation="search_resolution", envelope=resolution["envelope"],
        player_id=resolution["fallback"]["player_id"],
        resolution_sample_seed=29, resolution_sample_worlds=2, resolution_sample_workers=1,
        upgrade_checkpoints=resolution["upgrade_checkpoints"],
    )


def measure(worker, request, repetitions):
    results = []
    timings = []
    for _ in range(repetitions):
        start = time.perf_counter_ns()
        result, = worker_calls(worker, request)
        timings.append(time.perf_counter_ns() - start)
        if not 0 < result["nodes"] <= request["max_nodes"]:
            raise ValueError(f"Invalid work count: {result['nodes']}")
        results.append(result)
    if any(result != results[0] for result in results):
        raise ValueError("Fixed-node search result differed across repetitions")
    return dict(
        request={k: v for k, v in request.items() if k != "envelope"},
        envelope_sha256=digest(encode(request["envelope"])),
        result=results[0], wall_ns=timings,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True, type=Path)
    parser.add_argument("--fixtures", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--max-nodes", default=4096, type=int)
    parser.add_argument("--repetitions", default=3, type=int)
    args = parser.parse_args()
    if args.max_nodes < 1 or not 2 <= args.repetitions <= 20:
        parser.error("Use positive max-nodes and 2..20 repetitions")
    if args.output.exists():
        parser.error("Use a new output path")
    worker = args.worker.resolve()
    workloads = [
        dict(case=entry["name"], case_sha256=entry["case_sha256"],
             measurements=[measure(worker, req, args.repetitions)
                           for req in search_requests(worker, envelope, args.max_nodes)])
        for entry, envelope in pinned_cases(args.fixtures)
    ]
    with args.output.open("x") as stream:
        json.dump(dict(schema_version=1, worker_sha256=digest(worker.read_bytes()),
                       source_checkout_commit=subprocess.check_output(
                           ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                       architecture=platform.machine(), kernel=platform.release(),
                       scope="Fresh worker process, envelope compilation, and search",
                       workloads=workloads), stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
