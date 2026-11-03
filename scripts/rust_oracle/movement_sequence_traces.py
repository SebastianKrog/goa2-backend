"""Frozen effect-aware hex filters, movement sequences and controlled prompts."""

from __future__ import annotations

from fixture_cache import read_text, write_text

import argparse
import copy
import json
from pathlib import Path

from oracle import ROOT, Worker, digest, verify_sources
from state_codec import export_state


def spawn_points(state, h):
    from goa2.domain.models import MinionType, TeamColor
    from goa2.domain.models.spawn import SpawnPoint, SpawnType

    for point, kind, zone in [
        (h(1, 0), SpawnType.HERO, "area"),
        (h(2, -1), SpawnType.MINION, "far"),
        (h(-2, 1), SpawnType.MINION, "area"),
    ]:
        if point not in state.board.tiles:
            continue
        state.board.tiles[point].spawn_point = SpawnPoint(
            location=point,
            team=TeamColor.RED,
            type=kind,
            minion_type=MinionType.MELEE if kind == SpawnType.MINION else None,
        )
        state.board.tiles[point].zone_id = zone


def filter_states():
    from terrain_traces import states

    from goa2.data.heroes.registry import HeroRegistry

    for name, state, h, pieces, _heavy in states():
        spawn_points(state, h)
        yield name, state, h, pieces
    for variant in ["active", "level-seven", "facedown", "piece", "offboard"]:
        _, state, h, pieces, _ = next(c for c in states() if c[0] == "base-plain")
        hero = state.get_hero("hero_razzle" if variant == "piece" else "hero_arien")
        hero.level = 7 if variant == "level-seven" else 8
        hero.ultimate_card = HeroRegistry.get("Dodger").ultimate_card.model_copy(deep=True)
        hero.ultimate_card.is_facedown = variant == "facedown"
        state.current_actor_id = pieces[0] if variant == "piece" else str(hero.id)
        if variant == "offboard":
            state.remove_entity(hero.id)
        spawn_points(state, h)
        yield "tide-" + variant, state, h, pieces


def filter_cases():
    from goa2.engine.filters_geometry import (
        InStraightLineFilter,
        NotInStraightLineFilter,
        StraightLinePathFilter,
    )
    from goa2.engine.filters_hex import (
        AdjacentSpawnPointFilter,
        MovementPathFilter,
        ObstacleFilter,
        RangeFilter,
        SpawnPointFilter,
    )
    from goa2.engine.filters_units import AdjacencyFilter

    result = []
    for name, state, h, pieces in filter_states():
        state.execution_context = {
            "origin": pieces[0],
            "from_hex": h(1, 0).model_dump(),
            "low": False,
            "high": 2,
        }
        candidates = [*state.board.tiles, h(5, 0)]
        filters = []
        for unit in [None, "hero_arien", "hero_brogan", "hero_razzle", pieces[0], "unknown"]:
            for mode in ["topology", "geometric"]:
                filters.append(RangeFilter(origin_id=unit, max_range=2, distance_mode=mode))
            for distance in [0, 2]:
                for through in [False, True]:
                    filters.append(
                        MovementPathFilter(
                            unit_id=unit, range_val=distance, pass_through_obstacles=through
                        )
                    )
            filters.extend(
                [InStraightLineFilter(origin_id=unit), NotInStraightLineFilter(origin_id=unit)]
            )
            filters.extend(
                StraightLinePathFilter(origin_id=unit, pass_through_obstacles=through)
                for through in [False, True]
            )
        filters.extend(
            [
                RangeFilter(origin_key="origin", min_range_key="low", max_range_key="high"),
                RangeFilter(origin_hex_key="from_hex", min_range=1, max_range=3),
                RangeFilter(origin_hex_key="from_hex", distance_mode="geometric", max_range=2),
                MovementPathFilter(unit_key="origin", range_val=3),
            ]
        )
        filters.extend(
            ObstacleFilter(is_obstacle=wanted, exclude_id=exclude)
            for wanted in [False, True]
            for exclude in [None, "hero_arien", "hero_brogan"]
        )
        filters.extend(
            SpawnPointFilter(has_spawn_point=wanted, minion_only=minion)
            for wanted in [False, True]
            for minion in [False, True]
        )
        filters.extend(
            AdjacentSpawnPointFilter(
                is_empty=empty, must_not_have=forbidden, battle_zone_only=battle, minion_only=minion
            )
            for empty in [False, True]
            for forbidden in [False, True]
            for battle in [False, True]
            for minion in [False, True]
        )
        filters.extend(
            AdjacencyFilter(target_tags=tags)
            for tags in [
                [],
                ["FRIENDLY"],
                ["ENEMY"],
                ["HERO"],
                ["MINION"],
                ["FRIENDLY", "HERO"],
                ["ENEMY", "MINION"],
            ]
        )
        queries = []
        expected = []
        for f in filters:
            prepared = f.prepare(state, state.execution_context)
            queries.append(
                {
                    "filter": f.model_dump(mode="json"),
                    "candidates": [p.model_dump() for p in candidates],
                }
            )
            expected.append([prepared(p) for p in candidates])
        result.append(
            {
                "name": name,
                "envelope": export_state(state),
                "queries": queries,
                "expected": expected,
            }
        )
    return result


def aura_states():
    from terrain_traces import states

    from goa2.domain.models import StatType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape

    for variant in [
        "active",
        "inactive",
        "no-grant",
        "outside",
        "negative-cap",
        "action-cap",
        "immune",
        "source-moved",
        "bound-piece",
    ]:
        _, state, h, pieces, _ = next(c for c in states() if c[0] == "base-plain")
        aura = ActiveEffect(
            id="movement-aura",
            source_id="hero_arien",
            effect_type=EffectType.MOVEMENT_AURA_ZONE,
            scope=EffectScope(shape=Shape.RADIUS, range=2),
            duration=DurationType.PASSIVE,
            created_at_turn=state.turn,
            created_at_round=state.round,
            is_active=True,
            grants_pass_through_obstacles=True,
        )
        state.active_effects = [aura]
        state.board.tiles[h(2, 0)].is_terrain = True
        if variant == "inactive":
            aura.is_active = False
            aura.source_card_id = "absent"
        if variant == "no-grant":
            aura.grants_pass_through_obstacles = False
        if variant == "outside":
            aura.scope.range = 0
        if variant == "source-moved":
            state.move_unit("hero_arien", h(-3, 0))
        if variant in ["negative-cap", "action-cap"]:
            cap = aura.model_copy(
                deep=True,
                update={
                    "id": "movement-cap",
                    "effect_type": EffectType.MOVEMENT_ZONE,
                    "max_value": -1 if variant == "negative-cap" else 1,
                    "limit_actions_only": True,
                },
            )
            state.active_effects.append(cap)
        if variant == "immune":
            state.active_effects.append(
                aura.model_copy(
                    deep=True,
                    update={
                        "id": "aura-immunity",
                        "source_id": "hero_brogan",
                        "effect_type": EffectType.IMMUNITY_ENEMY_ACTIONS,
                        "scope": EffectScope(shape=Shape.GLOBAL),
                    },
                )
            )
        if variant == "bound-piece":
            state.current_actor_id = "hero_razzle"
            state.acting_piece_id = pieces[0]
        state.get_hero("hero_brogan").items[StatType.MOVEMENT] = 1
        yield "aura-" + variant, state, h, pieces


def drive(name, state, steps, *, mode="normal", emit=None):
    from stack_traces import reference_trace

    from goa2.domain.input import InputResponse, selection_value
    from goa2.engine.handler import process_stack, push_steps, submit_input

    for i, step in enumerate(steps):
        step.step_id = f"fixture/sequence/{name}/{i}"
    push_steps(state, steps)
    envelope = export_state(state) if emit is None else None
    probe = copy.deepcopy(state)
    ops = []
    invalid = False
    seen = []
    for _ in range(30):
        ops.append({"kind": "advance_prefix", "steps": 1})
        remaining = 1

        def boundary(_state, _step):
            nonlocal remaining
            if not remaining:
                return True
            remaining -= 1
            return False

        req = process_stack(probe, stop_before_step=boundary).input_request
        if req is None:
            if probe.execution_stack:
                continue
            break
        seen.append(req.model_dump(mode="json"))
        if mode == "invalid" and not invalid:
            value = {"q": 99, "r": 0, "s": -99}
            invalid = True
        elif mode == "skip" and req.can_skip:
            value = "SKIP"
        else:
            choices = [selection_value(o) for o in req.options]
            hexes = [v for v in choices if isinstance(v, dict)]
            origin = (
                probe.get_position(str(probe.current_actor_id)) if probe.current_actor_id else None
            )
            value = (
                max(
                    hexes,
                    key=lambda p: abs(p["q"] - (origin.q if origin else 0))
                    + abs(p["r"] - (origin.r if origin else 0)),
                )
                if hexes
                else choices[0]
            )
        ops.append({"kind": "input", "request_id": "current", "selection": value})
        submit_input(probe, InputResponse(request_id=req.id, selection=value))
    else:
        raise AssertionError((name, "did not finish"))
    if name == "movement-sequence-base-plain-normal":
        assert probe.get_position("hero_brogan") != state.get_position("hero_brogan")
        assert probe.execution_context.get("after") is True
    if name == "movement-sequence-base-plain-skip":
        assert probe.get_position("hero_brogan") == state.get_position("hero_brogan")
        assert probe.execution_context.get("after") is True
    if emit is not None:
        emit(name, state, ops)
        return None, seen
    return {
        "name": name,
        "envelope": envelope,
        "operations": ops,
        "expected": reference_trace(state, ops),
    }, seen


def controlled_cases():
    from terrain_traces import states

    from goa2.domain.models import TargetType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.steps import AskConfirmationStep, SelectStep, SetContextFlagStep

    variants = [
        "active",
        "inactive-cardless",
        "expired-active",
        "future-active",
        "wrong-card",
        "wrong-origin",
        "no-owner",
        "other-owner",
        "other-actor",
        "different-performed-card",
        "first-controller",
        "source-offboard",
        "unknown-controller",
        "self-controller",
        "no-current-card",
        "bound-owner",
    ]
    result = []
    for variant in variants:
        for kind in ["number", "discard-number", "hex", "confirm"]:
            _, state, _h, pieces, _ = next(c for c in states() if c[0] == "base-plain")
            actor = "hero_razzle" if variant == "bound-owner" else "hero_arien"
            hero = state.get_hero(actor)
            hero.current_turn_card = hero.hand[1]
            state.current_actor_id = actor
            state.resolution_owner_id = actor
            e = ActiveEffect(
                id="control",
                source_id="hero_hanu",
                effect_type=EffectType.CONTROL_NEXT_ACTION,
                scope=EffectScope(shape=Shape.GLOBAL, origin_id=actor),
                duration=DurationType.PASSIVE,
                is_active=True,
                created_at_turn=state.turn,
                created_at_round=state.round,
                controlled_card_id=hero.current_turn_card.id,
            )
            state.active_effects = [e]
            if variant == "inactive-cardless":
                e.is_active = False
            if variant == "expired-active":
                e.duration = DurationType.THIS_TURN
                e.created_at_turn = 0
            if variant == "future-active":
                e.duration = DurationType.NEXT_TURN
                e.created_at_turn = state.turn + 1
            if variant == "wrong-card":
                e.controlled_card_id = "wrong"
            if variant == "wrong-origin":
                e.scope.origin_id = "hero_brogan"
            if variant == "no-owner":
                state.resolution_owner_id = None
            if variant == "other-owner":
                state.resolution_owner_id = "hero_brogan"
            if variant == "other-actor":
                state.current_actor_id = "hero_brogan"
            if variant == "different-performed-card":
                state.execution_context = {"current_card_id": str(hero.hand[0].id)}
            if variant == "first-controller":
                state.active_effects.insert(
                    0, e.model_copy(deep=True, update={"id": "first", "source_id": "hero_brogan"})
                )
            if variant == "source-offboard":
                state.remove_entity("hero_hanu")
            if variant == "unknown-controller":
                e.source_id = "unknown-controller"
            if variant == "self-controller":
                e.source_id = actor
            if variant == "no-current-card":
                hero.current_turn_card = None
            if variant == "bound-owner":
                state.acting_piece_id = pieces[0]
            if kind == "confirm":
                step = AskConfirmationStep(
                    prompt="Confirm controlled action", output_key="answer", player_id=actor
                )
            else:
                step = SelectStep(
                    target_type=TargetType.HEX if kind == "hex" else TargetType.NUMBER,
                    prompt="Controlled decision",
                    output_key="answer",
                    number_options=[1, 2] if kind != "hex" else [],
                    decision_purpose="discard" if kind == "discard-number" else None,
                    override_player_id=actor,
                )
            case, seen = drive(
                "controlled-" + variant + "-" + kind,
                state,
                [step, SetContextFlagStep(key="after", value=True)],
                mode="invalid" if kind != "confirm" else "normal",
            )
            if variant == "active":
                assert seen and all(
                    r["player_id"] == "hero_hanu" and r["context"]["controlled_hero_id"] == actor
                    for r in seen
                )
            result.append(case)
    return result


def sequence_scenes():
    from terrain_traces import states

    from goa2.domain.models import StatType
    from goa2.engine.steps import MoveSequenceStep, SetContextFlagStep

    for name, state, h, _pieces, _ in states():
        if name.startswith("mine") or name == "petrify-missing-location":
            continue
        for mode in ["normal", "straight", "full", "preselected", "skip"]:
            source = copy.deepcopy(state)
            source.execution_context = (
                {"target_hex": h(3, -2).model_dump()} if mode == "preselected" else {}
            )
            step = MoveSequenceStep(
                unit_id="hero_brogan",
                range_val=3,
                force_straight_line=mode == "straight",
                force_full_distance=mode == "full",
                is_mandatory=mode != "skip",
            )
            yield "movement-sequence-" + name + "-" + mode, source, [
                step,
                SetContextFlagStep(key="after", value=True),
            ], mode
    for name, state, _h, _pieces in aura_states():
        for mode in ["normal", "straight", "full", "invalid"]:
            source = copy.deepcopy(state)
            unit = "hero_razzle" if name == "aura-bound-piece" else "hero_brogan"
            step = MoveSequenceStep(
                unit_id=unit,
                range_val=2,
                range_stat_type=StatType.MOVEMENT,
                force_straight_line=mode == "straight",
                force_full_distance=mode == "full",
            )
            yield "movement-sequence-" + name + "-" + mode, source, [
                step,
                SetContextFlagStep(key="after", value=True),
            ], mode


def cases(advance, answer):
    result = [
        drive(name, state, steps, mode=mode)[0] for name, state, steps, mode in sequence_scenes()
    ]
    result.extend(controlled_cases())
    return result


def export(path):
    m = verify_sources()
    data = {"schema_version": 1, "source_sha256": m["source_sha256"], "cases": filter_cases()}
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
        raise ValueError("hex filter corpus source/identity mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            got = worker.request("hex_filters", envelope=case["envelope"], queries=case["queries"])[
                "results"
            ]
            if len(got) != len(case["expected"]):
                raise AssertionError("hex filter response shape mismatch")
            for i, (actual, want) in enumerate(zip(got, case["expected"], strict=True)):
                if actual != want:
                    raise AssertionError((case["name"], case["queries"][i], actual, want))
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "queries": sum(len(c["queries"]) for c in data["cases"]),
        "verdicts": sum(sum(len(v) for v in c["expected"]) for c in data["cases"]),
        "corpus_sha256": identity,
        "full_filter_parity": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify", "export-transitions"])
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    args.corpus = args.corpus or ROOT / (
        "rust/artifacts/movement-sequences.json"
        if args.mode == "export-transitions"
        else "rust/tests/fixtures/hex-filters.json"
    )
    if args.mode == "export-transitions":
        data = export_transitions(args.corpus)
        print(len(data["cases"]), data["corpus_sha256"])
    elif args.mode == "export":
        data = export(args.corpus)
        print(
            len(data["cases"]), sum(len(c["queries"]) for c in data["cases"]), data["corpus_sha256"]
        )
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker executable required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))
