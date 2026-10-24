"""Decision-by-decision traces against the frozen real Python stack handler."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import sys
from pathlib import Path

from oracle import ROOT, Worker, canonical, digest, verify_sources
from state_codec import export_state


class TraceNormalizer:
    """Normalize only typed ephemeral request IDs; keep semantic IDs and ordering."""

    def __init__(self):
        self.requests = {}
        self.steps = {}
        self.python_steps = {}
        self.retained_steps = []

    def request_id(self, value):
        if value is None:
            return None
        if value not in self.requests:
            self.requests[value] = f"request/{len(self.requests)+1}"
        return self.requests[value]

    def bind_steps(self, steps):
        def bind(step):
            if not step.step_id.startswith("fixture/"):
                key = id(step)
                if key not in self.python_steps:
                    self.python_steps[key] = f"dynamic-step/{len(self.python_steps)+1}"
                    self.retained_steps.append(step)
            for child in getattr(step, "steps_template", []):
                bind(child)

        for step in steps:
            bind(step)

    def step(self, step, live=None):
        if live is not None and not live.step_id.startswith("fixture/"):
            step["step_id"] = self.python_steps[id(live)]
        elif str(step.get("step_id", "")).startswith("native-step-"):
            value = step["step_id"]
            if value not in self.steps:
                self.steps[value] = f"dynamic-step/{len(self.steps)+1}"
            step["step_id"] = self.steps[value]
        # These fixture instructions have only steps_template as nested frames.
        # No arbitrary metadata keys or entity IDs are stripped or rewritten.
        if "pending_request_id" in step:
            step["pending_request_id"] = self.request_id(step["pending_request_id"])
        for index, child in enumerate(step.get("steps_template", [])):
            self.step(
                child, getattr(live, "steps_template", [])[index] if live is not None else None
            )

    def record(self, record, live_steps=None):
        record = copy.deepcopy(record)
        if record["request"] is not None:
            record["request"]["id"] = self.request_id(record["request"]["id"])
        for index, step in enumerate(record["semantic_state"]["execution_stack"]):
            self.step(step, live_steps[index] if live_steps is not None else None)
        return record


def reference_trace(state, operations):
    from state_codec import export_state_with_instances

    from goa2.domain.input import InputResponse
    from goa2.engine import phases
    from goa2.engine.handler import process_stack, submit_input

    objects = (
        export_state_with_instances(state)[1]
        if any(op["kind"] == "commit" for op in operations)
        else ()
    )
    request = None
    result = []
    normalizer = TraceNormalizer()
    for operation in operations:
        current = None
        events = []
        error = None
        if operation["kind"] in ("advance", "advance_prefix"):
            boundary = None
            if operation["kind"] == "advance_prefix":
                remaining = operation["steps"]
                if (
                    isinstance(remaining, bool)
                    or not isinstance(remaining, int)
                    or not 0 <= remaining <= 1000
                ):
                    raise ValueError("invalid diagnostic step prefix")

                def boundary(_state, _step):
                    nonlocal remaining
                    if remaining == 0:
                        return True
                    remaining -= 1
                    return False

            stack_result = process_stack(state, stop_before_step=boundary)
            request = stack_result.input_request
            current = request.model_dump(mode="json") if request else None
            events = [event.model_dump(mode="json") for event in stack_result.events]
        elif operation["kind"] in ("commit", "uncommit", "pass", "finish"):
            try:
                if operation["kind"] == "commit":
                    phases.commit_card(state, operation["hero"], objects[operation["card"]])
                elif operation["kind"] == "uncommit":
                    phases.uncommit_card(state, operation["hero"])
                elif operation["kind"] == "pass":
                    phases.pass_turn(state, operation["hero"])
                else:
                    phases.finish_planning(state, operation["hero"])
            except ValueError:
                error = "invalid_planning_action"
        else:
            request_id = operation.get("request_id", "current")
            if request_id == "current":
                if request is None:
                    raise ValueError("trace input without current request")
                request_id = request.id
            try:
                submit_input(
                    state,
                    InputResponse(request_id=request_id, selection=operation.get("selection")),
                )
            except ValueError:
                error = "invalid_input"
        normalizer.bind_steps(state.execution_stack)
        result.append(
            normalizer.record(
                {
                    "semantic_state": state.model_dump(mode="json"),
                    "request": current,
                    "events": events,
                    "error": error,
                },
                state.execution_stack,
            )
        )
    return result


def corpus():
    from goa2.domain.board import Board
    from goa2.domain.models import GamePhase, TargetType, Team, TeamColor
    from goa2.domain.state import GameState
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import AskConfirmationStep, SelectStep
    from goa2.engine.steps.utility import (
        AddContextValueStep,
        FlipTieBreakerCoinStep,
        ForEachStep,
        RecordTargetStep,
        SetActorStep,
        SetContextFlagStep,
    )

    manifest = verify_sources()
    cases = []
    advance = {"kind": "advance"}

    def answer(value, request_id="current"):
        return {"kind": "input", "request_id": request_id, "selection": value}

    def add(
        name,
        steps,
        operations,
        context=None,
        actor="hero_arien",
        phase=GamePhase.RESOLUTION,
        pieces=False,
        movement=None,
    ):
        state = GameState(
            board=Board(),
            teams={color: Team(color=color, heroes=[], minions=[]) for color in TeamColor},
            phase=phase,
            rules_version=9,
            current_actor_id=actor,
            execution_context=context or {},
        )
        if movement is not None:
            from goa2.domain.hex import Hex
            from goa2.domain.models.unit import Hero
            from goa2.domain.tile import Tile
            from goa2.domain.types import BoardEntityID

            coords = movement.get(
                "hexes",
                [(q, r, -q - r) for q in range(-3, 4) for r in range(-3, 4) if abs(q + r) <= 3],
            )
            tiles = {Hex(q=q, r=r, s=t): Tile(hex=Hex(q=q, r=r, s=t)) for q, r, t in coords}
            state.board = Board(map_id="native_fixture", tiles=tiles)
            for q, r, t in movement.get("terrain", []):
                state.board.tiles[Hex(q=q, r=r, s=t)].is_terrain = True
            for hero_id, color, position in [
                ("hero_arien", TeamColor.RED, (0, 0, 0)),
                ("hero_hanu", TeamColor.BLUE, movement.get("other", (2, -2, 0))),
            ]:
                hero = Hero(id=hero_id, name=hero_id, deck=[], team=color)
                state.teams[color].heroes.append(hero)
                state.place_entity(
                    BoardEntityID(hero_id), Hex(q=position[0], r=position[1], s=position[2])
                )
            if movement.get("ghost"):
                state.place_entity(BoardEntityID("unknown_entity"), Hex(q=-1, r=1, s=0))
        if pieces:
            from goa2.data.heroes.registry import HeroRegistry
            from goa2.domain.models.unit import HeroPiece
            from goa2.engine.setup import GameSetup  # noqa: F401 - registers hero data

            hero = HeroRegistry.get("Razzle")
            hero.team = TeamColor.RED
            state.teams[TeamColor.RED].heroes.append(hero)
            piece = HeroPiece(
                id="hero_razzle_piece_1",
                name="Razzle piece",
                owner_hero_id=hero.id,
                team=TeamColor.RED,
            )
            state.misc_entities[piece.id] = piece
            if movement is not None:
                state.place_entity(piece.id, Hex(q=-2, r=2, s=0))
                state.acting_piece_id = piece.id
        if movement is not None:
            from goa2.domain.models import StatType
            from goa2.domain.models.marker import MarkerType

            for hero_id, items in movement.get("items", {}).items():
                state.get_hero(hero_id).items = {
                    StatType(key): value for key, value in items.items()
                }
            for kind, target, value in movement.get("markers", []):
                state.place_marker(MarkerType(kind), target, value, "hero_hanu")
        # Stable fixture-local step labels: preserve them through all copies.
        counter = 0

        def label(step):
            nonlocal counter
            step.step_id = f"fixture/{name}/{counter}"
            counter += 1
            for nested in getattr(step, "steps_template", []):
                label(nested)

        for step in steps:
            label(step)
        push_steps(state, steps)
        envelope = export_state(state)
        expected = reference_trace(state, operations)
        cases.append(
            {"name": name, "envelope": envelope, "operations": operations, "expected": expected}
        )

    add(
        "number-input-resume",
        [
            SetContextFlagStep(key="enabled", value=True),
            AddContextValueStep(key="count", amount=2),
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Select count",
                number_options=[1, 2, 3],
                number_labels={1: "One", 2: "Two"},
                output_key="count",
            ),
            RecordTargetStep(input_key="count", output_list_key="history"),
            FlipTieBreakerCoinStep(),
            AskConfirmationStep(prompt="Continue?", output_key="confirmed"),
            SetActorStep(actor_id="hero_hanu"),
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Hanu decides",
                number_options=[7, 8],
                output_key="hanu",
            ),
        ],
        [
            advance,
            advance,
            answer(2, "stale"),
            answer("bad"),
            advance,
            answer(" +0_2 "),
            advance,
            answer("YES"),
            advance,
            answer(8),
            advance,
            advance,
        ],
    )
    for value in (None, False, 0, [], {}, "", True):
        add(
            f"condition-{json.dumps(value,separators=(',',':'))}",
            [
                SetContextFlagStep(key="active_ran", active_if_key="condition"),
                SetContextFlagStep(key="skip_ran", skip_if_key="condition"),
                AddContextValueStep(key="n", amount=-2),
            ],
            [advance],
            {"condition": value, "n": False},
        )
    add(
        "optional-skip",
        [
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Optional",
                number_options=[0, 1],
                is_mandatory=False,
                output_key="old",
            ),
            AskConfirmationStep(prompt="No means false"),
        ],
        [advance, answer("SKIP"), advance, answer("anything else"), advance],
        {"old": 17},
    )
    add(
        "auto-and-empty-optional",
        [
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Auto",
                number_options=[9],
                auto_select_if_one=True,
                output_key="auto",
            ),
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Empty",
                number_options=[],
                is_mandatory=False,
                output_key="old",
            ),
            FlipTieBreakerCoinStep(),
        ],
        [advance],
        {"old": None},
    )
    add(
        "mandatory-abort",
        [
            FlipTieBreakerCoinStep(),
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Impossible",
                number_options=[],
                output_key="unchanged",
            ),
            SetContextFlagStep(key="must_not_run"),
        ],
        [advance],
        {"unchanged": 42},
    )
    add(
        "foreach-paused-copies",
        [
            ForEachStep(
                list_key="items",
                item_key="item",
                steps_template=[
                    RecordTargetStep(input_key="item", output_list_key="history"),
                    SelectStep(
                        target_type=TargetType.NUMBER,
                        prompt="Each item",
                        number_options=[1, 2],
                        output_key="choice",
                    ),
                    AddContextValueStep(key="visits"),
                ],
            ),
            FlipTieBreakerCoinStep(),
        ],
        [advance, answer(1), advance, answer(2), advance, answer(1), advance],
        {"items": ["hero_arien", "hero_hanu", "hero_arien"]},
    )
    add(
        "overrides-and-actor-save",
        [
            SetActorStep(actor_id="ignored_literal", actor_key="new_actor", save_key="before"),
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Team owner",
                number_options=[3, 4],
                override_player_id="team:BLUE",
                override_player_id_key="ignored_key",
                decision_purpose="discard",
            ),
            SetActorStep(actor_key="before", save_key="other"),
            AskConfirmationStep(prompt="Literal owner", player_id="hero_brogan"),
        ],
        [advance, answer(3), advance, answer("NO"), advance],
        {"new_actor": "hero_hanu", "ignored_key": "hero_ignored"},
    )
    add(
        "numeric-coercions",
        [
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Float",
                number_options=[-2, 1],
                output_key="float",
            ),
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Bool",
                number_options=[0, 1],
                output_key="boolean",
            ),
        ],
        [advance, answer(-2.9), advance, answer(True), advance],
    )
    add(
        "no-actor",
        [
            AskConfirmationStep(prompt="Silently complete"),
            SelectStep(target_type=TargetType.NUMBER, prompt="None owner", number_options=[1]),
        ],
        [advance, answer(1), advance],
        actor=None,
    )
    add("game-over-no-execution", [FlipTieBreakerCoinStep()], [advance], phase=GamePhase.GAME_OVER)
    add(
        "piece-prompt-owner",
        [
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="Piece owner",
                number_options=[1, 2],
                output_key="piece_choice",
            ),
            AskConfirmationStep(prompt="Piece confirmation", player_id="hero_razzle_piece_1"),
        ],
        [advance, answer(2), advance, answer("YES"), advance],
        actor="hero_razzle_piece_1",
        pieces=True,
    )
    from goa2.domain.hex import Hex
    from goa2.engine.steps import MoveUnitStep

    def h(q, r=None):
        r = -q if r is None else r
        return Hex(q=q, r=r, s=-q - r)

    add(
        "movement-resume-events",
        [
            SelectStep(target_type=TargetType.NUMBER, prompt="Before move", number_options=[1, 2]),
            MoveUnitStep(target_hex_arg=h(1), success_output_key="moved"),
            AskConfirmationStep(prompt="Inspect move"),
            MoveUnitStep(target_hex_arg=h(0)),
            FlipTieBreakerCoinStep(),
        ],
        [advance, answer(1), advance, answer("YES"), advance],
        movement={},
    )
    add(
        "forced-move-context",
        [
            MoveUnitStep(
                unit_key="victim", destination_key="destination", success_output_key="moved"
            )
        ],
        [advance],
        {"victim": "hero_hanu", "destination": h(1).model_dump()},
        movement={},
    )
    for name, destination, steps in [
        ("out-of-range", h(3), 1),
        ("occupied-destination", h(2), 4),
        ("same-zero", h(0), 0),
        ("same-negative", h(0), -1),
    ]:
        add(
            f"movement-{name}",
            [
                MoveUnitStep(
                    target_hex_arg=destination,
                    range_val=steps,
                    success_output_key="success",
                    crossed_obstacles_output_key="crossed",
                ),
                SetContextFlagStep(key="continued"),
            ],
            [advance],
            {"success": "stale", "crossed": ["old"]},
            movement={},
        )
    for through in (False, True):
        add(
            f"movement-straight-through-{through}",
            [
                MoveUnitStep(
                    target_hex_arg=h(3),
                    range_val=5,
                    force_straight_line=True,
                    pass_through_obstacles=through,
                    crossed_obstacles_output_key="crossed",
                    success_output_key="success",
                ),
                SetContextFlagStep(key="continued"),
            ],
            [advance],
            movement={"terrain": [(1, -1, 0)]},
        )
    add(
        "movement-bfs-detour",
        [MoveUnitStep(target_hex_arg=h(3), range_val=5, crossed_obstacles_output_key="crossed")],
        [advance],
        movement={"terrain": [(1, -1, 0)]},
    )
    add(
        "movement-nonstraight",
        [
            MoveUnitStep(target_hex_arg=h(2, -1), range_val=3, force_straight_line=True),
            SetContextFlagStep(key="continued"),
        ],
        [advance],
        movement={},
    )
    add(
        "movement-missing-map-bridge",
        [
            MoveUnitStep(target_hex_arg=h(2), range_val=2, pass_through_obstacles=True),
            SetContextFlagStep(key="continued"),
        ],
        [advance],
        movement={"hexes": [(0, 0, 0), (2, -2, 0), (3, -3, 0)], "other": (3, -3, 0)},
    )
    add(
        "movement-missing-unit",
        [
            MoveUnitStep(
                unit_id="unknown_entity", target_hex_arg=h(1), success_output_key="success"
            ),
            SetContextFlagStep(key="continued"),
        ],
        [advance],
        {"success": True},
        movement={},
    )
    for mandatory in (False, True):
        add(
            f"movement-unregistered-entity-{mandatory}",
            [
                MoveUnitStep(
                    unit_id="unknown_entity", target_hex_arg=h(-2), is_mandatory=mandatory
                ),
                SetContextFlagStep(key="continued"),
            ],
            [advance],
            movement={"ghost": True},
        )
    from goa2.engine.filters_geometry import (
        InStraightLineFilter,
        NotInStraightLineFilter,
        StraightLinePathFilter,
    )
    from goa2.engine.filters_hex import MovementPathFilter, ObstacleFilter, RangeFilter
    from goa2.engine.steps import MoveSequenceStep

    add(
        "hex-input-validation",
        [
            SelectStep(target_type=TargetType.HEX, prompt="Hex", output_key="hex"),
            MoveUnitStep(destination_key="hex", range_val=2),
        ],
        [
            advance,
            answer({"q": 1.5, "r": -1.5, "s": 0}),
            advance,
            answer({"q": "1.0", "r": "-1.00", "s": 0}),
            advance,
        ],
        movement={},
    )
    add(
        "hex-optional-skip",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Optional hex",
                output_key="old",
                is_mandatory=False,
            )
        ],
        [advance, answer("SKIP"), advance],
        {"old": True},
        movement={},
    )
    add(
        "hex-movement-current-and-terrain",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Cross terrain",
                filters=[MovementPathFilter(range_val=3, pass_through_obstacles=True)],
                output_key="cross",
            )
        ],
        [advance, answer(h(2).model_dump()), advance],
        movement={"terrain": [(1, -1, 0)]},
    )
    add(
        "hex-exclude-self",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Self allowed",
                filters=[ObstacleFilter(exclude_id="hero_arien"), MovementPathFilter(range_val=1)],
            )
        ],
        [advance, answer(h(0).model_dump()), advance],
        movement={},
    )
    add(
        "hex-occupied",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Occupied",
                filters=[ObstacleFilter(is_obstacle=True)],
            )
        ],
        [advance, answer(h(2).model_dump()), advance],
        movement={},
    )
    add(
        "hex-range-context",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Range keys",
                filters=[
                    RangeFilter(origin_hex_key="origin", min_range_key="min", max_range_key="max")
                ],
            )
        ],
        [advance, answer(h(1).model_dump()), advance],
        {"origin": h(0).model_dump(), "min": True, "max": 1},
        movement={},
    )
    add(
        "hex-origin-context",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Other origin",
                filters=[RangeFilter(origin_key="other", min_range=1, max_range=1)],
            )
        ],
        [advance, answer(h(1).model_dump()), advance],
        {"other": "hero_hanu"},
        movement={},
    )
    for inverted, filter_type in [(False, InStraightLineFilter), (True, NotInStraightLineFilter)]:
        destination = h(1, -1) if not inverted else h(2, -1)
        add(
            f"hex-straight-{inverted}",
            [
                SelectStep(
                    target_type=TargetType.HEX,
                    prompt="Line",
                    filters=[filter_type(), RangeFilter(max_range=2)],
                )
            ],
            [advance, answer(destination.model_dump()), advance],
            movement={},
        )
    add(
        "hex-clear-line",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Clear path",
                filters=[StraightLinePathFilter(), RangeFilter(max_range=3)],
            )
        ],
        [advance, answer(h(3).model_dump()), advance, answer(h(-1).model_dump()), advance],
        movement={"terrain": [(1, -1, 0)]},
    )
    add(
        "hex-no-origin-abort",
        [
            SelectStep(
                target_type=TargetType.HEX, prompt="No origin", filters=[RangeFilter(max_range=1)]
            ),
            SetContextFlagStep(key="must_not_run"),
        ],
        [advance],
        actor=None,
        movement={},
    )
    add(
        "hex-auto-single",
        [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Only origin",
                filters=[MovementPathFilter(range_val=0)],
                auto_select_if_one=True,
            )
        ],
        [advance],
        movement={},
    )
    add(
        "macro-default",
        [MoveSequenceStep(range_val=1), FlipTieBreakerCoinStep()],
        [advance, advance, answer(h(1).model_dump()), advance],
        movement={},
    )
    add(
        "macro-two-moves-fresh-scan",
        [
            MoveSequenceStep(range_val=1),
            SetContextFlagStep(key="target_hex", value=None),
            MoveSequenceStep(range_val=1),
        ],
        [advance, answer(h(1).model_dump()), advance, answer(h(1, 0).model_dump()), advance],
        movement={},
    )
    add(
        "macro-full-range",
        [MoveSequenceStep(range_val=2, force_full_distance=True)],
        [advance, answer(h(0, 2).model_dump()), advance],
        movement={},
    )
    add(
        "macro-other-unit-full-range",
        [MoveSequenceStep(unit_id="hero_hanu", range_val=2, force_full_distance=True)],
        [advance, answer(h(2, 0).model_dump()), advance],
        movement={},
    )
    add(
        "macro-cross-obstacles-straight",
        [MoveSequenceStep(range_val=3, pass_through_obstacles=True, force_straight_line=True)],
        [advance, answer(h(3).model_dump()), advance],
        movement={"terrain": [(1, -1, 0)]},
    )
    add(
        "macro-zero-range-still-prompts",
        [MoveSequenceStep(range_val=0)],
        [advance, answer(h(0).model_dump()), advance],
        movement={},
    )
    add(
        "macro-preset-destination",
        [MoveSequenceStep(range_val=1)],
        [advance],
        {"target_hex": h(1).model_dump()},
        movement={},
    )
    add(
        "macro-actor-switch",
        [SetActorStep(actor_id="hero_hanu"), MoveSequenceStep(range_val=1)],
        [advance, answer(h(1).model_dump()), advance],
        movement={},
    )
    add(
        "macro-no-actor",
        [MoveSequenceStep(range_val=1), SetContextFlagStep(key="must_not_run")],
        [advance],
        actor=None,
        movement={},
    )
    add(
        "macro-optional-skip",
        [MoveSequenceStep(range_val=1, is_mandatory=False), SetContextFlagStep(key="continued")],
        [advance, answer("SKIP"), advance],
        movement={},
    )
    add(
        "macro-bound-razzle-piece",
        [MoveSequenceStep(range_val=1)],
        [advance, answer(h(-1).model_dump()), advance],
        actor="hero_razzle",
        pieces=True,
        movement={},
    )
    from goa2.domain.models import StatType

    add(
        "macro-computed-items",
        [MoveSequenceStep(range_val=1, range_stat_type=StatType.MOVEMENT)],
        [advance, answer(h(0, 3).model_dump()), advance],
        movement={"items": {"hero_arien": {"MOVEMENT": 2}}},
    )
    add(
        "macro-computed-actor-switch",
        [
            SetActorStep(actor_id="hero_hanu"),
            MoveSequenceStep(range_val=1, range_stat_type=StatType.MOVEMENT),
        ],
        [advance, answer(h(1, 0).model_dump()), advance],
        movement={"items": {"hero_arien": {"MOVEMENT": 4}, "hero_hanu": {"MOVEMENT": 1}}},
    )
    add(
        "macro-computed-marker-stat",
        [MoveSequenceStep(range_val=3, range_stat_type=StatType.INITIATIVE)],
        [advance, answer(h(0, 3).model_dump()), advance],
        movement={
            "items": {"hero_arien": {"INITIATIVE": 2}},
            "markers": [("poison", "hero_arien", -2)],
        },
    )
    add(
        "macro-computed-negative",
        [MoveSequenceStep(range_val=1, range_stat_type=StatType.MOVEMENT)],
        [advance, answer(h(0).model_dump()), advance],
        movement={"items": {"hero_arien": {"MOVEMENT": -3}}},
    )
    add(
        "macro-computed-full-distance",
        [
            MoveSequenceStep(
                range_val=1, range_stat_type=StatType.MOVEMENT, force_full_distance=True
            )
        ],
        [advance, answer(h(0, 3).model_dump()), advance],
        movement={"items": {"hero_arien": {"MOVEMENT": 2}}},
    )
    add(
        "macro-computed-razzle-owner-items",
        [MoveSequenceStep(range_val=1, range_stat_type=StatType.MOVEMENT)],
        [advance, answer(h(1, 0).model_dump()), advance],
        actor="hero_razzle",
        pieces=True,
        movement={"items": {"hero_razzle": {"MOVEMENT": 2}}},
    )
    # Isolate the real revelation retrieval instruction on a registered game.
    # Full resolution/card effects are intentionally not replaced with test stubs.
    from model_traces import setup

    from goa2.engine import phases
    from goa2.engine.steps import RetrieveUnresolvedCardStep

    for variant in (
        "first",
        "second",
        "invalid",
        "pending",
        "missing_first",
        "missing_extra",
        "unknown",
        "conditions",
        "override",
        "physical_alias",
        "restored_request",
    ):
        state = setup(["Emmitt", "Arien"])
        emmitt = state.get_hero("hero_emmitt")
        emmitt.level = 8
        first, second = emmitt.hand[:2]
        phases.commit_card(state, emmitt.id, first)
        phases.commit_card(state, emmitt.id, second)
        for team in state.teams.values():
            for hero in team.heroes:
                if hero.id != emmitt.id:
                    phases.commit_card(state, hero.id, hero.hand[0])
        state.execution_stack = []
        step = RetrieveUnresolvedCardStep(step_id="fixture/retrieve", hero_id=str(emmitt.id))
        operations = [advance, answer(str(first.id if variant == "first" else second.id)), advance]
        if variant == "invalid":
            operations = [
                advance,
                answer("SKIP"),
                advance,
                answer(None),
                advance,
                answer(True),
                advance,
                answer("not_a_card"),
                advance,
                answer(str(first.id)),
                advance,
            ]
        elif variant == "pending":
            step.pending_input = {"selection": str(first.id)}
            operations = [advance]
        elif variant == "missing_first":
            emmitt.current_turn_card = None
            operations = [advance]
        elif variant == "missing_extra":
            emmitt.extra_turn_card = None
            operations = [advance]
        elif variant == "unknown":
            step.hero_id = "unknown"
            operations = [advance]
        elif variant == "conditions":
            step.active_if_key = "absent"
            step.skip_if_key = "exists"
            state.execution_context["exists"] = True
            operations = [advance]
        elif variant == "physical_alias":
            emmitt.extra_turn_card = first
            operations = [advance, answer(str(first.id)), advance]
        elif variant == "restored_request":
            step.pending_request_id = "fixture/restore-retrieval"
            operations = [answer(str(first.id), "fixture/restore-retrieval"), advance]
        elif variant == "override":
            first.initiative_override = -7
            second.initiative_override = 31
            first.is_facedown = True
        push_steps(state, [step])
        envelope = export_state(state)
        cases.append(
            {
                "name": f"retrieve-unresolved-{variant}",
                "envelope": envelope,
                "operations": operations,
                "expected": reference_trace(state, operations),
            }
        )

    from unit_traces import cases as unit_cases

    cases.extend(unit_cases(advance, answer))

    from travel_traces import cases as travel_cases

    cases.extend(travel_cases(advance, answer))

    from combat_traces import cases as combat_cases

    cases.extend(combat_cases(advance, answer))

    from reaction_traces import cases as reaction_cases

    cases.extend(reaction_cases(advance, answer))

    from attack_traces import cases as attack_cases

    cases.extend(attack_cases(advance, answer))

    from defense_effect_traces import cases as defense_effect_cases

    cases.extend(defense_effect_cases(advance, answer))

    from card_selection_traces import cases as card_selection_cases

    cases.extend(card_selection_cases(advance, answer))

    from action_execution_traces import cases as action_execution_cases

    cases.extend(action_execution_cases(advance, answer))

    from primary_traces import cases as primary_cases

    cases.extend(primary_cases(advance, answer))

    from push_traces import cases as push_cases

    cases.extend(push_cases(advance, answer))

    from minion_return_traces import cases as minion_return_cases

    cases.extend(minion_return_cases(advance, answer))

    from battle_traces import cases as battle_cases

    cases.extend(battle_cases(advance, answer))

    from turn_traces import cases as turn_cases

    cases.extend(turn_cases(advance, answer))

    from round_traces import cases as round_cases

    cases.extend(round_cases(advance, answer))

    from game_traces import cases as game_cases

    cases.extend(game_cases(advance, answer))

    from aura_traces import cases as aura_cases

    cases.extend(aura_cases(advance, answer))

    from conditional_aura_traces import cases as conditional_cases

    cases.extend(conditional_cases(advance, answer))

    from discard_or_defeat_traces import cases as discard_or_defeat_cases

    cases.extend(discard_or_defeat_cases(advance, answer))

    from passive_traces import cases as passive_cases

    cases.extend(passive_cases(advance, answer))

    from repeat_traces import cases as repeat_cases

    cases.extend(repeat_cases(advance, answer))

    from query_traces import cases as query_cases

    cases.extend(query_cases(advance, answer))

    from conditional_primary_traces import cases as conditional_primary_cases

    cases.extend(conditional_primary_cases(advance, answer))

    from retrieval_primary_traces import cases as retrieval_primary_cases
    from retrieval_traces import cases as retrieval_cases

    cases.extend(retrieval_cases(advance, answer))
    cases.extend(retrieval_primary_cases(advance, answer))

    from effect_step_traces import cases as effect_step_cases

    cases.extend(effect_step_cases(advance, answer))

    payload = {
        "schema_version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "cases": cases,
    }
    return {**payload, "corpus_sha256": digest(payload)}


def write_corpus(path, source):
    """Store complete snapshots separately; the small index checks every case."""
    source = dict(source)
    semantic_identity = source.pop("corpus_sha256")
    if source.get("schema_version") != 1 or digest(source) != semantic_identity:
        raise ValueError("invalid full trace corpus identity")
    directory = path.parent / path.stem
    directory.mkdir(parents=True, exist_ok=True)
    entries = []
    for case in source.pop("cases"):
        identity = digest(case)
        relative = f"{path.stem}/{identity}.json"
        target = path.parent / relative
        temporary = target.with_suffix(".json.tmp")
        temporary.unlink(missing_ok=True)
        temporary.write_text(json.dumps(case, separators=(",", ":")) + "\n")
        temporary.replace(target)
        entries.append({"name": case["name"], "path": relative, "case_sha256": identity})
    source["schema_version"] = 2
    source["semantic_sha256"] = semantic_identity
    source["cases"] = entries
    return _install_index(path, source)


def _install_index(path, source):
    directory = path.parent / path.stem
    entries = source["cases"]
    identity = digest(source)
    result = {**source, "corpus_sha256": identity}
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(result, indent=2) + "\n")
    temporary.replace(path)
    # This directory contains generated, content-addressed snapshots. Prune
    # superseded hashes only after the new index is safely in place.
    if not directory.is_symlink() and directory.resolve().parent == path.parent.resolve():
        keep = {Path(entry["path"]).name for entry in entries}
        for old in directory.glob("*.json"):
            if (
                old.name not in keep
                and len(old.stem) == 64
                and all(c in "0123456789abcdef" for c in old.stem)
            ):
                old.unlink()
    return result


def read_index(path):
    source = json.loads(path.read_text())
    identity = source.pop("corpus_sha256")
    if source.get("schema_version") != 2 or digest(source) != identity:
        raise ValueError("trace index identity mismatch")
    names = [entry["name"] for entry in source["cases"]]
    if len(names) != len(set(names)):
        raise ValueError("duplicate trace case identity")
    return source, identity


def read_case(path, entry):
    identity = entry["case_sha256"]
    if len(identity) != 64 or any(c not in "0123456789abcdef" for c in identity):
        raise ValueError("invalid trace case identity")
    relative = f"{path.stem}/{identity}.json"
    if entry["path"] != relative:
        raise ValueError("trace case path mismatch")
    case = json.loads((path.parent / relative).read_text())
    if digest(case) != identity or case["name"] != entry["name"]:
        raise ValueError("trace case content identity mismatch")
    return case


def merge_corpora(path, additions_path):
    """Replace/append case names while holding only one complete snapshot case.

    Recompute the exact schema-1 semantic digest incrementally, then atomically
    install the schema-2 index. Existing cases are verified, never rewritten.
    """
    old, _ = read_index(path)
    new, _ = read_index(additions_path)

    def header(index):
        return {
            key: 1 if key == "schema_version" else value
            for key, value in index.items()
            if key not in ("semantic_sha256", "cases")
        }

    metadata = header(old)
    if canonical(metadata) != canonical(header(new)):
        raise ValueError("incompatible trace corpus metadata")
    names = {entry["name"] for entry in new["cases"]}
    sources = [(path, entry) for entry in old["cases"] if entry["name"] not in names]
    sources.extend((additions_path, entry) for entry in new["cases"])
    hasher = hashlib.sha256()
    hasher.update(b"{")
    for index, (key, value) in enumerate(metadata.items()):
        if index:
            hasher.update(b",")
        hasher.update((canonical(key) + ":" + canonical(value)).encode())
    hasher.update(b',"cases":[')
    directory = path.parent / path.stem
    directory.mkdir(parents=True, exist_ok=True)
    entries = []
    for index, (source_path, entry) in enumerate(sources):
        case = read_case(source_path, entry)
        if index:
            hasher.update(b",")
        hasher.update(canonical(case).encode())
        relative = f"{path.stem}/{entry['case_sha256']}.json"
        if source_path != path:
            target = path.parent / relative
            temporary = target.with_suffix(".json.tmp")
            # A failed prior replacement may have left a hard-linked temporary.
            # Remove that directory entry before creating or writing a new file.
            temporary.unlink(missing_ok=True)
            # Content-addressed cases are immutable. Link verified same-volume
            # snapshots to avoid duplicating large full-round records. Atomic
            # replacement on export prevents later writes touching a shared inode.
            try:
                os.link(source_path.parent / entry["path"], temporary)
            except OSError:
                temporary.write_text(json.dumps(case, separators=(",", ":")) + "\n")
            temporary.replace(target)
        entries.append(
            {"name": entry["name"], "path": relative, "case_sha256": entry["case_sha256"]}
        )
    hasher.update(b"]}")
    metadata["schema_version"] = 2
    return _install_index(
        path, {**metadata, "semantic_sha256": hasher.hexdigest(), "cases": entries}
    )


def verify(path, command):
    manifest = verify_sources()
    source, identity = read_index(path)
    if source["source_sha256"] != manifest["source_sha256"]:
        raise ValueError("trace corpus source/identity mismatch")
    worker = Worker(command)
    count = 0
    try:
        for entry in source["cases"]:
            case = read_case(path, entry)
            actual = worker.request(
                "stack_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            normalizer = TraceNormalizer()
            actual = [normalizer.record(record) for record in actual]
            if actual != case["expected"]:
                failure = ROOT / "rust/artifacts/stack-trace-mismatch.json"
                failure.parent.mkdir(parents=True, exist_ok=True)
                failure.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError(f"trace mismatch in {case['name']}: {failure}")
            count += len(actual)
    finally:
        worker.close()
    return {
        "cases": len(source["cases"]),
        "operations": count,
        "corpus_sha256": identity,
        "state_request_event_parity": True,
    }


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/tests/fixtures/stack-traces.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        source = corpus()
        index = write_corpus(args.corpus, source)
        print(json.dumps({"cases": len(index["cases"]), "corpus_sha256": index["corpus_sha256"]}))
    else:
        command = args.worker
        if command and command[0] == "--":
            command = command[1:]
        if not command:
            parser.error("worker command required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))


if __name__ == "__main__":
    sys.path.insert(0, str(ROOT / "src"))
    main()
