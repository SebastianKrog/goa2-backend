"""Real after-action minion return traces, including lane and choice continuations."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer):
    import copy

    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.domain.board import Board, Zone
    from goa2.domain.hex import Hex
    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import GamePhase, TeamColor
    from goa2.domain.tile import Tile
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import ReturnMinionToZoneStep, SetContextFlagStep

    result = []

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def build():
        state = setup()
        for uid in list(state.entity_locations):
            state.remove_entity(uid)
        hexes = [h(q, r) for q in range(-3, 4) for r in range(-3, 4) if abs(q + r) <= 3]
        zone = {h(0, 0), h(0, 1), h(0, -1)}
        state.board = Board(
            map_id="return_fixture",
            tiles={p: Tile(hex=p) for p in hexes},
            zones={"battle": Zone(id="battle", hexes=zone)},
        )
        state.battle_zones = {"lane_1": "battle"}
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        state.execution_context = {"keep": True, "target_hex": h(-3).model_dump()}
        state.tie_breaker_team = TeamColor.RED
        red = state.teams[TeamColor.RED].minions[0]
        blue = state.teams[TeamColor.BLUE].minions[0]
        return state, red, blue

    def add(name, state, processed=(), invalid=None, last=False, prefix=False):
        step = ReturnMinionToZoneStep(processed_minion_ids=list(processed))
        steps = [step, SetContextFlagStep(key="return_done", value=True)]
        for i, frame in enumerate(steps):
            frame.step_id = f"fixture/return/{name}/{i}"
        push_steps(state, steps)
        initial = export_state(state)
        work = copy.deepcopy(state)
        operations = []
        if prefix:
            operations.append({"kind": "advance_prefix", "steps": 1})
        # Drive the real handler independently to capture all prompt choices.
        # Prefix cases capture an expansion separately, then finish it as normal.
        stack = process_stack(work)
        operations.append(advance)
        if invalid is not None and stack.input_request:
            submit_input(work, InputResponse(request_id=stack.input_request.id, selection=invalid))
            operations.extend([answer(invalid), advance])
            stack = process_stack(work)
        count = 0
        while stack.input_request:
            count += 1
            assert count <= 20, name
            req = stack.input_request
            value = selection_value(req.options[-1 if last else 0])
            operations.extend([answer(value), advance])
            submit_input(work, InputResponse(request_id=req.id, selection=value))
            stack = process_stack(work)
        assert work.execution_context.get("return_done"), name
        moved = [
            m
            for team in work.teams.values()
            for m in team.minions
            if work.get_position(m.id) != state.get_position(m.id)
        ]
        no_move = {
            "no-space",
            "disconnected",
            "inside",
            "offboard",
            "missing-zone",
            "missing-lane",
            "no-team",
            "processed",
        }
        assert bool(moved) == (name not in no_move), name
        for minion in moved:
            zone = work.board.zones[work.battle_zones[minion.lane_id]]
            assert work.get_position(minion.id) in zone.hexes, name
        expected = reference_trace(state, operations)
        result.append(
            {
                "name": f"return-{name}",
                "envelope": initial,
                "operations": operations,
                "expected": expected,
            }
        )

    variants = [
        "walk",
        "choice",
        "choice-last",
        "detour",
        "terrain-fallback",
        "occupied-fallback",
        "no-space",
        "disconnected",
        "inside",
        "offboard",
        "missing-zone",
        "missing-lane",
        "no-team",
        "processed",
        "two-teams",
        "blue-first",
        "two-red",
        "processed-first",
        "no-actor",
        "prefix",
        "prefix-fallback",
    ]
    for variant in variants:
        state, red, blue = build()
        state.place_entity(red.id, h(2))
        processed = []
        if variant in (
            "walk",
            "detour",
            "terrain-fallback",
            "occupied-fallback",
            "prefix-fallback",
        ):
            state.board.zones["battle"].hexes = {h(0)}
        if variant == "detour":
            state.board.tiles[h(1)].is_terrain = True
        if variant in ("terrain-fallback", "occupied-fallback", "prefix-fallback", "no-space"):
            for point in h(2).neighbors():
                if point not in state.board.tiles:
                    continue
                if variant == "occupied-fallback":
                    units = [hero.id for team in state.teams.values() for hero in team.heroes]
                    # Remaining neighbors use terrain; at least one unit blocks the walk.
                    if point == h(1):
                        state.place_entity(units[0], point)
                    else:
                        state.board.tiles[point].is_terrain = True
                else:
                    state.board.tiles[point].is_terrain = True
        if variant == "no-space":
            for point in state.board.zones["battle"].hexes:
                state.board.tiles[point].is_terrain = True
        if variant == "disconnected":
            state.board.tiles = {h(2): Tile(hex=h(2)), h(0): Tile(hex=h(0))}
        if variant == "inside":
            state.move_unit(red.id, h(0))
        if variant == "offboard":
            state.remove_entity(red.id)
        if variant == "missing-zone":
            state.board.zones = {}
        if variant == "missing-lane":
            red.lane_id = "missing"
        if variant == "no-team":
            red.team = None
        if variant == "processed":
            processed = [str(red.id)]
        if variant in ("two-teams", "blue-first", "processed-first"):
            state.place_entity(blue.id, h(-2))
            if variant == "blue-first":
                state.tie_breaker_team = TeamColor.BLUE
            if variant == "processed-first":
                processed = [str(red.id)]
        if variant == "two-red":
            state.place_entity(state.teams[TeamColor.RED].minions[1].id, h(-2))
        if variant == "no-actor":
            state.current_actor_id = None
        add(
            variant,
            state,
            processed,
            last=variant == "choice-last",
            prefix=variant.startswith("prefix"),
        )

    for i, invalid in enumerate(
        [
            "SKIP",
            "(0, 0, 0)",
            [],
            False,
            {},
            {"q": 0, "r": 0, "s": 1},
            {"q": 40, "r": -40, "s": 0},
            {"q": 0.5, "r": 0, "s": -0.5},
        ]
    ):
        state, red, _ = build()
        state.place_entity(red.id, h(2))
        add(f"invalid-{i}", state, invalid=invalid)

    state, red, blue = build()
    for point in state.board.zones["battle"].hexes:
        state.board.tiles[point].is_terrain = True
    state.board.zones["other"] = Zone(id="other", hexes={h(0, 2)})
    state.battle_zones["lane_2"] = "other"
    blue.lane_id = "lane_2"
    state.place_entity(red.id, h(2))
    state.place_entity(blue.id, h(-2))
    add("blocked-then-legal", state, prefix=True)

    from oracle import ROOT

    from goa2.engine.setup import GameSetup

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        for uid in list(state.entity_locations):
            state.remove_entity(uid)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        for team in state.teams.values():
            for lane, zone_id in state.battle_zones.items():
                minion = next(m for m in team.minions if m.lane_id == lane)
                outside = next(
                    p
                    for p, t in state.board.tiles.items()
                    if p not in state.board.zones[zone_id].hexes and not t.is_obstacle
                )
                state.place_entity(minion.id, outside)
        add(f"map-{path.stem}", state)
    return result
