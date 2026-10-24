"""Real card-aura stats and movement-action traces from the frozen rules."""

from __future__ import annotations

from state_codec import export_state, export_state_with_instances


def _board(state):
    from goa2.domain.board import Board
    from goa2.domain.hex import Hex
    from goa2.domain.tile import Tile

    for entity in list(state.entity_locations):
        state.remove_entity(entity)
    tiles = {}
    for q in range(-4, 5):
        for r in range(-4, 5):
            if abs(-q - r) <= 4:
                h = Hex(q=q, r=r, s=-q - r)
                tiles[h] = Tile(hex=h)
    state.board = Board(tiles=tiles)
    return lambda q, r: Hex(q=q, r=r, s=-q - r)


def _pieces(state, hero):
    from goa2.domain.models.unit import HeroPiece

    return [
        str(key)
        for key, entity in state.misc_entities.items()
        if isinstance(entity, HeroPiece) and str(entity.owner_hero_id) == str(hero)
    ]


def aura_stats_cases():
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import Card, CardState, StatType, TeamColor, Token, TokenType
    from goa2.domain.models.marker import MarkerType
    from goa2.engine.stats import (
        calculate_minion_defense_modifier,
        compute_card_stats,
        get_computed_stat,
    )

    states = []
    for name in ["Xargatha", "Trinkets"]:
        for variant in [
            "active",
            "level-seven",
            "facedown",
            "inactive-flag",
            "resolved",
            "unresolved",
            "duplicate-alias",
            "offboard",
            "extra-enemy",
            "enemy-away",
            "markers-items",
        ]:
            state = setup([name, "Arien"], ["Razzle", "Brogan"])
            h = _board(state)
            hero = state.teams[TeamColor.RED].heroes[0]
            hero.level = 8
            hero.ultimate_card.is_facedown = False
            enemy = state.get_hero("hero_brogan")
            pieces = _pieces(state, "hero_razzle")
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_arien", h(3, -3))
            state.place_entity(enemy.id, h(0, -1))
            state.place_entity(pieces[0], h(1, 0))
            state.place_entity(pieces[1], h(2, 0))
            minion = state.teams[TeamColor.BLUE].minions[0]
            state.place_entity(minion.id, h(-1, 0))
            state.place_entity(state.teams[TeamColor.RED].minions[0].id, h(0, 1))
            token = Token(
                id="aura_token",
                name="Not a unit",
                token_type=TokenType.TOTEM,
                owner_id="hero_razzle",
            )
            state.misc_entities[token.id] = token
            state.place_entity(token.id, h(-1, 1))
            if variant == "level-seven":
                hero.level = 7
            if variant == "facedown":
                hero.ultimate_card.is_facedown = True
            if variant == "inactive-flag":
                hero.ultimate_card.is_active = False
            if variant in ["resolved", "unresolved", "duplicate-alias"]:
                hero.played_cards = [hero.ultimate_card]
                hero.ultimate_card.state = (
                    CardState.UNRESOLVED if variant == "unresolved" else CardState.RESOLVED
                )
                if variant != "duplicate-alias":
                    hero.level = 1
            if variant == "offboard":
                state.remove_entity(hero.id)
            if variant == "extra-enemy":
                state.move_unit(pieces[1], h(1, -1))
            if variant == "enemy-away":
                for entity in [enemy.id, pieces[0], minion.id]:
                    state.remove_entity(entity)
            if variant == "markers-items":
                hero.items = {
                    StatType.MOVEMENT: 2,
                    StatType.INITIATIVE: -1,
                    StatType.RANGE: 1,
                    StatType.RADIUS: 2,
                }
                state.place_marker(MarkerType.POISON, hero.id, -2, enemy.id)
            if name == "Xargatha" and variant == "active":
                assert get_computed_stat(state, hero.id, StatType.MOVEMENT, 0) == 3
            states.append((f"aura-{name}-{variant}", state))
    for bound in [None, 0, 1]:
        state = setup(["Razzle", "Trinkets"], ["Xargatha", "Brogan"])
        h = _board(state)
        hero = state.get_hero("hero_razzle")
        hero.ultimate_card = HeroRegistry.get("Xargatha").ultimate_card.model_copy(deep=True)
        hero.level = 8
        pieces = _pieces(state, hero.id)
        state.place_entity(pieces[0], h(0, 0))
        state.place_entity(pieces[1], h(1, -1))
        state.place_entity("hero_brogan", h(1, 0))
        state.place_entity("hero_xargatha", h(2, -1))
        if bound is not None:
            state.acting_piece_id = pieces[bound]
        state.place_marker(MarkerType.VENOM, pieces[1], -1, "hero_brogan")
        assert get_computed_stat(state, hero.id, StatType.INITIATIVE, 0) == 1
        states.append((f"aura-multi-{bound}", state))
    from oracle import ROOT

    from goa2.engine.setup import GameSetup

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Xargatha", "Trinkets"], ["Arien", "Brogan"], seed=29
        )
        for entity in list(state.entity_locations):
            state.remove_entity(entity)
        hero = state.get_hero("hero_xargatha")
        hero.level = 8
        state.get_hero("hero_trinkets").level = 8
        center = next(
            h
            for h in state.board.tiles
            if not state.board.get_tile(h).is_obstacle
            and any(
                n in state.board.tiles and not state.board.get_tile(n).is_obstacle
                for n in h.neighbors()
            )
        )
        adjacent = next(
            n
            for n in center.neighbors()
            if n in state.board.tiles and not state.board.get_tile(n).is_obstacle
        )
        state.place_entity(hero.id, center)
        state.place_entity("hero_brogan", adjacent)
        assert get_computed_stat(state, hero.id, StatType.MOVEMENT, 0) == 1
        states.append((f"aura-map-{path.stem}", state))
    from conditional_aura_traces import states as conditional_states

    states.extend(conditional_states())
    result = []
    for name, state in states:
        envelope, objects = export_state_with_instances(state)
        queries = []
        expected = []
        units = [str(h.id) for t in state.teams.values() for h in t.heroes]
        units += [
            p for t in state.teams.values() for hero in t.heroes for p in _pieces(state, hero.id)
        ]
        units += [str(m.id) for t in state.teams.values() for m in t.minions[:1]] + ["unknown"]
        for unit in units:
            for stat in StatType:
                for base in [-2, 0, 4]:
                    queries.append({"kind": "stat", "unit": unit, "stat": stat.value, "base": base})
                    expected.append(get_computed_stat(state, unit, stat, base))
            queries.append({"kind": "minion_defense", "unit": unit})
            expected.append(calculate_minion_defense_modifier(state, unit))
        for i, card in enumerate(objects):
            if not isinstance(card, Card):
                continue
            owner = next(
                (
                    h
                    for t in state.teams.values()
                    for h in t.heroes
                    if any(c is card for c in [*h.deck, h.ultimate_card])
                ),
                None,
            )
            if owner:
                queries.append({"kind": "card", "unit": str(owner.id), "card": i})
                expected.append(vars(compute_card_stats(state, owner.id, card)))
        result.append(
            {"name": name, "envelope": envelope, "queries": queries, "expected": expected}
        )
    return result


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.domain.board import Board
    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import GamePhase, StatType
    from goa2.domain.tile import Tile
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import (
        MoveSequenceStep,
        MoveUnitStep,
        ResolveCardStep,
        SetContextFlagStep,
    )

    result = []
    for variant in [
        "ordinary",
        "straight",
        "full-distance",
        "computed-range",
        "invalid",
        "atomic",
        "raw-action",
        "effect-move",
        "level-seven",
        "facedown",
        "basic-action",
    ]:
        state = setup(["Xargatha", "Trinkets"], ["Brogan", "Arien"])
        h = _board(state)
        hero = state.get_hero("hero_xargatha")
        hero.level = 8
        hero.ultimate_card.is_facedown = False
        origin, wall, dest, side = h(0, 0), h(1, 0), h(2, 0), h(0, 1)
        state.board = Board(
            tiles={p: Tile(hex=p, is_terrain=p == wall) for p in [origin, wall, dest, side]}
        )
        state.place_entity(hero.id, origin)
        state.current_actor_id = hero.id
        state.phase = GamePhase.RESOLUTION
        if variant == "level-seven":
            hero.level = 7
        if variant == "facedown":
            hero.ultimate_card.is_facedown = True
        if variant == "computed-range":
            state.place_entity("hero_brogan", side)
        if variant in ["raw-action", "effect-move"]:
            step = MoveUnitStep(
                unit_id=hero.id,
                target_hex_arg=dest,
                range_val=2,
                is_movement_action=variant == "raw-action",
            )
        elif variant == "basic-action":
            card = max(hero.hand, key=lambda c: c.secondary_actions.get("MOVEMENT", -1))
            hero.play_card(card)
            card.is_facedown = False
            step = ResolveCardStep(hero_id=hero.id)
        else:
            step = MoveSequenceStep(
                unit_id=hero.id,
                range_val=1 if variant == "computed-range" else 2,
                range_stat_type=StatType.MOVEMENT if variant == "computed-range" else None,
                force_straight_line=variant == "straight",
                force_full_distance=variant == "full-distance",
            )
        steps = [step, SetContextFlagStep(key="aura_done")]
        for i, s in enumerate(steps):
            s.step_id = f"fixture/aura/{variant}/{i}"
        push_steps(state, steps)
        envelope = export_state(state)
        probe = state.model_copy(deep=True)
        ops = []
        tried = False
        for _ in range(30):
            if variant == "atomic":
                remaining = 1

                def boundary(_state, _step):
                    nonlocal remaining
                    if not remaining:
                        return True
                    remaining -= 1
                    return False

                out = process_stack(probe, stop_before_step=boundary)
                ops.append({"kind": "advance_prefix", "steps": 1})
            else:
                out = process_stack(probe)
                ops.append(advance)
            req = out.input_request
            if req is None:
                if probe.execution_stack:
                    continue
                break
            if req.request_type.value == "CHOOSE_ACTION":
                value = "MOVEMENT"
            elif variant == "invalid" and not tried:
                value = {"q": 99, "r": -99, "s": 0}
                tried = True
            else:
                options = [selection_value(o) for o in req.options]
                value = dest.model_dump() if dest.model_dump() in options else options[0]
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=req.id, selection=value))
        else:
            raise AssertionError(variant)
        moved = probe.get_position(hero.id) == dest
        assert moved == (variant not in ["raw-action", "effect-move", "level-seven", "facedown"]), (
            variant,
            probe.get_position(hero.id),
        )
        result.append(
            {
                "name": f"aura-movement-{variant}",
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    from goa2.domain.models import TeamColor
    from goa2.engine.steps import AttackSequenceStep, FindNextActorStep

    for variant in ["ranged", "ranged-facedown", "initiative-handoff"]:
        state = setup(["Xargatha", "Arien"], ["Trinkets", "Hanu"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        if variant.startswith("ranged"):
            hero = state.get_hero("hero_trinkets")
            hero.level = 8
            hero.ultimate_card.is_facedown = variant == "ranged-facedown"
            target = next(m for m in state.teams[TeamColor.RED].minions if not m.is_heavy)
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(target.id, h(2, 0))
            state.current_actor_id = hero.id
            steps = [AttackSequenceStep(damage=3, range_val=1, range_stat_type=StatType.RANGE)]
        else:
            hero = state.get_hero("hero_xargatha")
            hero.level = 8
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_hanu", h(1, 0))
            state.place_entity("hero_arien", h(3, -3))
            for actor in [state.get_hero("hero_arien"), hero]:
                card = actor.hand[0]
                actor.play_card(card)
                card.is_facedown = False
                card.initiative_override = 1
            state.unresolved_hero_ids = ["hero_arien", str(hero.id)]
            state.current_actor_id = None
            steps = [FindNextActorStep()]
        for n, step in enumerate(steps):
            step.step_id = f"fixture/aura/{variant}/{n}"
        push_steps(state, steps)
        envelope = export_state(state)
        probe = state.model_copy(deep=True)
        ops = []
        first_actor = None
        for _ in range(30):
            ops.append(advance)
            req = process_stack(probe).input_request
            if req is None:
                break
            if first_actor is None:
                first_actor = str(probe.current_actor_id)
            options = [opt.id for opt in req.options]
            if "HOLD" in options:
                value = "HOLD"
            elif "CONFIRM" in options:
                value = "CONFIRM"
            elif req.can_skip:
                value = "SKIP"
            else:
                value = selection_value(req.options[0])
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=req.id, selection=value))
        else:
            raise AssertionError(variant)
        if variant.startswith("ranged"):
            assert (probe.get_position(target.id) is None) == (variant == "ranged"), variant
        else:
            assert first_actor == str(hero.id), first_actor
            assert probe.phase == GamePhase.PLANNING and probe.turn == 2
        result.append(
            {
                "name": f"aura-{variant}",
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    return result
