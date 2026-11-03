"""Frozen mine paths, path-choice sets, ordered triggers and reference defects."""

from __future__ import annotations

from fixture_cache import read_text, write_text

import argparse
import copy
import json
import os
import subprocess
from pathlib import Path

from oracle import ROOT, Worker, digest, verify_sources
from state_codec import export_state


def states():
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.domain.board import Board
    from goa2.domain.models import GamePhase, Token, TokenType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape

    variants = [
        "plain",
        "one-dud",
        "one-blast",
        "mixed",
        "friendly",
        "unowned",
        "unknown-owner",
        "supply-only",
        "entity-priority",
        "pool-mismatch",
        "terrain-mine",
        "map-gap",
        "offboard-start",
        "no-actor",
        "actor-other",
        "actor-empty",
        "barrier",
        "empty-denial",
        "split",
        "isolation",
        "topology-dormant",
        "bound-owner",
        "controlled",
    ]
    for variant in variants:
        state = setup(["Min", "Razzle"], ["Arien", "Hanu"])
        h = _board(state)
        state.board = Board(
            tiles={
                p: tile
                for p, tile in state.board.tiles.items()
                if max(abs(p.q), abs(p.r), abs(p.s)) <= 2
            }
        )
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        state.resolution_owner_id = "hero_arien"
        pieces = _pieces(state, "hero_razzle")
        for uid, point in [
            ("hero_arien", h(-2, 0)),
            ("hero_min", h(2, 0)),
            ("hero_hanu", h(-1, 2)),
            (pieces[0], h(2, -2)),
            (pieces[1], h(1, 1)),
        ]:
            state.place_entity(uid, point)
        mines = []
        for i, (mid, point) in enumerate(
            [("mine_a", h(-1, 0)), ("mine_b", h(0, -1)), ("mine_c", h(0, 1)), ("mine_d", h(1, -1))]
        ):
            if variant == "plain" or (variant in ["one-dud", "one-blast"] and i > 0):
                continue
            kind = TokenType.MINE_BLAST if variant == "one-blast" or i % 2 else TokenType.MINE_DUD
            token = Token(
                id=mid,
                name=mid,
                token_type=kind,
                owner_id="hero_min",
                is_passable=True,
                is_facedown=True,
            )
            state.misc_entities[mid] = token
            state.token_pool.setdefault(kind, []).append(token)
            state.place_entity(mid, point)
            mines.append(mid)
        if variant == "friendly":
            for mid in mines:
                state.misc_entities[mid].owner_id = "hero_hanu"
        if variant == "unowned":
            for mid in mines:
                state.misc_entities[mid].owner_id = None
        if variant == "unknown-owner":
            for mid in mines:
                state.misc_entities[mid].owner_id = "unknown"
        if variant == "supply-only":
            for mid in mines:
                state.misc_entities.pop(mid)
        if variant == "entity-priority":
            state.misc_entities["mine_a"] = state.misc_entities["mine_a"].model_copy(
                update={"is_passable": False}
            )
        if variant == "pool-mismatch":
            for kind, tokens in state.token_pool.items():
                state.token_pool[kind] = [
                    t.model_copy(update={"is_passable": False}) if str(t.id) == "mine_a" else t
                    for t in tokens
                ]
        if variant == "terrain-mine":
            state.board.tiles[h(-1, 0)].is_terrain = True
        if variant == "map-gap":
            del state.board.tiles[h(0, 0)]
        if variant == "offboard-start":
            state.remove_entity("hero_arien")
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "actor-other":
            state.current_actor_id = "hero_min"
        if variant == "actor-empty":
            state.current_actor_id = ""
        if variant in ["barrier", "empty-denial", "split", "isolation", "topology-dormant"]:
            kind = {
                "barrier": EffectType.STATIC_BARRIER,
                "empty-denial": EffectType.EMPTY_HEX_OBSTACLE,
                "split": EffectType.TOPOLOGY_SPLIT,
                "isolation": EffectType.TOPOLOGY_ISOLATION,
                "topology-dormant": EffectType.TOPOLOGY_SPLIT,
            }[variant]
            e = ActiveEffect(
                id="mine-rule",
                source_id="hero_min",
                effect_type=kind,
                scope=EffectScope(shape=Shape.RADIUS, range=1),
                duration=DurationType.PASSIVE,
                created_at_turn=state.turn,
                created_at_round=state.round,
                is_active=variant != "topology-dormant",
                barrier_radius=1,
                split_axis="q",
                split_value=0,
            )
            state.active_effects.append(e)
        if variant == "bound-owner":
            state.current_actor_id = "hero_razzle"
            state.acting_piece_id = pieces[0]
        if variant == "controlled":
            hero = state.get_hero("hero_arien")
            hero.current_turn_card = hero.hand[1]
            state.active_effects.append(
                ActiveEffect(
                    id="control",
                    source_id="hero_min",
                    effect_type=EffectType.CONTROL_NEXT_ACTION,
                    scope=EffectScope(shape=Shape.GLOBAL, origin_id="hero_arien"),
                    duration=DurationType.PASSIVE,
                    created_at_turn=state.turn,
                    created_at_round=state.round,
                    is_active=True,
                    controlled_card_id=hero.current_turn_card.id,
                )
            )
        yield variant, state, h, pieces, mines


def path_cases():
    from goa2.engine.rules import find_reachable_with_mines

    result = []
    for name, state, h, pieces, _ in states():
        points = [*state.board.tiles, h(3, 0)]
        queries = []
        expected = []
        for start in [h(-2, 0), h(3, 0)]:
            for range_val in [-1, 0, 1, 3, 4]:
                for actor, team, units in [
                    ("hero_arien", "BLUE", ["hero_arien"]),
                    (None, None, None),
                    ("hero_min", "RED", []),
                    (None, "BLUE", [pieces[0]]),
                ]:
                    reachable = find_reachable_with_mines(
                        state.board,
                        start,
                        range_val,
                        state,
                        actor_id=actor,
                        moving_team=team,
                        topology_unit_ids=units,
                    )
                    queries.append(
                        {
                            "start": start.model_dump(),
                            "range": range_val,
                            "actor": actor,
                            "team": team,
                            "units": units,
                            "ends": [p.model_dump() for p in points],
                        }
                    )
                    expected.append(
                        [
                            [
                                {
                                    "mine_ids": sorted(option.mine_ids),
                                    "path": [p.model_dump() for p in option.path],
                                }
                                for option in reachable.get(end, [])
                            ]
                            for end in points
                        ]
                    )
        result.append(
            {
                "name": name,
                "envelope": export_state(state),
                "queries": queries,
                "expected": expected,
            }
        )
    return result


def normalize_choice_record(record, keys):
    """Only declared choice-set fields; never sort event queues, paths or options."""
    for key in keys:
        values = record.get("semantic_state", {}).get("execution_context", {}).get(key)
        if isinstance(values, list) and all(isinstance(v, str) for v in values):
            values.sort()
    request = record.get("request")
    if request and request.get("prompt") == "Choose which mines to move through":
        for option in request["options"]:
            option["metadata"]["metadata"]["mine_hexes"].sort(
                key=lambda p: (p["q"], p["r"], p["s"])
            )
    return record


def drive(name, state, steps, mode="first", unordered_keys=(), select=None):
    from stack_traces import reference_trace

    from goa2.domain.input import InputResponse, selection_value
    from goa2.engine.handler import process_stack, push_steps, submit_input

    for i, step in enumerate(steps):
        step.step_id = f"fixture/mines/{name}/{i}"
    push_steps(state, steps)
    envelope = export_state(state)
    probe = copy.deepcopy(state)
    ops = []
    bad = False
    for _ in range(60):
        remaining = 1

        def boundary(_state, _step):
            nonlocal remaining
            if not remaining:
                return True
            remaining -= 1
            return False

        out = process_stack(probe, stop_before_step=boundary)
        ops.append({"kind": "advance_prefix", "steps": 1})
        req = out.input_request
        if req is None:
            if probe.execution_stack:
                continue
            break
        if select is not None:
            value = select(req, mode)
        elif mode in ["invalid", "numeric"] and not bad:
            value = "invalid" if mode == "invalid" else 0
            bad = True
        else:
            option = req.options[-1] if mode == "last" else req.options[0]
            value = (
                option.id
                if req.prompt == "Choose which mines to move through"
                else selection_value(option)
            )
        submit_input(probe, InputResponse(request_id=req.id, selection=value))
        ops.append({"kind": "input", "request_id": "current", "selection": value})
    else:
        raise AssertionError((name, "did not finish"))
    expected = reference_trace(state, ops)
    if unordered_keys:
        expected = [normalize_choice_record(record, unordered_keys) for record in expected]
    case = {"name": name, "envelope": envelope, "operations": ops, "expected": expected}
    if unordered_keys:
        case["unordered_mine_keys"] = list(unordered_keys)
    return case, probe


def cases(advance, answer):
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.steps import MinePathChoiceStep, SetContextFlagStep, TriggerMineStep

    result = []
    for name, original, h, _pieces, _ in states():
        for mode in ["first", "last", "invalid", "numeric"]:
            state = copy.deepcopy(original)
            state.execution_context = {
                "target_hex": h(1, 0).model_dump(),
                "triggered_mine_ids": ["stale"],
            }
            step = MinePathChoiceStep(unit_id="hero_arien", range_val=4)
            case, _ = drive(
                "mine-choice-" + name + "-" + mode,
                state,
                [step, SetContextFlagStep(key="after", value=True)],
                mode,
                ("triggered_mine_ids",),
            )
            result.append(case)
    for variant in [
        "missing-destination",
        "zero-range",
        "unknown-unit",
        "unit-key",
        "offboard",
        "custom-output",
        "skip-destination",
        "no-actor",
    ]:
        _, state, h, _pieces, _ = next(c for c in states() if c[0] == "mixed")
        state.execution_context = {
            "target_hex": h(1, 0).model_dump(),
            "who": "hero_arien",
            "triggered_mine_ids": ["stale"],
        }
        step = MinePathChoiceStep(unit_id="hero_arien", range_val=4)
        if variant == "missing-destination":
            state.execution_context.pop("target_hex")
        if variant == "zero-range":
            step.range_val = 0
        if variant == "unknown-unit":
            step.unit_id = "unknown"
        if variant == "unit-key":
            step.unit_id = None
            step.unit_key = "who"
        if variant == "offboard":
            state.remove_entity("hero_arien")
        if variant == "custom-output":
            step.output_key = "chosen_set"
        if variant == "skip-destination":
            state.execution_context["target_hex"] = "SKIP"
        if variant == "no-actor":
            state.current_actor_id = None
            step.unit_id = None
        case, _ = drive(
            "mine-choice-" + variant,
            state,
            [step, SetContextFlagStep(key="after", value=True)],
            "first",
            (step.output_key,),
        )
        result.append(case)
    for variant in [
        "dud",
        "blast",
        "mixed",
        "reverse",
        "duplicate-blast",
        "offboard-blast",
        "unknown",
        "hero-id",
        "empty-literal",
        "context-list",
        "literal-victim",
        "missing-victim",
        "empty-hand",
        "actor-other",
        "actor-empty",
        "dud-effects",
        "offboard-dud-effects",
    ]:
        _, state, h, _pieces, _mines = next(c for c in states() if c[0] == "mixed")
        ids = (
            ["mine_a"] if variant in ["dud", "dud-effects", "offboard-dud-effects"] else ["mine_b"]
        )
        if variant == "mixed":
            ids = ["mine_b", "mine_a", "mine_d"]
        if variant == "reverse":
            ids = ["mine_d", "mine_a", "mine_b"]
        if variant == "duplicate-blast":
            ids = ["mine_b", "mine_b"]
        if variant == "offboard-blast":
            state.remove_entity("mine_b")
        if variant == "unknown":
            ids = ["unknown"]
        if variant == "hero-id":
            ids = ["hero_arien"]
        state.execution_context = {"triggered_mine_ids": ["mine_c"], "mine_victim_id": "hero_arien"}
        step = TriggerMineStep(mine_ids=ids, victim_id="hero_arien")
        if variant == "empty-literal":
            step.mine_ids = []
        if variant == "context-list":
            step.mine_ids = None
            state.execution_context["triggered_mine_ids"] = ["mine_a", "mine_b"]
        if variant == "literal-victim":
            state.execution_context["mine_victim_id"] = "hero_hanu"
        if variant == "missing-victim":
            step.victim_id = None
            state.execution_context.pop("mine_victim_id")
        if variant == "empty-hand":
            state.get_hero("hero_arien").hand = []
        if variant == "actor-other":
            state.current_actor_id = "hero_min"
        if variant == "actor-empty":
            state.current_actor_id = ""
        if variant in ["dud-effects", "offboard-dud-effects"]:
            e = ActiveEffect(
                id="mine-linked",
                source_id="hero_min",
                token_id="mine_a",
                effect_type=EffectType.AREA_STAT_MODIFIER,
                scope=EffectScope(shape=Shape.GLOBAL),
                duration=DurationType.PASSIVE,
                created_at_turn=state.turn,
                created_at_round=state.round,
                is_active=True,
            )
            state.active_effects.append(e)
            if variant == "offboard-dud-effects":
                state.remove_entity("mine_a")
        case, probe = drive(
            "mine-trigger-" + variant, state, [step, SetContextFlagStep(key="after", value=True)]
        )
        if variant == "empty-literal":
            assert "rollback_reanchor_pending" not in probe.execution_context
        else:
            assert probe.execution_context.get("rollback_reanchor_pending") is True
        if variant == "blast":
            assert len(probe.get_hero("hero_arien").hand) == 4
        if variant == "duplicate-blast":
            assert len(probe.get_hero("hero_arien").hand) == 3
        result.append(case)
    return result


def reference_defects():
    """Reproduce defects without editing or monkeypatching the frozen engine."""
    import sys

    code = """from model_traces import setup
from aura_traces import _board
from goa2.domain.board import Board
from goa2.domain.tile import Tile
from goa2.domain.models import Token,TokenType
from goa2.engine.steps import MinePathChoiceStep
import json
s=setup(['Min','Hanu'],['Arien','Brogan']);h=_board(s)
s.board=Board(tiles={h(q,0):Tile(hex=h(q,0)) for q in range(4)})
s.place_entity('hero_arien',h(0,0));s.current_actor_id='hero_arien'
for i,mid in enumerate(['blast-a','blast-b'],1):
 t=Token(id=mid,name=mid,token_type=TokenType.MINE_BLAST,owner_id='hero_min',is_passable=True)
 s.misc_entities[mid]=t;s.token_pool.setdefault(TokenType.MINE_BLAST,[]).append(t);s.place_entity(mid,h(i,0))
c={'target_hex':h(3,0).model_dump()}
MinePathChoiceStep(unit_id='hero_arien',range_val=3).resolve(s,c)
print(json.dumps(c['triggered_mine_ids']))
"""
    outputs = {}
    for seed in [1, 2]:
        env = dict(
            os.environ,
            PYTHONHASHSEED=str(seed),
            PYTHONPATH=os.pathsep.join([str(ROOT / "scripts/rust_oracle"), str(ROOT / "src")]),
        )
        outputs[str(seed)] = json.loads(
            subprocess.check_output([sys.executable, "-c", code], cwd=ROOT, env=env, text=True)
        )
    assert outputs["1"] != outputs["2"] and set(outputs["1"]) == set(outputs["2"])
    from goa2.domain.input import selection_value
    from goa2.engine.steps import MinePathChoiceStep

    _, state, h, _, _ = next(c for c in states() if c[0] == "one-dud")
    step = MinePathChoiceStep(unit_id="hero_arien", range_val=4)
    context = {"target_hex": h(1, 0).model_dump()}
    request = step.resolve(state, context).input_request
    assert request is not None
    option = request.options[0]
    coerced = selection_value(option)
    step.pending_input = {"selection": coerced}
    bad = step.resolve(state, context)
    step.pending_input = {"selection": option.id}
    good = step.resolve(state, context)
    assert bad.requires_input and not good.requires_input
    from goa2.engine.steps import MoveUnitStep

    _, literal_state, h, _, _ = next(c for c in states() if c[0] == "one-dud")
    literal_context = {}
    move = MoveUnitStep(unit_id="hero_arien", target_hex_arg=h(0, 0), range_val=3)
    queued = move.resolve(literal_state, literal_context).new_steps
    assert len(queued) == 2
    queued[0].resolve(literal_state, literal_context)
    moved = queued[1].resolve(literal_state, literal_context)
    assert literal_context["triggered_mine_ids"] == []
    assert literal_state.entity_locations["hero_arien"] == h(0, 0)
    assert "mine_a" in literal_state.entity_locations and not moved.new_steps
    return [
        {
            "kind": "literal-destination-lost-in-mine-choice",
            "generated_choice_destination_key": queued[0].destination_key,
            "choice_context_destination_present": queued[0].destination_key in literal_context,
            "chosen_mines": literal_context["triggered_mine_ids"],
            "destination_reached": True,
            "mine_remains_deployed": True,
        },
        {
            "kind": "hash-dependent-mine-order",
            "hash_seed_outputs": outputs,
            "meaning": "identical mine sets become different trigger lists; implicit ordering is not deterministic",
        },
        {
            "kind": "mine-option-helper-coercion",
            "helper_value": coerced,
            "raw_option_id": option.id,
            "helper_value_accepted": False,
            "raw_id_accepted": True,
            "wire_metadata_keys": list(option.metadata),
        },
    ]


def export_errors(path):
    m = verify_sources()
    data = {
        "schema_version": 1,
        "source_sha256": m["source_sha256"],
        "errors": reference_defects(),
        "native_policy": "mine path identity is an unordered set with stable native order; explicit trigger/event order is preserved; literal-destination loss remains a documented source compatibility defect",
    }
    data["corpus_sha256"] = digest(data)
    write_text(path, json.dumps(data, indent=2) + "\n")
    return data


def verify_errors(path):
    data = json.loads(read_text(path))
    identity = data.pop("corpus_sha256")
    assert digest(data) == identity and data["source_sha256"] == verify_sources()["source_sha256"]
    assert data["errors"] == reference_defects()
    return {"reference_errors": len(data["errors"]), "corpus_sha256": identity, "reproduced": True}


def export(path):
    m = verify_sources()
    data = {
        "schema_version": 1,
        "source_sha256": m["source_sha256"],
        "mine_identity_contract": "unordered-set; path/option order preserved",
        "cases": path_cases(),
    }
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
        "cases": cases(None, None),
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
        raise ValueError("mine corpus identity/source mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request(
                "mine_paths", envelope=case["envelope"], queries=case["queries"]
            )["results"]
            if len(actual) != len(case["expected"]):
                raise AssertionError("mine response shape mismatch")
            for i, (got, want) in enumerate(zip(actual, case["expected"], strict=True)):
                if got != want:
                    raise AssertionError((case["name"], case["queries"][i], got, want))
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "queries": sum(len(c["queries"]) for c in data["cases"]),
        "corpus_sha256": identity,
        "mine_set_and_exact_path_parity": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument(
        "mode", choices=["export", "verify", "export-transitions", "export-errors", "verify-errors"]
    )
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    args.corpus = args.corpus or ROOT / (
        "rust/artifacts/mine-steps.json"
        if args.mode == "export-transitions"
        else (
            "rust/tests/fixtures/mine-reference-errors.json"
            if args.mode.endswith("errors")
            else "rust/tests/fixtures/mine-paths.json"
        )
    )
    if args.mode == "export-errors":
        d = export_errors(args.corpus)
        print(d["corpus_sha256"])
    elif args.mode == "verify-errors":
        print(json.dumps(verify_errors(args.corpus), indent=2))
    elif args.mode == "export-transitions":
        d = export_transitions(args.corpus)
        print(len(d["cases"]), d["corpus_sha256"])
    elif args.mode == "export":
        d = export(args.corpus)
        print(len(d["cases"]), sum(len(c["queries"]) for c in d["cases"]), d["corpus_sha256"])
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker executable required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))
