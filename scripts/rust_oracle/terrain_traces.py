"""Frozen terrain, actor obstacles, stateful paths and real movement traces."""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

from fixture_cache import read_text, write_text
from oracle import ROOT, Worker, digest, verify_sources
from state_codec import export_state


def states():
    from action_rule_traces import restriction_states
    from displacement_effect_traces import states as displacement_states

    from goa2.domain.models import TeamColor, Token, TokenType
    from goa2.domain.models.effect import EffectType, Shape

    for name, state, h, pieces, heavy in displacement_states():
        e = state.active_effects[-1]
        if e.effect_type == EffectType.STATIC_BARRIER:
            e.barrier_radius = 2
        yield "base-" + name, state, h, pieces, heavy
    for name, state in restriction_states():
        if name.startswith("cap-") or name in [
            "actions-only",
            "no-flags-cap",
            "prerequisite-order",
            "all-actions",
            "gold-exception",
        ]:
            from aura_traces import _pieces

            from goa2.domain.hex import Hex

            def h(q, r):
                return Hex(q=q, r=r, s=-q - r)

            heavy = next(m for m in state.teams[TeamColor.RED].minions if m.is_heavy)
            yield "restriction-" + name, state, h, _pieces(state, "hero_razzle"), heavy
    variants = [
        "barrier",
        "barrier-outside",
        "barrier-source-offboard",
        "barrier-token-origin",
        "barrier-piece-origin",
        "barrier-empty-origin",
        "barrier-missing-origin",
        "barrier-ignore-scope",
        "barrier-negative",
        "barrier-zero",
        "barrier-inactive",
        "barrier-expired",
        "empty",
        "empty-negative",
        "empty-fixed-ignored",
        "empty-token-origin",
        "empty-scope-ignored",
        "petrify",
        "petrify-missing-location",
        "mine",
        "mine-supply-only",
        "mine-pool-false",
        "mine-entity-false",
        "mine-on-terrain",
        "mine-unknown-occupant",
        "tile-terrain",
        "map-gap",
    ]
    for variant in variants:
        _, state, h, pieces, heavy = next(c for c in displacement_states() if c[0] == "blocked")
        e = state.active_effects[-1]
        e.effect_type = EffectType.STATIC_BARRIER
        e.barrier_radius = 2
        if variant == "barrier-outside":
            state.place_entity("hero_brogan", h(3, 0))
        if variant == "barrier-source-offboard":
            state.remove_entity("hero_arien")
        if variant == "barrier-token-origin":
            e.barrier_origin_id = "restriction-token"
        if variant == "barrier-piece-origin":
            e.barrier_origin_id = "hero_razzle"
            state.acting_piece_id = pieces[0]
        if variant == "barrier-empty-origin":
            e.barrier_origin_id = ""
        if variant == "barrier-missing-origin":
            e.barrier_origin_id = "missing"
        if variant == "barrier-ignore-scope":
            e.scope.origin_hex = h(4, 0)
            e.scope.range = -1
            e.scope.shape = Shape.POINT
        if variant == "barrier-negative":
            e.barrier_radius = -1
        if variant == "barrier-zero":
            e.barrier_radius = 0
        if variant == "barrier-inactive":
            e.is_active = False
            e.source_card_id = "absent"
        if variant == "barrier-expired":
            from goa2.domain.models.effect import DurationType

            e.duration = DurationType.THIS_TURN
            e.created_at_turn = 1
        if variant.startswith("empty"):
            e.effect_type = EffectType.EMPTY_HEX_OBSTACLE
        if variant == "empty-negative":
            e.scope.range = -1
        if variant == "empty-fixed-ignored":
            e.scope.origin_hex = h(4, 0)
        if variant == "empty-token-origin":
            e.scope.origin_id = "restriction-token"
        if variant == "empty-scope-ignored":
            from goa2.domain.models.effect import AffectsFilter

            e.scope.shape = Shape.POINT
            e.scope.affects = AffectsFilter.SELF
        if variant.startswith("petrify"):
            e.effect_type = EffectType.PETRIFY
        if variant == "petrify-missing-location":
            state.entity_locations.pop("hero_brogan")
            # Reproduce a genuinely stale tile occupant; keep it in the export too.
        if variant.startswith("mine"):
            state.active_effects.clear()
            mine = Token(
                id="mine",
                name="Passable token",
                token_type=TokenType.MINE_BLAST,
                owner_id="hero_arien",
                is_passable=True,
            )
            state.misc_entities[mine.id] = mine
            state.token_pool.setdefault(TokenType.MINE_BLAST, []).append(mine)
            state.place_entity(mine.id, h(2, -1))
            if variant == "mine-supply-only":
                del state.misc_entities[mine.id]
            if variant == "mine-pool-false":
                state.token_pool[TokenType.MINE_BLAST][-1] = mine.model_copy(
                    update={"is_passable": False}
                )
            if variant == "mine-entity-false":
                state.misc_entities[mine.id] = mine.model_copy(update={"is_passable": False})
            if variant == "mine-on-terrain":
                state.board.tiles[h(2, -1)].is_terrain = True
            if variant == "mine-unknown-occupant":
                state.misc_entities.pop(mine.id)
                state.token_pool[TokenType.MINE_BLAST].clear()
        if variant == "tile-terrain":
            state.board.tiles[h(2, -1)].is_terrain = True
        if variant == "map-gap":
            del state.board.tiles[h(2, -1)]
        yield variant, state, h, pieces, heavy


def validation_cases():
    from goa2.engine.rules import find_reachable_hexes, validate_movement_paths

    result = []
    for name, state, h, pieces, heavy in states():
        points = [h(0, 0), h(1, 0), h(2, -1), h(3, -1), h(-1, 0), h(-1, 1), h(5, 0)]
        actors = [
            "hero_arien",
            "hero_brogan",
            "hero_razzle",
            pieces[0],
            str(heavy.id),
            "restriction-token",
            "mine",
            None,
            "",
        ]
        queries, expected = [], []
        for actor in actors:
            for point in points:
                queries.append(
                    {"kind": "obstacle", "hex": point.model_dump(), "actor": actor, "moved": []}
                )
                expected.append(state.validator.is_obstacle_for_actor(state, point, actor))
        for point in points:
            queries.append(
                {"kind": "terrain", "hex": point.model_dump(), "actor": None, "moved": []}
            )
            expected.append(state.validator.is_terrain_hex(state, point))
            for moved in [[], ["hero_brogan"], ["restriction-token"], ["mine"]]:
                queries.append(
                    {"kind": "passable", "hex": point.model_dump(), "actor": None, "moved": moved}
                )
                expected.append(state.validator.is_passable_token(state, point, moved))
        for actor, units, moved, start in [
            ("hero_brogan", None, [], h(1, 0)),
            (None, None, [], h(1, 0)),
            ("hero_brogan", [], ["restriction-token"], h(1, 0)),
            (pieces[0], ["hero_razzle"], [pieces[0]], h(-1, 0)),
            ("hero_arien", [str(heavy.id)], ["mine"], h(5, 0)),
        ]:
            for distance in [0, 1, 3]:
                for ignore in [False, True]:
                    for through in [False, True]:
                        query = {
                            "start": start.model_dump(),
                            "ends": [p.model_dump() for p in points],
                            "max_steps": distance,
                            "ignore_obstacles": ignore,
                            "pass_through_obstacles": through,
                        }
                        kwargs = dict(
                            board=state.board,
                            start=start,
                            max_steps=distance,
                            ignore_obstacles=ignore,
                            state=state,
                            actor_id=actor,
                            pass_through_obstacles=through,
                            topology_unit_ids=units,
                            moved_ids=moved,
                        )
                        queries.append(
                            {
                                "kind": "paths",
                                "actor": actor,
                                "units": units,
                                "moved": moved,
                                "query": query,
                            }
                        )
                        expected.append(validate_movement_paths(ends=points, **kwargs))
                        reachable = find_reachable_hexes(**kwargs)
                        queries.append(
                            {
                                "kind": "reachable",
                                "actor": actor,
                                "units": units,
                                "moved": moved,
                                "query": query,
                            }
                        )
                        expected.append([point in reachable for point in points])
        result.append(
            {
                "name": name,
                "envelope": export_state(state),
                "queries": queries,
                "expected": expected,
            }
        )
    return result


def cases(advance, answer):
    from stack_traces import reference_trace

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import MoveUnitStep, SetContextFlagStep

    result = []
    for name, original, h, _pieces, _heavy in states():
        if name.startswith("mine") or name == "petrify-missing-location":
            continue  # Actual mine triggers and incoherent imported occupancy are separate gates.
        for mode in ["mandatory", "optional", "action", "straight"]:
            state = copy.deepcopy(original)
            state.execution_context = {"moved": "old", "crossed": "old"}
            move = MoveUnitStep(
                unit_id="hero_brogan",
                target_hex_arg=h(3, -2),
                range_val=3,
                is_mandatory=mode != "optional",
                is_movement_action=mode == "action",
                force_straight_line=mode == "straight",
                crossed_obstacles_output_key="crossed",
                success_output_key="moved",
                step_id="fixture/terrain/move",
            )
            if name in ("base-type-topology_split", "base-type-topology_isolation"):
                move.target_hex_arg = h(-2, 0)
            if mode == "straight":
                move.target_hex_arg = h(3, 0)
                move.pass_through_obstacles = True
            push_steps(
                state,
                [
                    move,
                    SetContextFlagStep(key="after", value=True, step_id="fixture/terrain/after"),
                ],
            )
            envelope = export_state(state)
            ops = [{"kind": "advance_prefix", "steps": 1}, advance]
            expected = reference_trace(state, ops)
            if name == "base-plain":
                assert state.execution_context.get("moved") is True
                assert state.execution_context.get("after") is True
            if name == "base-blocked":
                assert "moved" not in state.execution_context
                assert ("after" in state.execution_context) == (mode == "optional")
            result.append(
                {
                    "name": "terrain-move-" + name + "-" + mode,
                    "envelope": envelope,
                    "operations": ops,
                    "expected": expected,
                }
            )
    return result


def export(path, *, admitted_envelope=False):
    manifest = verify_sources()
    data = {
        "schema_version": 1,
        "source_sha256": manifest["source_sha256"],
        "cases": validation_cases(),
    }
    if admitted_envelope:
        from state_codec import omit_instance_equality

        omit_instance_equality(data["cases"])
    data["corpus_sha256"] = digest(data)
    write_text(path, json.dumps(data, separators=(",", ":")) + "\n")
    return data


def export_transitions(path):
    from stack_traces import write_corpus

    m = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": m["oracle_revision"],
        "source_sha256": m["source_sha256"],
        "cases": cases({"kind": "advance"}, None),
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


def verify(path, command):
    data = json.loads(read_text(path))
    identity = data.pop("corpus_sha256")
    if (
        data["schema_version"] != 1
        or digest(data) != identity
        or data["source_sha256"] != verify_sources()["source_sha256"]
    ):
        raise ValueError("terrain corpus source/identity mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request("terrain", envelope=case["envelope"], queries=case["queries"])[
                "results"
            ]
            if len(actual) != len(case["expected"]):
                raise AssertionError("terrain response shape mismatch")
            for index, (got, want) in enumerate(zip(actual, case["expected"], strict=True)):
                if got != want:
                    raise AssertionError((case["name"], case["queries"][index], got, want))
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "queries": sum(len(c["queries"]) for c in data["cases"]),
        "corpus_sha256": identity,
        "terrain_and_path_parity": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify", "export-transitions"])
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--admitted-envelope", action="store_true")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    args.corpus = args.corpus or ROOT / (
        "rust/artifacts/terrain-moves.json"
        if args.mode == "export-transitions"
        else "rust/tests/fixtures/terrain-validation.json"
    )
    if args.mode == "export-transitions":
        data = export_transitions(args.corpus)
        print(len(data["cases"]), data["corpus_sha256"])
    elif args.mode == "export":
        data = export(args.corpus, admitted_envelope=args.admitted_envelope)
        print(
            len(data["cases"]), sum(len(c["queries"]) for c in data["cases"]), data["corpus_sha256"]
        )
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker executable required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))
