"""Measure fixed-budget arena decisions against representative tuning positions."""

import argparse
import os
import platform
from collections import defaultdict
from contextlib import ExitStack
from math import ceil
from pathlib import Path
from statistics import median
from time import perf_counter

from arena_match import play_match
from arena_reports import _write, sha256_file
from oracle import ROOT, Worker, canonical, digest


def reference_starts():
    from model_traces import setup
    from rollout_parity import strength_start_positions
    from shared_objects import export_state_with_shared_objects

    setup()
    maps = ROOT / "src/goa2/data/maps"
    for midgame in (False, True):
        for map_id, seed, state in strength_start_positions(
            maps, None, held_out=False, midgame=midgame, game_type="LONG",
        ):
            label = f"{map_id}-r{state.round}t{state.turn}"
            yield label, seed, export_state_with_shared_objects(state)[0]


def stress_starts():
    from model_traces import setup
    from shared_objects import export_state_with_shared_objects

    from goa2.domain.input import InputResponse
    from goa2.engine.handler import process_stack, submit_input
    from goa2.engine.phases import commit_card
    from goa2.engine.setup import GameSetup

    setup()
    for map_id in ("forgotten_island", "magma_and_ice"):
        for action in ("MOVEMENT", "FAST_TRAVEL"):
            state = GameSetup.create_game(str(ROOT / "src/goa2/data/maps" / f"{map_id}.json"),
                                          ["Arien", "Hanu"], ["Brogan", "Tigerclaw"],
                                          seed=29, game_type="LONG")
            for team in state.teams.values():
                for hero in team.heroes:
                    commit_card(state, hero.id, hero.hand[1])
            prompt = process_stack(state).input_request
            submit_input(state, InputResponse(request_id=prompt.id, selection=action))
            prompt = process_stack(state).input_request
            if prompt is None or prompt.request_type.value != "SELECT_HEX":
                raise ValueError(f"{map_id} did not reach the planned {action} choice")
            yield f"{map_id}-{action.lower()}", 29, export_state_with_shared_objects(state)[0]


def select_cases(referee, workers, roots, per_root, phases=("planning", "resolution")):
    cases = []
    for label, seed, envelope in roots:
        by_phase = defaultdict(list)

        def collect(phase, operation, request, position):
            choices = position["legal_choices"]
            row = {
                "root": label, "round": position["round"], "turn": position["turn"],
                "revision": position["revision"], "kind": choices.get("request", choices["kind"]),
                "phase": phase, "operation": operation, "request": request,
            }
            by_phase[phase].append(row)

        play_match(referee, workers, envelope, seed=seed, turns=2,
                   planning_nodes=8192, resolution_nodes=2048, coordinated=True,
                   observe_request=collect)
        for phase in phases:
            available = by_phase[phase]
            count = min(len(available), per_root)
            if count == 1:
                cases.append(available[0])
            elif count:
                cases.extend(available[index * (len(available) - 1) // (count - 1)]
                             for index in range(count))
    if {case["phase"] for case in cases} != set(phases):
        raise ValueError("calibration did not reach every requested decision phase")
    return cases


def run_cases(worker, cases, budgets, repeats):
    measurements = []
    identities = {}
    for repeat in range(repeats):
        for index, case in enumerate(cases):
            phase, operation = case["phase"], case["operation"]
            request = case["request"]
            caps = budgets[phase]
            # Counterbalance the cap order across positions and repeats.
            offset = (index + repeat) % len(caps)
            for cap in caps[offset:] + caps[:offset]:
                fields = dict(request, max_nodes=cap)
                started = perf_counter()
                try:
                    result = worker.request(operation, timeout=120, **fields)
                except RuntimeError as error:
                    if not str(error).startswith("worker rejected request: Unsupported("):
                        raise
                    result = {"unsupported": str(error)}
                elapsed_ms = (perf_counter() - started) * 1000
                identity = digest(result)
                if identities.setdefault((index, cap), identity) != identity:
                    raise ValueError(f"node-budget result changed at case {index}, cap {cap}")
                used = result.get("nodes")
                if used is not None and (type(used) is not int or not 0 <= used <= cap):
                    raise ValueError("worker exceeded the fixed calibration budget")
                measurements.append({
                    "case": index, "phase": phase, "cap": cap, "repeat": repeat,
                    "elapsed_ms": round(elapsed_ms, 3), "nodes": used,
                    "scored": result.get("score") is not None or result.get("shallow_score") is not None,
                    "completed_depth": result.get("completed_depth"),
                    "completed_rows": result.get("completed_rows"),
                    "result_sha256": identity,
                })
    return measurements


def summarize(measurements):
    groups = defaultdict(list)
    for row in measurements:
        groups[row["phase"], row["cap"]].append(row)
    output = []
    for (phase, cap), rows in sorted(groups.items()):
        samples = sorted(row["elapsed_ms"] for row in rows)
        used = [row["nodes"] for row in rows if row["nodes"] is not None]
        output.append({
            "phase": phase, "cap": cap, "samples": len(rows),
            "scored": sum(row["scored"] for row in rows),
            "p50_ms": median(samples), "p95_ms": samples[ceil(0.95 * len(samples)) - 1],
            "max_ms": samples[-1], "median_nodes": median(used) if used else None,
            "max_nodes": max(used, default=None),
        })
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--referee", type=Path, required=True)
    parser.add_argument("--planning-nodes", type=int, nargs="+", required=True)
    parser.add_argument("--resolution-nodes", type=int, nargs="+", required=True)
    parser.add_argument("--per-root", type=int, default=3)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--stress-resolution", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    budgets = {"planning": args.planning_nodes, "resolution": args.resolution_nodes}
    if (min(args.repeats, args.per_root) < 1
            or any(min(caps) < 1 or len(set(caps)) != len(caps) for caps in budgets.values())):
        parser.error("caps must be positive and unique, and sample and repeat counts must be positive")
    roots = list(reference_starts())
    stress = list(stress_starts()) if args.stress_resolution else []
    with ExitStack() as stack:
        def start(path):
            worker = Worker([str(path.resolve())])
            stack.callback(worker.close)
            return worker

        referee = start(args.referee)
        policy = start(args.worker)
        workers = {"RED": policy, "BLUE": policy}
        cases = select_cases(referee, workers, roots, args.per_root)
        if stress:
            cases.extend(select_cases(referee, workers, stress, 1, ("resolution",)))
        policy.request("prepare_search", timeout=120)
        measurements = run_cases(policy, cases, budgets, args.repeats)
    summary = summarize(measurements)
    report = {
        "schema_version": 1, "protocol": "nodes_v1",
        "worker_sha256": sha256_file(args.worker), "referee_sha256": sha256_file(args.referee),
        "machine": platform.platform(), "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "cpu_model": next((line.partition(":")[2].strip()
                           for line in Path("/proc/cpuinfo").read_text().splitlines()
                           if line.startswith("model name")), platform.processor()),
        "roots": [{"name": label, "seed": seed, "sha256": digest(envelope)}
                  for label, seed, envelope in roots + stress],
        "cases": [dict({key: value for key, value in case.items() if key != "request"},
                       request_sha256=digest(case["request"]), player_id=case["request"]["player_id"])
                  for case in cases],
        "measurements": measurements, "summary": summary,
    }
    _write(args.output, canonical(report).encode("utf-8"))
    for row in summary:
        print(canonical(row))


if __name__ == "__main__":
    main()
