"""Time native two-turn replays against untimed Python rule validation."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import platform
import statistics
from contextlib import ExitStack
from pathlib import Path

from object_execution_traces import ObjectTraceNormalizer
from oracle import ROOT, Worker, digest, verify_sources
from shared_objects import export_state_with_shared_objects, instance_bindings
from stack_traces import read_case, read_index
from state_codec import export_state_with_instances, materialize

from goa2.domain.input import InputResponse
from goa2.engine.handler import process_stack, submit_input
from goa2.engine.phases import commit_card, pass_turn


class PythonRunner:
    def __init__(self, state, physical):
        self.state, self.physical, self.request = state, physical, None
        self.counts = dict(operations=0, requests=0, events=0)

    def step(self, command):
        kind, payload = command
        if kind in ("advance", "advance_prefix"):
            boundary = None
            if kind == "advance_prefix":
                remaining = payload

                def boundary(_state, _step):
                    nonlocal remaining
                    if not remaining:
                        return True
                    remaining -= 1
                    return False

            output = process_stack(self.state, stop_before_step=boundary)
            self.request = output.input_request
            self.counts["requests"] += int(self.request is not None)
            self.counts["events"] += len(output.events)
        elif kind == "commit":
            hero, card = payload
            commit_card(self.state, hero, self.physical[card])
        elif kind == "pass":
            pass_turn(self.state, payload)
        elif kind == "input":
            if self.request is None:
                raise ValueError("benchmark input without a request")
            response, selection = payload
            response.request_id = self.request.id
            response.selection = copy.deepcopy(selection)
            submit_input(self.state, response)
        else:
            raise ValueError(f"unsupported benchmark command {kind}")
        self.counts["operations"] += 1

    def run(self, commands):
        for command in commands:
            self.step(command)
        return self.counts


def decode(operations):
    result = []
    for op in operations:
        kind = op["kind"]
        if kind == "advance":
            payload = None
        elif kind == "advance_prefix":
            payload = op["steps"]
        elif kind == "commit":
            payload = (op["hero"], op["card"])
        elif kind == "pass":
            payload = op["hero"]
        elif kind == "input":
            if op.get("request_id", "current") != "current":
                raise ValueError("benchmark requires live input tokens")
            payload = (
                InputResponse(request_id="benchmark/current"),
                copy.deepcopy(op["selection"]),
            )
        else:
            raise ValueError(f"unsupported benchmark command {kind}")
        result.append((kind, payload))
    return result


def coalesce(operations):
    result = []
    for op in operations:
        if op["kind"] == "advance_prefix":
            if op["steps"] != 1:
                raise ValueError("expected atomic gold trace")
            if not result or result[-1]["kind"] != "advance":
                result.append({"kind": "advance"})
        else:
            result.append(copy.deepcopy(op))
    return result


def ordinal(round_number, turn):
    return (round_number - 1) * 4 + turn - 1


def planning_boundary(state):
    return (
        state.phase.value == "PLANNING"
        and not state.execution_stack
        and not state.input_stack
        and not state.pending_inputs
    )


def canonical_set_fields(record):
    """Only Zone.hexes is a declared set; no gameplay values are relaxed."""
    for binding in record["object_bindings"]:
        for path in binding["paths"]:
            if len(path) >= 4 and path[:2] == ["board", "zones"] and path[3] == "hexes":
                raise ValueError("zone-set identity paths need explicit rebasing")
    for zone in record["semantic_state"]["board"]["zones"].values():
        zone["hexes"].sort(key=lambda point: (point["q"], point["r"], point["s"]))
    return record


def snapshot(state, original):
    envelope, objects = export_state_with_shared_objects(state)
    norm = ObjectTraceNormalizer()
    norm.bind_originals(*original)
    assert all(isinstance(zone.hexes, (set, frozenset)) for zone in state.board.zones.values())
    return canonical_set_fields(
        norm.record(
            dict(
                semantic_state=materialize(envelope),
                object_bindings=envelope["object_bindings"],
                context_keys=list(state.execution_context),
                instance_bindings=instance_bindings(envelope["state"]),
                request=None,
                events=[],
                error=None,
            ),
            objects,
        )
    )


def initial_state(case):
    from model_traces import setup

    from goa2.engine.setup import GameSetup

    setup()
    map_id = next(name for name in ("forgotten_island", "magma_and_ice") if name in case["name"])
    teams = case["envelope"]["state"]["teams"]
    state = GameSetup.create_game(
        str(ROOT / f"src/goa2/data/maps/{map_id}.json"),
        [h["name"] for h in teams["RED"]["heroes"]],
        [h["name"] for h in teams["BLUE"]["heroes"]],
        seed=29,
    )
    state.execution_context.update(
        locations_alias=state.entity_locations,
        snapshot_alias=state.last_turn_positions,
        stack_alias=state.execution_stack,
        effects_alias=state.active_effects,
    )
    envelope, _ = export_state_with_shared_objects(state)
    if envelope != case["envelope"]:
        raise AssertionError("benchmark setup differs from verified initial envelope")
    return state, map_id


def workloads(case, *, midgame=False):
    from goa2.domain.models import Card

    state, map_id = initial_state(case)
    physical = export_state_with_instances(state)[1]
    card_ids = {
        index: str(card.id) for index, card in enumerate(physical) if isinstance(card, Card)
    }
    runner = PythonRunner(state, physical)
    starts = (
        {(3, 1): "midgame-upgraded", (3, 3): "midgame-round-boundary", (4, 1): "midgame-combat"}
        if midgame else
        {(1, 1): "opening", (1, 3): "round-boundary", (2, 1): "combat"}
    )
    active = []
    captured = set()
    for index in range(len(case["operations"]) + 1):
        if planning_boundary(state):
            point = (state.round, state.turn)
            now = ordinal(*point)
            for window in list(active):
                if now == window["ordinal"] + 2:
                    root = window.pop("root")
                    raw = copy.deepcopy(case["operations"][window.pop("offset") : index])
                    envelope, original_objects = export_state_with_shared_objects(root)
                    local = export_state_with_instances(root)[1]
                    by_id = {
                        str(card.id): i for i, card in enumerate(local) if isinstance(card, Card)
                    }
                    if len(by_id) != sum(isinstance(card, Card) for card in local):
                        raise ValueError(
                            "independent duplicate card IDs require explicit benchmark binding"
                        )
                    for op in raw:
                        if op["kind"] == "commit":
                            op["card"] = by_id[card_ids[op["card"]]]
                    operations = coalesce(raw)
                    originals = (envelope["object_bindings"], original_objects)
                    expected = snapshot(state, originals)
                    probe, probe_physical = copy.deepcopy((root, local))
                    counts = PythonRunner(probe, probe_physical).run(decode(operations))
                    actual = snapshot(probe, originals)
                    if actual != expected:
                        path = ROOT / "rust/artifacts/benchmark-source-mismatch.json"
                        path.write_text(
                            json.dumps(dict(expected=expected, actual=actual), indent=2)
                        )
                        raise AssertionError(f"coalescing/copying changes source endpoint: {path}")
                    yield dict(
                        name=f"{map_id}-{window['label']}",
                        state=root,
                        physical=local,
                        envelope=envelope,
                        originals=originals,
                        operations=operations,
                        counts=counts,
                        expected=expected,
                        start=window["start"],
                        end=point,
                        game_turns=2,
                        atomic_operations=len(raw),
                        source_game=case["name"],
                    )
                    active.remove(window)
            if point in starts and point not in captured:
                active.append(
                    dict(
                        root=copy.deepcopy(state),
                        offset=index,
                        ordinal=now,
                        label=starts[point],
                        start=point,
                    )
                )
                captured.add(point)
            if len(captured) == len(starts) and not active:
                return
        if index == len(case["operations"]):
            break
        runner.step(decode([case["operations"][index]])[0])
    raise AssertionError("verified game did not supply all two-turn windows")


def summarize(samples):
    keys = ("clone_seconds", "advance_seconds", "cleanup_seconds", "total_seconds")
    result = {
        key: statistics.median(row[key] / row["iterations"] for row in samples) for key in keys
    }
    if "rollback_seconds" in samples[0]:
        result["rollback_seconds"] = statistics.median(
            row["rollback_seconds"] / row["iterations"] for row in samples
        )
    result["simulations_per_second"] = 1 / result["total_seconds"]
    result["sample_seconds_per_simulation"] = [
        row["total_seconds"] / row["iterations"] for row in samples
    ]
    return result


def benchmark(
    worker_command, *, samples=5, iterations=8, compare_rollback=False, baseline_command=None
):
    manifest = verify_sources()
    corpus = ROOT / "rust/tests/fixtures/complete-games-hanu.json"
    index, identity = read_index(corpus)
    rows = []
    with ExitStack() as stack:
        worker = Worker(worker_command)
        stack.callback(worker.close)
        targets = {"rust": (worker, False)}
        if compare_rollback:
            targets["rust_rollback"] = (worker, True)
        if baseline_command is not None:
            baseline = Worker(baseline_command)
            stack.callback(baseline.close)
            targets["baseline"] = (baseline, False)
            if compare_rollback:
                targets["baseline_rollback"] = (baseline, True)
        for entry in index["cases"]:
            case = read_case(corpus, entry)
            for workload in workloads(case):
                modes = {mode: [] for mode in targets}
                order = tuple(targets)
                for sample in range(samples):
                    offset = sample % len(order)
                    for mode in order[offset:] + order[:offset]:
                        target, rollback = targets[mode]
                        result = target.request(
                            "simulation_bench",
                            envelope=workload["envelope"],
                            operations=workload["operations"],
                            iterations=iterations,
                            **({"rollback": True} if rollback else {}),
                        )
                        actual = canonical_set_fields(
                            ObjectTraceNormalizer().record(result.pop("validation_record"))
                        )
                        if actual != workload["expected"]:
                            path = ROOT / "rust/artifacts/simulation-bench-mismatch.json"
                            path.write_text(
                                json.dumps(
                                    dict(
                                        name=workload["name"],
                                        expected=workload["expected"],
                                        actual=actual,
                                    ),
                                    indent=2,
                                )
                            )
                            raise AssertionError(f"benchmark endpoint mismatch: {path}")
                        assert (
                            result["counts"] == workload["counts"] and result["root_unchanged"]
                        )
                        modes[mode].append(result)
                summary = {mode: summarize(values) for mode, values in modes.items()}
                row = {
                    key: workload[key]
                    for key in (
                        "name",
                        "start",
                        "end",
                        "game_turns",
                        "atomic_operations",
                        "source_game",
                    )
                }
                row.update(
                    commands=len(workload["operations"]),
                    counts=workload["counts"],
                    command_sha256=digest(workload["operations"]),
                    results=summary,
                    samples=modes,
                )
                rows.append(row)
                print(
                    json.dumps(
                        {key: row[key] for key in ("name", "results")}
                    ),
                    flush=True,
                )
    return dict(
        schema_version=2,
        source_sha256=manifest["source_sha256"],
        corpus_sha256=identity,
        platform=platform.platform(),
        cpu_affinity=sorted(os.sched_getaffinity(0)),
        samples_per_workload=samples,
        iterations_per_sample=iterations,
        scope=f"{len(rows)} verified two-complete-game-turn serial replays; not minimax or full roster throughput.",
        setup_json_export_comparison_excluded_from_clocks=True,
        typed_inputs_prepared_before_clocks=True,
        endpoint_set_normalization="Only declared Board Zone.hexes sets; identity paths are rejected",
        turn_semantics="Two global game turns, resolving all heroes; not two individual hero actions",
        workloads=rows,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--baseline-worker", help="Interleave samples with this Rust worker")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--iterations", type=int, default=8)
    parser.add_argument("--compare-rollback", action="store_true")
    parser.add_argument(
        "--output", type=Path, default=ROOT / "rust/artifacts/simulation-benchmark-rust-only.json"
    )
    args = parser.parse_args()
    if os.environ.get("PYTHONHASHSEED") != "0":
        parser.error("run with PYTHONHASHSEED=0")
    if args.samples < 1 or not 1 <= args.iterations <= 100000:
        parser.error("positive samples and 1..100000 iterations required")
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    result = benchmark(
        [args.worker], samples=args.samples, iterations=args.iterations,
        compare_rollback=args.compare_rollback,
        baseline_command=[args.baseline_worker] if args.baseline_worker else None,
    )
    with open(args.worker, "rb") as executable:
        result["worker_sha256"] = hashlib.file_digest(executable, "sha256").hexdigest()
    if args.baseline_worker:
        with open(args.baseline_worker, "rb") as executable:
            result["baseline_worker_sha256"] = hashlib.file_digest(executable, "sha256").hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
