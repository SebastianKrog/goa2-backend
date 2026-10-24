"""Real repeat decisions, nested copies, validation and action-boundary lane pushes."""

from __future__ import annotations


def cases(advance, answer):
    from aura_traces import _board, _pieces
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import GamePhase, TeamColor
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import (
        AddContextValueStep,
        MayRepeatNTimesStep,
        MayRepeatOnceStep,
        ValidateRepeatStep,
    )

    result = []
    variants = [
        "once",
        "twice",
        "decline",
        "skip",
        "once-then-no",
        "nested",
        "already-one",
        "negative-done",
        "zero-max",
        "negative-max",
        "done",
        "no-actor",
        "offboard",
        "unknown-actor",
        "piece",
        "lane-push",
        "lane-checked",
        "lane-empty",
    ]
    for variant in variants:
        state = setup(["Arien", "Hanu"], ["Razzle", "Brogan"])
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        if not variant.startswith("lane-"):
            h = _board(state)
            state.battle_zones = {}
            state.place_entity("hero_arien", h(0, 0))
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "offboard":
            state.remove_entity("hero_arien")
        if variant == "unknown-actor":
            state.current_actor_id = "missing"
        if variant == "piece":
            piece = _pieces(state, "hero_razzle")[0]
            state.place_entity(piece, h(1, 0))
            state.current_actor_id = piece
        state.execution_context = {"repeat_count": 0}
        child = AddContextValueStep(key="repeat_count", amount=1, step_id="fixture/repeat/child")
        max_repeats = 1 if variant == "once" else 2
        done = (
            1
            if variant == "already-one"
            else -1 if variant == "negative-done" else 2 if variant == "done" else 0
        )
        if variant in ["zero-max", "negative-max"]:
            max_repeats = 0 if variant == "zero-max" else -1
        if variant == "nested":
            child = MayRepeatOnceStep(steps_template=[child], step_id="fixture/repeat/inner")
        checked = variant == "lane-checked"
        initial_lane = None
        if variant.startswith("lane-"):
            lane = next(iter(state.battle_zones))
            initial_lane = state.battle_zones[lane]
            if variant != "lane-empty":
                for team in state.teams.values():
                    for minion in team.minions:
                        state.remove_entity(minion.id)
                minion = state.teams[TeamColor.RED].minions[0]
                minion.lane_id = lane
                zone = state.board.zones[initial_lane]
                location = next(
                    h
                    for h in state.board.tiles
                    if h in zone.hexes and not state.board.tiles[h].is_occupied
                )
                state.place_entity(minion.id, location)
            if checked:
                max_repeats = 1
        step = MayRepeatNTimesStep(
            max_repeats=max_repeats,
            repeats_done=done,
            lane_push_checked=checked,
            steps_template=[child],
            prompt="Repeat test?",
            step_id=f"fixture/repeat/{variant}",
        )
        push_steps(state, [step])
        envelope = export_state(state)
        probe = state.model_copy(deep=True)
        ops = [{"kind": "advance_prefix", "steps": 1}]
        confirms = 0
        for _ in range(100):
            ops.append(advance)
            request = process_stack(probe).input_request
            if request is None:
                break
            if request.request_type.value == "SELECT_OPTION" and "Repeat" in request.prompt:
                value = (
                    "SKIP"
                    if variant == "skip"
                    else (
                        "NO"
                        if variant == "decline" or (variant == "once-then-no" and confirms)
                        else "YES"
                    )
                )
                confirms += 1
            else:
                value = selection_value(request.options[0])
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=request.id, selection=value))
        else:
            raise AssertionError(variant)
        if not variant.startswith("lane-"):
            expected = (
                0
                if variant in ["decline", "skip", "zero-max", "negative-max", "done", "no-actor"]
                else (
                    1
                    if variant in ["once", "once-then-no", "already-one"]
                    else 3 if variant == "negative-done" else 2
                )
            )
            assert probe.execution_context["repeat_count"] == expected, (
                variant,
                probe.execution_context,
            )
        if variant == "lane-push":
            assert probe.battle_zones[lane] != initial_lane
        if variant == "lane-checked":
            assert probe.battle_zones[lane] == initial_lane
        result.append(
            {
                "name": f"repeat-{variant}",
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    for variant in [
        "actor",
        "no-actor",
        "literal",
        "false",
        "true",
        "null",
        "number",
        "text",
        "missing",
    ]:
        state = setup()
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = None if variant == "no-actor" else "hero_arien"
        state.execution_context = {
            "previous": {
                "false": False,
                "true": True,
                "null": None,
                "number": 3,
                "text": "keep",
            }.get(variant, True)
        }
        if variant == "missing":
            state.execution_context.clear()
        and_key = (
            "previous"
            if variant in ["false", "true", "null", "number", "text", "missing"]
            else None
        )
        step = ValidateRepeatStep(
            actor_id="missing" if variant == "literal" else None,
            and_with_key=and_key,
            output_key="allowed",
            step_id=f"fixture/repeat-validation/{variant}",
        )
        push_steps(state, [step])
        result.append(
            {
                "name": f"repeat-validation-{variant}",
                "envelope": export_state(state),
                "operations": [advance],
                "expected": reference_trace(state, [advance]),
            }
        )
    return result
