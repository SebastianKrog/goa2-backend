"""Seeded complete games through real planning commands and varied legal inputs."""

from __future__ import annotations

import copy
import json
import random


def enemy_targets(state, hero_id):
    hero = state.get_hero(hero_id)
    if hero is None:
        return []
    targets = [
        (str(unit.id), point)
        for team in state.teams.values()
        if team.color != hero.team
        for unit in team.minions
        for point in state.get_positions(str(unit.id))
    ]
    return targets or [
        (str(unit.id), point)
        for team in state.teams.values()
        if team.color != hero.team
        for unit in team.heroes
        for point in state.get_positions(str(unit.id))
    ]


def distance_to_targets(state, hero_id, point):
    return min((point.distance(h) for _, h in enemy_targets(state, hero_id)), default=0)


def choose_card(state, hero, rng):
    from goa2.domain.models import ActionType, StatType

    positions = state.get_positions(str(hero.id))
    distance = min((distance_to_targets(state, str(hero.id), p) for p in positions), default=0)

    def score(card):
        secondary = card.current_secondary_actions
        attack = (
            card.current_primary_action_value or 0
            if card.current_primary_action == ActionType.ATTACK
            else secondary.get(ActionType.ATTACK, 0) or 0
        )
        movement = (
            card.current_primary_action_value or 0
            if card.current_primary_action == ActionType.MOVEMENT
            else secondary.get(ActionType.MOVEMENT, 0) or 0
        )
        reach = card.get_base_stat_value(StatType.RANGE) or 1
        if 0 < distance <= reach and attack:
            return (3, attack, reach)
        if distance > 4 and ActionType.FAST_TRAVEL in secondary:
            return (2, 1, 0)
        return (1, movement, attack)

    shuffled = list(hero.hand)
    rng.shuffle(shuffled)
    return max(shuffled, key=score)


def choose_input(state, request, rng):
    from goa2.domain.input import selection_value
    from goa2.domain.models import StatType

    kind = request.request_type.value
    hero_id = str(state.current_actor_id or request.player_id)
    ids = [o.id for o in request.options]
    if kind == "CHOOSE_ACTION":
        if "CONFIRM" in ids:
            return "CONFIRM"
        hero = state.get_hero(hero_id)
        card = hero.current_turn_card if hero else None
        reach = (card.get_base_stat_value(StatType.RANGE) or 1) if card else 1
        positions = state.get_positions(hero_id)
        distance = min((distance_to_targets(state, hero_id, p) for p in positions), default=0)
        if "ATTACK" in ids and 0 < distance <= reach:
            return "ATTACK"
        if "FAST_TRAVEL" in ids and distance > 4:
            return "FAST_TRAVEL"
        if "MOVEMENT" in ids:
            return "MOVEMENT"
        active = [o for o in request.options if o.id != "HOLD"]
        return selection_value(rng.choice(active or request.options))
    if kind == "CONFIRM_PASSIVE" and "YES" in ids:
        return "YES"

    def score(option):
        value = selection_value(option)
        if isinstance(value, dict) and all(k in value for k in ("q", "r", "s")):
            from goa2.domain.hex import Hex

            return distance_to_targets(state, hero_id, Hex(**value))
        if isinstance(value, str):
            minions = [uid for uid, _ in enemy_targets(state, hero_id)]
            if value in minions:
                return -100
            positions = state.get_positions(value)
            if positions:
                return min(distance_to_targets(state, hero_id, p) for p in positions)
            zone = state.board.zones.get(value)
            if zone:
                return min(
                    (distance_to_targets(state, hero_id, p) for p in zone.hexes), default=1000
                )
        return 1000

    options = list(request.options)
    rng.shuffle(options)
    return selection_value(min(options, key=score))


def play(state, seed, *, max_steps=12000, input_policy=None):
    from state_codec import export_state_with_instances

    from goa2.domain.input import InputResponse
    from goa2.domain.models import Card, GamePhase
    from goa2.engine.handler import process_stack, submit_input
    from goa2.engine.phases import commit_card, pass_turn

    rng = random.Random(seed)
    physical = export_state_with_instances(state)[1]
    cards = {str(card.id): i for i, card in enumerate(physical) if isinstance(card, Card)}
    probe = copy.deepcopy(state)
    ops = []
    request = None
    decisions = {}
    for _ in range(max_steps):
        if probe.phase == GamePhase.GAME_OVER:
            return ops, dict(round=probe.round, turn=probe.turn, decisions=decisions)
        if request is not None:
            kind = request.request_type.value
            decisions[kind] = decisions.get(kind, 0) + 1
            if kind == "UPGRADE_PHASE":
                hero = next(
                    h
                    for h, row in request.context["players"].items()
                    if row["remaining"] > 0 and row["options"]
                )
                option = rng.choice(request.context["players"][hero]["options"])
                value = {"hero_id": hero, "card_id": rng.choice(option["pair"])}
            elif request.can_skip and (not request.options or rng.randrange(4) == 0):
                value = "SKIP"
            else:
                assert request.options, (kind, request)
                value = (input_policy or choose_input)(probe, request, rng)
            submit_input(probe, InputResponse(request_id=request.id, selection=value))
            ops.append({"kind": "input", "selection": value})
            request = None
        elif probe.phase == GamePhase.PLANNING:
            heroes = [hero for team in probe.teams.values() for hero in team.heroes]
            for hero in heroes:
                if str(hero.id) in probe.pending_inputs:
                    continue
                if hero.hand:
                    card = choose_card(probe, hero, rng)
                    ops.append(
                        {"kind": "commit", "hero": str(hero.id), "card": cards[str(card.id)]}
                    )
                    commit_card(probe, hero.id, card)
                else:
                    ops.append({"kind": "pass", "hero": str(hero.id)})
                    pass_turn(probe, hero.id)
            assert probe.phase != GamePhase.PLANNING, "planning did not progress"
        else:
            assert probe.execution_stack, (probe.phase, probe.round, probe.turn)
            remaining = 1

            def stop(_state, _step):
                nonlocal remaining
                if remaining == 0:
                    return True
                remaining -= 1
                return False

            out = process_stack(probe, stop_before_step=stop)
            ops.append({"kind": "advance_prefix", "steps": 1})
            request = out.input_request
    raise AssertionError("complete game exceeded source step budget")


def scenes(names=None, *, mixed=False):
    from model_traces import setup
    from oracle import ROOT

    from goa2.engine.setup import GameSetup

    setup()
    if mixed:
        games = (("forgotten_island", 47),)
        red, blue = ["Garrus", "Gydion"], ["Razzle", "Mortimer"]
    else:
        games = (("forgotten_island", 11), ("magma_and_ice", 19))
        red, blue = ["Arien", "Brogan"], ["Tigerclaw", "Sabina"]
    roster = "mixed" if mixed else "live"
    for map_id, seed in games:
        name = f"complete-game-{roster}-{map_id}-progress-{seed}"
        if names is not None and name not in names:
            continue
        state = GameSetup.create_game(
            str(ROOT / f"src/goa2/data/maps/{map_id}.json"),
            red,
            blue,
            seed=29,
        )
        state.execution_context.update(
            locations_alias=state.entity_locations,
            snapshot_alias=state.last_turn_positions,
            stack_alias=state.execution_stack,
            effects_alias=state.active_effects,
        )
        ops, facts = play(state, seed)
        assert facts["round"] > 1 and facts["decisions"].get("CHOOSE_ACTION", 0) > 0
        yield name, state, ops


def choose_hanu_input(state, request, rng):
    """Exercise Hanu primary skills alongside the baseline progress policy."""
    from goa2.domain.models import ActionType, StatType

    if request.request_type.value == "CHOOSE_ACTION" and state.current_actor_id == "hero_hanu":
        hero = state.get_hero("hero_hanu")
        card = hero.current_turn_card
        if card and card.current_primary_action in (ActionType.SKILL, ActionType.DEFENSE_SKILL):
            positions = state.get_positions(hero.id)
            distance = min((distance_to_targets(state, hero.id, p) for p in positions), default=0)
            reach = (
                card.get_base_stat_value(StatType.RADIUS)
                or card.get_base_stat_value(StatType.RANGE)
                or 3
            )
            if (
                0 < distance <= reach
                and any(option.id == "SKILL" for option in request.options)
                and rng.randrange(3)
            ):
                return "SKILL"
    return choose_input(state, request, rng)


def hanu_scenes(names=None):
    from model_traces import setup
    from oracle import ROOT

    from goa2.engine.setup import GameSetup

    setup()
    for map_id, seed in (("forgotten_island", 41), ("magma_and_ice", 43)):
        name = f"complete-game-hanu-{map_id}-progress-{seed}"
        if names is not None and name not in names:
            continue
        state = GameSetup.create_game(
            str(ROOT / f"src/goa2/data/maps/{map_id}.json"),
            ["Hanu", "Arien"],
            ["Brogan", "Tigerclaw"],
            seed=29,
        )
        state.execution_context.update(
            locations_alias=state.entity_locations,
            snapshot_alias=state.last_turn_positions,
            stack_alias=state.execution_stack,
            effects_alias=state.active_effects,
        )
        ops, facts = play(state, seed, input_policy=choose_hanu_input)
        assert facts["round"] > 1 and facts["decisions"].get("CHOOSE_ACTION", 0) > 0
        print(json.dumps({"source_game_ready": name, "operations": len(ops), **facts}), flush=True)
        yield name, state, ops
