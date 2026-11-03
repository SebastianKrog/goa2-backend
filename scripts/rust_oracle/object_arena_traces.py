"""Python object identity gold standard for the native mutable object arena.

This validates storage/alias semantics, not gameplay transition coverage. The
real source GameState, registered cards and GameStep objects form every scene;
ordinary Python mutations provide expected values and identity topology.
"""

from __future__ import annotations

from fixture_cache import write_text

import argparse
import copy
import json
from pathlib import Path

from oracle import ROOT, Worker, digest, verify_sources
from shared_objects import export_state_with_shared_objects


def scenes():
    from combat_effect_traces import effect
    from model_traces import setup

    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase, TargetType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import SelectStep

    for variant in [
        "list",
        "independent",
        "nested",
        "snapshot",
        "step",
        "independent-steps",
        "hero",
        "effect",
        "tuple",
        "frozen",
        "detached",
        "mapping-order",
    ]:
        state = setup()
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        plan = []
        x, y = [1, 2], [1, 2]
        if variant == "list":
            state.execution_context = {"a": x, "b": x}
            plan = [("append", x, 3), ("pop", x, -2), ("append", x, {"nested": [4]})]
        elif variant == "independent":
            state.execution_context = {"a": x, "a2": x, "b": y, "b2": y}
            plan = [("append", x, 3), ("append", y, 4), ("pop", x, 0)]
        elif variant == "nested":
            box = {"items": x, "count": 0}
            state.execution_context = {"a": box, "b": box, "items": x}
            plan = [
                ("append", x, 3),
                ("set", box, "count", 2),
                ("snapshot", "old"),
                ("delete", box, "count"),
                ("pop", x, -1),
            ]
        elif variant == "snapshot":
            state.execution_context = {"items": x, "scalar": 1}
            plan = [
                ("snapshot", "old"),
                ("append", x, 3),
                ("context-set", "scalar", 2),
                ("context-set", "items", [9]),
                ("append", x, 4),
                ("snapshot", "new"),
            ]
        elif variant in ["step", "independent-steps"]:
            left = SelectStep(
                step_id="fixture/alias/same-step-id",
                target_type=TargetType.NUMBER,
                number_options=[1, 2],
                prompt="Alias identity",
                output_key="chosen",
            )
            right = left if variant == "step" else left.model_copy(deep=True)
            first = effect(
                state, "first", EffectType.MOVEMENT_ZONE, source="hero_arien", max_value=1
            )
            second = effect(
                state, "second", EffectType.MOVEMENT_ZONE, source="hero_arien", max_value=2
            )
            first.finishing_steps, second.finishing_steps = [left], [right]
            state.active_effects = [first, second]
            state.execution_stack = [left, right]
            box = {"selection": 1}
            state.execution_context = {"left": left, "right": right, "box": box}
            plan = [
                ("set-alias", left, "pending_input", box),
                ("set", box, "selection", 2),
                ("set", right, "pending_input", {"selection": 1}),
                ("snapshot", "old"),
                ("set", left, "is_mandatory", False),
                ("set", box, "selection", 3),
            ]
        elif variant == "hero":
            hero = state.get_hero("hero_arien")
            state.execution_context = {"hero": hero, "hand": hero.hand}
            plan = [
                ("set", hero, "gold", 9),
                ("pop", hero.hand, 0),
                ("snapshot", "old"),
                ("set", hero, "level", 2),
            ]
        elif variant == "effect":
            e = effect(state, "shared", EffectType.MOVEMENT_ZONE, source="hero_arien", max_value=2)
            state.active_effects = [e, e]
            state.execution_context = {"effect": e}
            plan = [
                ("set", e, "max_value", 1),
                ("set", e, "passive_uses_this_turn", 3),
                ("set", e, "is_active", False),
            ]
        elif variant == "tuple":
            value = (x, y)
            state.execution_context = {"a": value, "b": value}
            plan = [("append", x, 3), ("snapshot", "old"), ("pop", y, 0)]
        elif variant == "frozen":
            h = Hex(q=0, r=0, s=0)
            state.execution_context = {"a": h, "b": h}
            plan = [("set", h, "q", 1), ("context-set", "after", True)]
        elif variant == "detached":
            state.execution_context = {"a": x, "b": x}
            plan = [
                ("delete-context", "a"),
                ("delete-context", "b"),
                ("append", x, 3),
                ("context-alias", "restored", x),
            ]
        elif variant == "mapping-order":
            box = {"z": 1, "a": 2, "m": 3}
            state.execution_context = {"a": box, "b": box}
            plan = [
                ("set", box, "a", 9),
                ("delete", box, "z"),
                ("set", box, "z", 0),
                ("snapshot", "old"),
            ]
        envelope, objects = export_state_with_shared_objects(state)

        def identity(value, objects=objects):
            return next(i for i, candidate in enumerate(objects) if candidate is value)

        operations = []
        for step in plan:
            kind = step[0]
            if kind == "snapshot":
                op = {"kind": "snapshot", "key": step[1]}
            elif kind == "context-set":
                op = {
                    "kind": "set",
                    "target": "context",
                    "key": step[1],
                    "value": {"literal": step[2]},
                }
            elif kind == "context-alias":
                op = {
                    "kind": "set",
                    "target": "context",
                    "key": step[1],
                    "value": {"alias": identity(step[2])},
                }
            elif kind == "delete-context":
                op = {"kind": "delete", "target": "context", "key": step[1]}
            elif kind == "append":
                op = {"kind": "append", "target": identity(step[1]), "value": {"literal": step[2]}}
            elif kind == "pop":
                op = {"kind": "pop", "target": identity(step[1]), "index": step[2]}
            elif kind == "delete":
                op = {"kind": "delete", "target": identity(step[1]), "key": step[2]}
            else:
                op = {
                    "kind": "set",
                    "target": identity(step[1]),
                    "key": step[2],
                    "value": (
                        {"alias": identity(step[3])}
                        if kind == "set-alias"
                        else {"literal": step[3]}
                    ),
                }
            operations.append(op)
        yield variant, state, envelope, objects, operations


def reference(state, objects, operations):
    from state_codec import materialize

    result = []
    for op in operations:
        error = None
        target = (
            state.execution_context
            if op.get("target") == "context"
            else objects[op["target"]] if "target" in op else None
        )
        try:
            kind = op["kind"]
            value = op.get("value")
            value = (
                objects[value["alias"]]
                if value and "alias" in value
                else copy.deepcopy(value["literal"]) if value else None
            )
            if kind == "snapshot":
                state.execution_context[op["key"]] = dict(state.execution_context)
            elif kind == "append":
                target.append(value)
            elif kind == "pop":
                target.pop(op.get("index", -1))
            elif kind == "delete":
                del target[op["key"]]
            elif isinstance(target, dict):
                target[op["key"]] = value
            else:
                setattr(target, op["key"], value)
        except (ValueError, TypeError, IndexError, KeyError, AttributeError):
            error = "invalid_operation"
        current, _ = export_state_with_shared_objects(state)
        bindings = current.get("object_bindings", [])

        def at(path, current=current):
            value = current["state"]
            for key in path:
                value = value[key]
            return value

        result.append(
            {
                "semantic_state": materialize(current),
                "object_bindings": bindings,
                "object_keys": [
                    list(at(b["paths"][0])) if isinstance(at(b["paths"][0]), dict) else None
                    for b in bindings
                ],
                "error": error,
            }
        )
    return result


def export(path):
    from stack_traces import write_corpus

    manifest = verify_sources()
    cases = []
    for name, state, envelope, objects, operations in scenes():
        expected = reference(state, objects, operations)
        cases.append(
            {
                "name": "objects-" + name,
                "envelope": envelope,
                "operations": operations,
                "expected": expected,
            }
        )
    data = {
        "schema_version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "cases": cases,
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


def verify(path, command):
    from stack_traces import read_case, read_index

    verify_sources()
    data, identity = read_index(path)
    worker = Worker(command)
    count = 0
    try:
        for entry in data["cases"]:
            case = read_case(path, entry)
            got = worker.request(
                "aggregate_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            if got != case["expected"]:
                dest = ROOT / "rust/artifacts/object-arena-mismatch.json"
                write_text(dest, json.dumps({"case": case, "actual": got}, indent=2))
                raise AssertionError(f"object arena mismatch in {case['name']}: {dest}")
            count += len(got)
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "operations": count,
        "corpus_sha256": identity,
        "value_and_identity_topology_parity": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/tests/fixtures/object-arena.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        print(
            json.dumps(
                verify(args.corpus, args.worker[1:] if args.worker[:1] == ["--"] else args.worker),
                indent=2,
            )
        )
