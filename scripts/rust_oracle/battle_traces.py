"""Full real-handler minion battle, push and displacement transitions."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer, *, effect_factory=None, live_sink=None):
    import copy

    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.domain.board import Board, Zone
    from goa2.domain.hex import Hex
    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import GamePhase, MinionType, TeamColor, Token, TokenType
    from goa2.domain.models.spawn import SpawnPoint, SpawnType
    from goa2.domain.tile import Tile
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import (
        CheckLanePushStep,
        ChooseMinionRemovalStep,
        LanePushStep,
        MinionBattleStep,
        ResolveDisplacementStep,
        SetContextFlagStep,
    )

    result = []

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def build(multi=False):
        state = setup()
        for uid in list(state.entity_locations):
            state.remove_entity(uid)
        tiles = {h(q, r): Tile(hex=h(q, r)) for q in range(-12, 13) for r in range(-2, 9)}
        zones = {}
        lanes = {}
        for row in range(2 if multi else 1):
            lane = f"lane_{row+1}"
            lanes[lane] = []
            for i, q in enumerate([-10, -5, 0, 5, 10]):
                name = f"{lane}-{i}"
                lanes[lane].append(name)
                points = {
                    h(q + dq, row * 6 + dr)
                    for dq in range(-2, 3)
                    for dr in range(-2, 3)
                    if abs(dq + dr) <= 2
                }
                spawns = []
                for team, offset in [(TeamColor.RED, -1), (TeamColor.BLUE, 1)]:
                    for dr in [-1, 0, 1]:
                        spawns.append(
                            SpawnPoint(
                                location=h(q + offset, row * 6 + dr),
                                team=team,
                                type=SpawnType.MINION,
                                minion_type=MinionType.MELEE,
                            )
                        )
                zones[name] = Zone(id=name, hexes=points, spawn_points=spawns)
                for point in points:
                    tiles[point].zone_id = name
        state.board = Board(
            map_id="battle_fixture",
            tiles=tiles,
            zones=zones,
            lanes=lanes,
            spawn_points=[sp for z in zones.values() for sp in z.spawn_points],
        )
        state.battle_zones = {lane: ids[2] for lane, ids in lanes.items()}
        state.wave_counters = {lane: 5 for lane in lanes}
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        state.tie_breaker_team = TeamColor.RED
        state.execution_context = {"keep": True}
        if multi:
            for team in state.teams.values():
                for i, m in enumerate(team.minions):
                    m.lane_id = "lane_1" if i < 3 else "lane_2"
        return state

    def populate(state, red=1, blue=0, lane="lane_1"):
        zone = state.board.zones[state.battle_zones[lane]]
        available = [
            p
            for p in state.board.tiles
            if p in zone.hexes and not state.board.get_tile(p).is_occupied
        ]
        for team, n in [(TeamColor.RED, red), (TeamColor.BLUE, blue)]:
            for m in [m for m in state.teams[team].minions if m.lane_id == lane][:n]:
                state.place_entity(m.id, available.pop(0))

    def add(name, state, step, invalid=None, prefix=False, winner=None, condition=None):
        if effect_factory is not None:
            effect_factory(state, name)
        steps = [step, SetContextFlagStep(key="battle_done", value=True)]
        for i, step in enumerate(steps):
            step.step_id = f"fixture/battle/{name}/{i}"
        push_steps(state, steps)
        initial = export_state(state)
        work = copy.deepcopy(state)
        operations = [{"kind": "advance_prefix", "steps": 1}] if prefix else []
        operations.append(advance)
        outcome = process_stack(work)
        if invalid is not None and outcome.input_request:
            operations.extend([answer(invalid), advance])
            submit_input(
                work, InputResponse(request_id=outcome.input_request.id, selection=invalid)
            )
            outcome = process_stack(work)
        count = 0
        while outcome.input_request:
            count += 1
            assert count <= 60, name
            request = outcome.input_request
            value = selection_value(request.options[0])
            operations.extend([answer(value), advance])
            submit_input(work, InputResponse(request_id=request.id, selection=value))
            outcome = process_stack(work)
        assert work.phase == GamePhase.GAME_OVER or work.execution_context.get("battle_done"), name
        if winner is not None:
            assert str(work.winner) == winner, (name, work.winner)
        if condition is not None:
            assert work.victory_condition == condition, (name, work.victory_condition)
        if live_sink is not None:
            live_sink(name, state, operations)
        else:
            expected = reference_trace(state, operations)
            result.append(
                {
                    "name": f"battle-{name}",
                    "envelope": initial,
                    "operations": operations,
                    "expected": expected,
                }
            )
        return work

    for name, red, blue in [
        ("empty", 0, 0),
        ("tied", 2, 2),
        ("red-push", 1, 0),
        ("blue-push", 0, 1),
    ]:
        state = build()
        populate(state, red, blue)
        final = add(name, state, CheckLanePushStep(), prefix=True)
        if red != blue:
            assert final.battle_zones["lane_1"] == ("lane_1-3" if red > blue else "lane_1-1")
    for team in [TeamColor.RED, TeamColor.BLUE]:
        for waves in [1, 0, -2, 4]:
            state = build()
            state.wave_counters["lane_1"] = waves
            if waves == 4:
                state.battle_zones["lane_1"] = "lane_1-1" if team == TeamColor.RED else "lane_1-3"
            add(
                f"direct-victory-{team}-{waves}",
                state,
                LanePushStep(losing_team=team),
                prefix=True,
                winner="BLUE" if team == TeamColor.RED else "RED",
                condition="LANE_PUSH" if waves == 4 else "LAST_PUSH",
            )
    for variant in [
        "skip",
        "explicit",
        "empty-target",
        "unknown-target",
        "missing-lane",
        "missing-old",
        "missing-path",
        "empty-path",
    ]:
        state = build()
        populate(state, 1, 1)
        step = LanePushStep(losing_team=TeamColor.BLUE)
        if variant == "skip":
            step.skip_wave_counter = True
        if variant == "explicit":
            step.target_zone_id = "lane_1-3"
        if variant == "empty-target":
            step.target_zone_id = ""
        if variant == "unknown-target":
            step.target_zone_id = "unknown"
        if variant == "missing-lane":
            step.lane_id = "missing"
        if variant == "missing-old":
            step.skip_wave_counter = True
            state.battle_zones = {}
        if variant == "missing-path":
            state.board.lanes = {}
        if variant == "empty-path":
            state.board.lanes = {"lane_1": []}
        add(f"direct-{variant}", state, step, prefix=True)
    for variant in [
        "same-direction",
        "opposite",
        "last-red",
        "last-blue",
        "last-tie",
        "throne-red",
        "throne-blue",
        "both-thrones",
        "one-lane",
        "already-zero",
        "missing-target",
    ]:
        state = build(True)
        if variant in ["throne-red", "both-thrones"]:
            state.battle_zones["lane_1"] = "lane_1-3"
        if variant in ["throne-blue", "both-thrones"]:
            state.battle_zones["lane_2"] = "lane_2-1"
        populate(state, 1, 0, "lane_1")
        populate(
            state,
            0 if variant in ["opposite", "last-tie", "throne-blue", "both-thrones"] else 1,
            (
                0
                if variant == "one-lane"
                else (
                    1 if variant in ["opposite", "last-tie", "throne-blue", "both-thrones"] else 0
                )
            ),
            "lane_2",
        )
        if variant == "one-lane":
            # Other lane remains a tied empty zone.
            for m in state.teams[TeamColor.RED].minions:
                if m.lane_id == "lane_2":
                    state.remove_entity(m.id)
        if variant.startswith("last") or variant == "already-zero":
            state.wave_counters = {"lane_1": 0 if variant == "already-zero" else 1, "lane_2": 1}
        if variant == "last-blue":
            for uid in list(state.entity_locations):
                state.remove_entity(uid)
            populate(state, 0, 1, "lane_1")
            populate(state, 0, 1, "lane_2")
        if variant == "missing-target":
            state.board.lanes["lane_1"] = []
        initial_zones = dict(state.battle_zones)
        final = add(f"multi-{variant}", state, CheckLanePushStep(), prefix=True)
        victories = {
            "last-red": ("RED", "LAST_PUSH"),
            "last-blue": ("BLUE", "LAST_PUSH"),
            "throne-red": ("RED", "LANE_PUSH"),
            "throne-blue": ("BLUE", "LANE_PUSH"),
            "already-zero": ("RED", "LAST_PUSH"),
        }
        if variant in victories:
            winner, condition = victories[variant]
            assert str(final.winner) == winner and final.victory_condition == condition, variant
        if variant in ["both-thrones", "last-tie"]:
            assert final.winner is None and final.battle_zones == initial_zones, variant
            assert all(
                n == (0 if variant == "last-tie" else 4) for n in final.wave_counters.values()
            ), variant
        if variant == "same-direction":
            assert final.battle_zones == {"lane_1": "lane_1-3", "lane_2": "lane_2-3"}
            assert list(final.wave_counters.values()) == [4, 4]
    for variant in [
        "blocked-one",
        "blocked-two",
        "blocked-token",
        "terrain-spawn",
        "supply-short",
        "wrong-lane",
    ]:
        state = build()
        target = "lane_1-3"
        spawns = state.board.zones[target].spawn_points
        if variant.startswith("blocked"):
            for hero, sp in zip(
                ["hero_arien", "hero_hanu"],
                spawns[: 2 if variant == "blocked-two" else 1],
                strict=False,
            ):
                state.place_entity(hero, sp.location)
        if variant == "blocked-token":
            state.remove_entity("hero_arien")
            token = Token(
                id="battle_token",
                name="Battle token",
                team=TeamColor.RED,
                owner_id="hero_arien",
                token_type=TokenType.TOTEM,
            )
            state.misc_entities[token.id] = token
            state.place_entity(token.id, spawns[0].location)
        if variant == "terrain-spawn":
            state.board.tiles[spawns[0].location].is_terrain = True
        if variant == "supply-short":
            for m in state.teams[TeamColor.RED].minions[1:]:
                m.type = MinionType.HEAVY
        if variant == "wrong-lane":
            for m in state.teams[TeamColor.RED].minions:
                m.lane_id = "other"
        final = add(
            f"spawn-{variant}",
            state,
            LanePushStep(losing_team=TeamColor.BLUE),
            invalid="SKIP",
            prefix=True,
        )
        red_spawned = [
            m for m in final.teams[TeamColor.RED].minions if final.get_position(m.id) is not None
        ]
        assert len(red_spawned) == (
            1 if variant == "supply-short" else 0 if variant == "wrong-lane" else 3
        ), variant
        if variant == "blocked-token":
            assert final.get_position("battle_token") is None
    for variant in [
        "empty",
        "one-choice",
        "two-red",
        "two-teams",
        "blue-first",
        "same-origin",
        "unknown",
        "missing-zone",
        "missing-lane",
        "no-space",
        "origin-empty",
    ]:
        state = build(True)
        red = state.teams[TeamColor.RED].minions
        blue = state.teams[TeamColor.BLUE].minions
        origin = h(0)
        items = [(str(red[0].id), origin)]
        state.place_entity("hero_arien", origin)
        if variant in ["two-red", "same-origin"]:
            items.append((str(red[1].id), origin if variant == "same-origin" else h(1)))
        if variant in ["two-teams", "blue-first", "missing-lane", "no-space"]:
            items.append((str(blue[0].id), h(1)))
        if variant == "empty":
            items = []
        if variant == "unknown":
            items = [("missing", origin)]
        if variant == "blue-first":
            state.tie_breaker_team = TeamColor.BLUE
        if variant == "missing-zone":
            state.board.zones = {}
        if variant == "missing-lane":
            red[0].lane_id = "missing"
        if variant == "origin-empty":
            state.remove_entity("hero_arien")
        if variant == "one-choice":
            state.board.zones["lane_1-2"].hexes = {h(1)}
        if variant == "no-space":
            for p in state.board.zones["lane_1-2"].hexes:
                state.board.tiles[p].is_terrain = True
            blue[0].lane_id = "lane_2"
            items[-1] = (str(blue[0].id), h(0, 6))
        add(
            f"displace-{variant}",
            state,
            ResolveDisplacementStep(displacements=items),
            invalid="bad",
            prefix=True,
        )
    for variant in [
        "choice",
        "invalid-heavy",
        "force-all",
        "force-over",
        "zero",
        "negative",
        "missing-zone",
        "missing-team",
        "heavies",
        "reverse-order",
    ]:
        state = build()
        populate(state, 4, 0)
        red = state.teams[TeamColor.RED].minions
        red[3].type = MinionType.HEAVY
        step = ChooseMinionRemovalStep(losing_team="RED", remaining_to_remove=1, zone_id="lane_1-2")
        if variant == "force-all":
            step.remaining_to_remove = 4
        if variant == "force-over":
            step.remaining_to_remove = 99
        if variant == "zero":
            step.remaining_to_remove = 0
        if variant == "negative":
            step.remaining_to_remove = -1
        if variant == "missing-zone":
            step.zone_id = "missing"
        if variant == "missing-team":
            step.losing_team = "BLUE"
        if variant == "heavies":
            for m in red:
                m.type = MinionType.HEAVY
        if variant == "reverse-order":
            loc = list(state.entity_locations.items())
            state.entity_locations = dict(reversed(loc))
        add(
            f"casualties-{variant}",
            state,
            step,
            invalid=str(red[3].id) if variant == "invalid-heavy" else "SKIP",
            prefix=True,
        )
    for variant in [
        "tie",
        "red-loses",
        "blue-loses",
        "only-red",
        "specific-lane",
        "all-lanes",
        "army",
        "army-offboard",
        "army-level7",
        "team-mismatch",
    ]:
        state = build(variant in ["specific-lane", "all-lanes"])
        populate(state, 2, 2 if variant == "tie" else 3)
        if variant in ["blue-loses", "only-red"]:
            for uid in list(state.entity_locations):
                state.remove_entity(uid)
            populate(state, 3, 0 if variant == "only-red" else 2)
        if variant in ["specific-lane", "all-lanes"]:
            populate(state, 3, 1, "lane_2")
        if variant.startswith("army"):
            for uid in list(state.entity_locations):
                state.remove_entity(uid)
            populate(state, 2, 2)
            brogan = state.get_hero("hero_brogan")
            brogan.level = 7 if variant == "army-level7" else 8
            if variant != "army-offboard":
                state.place_entity(brogan.id, h(1, 1))
        if variant == "team-mismatch":
            state.teams[TeamColor.RED].minions[0].team = TeamColor.BLUE
        state.phase = GamePhase.CLEANUP
        final = add(
            f"minions-{variant}",
            state,
            MinionBattleStep(lane_id="lane_1" if variant == "specific-lane" else None),
            invalid="SKIP",
            prefix=True,
        )
        if variant.startswith("army"):
            assert brogan.ultimate_card.effect_id == "one_man_army"
            red_count = sum(
                final.get_position(m.id) is not None for m in final.teams[TeamColor.RED].minions
            )
            assert red_count == (1 if variant == "army" else 2), variant
    from goa2.engine.steps import ReturnMinionToZoneStep

    for variant in ["push", "return"]:
        state = build()
        zone = state.board.zones["lane_1-2"]
        state.board.zones[""] = Zone(id="", hexes=zone.hexes)
        state.battle_zones["lane_1"] = ""
        if variant == "push":
            populate(state, 1, 0)
        else:
            state.place_entity(state.teams[TeamColor.RED].minions[0].id, h(4))
        final = add(
            f"empty-current-{variant}",
            state,
            CheckLanePushStep() if variant == "push" else ReturnMinionToZoneStep(),
            prefix=True,
        )
        assert final.wave_counters["lane_1"] == 5

    from oracle import ROOT

    from goa2.engine.setup import GameSetup

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        for uid in list(state.entity_locations):
            state.remove_entity(uid)
        state.phase = GamePhase.RESOLUTION
        for lane, zone in state.battle_zones.items():
            minion = next(m for m in state.teams[TeamColor.RED].minions if m.lane_id == lane)
            point = next(
                p
                for p in state.board.tiles
                if p in state.board.zones[zone].hexes and not state.board.get_tile(p).is_obstacle
            )
            state.place_entity(minion.id, point)
        add(f"map-push-{path.stem}", state, CheckLanePushStep(), prefix=True)
    return result
