"""Nested primary actions with physical selected-card and shallow scope identities.

Raw setup exposes construction, nested filters and retained aliases separately
from full card-flow coverage.
"""

from __future__ import annotations

import copy


def base(effect, name):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import ActionType, CardState, GamePhase

    foe = "Brogan" if name != "Brogan" else "Arien"
    other = [n for n in ("Swift", "Hanu", "Tigerclaw", "Sabina") if n not in (name, foe)][:2]
    state = setup([name, other[0]], [foe, other[1]])
    h = _board(state)
    hero = state.get_hero("hero_" + name.lower())
    card = next(
        c
        for c in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
        if c.effect_id == effect
    )
    hero.current_turn_card = card
    card.is_facedown = False
    card.state = CardState.UNRESOLVED
    state.phase = GamePhase.RESOLUTION
    state.battle_zones = {}
    state.current_actor_id = hero.id
    state.place_entity(hero.id, h(0, 0))
    state.place_entity("hero_" + foe.lower(), h(1, 0))
    state.execution_context.update(
        selected_card=card.id,
        blocked_target="hero_" + foe.lower(),
        current_action_type=ActionType.SKILL,
        current_card_id="outer-card",
        performing_card_id="outer-card",
        performing_card_owner_id=str(hero.id),
        reperforming_card_id={"retained": True},
        hero_alias=hero,
        card_alias=card,
    )
    state.execution_context["old_reperforming"] = state.execution_context["reperforming_card_id"]
    return state, hero, card


def scenes(*, effects=None, include_common=True):
    from model_traces import setup
    from primary_templates import ALL_EFFECTS as EFFECTS

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import StatType
    from goa2.engine.effects import CardEffect, CardEffectRegistry
    from goa2.engine.steps import PerformPrimaryActionStep, SetContextFlagStep

    setup()
    for effect, name in (EFFECTS if effects is None else effects).items():
        for variant in ("ordinary", "items", "exclude", "facedown"):
            state, hero, card = base(effect, name)
            if variant == "items":
                hero.items = {
                    StatType.ATTACK: -7,
                    StatType.MOVEMENT: 2,
                    StatType.RANGE: 3,
                    StatType.RADIUS: 1,
                }
            if variant == "facedown":
                card.is_facedown = True
            step = PerformPrimaryActionStep(
                hero_id=str(hero.id),
                exclude_target_key="blocked_target" if variant == "exclude" else None,
                step_id="fixture/perform-primary",
            )
            tail = SetContextFlagStep(key="tail", value=True, step_id="fixture/perform-tail")
            state.execution_stack = [tail, step]
            state.execution_context["perform_step"] = step
            yield "perform-primary-build-" + effect + "-" + variant, state, [
                {"kind": "advance_prefix", "steps": 1},
                {"kind": "advance_prefix", "steps": 0},
            ]
    if not include_common:
        return

    emitted = set()
    for name, definition in sorted(HeroRegistry._heroes.items()):
        for original in [
            *definition.deck,
            *([definition.ultimate_card] if definition.ultimate_card else []),
        ]:
            effect = CardEffectRegistry.get(original.effect_id)
            if (
                not effect
                or type(effect).build_steps is not CardEffect.build_steps
                or original.effect_id in emitted
            ):
                continue
            emitted.add(original.effect_id)
            state, hero, card = base(original.effect_id, name)
            step = PerformPrimaryActionStep(hero_id=str(hero.id), step_id="fixture/perform-empty")
            state.execution_stack = [step]
            state.execution_context["perform_step"] = step
            yield "perform-primary-empty-" + original.effect_id, state, [
                {"kind": "advance_prefix", "steps": 1},
                {"kind": "advance_prefix", "steps": 0},
            ]
    yield from flow_scenes()
    yield from replacement_scenes()
    yield from edge_scenes()


def flow_scenes():
    from live_defeat_traces import operations
    from live_utility_traces import scenes as utility_scenes

    from goa2.engine.steps import PerformPrimaryActionStep

    for name, state, _ in utility_scenes():
        if not name.startswith("utility-card-") or not name.endswith("-first"):
            continue
        hero = state.get_hero(str(state.current_actor_id))
        card = hero.current_turn_card
        for variant in ("ordinary", "exclude"):
            world = copy.deepcopy(state)
            step = PerformPrimaryActionStep(
                hero_id=str(hero.id),
                exclude_target_key="excluded" if variant == "exclude" else None,
                step_id="fixture/perform-flow",
            )
            world.execution_stack[-1] = step
            world.execution_context.update(
                selected_card=card.id,
                excluded="hero_brogan",
                perform_step=step,
                current_card_id="outer",
                reperforming_card_id={"retained": True},
            )
            world.execution_context["old_reperforming"] = world.execution_context[
                "reperforming_card_id"
            ]

            def select(request, _mode):
                from goa2.domain.input import selection_value

                if request.request_type.value == "CHOOSE_ACTION":
                    return request.options[0].id
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                return selection_value(request.options[0])

            ops, _ = operations(world, select=select)
            yield "perform-primary-flow-" + name.removeprefix(
                "utility-card-"
            ) + "-" + variant, world, [*ops, {"kind": "advance_prefix", "steps": 0}]


def replacement_scenes():
    from live_defeat_traces import operations
    from live_travel_traces import captured_worlds
    from model_traces import setup

    from goa2.domain.models import CardState
    from goa2.engine.steps import PerformPrimaryActionStep, SetContextFlagStep

    original = dict(captured_worlds())["travel-sequence"]
    for effect, replacement in (
        ("liquid_leap", "FAST_TRAVEL"),
        ("noble_blade", "CLEAR"),
        ("deluge", "FAST_TRAVEL"),
    ):
        for mode in ("prompt", "primary", "replacement", "invalid"):
            state = copy.deepcopy(original)
            hero = state.get_hero("hero_arien")
            cards = setup().get_hero("hero_arien").deck
            hero.deck = cards
            hero.hand = cards
            card = next(c for c in cards if c.effect_id == effect)
            card.state = CardState.UNRESOLVED
            card.is_facedown = False
            hero.current_turn_card = card
            step = PerformPrimaryActionStep(
                hero_id="hero_arien", step_id="fixture/perform-replacement"
            )
            if mode == "invalid":
                step.pending_input = {"selection": "invalid"}
            tail = SetContextFlagStep(
                key="continued", value=True, step_id="fixture/perform-replacement-tail"
            )
            state.execution_stack = [tail, step]
            state.execution_context.update(
                selected_card=card.id,
                current_card_id="outer",
                perform_step=step,
                locations_alias=state.entity_locations,
            )

            def select(request, _mode, mode=mode, replacement=replacement):
                from goa2.domain.input import selection_value

                if request.request_type.value == "CHOOSE_ACTION":
                    return request.options[0].id if mode == "primary" else replacement
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                return selection_value(request.options[0])

            ops = (
                [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance_prefix", "steps": 0}]
                if mode == "prompt"
                else [*operations(state, select=select)[0], {"kind": "advance_prefix", "steps": 0}]
            )
            yield "perform-primary-replacement-" + effect + "-" + mode, state, ops


def edge_scenes():
    from goa2.engine.steps import PerformPrimaryActionStep, SetContextFlagStep

    for variant in (
        "missing-key",
        "missing-card",
        "missing-hero",
        "no-actor",
        "unregistered",
        "empty-effect",
        "played",
        "discard",
        "hand",
        "deck",
        "spells",
        "current-first",
    ):
        state, hero, card = base("noble_blade", "Arien")
        step = PerformPrimaryActionStep(hero_id=str(hero.id), step_id="fixture/perform-edge")
        if variant == "missing-key":
            state.execution_context.pop("selected_card")
        if variant == "missing-card":
            state.execution_context["selected_card"] = "absent"
        if variant == "missing-hero":
            step.hero_id = "absent"
        if variant == "no-actor":
            step.hero_id = None
            state.current_actor_id = None
        if variant == "unregistered":
            card.effect_id = "unregistered-effect"
        if variant == "empty-effect":
            card.effect_id = ""
        if variant in ("played", "discard", "hand", "deck", "spells"):
            hero.current_turn_card = None
            for field in ("played_cards", "discard_pile", "hand", "deck", "spells"):
                setattr(hero, field, [])
            getattr(
                hero,
                {
                    "played": "played_cards",
                    "discard": "discard_pile",
                    "hand": "hand",
                    "deck": "deck",
                    "spells": "spells",
                }[variant],
            ).append(card)
        if variant == "current-first":
            card.is_facedown = True
            visible = card.model_copy(deep=True)
            visible.is_facedown = False
            hero.hand.insert(0, visible)
        tail = SetContextFlagStep(key="tail", value=True, step_id="fixture/perform-edge-tail")
        state.execution_stack = [tail, step]
        state.execution_context["perform_step"] = step
        yield "perform-primary-edge-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]
