"""Registered defense bodies, actor wrappers and complete source continuations.

Raw boundary setup is deliberate: cases retain card, step, context and generated
world aliases. Source handler execution supplies all expected results.
"""

from __future__ import annotations


def scenes():
    import copy
    from unittest.mock import patch

    import defense_effect_traces
    from live_defeat_traces import retain

    captured = []

    def capture(state, ops):
        snapshot = copy.deepcopy(state)
        steps = list(reversed(snapshot.execution_stack))
        choices = [op["selection"] for op in ops if op["kind"] == "input"]

        def choose(request, _mode):
            from goa2.domain.input import selection_value

            return choices.pop(0) if choices else selection_value(request.options[0])

        live_ops, _ = retain(snapshot, steps, select=choose)
        captured.append((snapshot, live_ops))
        return []

    with patch("stack_traces.reference_trace", capture):
        cases = defense_effect_traces.cases(
            {"kind": "advance"},
            lambda selection: {"kind": "input", "request_id": "current", "selection": selection},
        )
    assert len(cases) == len(captured)
    for case, (state, ops) in zip(cases, captured, strict=True):
        yield "live-" + case["name"], state, ops
    yield from registered_scenes()
    yield from focused_scenes()
    yield from copied_smoke_scenes()
    yield from predicate_scenes()


def registered_scenes():
    from aura_traces import _board, _pieces
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import (
        ActionType,
        CardState,
        GamePhase,
        MarkerType,
        RuneType,
        Token,
        TokenType,
    )
    from goa2.engine.steps import ResolveDefenseTextStep, ResolveOnBlockEffectStep

    setup()
    cards = []
    for name, definition in sorted(HeroRegistry._heroes.items()):
        for card in [
            *definition.deck,
            *([definition.ultimate_card] if definition.ultimate_card else []),
        ]:
            if card.primary_action in {ActionType.DEFENSE, ActionType.DEFENSE_SKILL}:
                cards.append((name, card.id, card.effect_id))
    assert len(cards) == 26
    for name, printed, effect_id in cards:
        for on_block in (False, True):
            variants = [
                "first",
                "skip",
                "discard",
                "secondary",
                "offboard",
                "facedown",
                "missing-card",
                "unknown-defender",
            ]
            if on_block:
                variants += ["failed"]
            else:
                variants += ["melee", "repeated", "skip-gate", "inactive"]
            if effect_id.startswith("oath_of_"):
                variants += [
                    "no-runes",
                    "one-nonmatch",
                    "many-runes",
                    "many-nonmatch",
                    "other-owner",
                ]
            if name == "Dodger":
                variants += ["tide", "attacker-tide", "no-spawns"]
            if name == "Razzle":
                variants += ["owner-unbound", "owner-bound", "piece-far"]
            if name == "Min":
                variants += ["no-smoke", "two-smokes"]
            for variant in variants:
                state = setup([name, "Swift"], ["Brogan", "Hanu"])
                h = _board(state)
                owner = "hero_" + name.lower()
                hero = state.get_hero(owner)
                state.phase = GamePhase.RESOLUTION
                state.current_actor_id = state.resolution_owner_id = "hero_brogan"
                state.acting_piece_id = None
                for unit, point in [
                    ("hero_brogan", h(2, 0)),
                    ("hero_hanu", h(3, -1)),
                    ("hero_swift", h(-3, 0)),
                ]:
                    state.place_entity(unit, point)
                if name == "Razzle":
                    for index, piece in enumerate(_pieces(state, owner)):
                        state.place_entity(piece, h(0, index))
                    defender = _pieces(state, owner)[0]
                    if variant == "owner-unbound":
                        defender = owner
                    if variant == "owner-bound":
                        defender = owner
                        state.acting_piece_id = _pieces(state, owner)[-1]
                    if variant == "piece-far":
                        state.move_unit(_pieces(state, owner)[1], h(-4, 0))
                else:
                    state.place_entity(owner, h(0, 0))
                    defender = owner
                card = next(
                    card
                    for card in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
                    if card.id == printed
                )
                card.state, card.is_facedown = CardState.HAND, False
                if all(item is not card for item in hero.hand):
                    hero.hand.append(card)
                if variant == "discard":
                    hero.hand = [item for item in hero.hand if item is not card]
                    hero.discard_pile.append(card)
                    card.state = CardState.DISCARD
                if name == "Bain":
                    state.get_marker(MarkerType.BOUNTY).target_id = "hero_brogan"
                if name == "Min" and variant != "no-smoke":
                    for index in range(2 if variant == "two-smokes" else 1):
                        token = Token(
                            id=f"fixture-smoke-{index}",
                            name="Smoke",
                            token_type=TokenType.SMOKE_BOMB,
                            owner_id=owner,
                        )
                        state.token_pool.setdefault(TokenType.SMOKE_BOMB, []).append(token)
                        state.misc_entities[token.id] = token
                        state.place_entity(token.id, h(1, -index))
                if name == "Snorri":
                    hero.rune_slots = {state.turn: RuneType.AXE}
                    if variant == "no-runes":
                        hero.rune_slots = {}
                    if variant == "one-nonmatch":
                        hero.rune_slots = {state.turn: RuneType.BIRD}
                context = state.execution_context
                context.update(
                    defense_card_id=str(card.id),
                    defender_id=defender,
                    is_primary_defense=variant != "secondary",
                    block_succeeded=variant != "failed",
                    attack_is_ranged=variant != "melee",
                    attack_is_basic=True,
                    attacker_id="hero_brogan",
                    current_action_type="ATTACK",
                    defense_bonus=0,
                )
                if variant in {"many-runes", "many-nonmatch"}:
                    context.update(
                        snorri_ult_rune_action=RuneType.HORN, snorri_ult_rune_defense=RuneType.BIRD
                    )
                    if variant == "many-nonmatch":
                        context.update(attack_is_basic=False, attack_is_ranged=False)
                if variant == "other-owner":
                    state.teams[hero.team].heroes.reverse()
                    foreign = state.get_hero("hero_swift")
                    foreign.rune_slots = {state.turn: RuneType.HORN}
                    foreign.current_turn_card = card
                if variant == "offboard":
                    for unit in state.get_piece_ids(owner):
                        state.remove_entity(unit)
                    state.remove_entity(owner)
                if variant == "facedown":
                    card.is_facedown = True
                if variant == "missing-card":
                    context["defense_card_id"] = "missing"
                if variant == "unknown-defender":
                    context["defender_id"] = "missing"
                if name == "Dodger" and variant == "tide":
                    hero.level = 8
                if name == "Dodger" and variant == "attacker-tide":
                    attacker = state.get_hero("hero_brogan")
                    attacker.level = 8
                    attacker.ultimate_card = hero.ultimate_card
                step = ResolveOnBlockEffectStep() if on_block else ResolveDefenseTextStep()
                if variant == "skip-gate":
                    step.skip_if_key = "skip"
                    context["skip"] = False
                if variant == "inactive":
                    step.active_if_key = "absent"
                context.update(
                    defense_step_alias=step, defense_card_alias=card, defender_hand_alias=hero.hand
                )
                steps = [step, step] if variant == "repeated" else [step]
                ops, _ = retain(state, steps, "skip" if variant == "skip" else "first")
                yield f"defense-body-{effect_id}-{'on-block' if on_block else 'text'}-{variant}", state, ops


def focused_scenes():
    from aura_traces import _board
    from combat_effect_traces import effect
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.board import Zone
    from goa2.domain.models import CardState, GamePhase, TeamColor
    from goa2.domain.models.effect import EffectType
    from goa2.domain.models.spawn import SpawnPoint, SpawnType
    from goa2.engine.steps import ResolveDefenseTextStep

    for effect_id in ("shield_of_decay", "vampiric_shield", "aegis_of_doom"):
        for variant in (
            "two-spawns",
            "one-spawn",
            "outside-zone",
            "occupied",
            "barrier-friendly",
            "barrier-enemy",
            "tide-override",
            "topology",
        ):
            state = setup(["Dodger", "Swift"], ["Brogan", "Hanu"])
            h = _board(state)
            hero = state.get_hero("hero_dodger")
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = state.resolution_owner_id = "hero_brogan"
            for unit, point in [
                ("hero_dodger", h(0, 0)),
                ("hero_brogan", h(2, 0)),
                ("hero_hanu", h(3, -1)),
                ("hero_swift", h(-3, 0)),
            ]:
                state.place_entity(unit, point)
            points = [h(1, 0), h(0, 1)]
            state.board.zones = {"area": Zone(id="area", hexes=set(points))}
            state.battle_zones = {next(iter(state.battle_zones), "main"): "area"}
            for point in points:
                state.board.tiles[point].spawn_point = SpawnPoint(
                    location=point, team=TeamColor.RED, type=SpawnType.HERO
                )
                state.board.tiles[point].zone_id = "area"
            if variant == "one-spawn":
                state.board.tiles[points[1]].spawn_point = None
            if variant == "outside-zone":
                state.board.tiles[points[1]].zone_id = "other"
            if variant == "occupied":
                state.move_unit("hero_swift", points[1])
            if variant in {"barrier-friendly", "barrier-enemy"}:
                state.active_effects = [
                    effect(
                        state,
                        "fixture/barrier",
                        EffectType.STATIC_BARRIER,
                        source="hero_swift" if variant == "barrier-friendly" else "hero_brogan",
                        barrier_radius=1,
                        barrier_origin_id="hero_brogan",
                    )
                ]
            if variant == "tide-override":
                hero.level = 8
                for point in points:
                    state.board.tiles[point].spawn_point = None
            if variant == "topology":
                state.move_unit("hero_brogan", h(3, 0))
                state.board.tiles[points[0]].spawn_point = None
                point = h(2, 0)
                state.board.tiles[point].spawn_point = SpawnPoint(
                    location=point, team=TeamColor.RED, type=SpawnType.HERO
                )
                state.board.tiles[point].zone_id = "area"
                state.board.zones["area"].hexes.add(point)
                state.active_effects = [
                    effect(
                        state,
                        "fixture/split",
                        EffectType.TOPOLOGY_SPLIT,
                        split_axis="q",
                        split_value=1,
                    )
                ]
            card = next(card for card in hero.deck if card.effect_id == effect_id)
            card.state, card.is_facedown = CardState.HAND, False
            if all(item is not card for item in hero.hand):
                hero.hand.append(card)
            state.execution_context.update(
                defender_id=str(hero.id),
                defense_card_id=str(card.id),
                is_primary_defense=True,
                defense_bonus=0,
                attack_is_ranged=True,
            )
            step = ResolveDefenseTextStep()
            ops, _ = retain(state, [step])
            yield "defense-spawn-" + effect_id + "-" + variant, state, ops


def copied_smoke_scenes():
    from aura_traces import _board, _pieces
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase
    from goa2.engine.steps import ResolveDefenseTextStep

    for effect_id in ("poof", "vanish", "ruse"):
        state = setup(["Razzle", "Swift"], ["Min", "Brogan"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = state.resolution_owner_id = "hero_brogan"
        state.acting_piece_id = None
        for index, piece in enumerate(_pieces(state, "hero_razzle")):
            state.place_entity(piece, h(0, index))
        state.place_entity("hero_brogan", h(2, 0))
        state.place_entity("hero_min", h(3, -1))
        state.place_entity("hero_swift", h(-3, 0))
        hero = state.get_hero("hero_razzle")
        card = next(card for card in state.get_hero("hero_min").deck if card.effect_id == effect_id)
        card.state, card.is_facedown = CardState.HAND, False
        hero.hand.append(card)
        state.execution_context.update(
            defender_id=_pieces(state, "hero_razzle")[0],
            defense_card_id=str(card.id),
            is_primary_defense=True,
        )
        ops, _ = retain(state, [ResolveDefenseTextStep()])
        yield "defense-copied-smoke-" + effect_id + "-unbound-owner", state, ops


def predicate_scenes():
    from combat_effect_traces import state_for
    from live_defeat_traces import retain

    from goa2.domain.models import TargetType
    from goa2.engine.filters import AndFilter, ExcludeIdentityFilter, OrFilter, RangeFilter
    from goa2.engine.steps import SelectStep

    for variant in (
        "hex",
        "dict",
        "hex-list",
        "dict-list",
        "tuple",
        "mixed-list",
        "none",
        "skip-text",
        "nested-list",
        "and-empty",
        "or-empty",
        "or-short",
        "and-short",
        "nested-exclude",
    ):
        state, h, _ = state_for()
        point = h(1, 0)
        value = {
            "hex": point,
            "dict": point.model_dump(),
            "hex-list": [point],
            "dict-list": [point.model_dump()],
            "tuple": (point,),
            "mixed-list": [point, h(0, 1).model_dump()],
            "none": None,
            "skip-text": "SKIP",
            "nested-list": [[point]],
        }.get(variant, point)
        state.execution_context.update(blocked=value, bad_origin=True)
        exclude = ExcludeIdentityFilter(exclude_keys=["blocked"])
        filters = [exclude]
        if variant == "and-empty":
            filters = [AndFilter(filters=[])]
        if variant == "or-empty":
            filters = [OrFilter(filters=[])]
        if variant == "or-short":
            filters = [
                OrFilter(
                    filters=[
                        AndFilter(filters=[]),
                        RangeFilter(origin_key="bad_origin", max_range=1),
                    ]
                )
            ]
        if variant == "and-short":
            filters = [
                AndFilter(
                    filters=[
                        OrFilter(filters=[]),
                        RangeFilter(origin_key="bad_origin", max_range=1),
                    ]
                )
            ]
        if variant == "nested-exclude":
            filters = [OrFilter(filters=[OrFilter(filters=[]), AndFilter(filters=[exclude])])]
        step = SelectStep(
            target_type=TargetType.HEX,
            prompt="Choose a hex",
            output_key="selected",
            is_mandatory=False,
            filters=filters,
        )
        state.execution_context["predicate_alias"] = step
        ops, _ = retain(state, [step])
        yield "defense-hex-predicate-" + variant, state, ops
