"""Tigerclaw active bodies, terrain predicates and coin-transfer boundaries.

Raw setup preserves physical hero/card/gold aliases and live filter models;
these execution-boundary checks require those identities to remain observable.
"""

from __future__ import annotations

EFFECTS = (
    "backstab",
    "backstab_with_a_ballista",
    "combat_reflexes",
    "light_fingered",
    "pick_pocket",
    "master_thief",
    "blend_into_shadows",
)
COINS = {"light_fingered", "pick_pocket", "master_thief"}


def base(effect, flank=True, terrain=True):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase

    state = setup(["Tigerclaw", "Hanu"], ["Brogan", "Arien"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.battle_zones = {}
    state.current_actor_id = "hero_tigerclaw"
    hero = state.get_hero("hero_tigerclaw")
    card = next(card for card in hero.deck if card.effect_id == effect)
    hero.current_turn_card = card
    card.state = CardState.UNRESOLVED
    card.is_facedown = False
    if card in hero.hand:
        hero.hand.remove(card)
    for name, point in (
        ("hero_tigerclaw", h(0, 0)),
        ("hero_hanu", h(0, 1) if flank else h(-2, 0)),
        ("hero_brogan", h(1, 0)),
        ("hero_arien", h(2, 0)),
    ):
        state.place_entity(name, point)
    state.place_entity(state.teams["BLUE"].minions[0].id, h(1, -1))
    if terrain:
        state.board.tiles[h(0, -1)].is_terrain = True
    hero.gold = 1
    state.get_hero("hero_brogan").gold = 3
    state.get_hero("hero_arien").gold = 2
    state.execution_context.update(
        hero_alias=hero,
        card_alias=card,
        victim_alias=state.get_hero("hero_brogan"),
        locations_alias=state.entity_locations,
        effects_alias=state.active_effects,
    )
    return state, hero, card, h


def action_scene(effect, entry, mode, variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.domain.models import StatType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.steps import (
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SetContextFlagStep,
    )

    state, hero, card, h = base(
        effect, flank=variant != "no-flank", terrain=variant != "no-terrain"
    )
    if variant == "no-coins":
        state.get_hero("hero_brogan").gold = 0
    if variant == "one-coin":
        state.get_hero("hero_brogan").gold = 1
    if variant == "negative-attack":
        hero.items = {StatType.ATTACK: -9}
    if variant == "radius-none":
        card.radius_value = None
    if variant == "radius-zero":
        card.radius_value = 0
    if variant == "radius-negative":
        card.radius_value = -1
    if variant == "pre-hit":
        state.board.tiles[h(0, -1)].is_terrain = False
    if variant == "petrify":
        state.board.tiles[h(0, -1)].is_terrain = False
        state.active_effects.append(
            ActiveEffect(
                id="tiger-petrify",
                source_id="hero_brogan",
                effect_type=EffectType.PETRIFY,
                scope=EffectScope(shape=Shape.POINT, origin_id="hero_hanu"),
                duration=DurationType.THIS_TURN,
                created_at_round=state.round,
                created_at_turn=state.turn,
                is_active=True,
            )
        )
    if entry == "text":
        step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
    elif entry == "action":
        step = ResolveCardStep(hero_id=hero.id)
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/tiger-primary"
    state.execution_context.update(selected_card=card.id, tiger_step=step)
    state.execution_stack = [
        SetContextFlagStep(key="tiger_tail", value=True, step_id="fixture/tiger-tail"),
        step,
    ]
    bad = False
    pre_seen = False

    def select(request, _mode):
        nonlocal bad, pre_seen
        if mode == "invalid" and not bad:
            bad = True
            return "not-offered"
        if request.request_type.value == "CHOOSE_ACTION":
            return card.current_primary_action.value
        if request.request_type.value == "SELECT_CARD_OR_PASS":
            return "PASS"
        if mode == "skip" and request.can_skip:
            return "SKIP"
        if (
            request.request_type.value == "SELECT_HEX"
            and effect == "combat_reflexes"
            and not pre_seen
        ):
            pre_seen = True
            if variant == "pre-skip":
                return "SKIP"
            if variant == "pre-hit":
                return {"q": 0, "r": -1, "s": 1}
        if request.request_type.value == "SELECT_HEX" and effect in COINS and not pre_seen:
            pre_seen = True
            if variant != "pre-move":
                return "SKIP"
        if request.request_type.value == "SELECT_UNIT" and any(
            option.id == "hero_brogan" for option in request.options
        ):
            return "hero_brogan"
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, _ = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}]


def scenes():
    for effect in EFFECTS:
        for entry in ("text", "action", "perform"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops = action_scene(effect, entry, mode)
                yield "tiger-primary-" + effect + "-" + entry + "-" + mode, state, ops
    variants = {
        "combat_reflexes": ("pre-hit", "pre-skip"),
        "backstab": ("no-flank", "negative-attack"),
        "backstab_with_a_ballista": ("no-flank", "negative-attack"),
        "light_fingered": ("no-coins", "pre-move"),
        "pick_pocket": ("no-coins", "pre-move"),
        "master_thief": ("no-coins", "one-coin", "pre-move"),
        "blend_into_shadows": (
            "no-terrain",
            "petrify",
            "radius-none",
            "radius-zero",
            "radius-negative",
        ),
    }
    for effect, choices in variants.items():
        for variant in choices:
            for entry in ("text", "perform"):
                state, ops = action_scene(effect, entry, "last", variant)
                yield "tiger-primary-edge-" + effect + "-" + variant + "-" + entry, state, ops
    yield from terrain_scenes()
    yield from steal_scenes()


def terrain_scenes():
    from goa2.domain.models import TargetType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.filters import TerrainFilter
    from goa2.engine.steps import CountStep, SelectStep

    for kind in (TargetType.HEX, TargetType.UNIT, TargetType.UNIT_OR_TOKEN):
        for wanted in (True, False):
            for petrify in (False, True):
                state, _hero, _card, _h = base("blend_into_shadows")
                if petrify:
                    state.active_effects.append(
                        ActiveEffect(
                            id="tiger-filter-petrify",
                            source_id="hero_brogan",
                            effect_type=EffectType.PETRIFY,
                            scope=EffectScope(shape=Shape.POINT, origin_id="hero_hanu"),
                            duration=DurationType.THIS_TURN,
                            created_at_round=state.round,
                            created_at_turn=state.turn,
                            is_active=True,
                        )
                    )
                predicate = TerrainFilter(is_terrain=wanted)
                step = SelectStep(
                    prompt="Terrain predicate",
                    target_type=kind,
                    filters=[predicate],
                    output_key="terrain_choices",
                    is_mandatory=False,
                    step_id="fixture/tiger-terrain",
                )
                state.execution_stack = [step]
                state.execution_context.update(terrain_filter=step.filters[0], terrain_step=step)
                yield f"tiger-primary-terrain-{kind.value}-{wanted}-{petrify}", state, [
                    {"kind": "advance_prefix", "steps": 1},
                    {"kind": "advance_prefix", "steps": 0},
                ]
    for wanted in (True, False):
        state, _hero, _card, _h = base("blend_into_shadows")
        step = CountStep(
            target_type=TargetType.HEX,
            filters=[TerrainFilter(is_terrain=wanted)],
            output_key="terrain_total",
            step_id="fixture/tiger-terrain-count",
        )
        state.execution_stack = [step]
        state.execution_context["terrain_count_step"] = step
        yield f"tiger-primary-terrain-count-{wanted}", state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]


def steal_scenes():
    from goa2.engine.steps import StealCoinsStep

    variants = (
        "plain",
        "two",
        "cap",
        "zero",
        "negative",
        "missing-victim",
        "unknown-victim",
        "empty-victim",
        "no-actor",
        "unknown-actor",
        "override-actor",
        "false-override",
        "dynamic",
        "dynamic-missing",
        "dynamic-true",
        "dynamic-false",
        "empty-output",
        "skip",
        "inactive",
        "no-gold",
        "stale-output",
        "self",
        "negative-victim-gold",
        "negative-actor-gold",
        "repeated",
        "piece-actor",
        "piece-self",
    )
    for variant in variants:
        state, hero, _card, _h = base("light_fingered")
        if variant.startswith("piece-"):
            from model_traces import setup

            state = setup(["Razzle", "Hanu"], ["Brogan", "Arien"])
            hero = state.get_hero("hero_razzle")
            state.current_actor_id = "hero_razzle_piece_1"
        victim = state.get_hero("hero_brogan")
        hero.gold = 2
        victim.gold = 3
        state.execution_context.update(
            coin_victim=victim.id, steal_actor_alias=hero, steal_victim_alias=victim
        )
        step = StealCoinsStep(
            victim_key="coin_victim", amount=1, output_key="stolen", step_id="fixture/tiger-steal"
        )
        if variant in ("two", "cap"):
            step.amount = 2
        if variant == "cap":
            victim.gold = 1
        if variant == "zero":
            step.amount = 0
        if variant == "negative":
            step.amount = -2
        if variant == "missing-victim":
            state.execution_context.pop("coin_victim")
        if variant == "unknown-victim":
            state.execution_context["coin_victim"] = "absent"
        if variant == "empty-victim":
            state.execution_context["coin_victim"] = ""
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "unknown-actor":
            state.execution_context["current_actor_id"] = "absent"
        if variant == "override-actor":
            state.execution_context["current_actor_id"] = "hero_hanu"
        if variant == "false-override":
            state.execution_context["current_actor_id"] = ""
        if variant.startswith("dynamic"):
            step.amount = 2
            step.amount_key = "requested_coins"
            if variant == "dynamic":
                state.execution_context["requested_coins"] = 3
            if variant == "dynamic-true":
                state.execution_context["requested_coins"] = True
            if variant == "dynamic-false":
                state.execution_context["requested_coins"] = False
        if variant == "empty-output":
            step.output_key = ""
        if variant == "skip":
            step.skip_if_key = "skip"
            state.execution_context["skip"] = True
        if variant == "inactive":
            step.active_if_key = "enabled"
        if variant in ("no-gold", "stale-output"):
            victim.gold = 0
        if variant == "stale-output":
            state.execution_context["stolen"] = True
        if variant == "self":
            state.execution_context["coin_victim"] = hero.id
        if variant == "piece-self":
            state.execution_context["coin_victim"] = "hero_razzle_piece_1"
        if variant == "negative-victim-gold":
            victim.gold = -2
        if variant == "negative-actor-gold":
            hero.gold = -2
        state.execution_stack = [step, step] if variant == "repeated" else [step]
        state.execution_context["steal_step"] = step
        yield "tiger-primary-steal-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 1 if variant == "repeated" else 0},
        ]
