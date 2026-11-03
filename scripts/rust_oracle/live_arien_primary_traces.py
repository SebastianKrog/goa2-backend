"""Complete active Arien primary flows and direct behind-target predicates.

Raw states retain card/position/effect aliases and isolate instruction boundaries
rather than hiding source model identity behind a card-test fixture helper.
"""

from __future__ import annotations

EFFECTS = (
    "spell_break",
    "ebb_and_flow",
    "dangerous_current",
    "raging_stream",
    "violent_torrent",
    "slippery_ground",
    "deluge",
)


def state_for(effect):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, TeamColor

    state = setup(["Arien", "Hanu"], ["Brogan", "Tigerclaw"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.battle_zones = {}
    state.current_actor_id = "hero_arien"
    hero = state.get_hero("hero_arien")
    card = next(c for c in hero.deck if c.effect_id == effect)
    hero.current_turn_card = card
    card.state = CardState.UNRESOLVED
    card.is_facedown = False
    if card in hero.hand:
        hero.hand.remove(card)
    state.place_entity("hero_arien", h(0, 0))
    state.place_entity("hero_hanu", h(-1, 1))
    state.place_entity("hero_brogan", h(2, 0))
    state.place_entity("hero_tigerclaw", h(3, 0))
    enemy = state.teams[TeamColor.BLUE]
    state.place_entity(enemy.minions[0].id, h(1, 0))
    state.place_entity(enemy.minions[1].id, h(1, -1))
    state.execution_context.update(
        card_alias=card,
        hero_alias=hero,
        locations_alias=state.entity_locations,
        effects_alias=state.active_effects,
    )
    return state, hero, card, h


def scenes():
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SetContextFlagStep,
    )

    for effect in EFFECTS:
        for entry in ("text", "action", "perform"):
            for mode in ("first", "last", "skip", "invalid"):
                state, hero, card, _h = state_for(effect)
                if entry == "text":
                    step = ResolveCardTextStep(hero_id=str(hero.id), card_id=card.id)
                elif entry == "action":
                    step = ResolveCardStep(hero_id=str(hero.id))
                else:
                    step = PerformPrimaryActionStep(hero_id=str(hero.id))
                step.step_id = "fixture/arien-primary"
                tail = SetContextFlagStep(
                    key="arien_tail", value=True, step_id="fixture/arien-tail"
                )
                state.execution_stack = [tail, step]
                state.execution_context.update(selected_card=card.id, source_step=step)
                bad = False

                def select(request, _mode, card=card, mode=mode):
                    nonlocal bad
                    if mode == "invalid" and not bad:
                        bad = True
                        return "not-offered"
                    if request.request_type.value == "CHOOSE_ACTION":
                        return card.current_primary_action.value
                    if request.request_type.value == "SELECT_CARD_OR_PASS":
                        return "PASS"
                    if mode == "skip" and request.can_skip:
                        return "SKIP"
                    return selection_value(
                        request.options[-1] if mode == "last" else request.options[0]
                    )

                ops, _ = operations(state, select=select)
                yield "arien-primary-" + effect + "-" + entry + "-" + mode, state, [
                    *ops,
                    {"kind": "advance_prefix", "steps": 0},
                ]
    for effect, hero_name in (
        ("spell_break", "Arien"),
        ("deluge", "Arien"),
        ("devoted_followers", "Xargatha"),
        ("fresh_converts", "Xargatha"),
    ):
        from live_perform_primary_traces import base

        for radius in (None, 0):
            for entry in ("text", "perform"):
                state, hero, card = base(effect, hero_name)
                card.radius_value = radius
                step = (
                    ResolveCardTextStep(hero_id=str(hero.id), card_id=card.id)
                    if entry == "text"
                    else PerformPrimaryActionStep(hero_id=str(hero.id))
                )
                step.step_id = "fixture/arien-radius"
                state.execution_stack = [step]
                state.execution_context["source_step"] = step
                yield "arien-primary-radius-" + effect + "-" + str(radius) + "-" + entry, state, [
                    {"kind": "advance_prefix", "steps": 1},
                    {"kind": "advance_prefix", "steps": 0},
                ]

    yield from line_scenes()


def line_scenes():
    from goa2.domain.models import TargetType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.filters_geometry import LineBehindTargetFilter
    from goa2.engine.steps import SelectStep

    for direction in range(6):
        for target_type in (TargetType.UNIT, TargetType.HEX):
            for length in (1, 2):
                state, hero, _card, h = state_for("dangerous_current")
                for entity in list(state.entity_locations):
                    state.remove_entity(entity)
                target = state.teams["BLUE"].minions[0].id
                point = h(0, 0).neighbor(direction)
                state.place_entity(hero.id, h(0, 0))
                state.place_entity(target, point)
                state.place_entity("hero_brogan", point.neighbor(direction))
                state.place_entity("hero_tigerclaw", point.neighbor(direction).neighbor(direction))
                step = SelectStep(
                    prompt="Select behind target",
                    target_type=target_type,
                    filters=[LineBehindTargetFilter(target_key="anchor", length=length)],
                    output_key="behind",
                    step_id="fixture/arien-line",
                )
                state.execution_context.update(anchor=target, source_step=step)
                state.execution_stack = [step]
                yield f"arien-primary-line-direction-{direction}-{target_type.value}-{length}", state, [
                    {"kind": "advance_prefix", "steps": 1},
                    {"kind": "advance_prefix", "steps": 0},
                ]
    for variant in (
        "zero",
        "negative",
        "no-target",
        "unknown-target",
        "hex-target",
        "no-origin",
        "origin-empty",
        "origin-unknown",
        "split-before",
        "split-after",
        "not-straight",
        "on-target",
    ):
        state, hero, _card, h = state_for("dangerous_current")
        for entity in list(state.entity_locations):
            state.remove_entity(entity)
        target = state.teams["BLUE"].minions[0].id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(target, h(1, 0))
        state.place_entity("hero_brogan", h(2, 0))
        state.place_entity("hero_tigerclaw", h(3, 0))
        state.execution_context["anchor"] = target
        predicate = LineBehindTargetFilter(target_key="anchor", length=2)
        if variant in ("zero", "negative"):
            predicate.length = 0 if variant == "zero" else -1
        if variant == "no-target":
            state.execution_context.pop("anchor")
        if variant == "unknown-target":
            state.execution_context["anchor"] = "absent"
        if variant == "hex-target":
            state.execution_context["anchor"] = h(1, 0)
        if variant == "no-origin":
            state.current_actor_id = None
        if variant == "origin-empty":
            predicate.origin_id = ""
        if variant == "origin-unknown":
            predicate.origin_id = "absent"
        if variant == "not-straight":
            state.move_unit(target, h(1, 1))
        if variant == "on-target":
            state.execution_context["anchor"] = "hero_brogan"
        if variant.startswith("split-"):
            state.active_effects.append(
                ActiveEffect(
                    id="line-split",
                    source_id="hero_arien",
                    effect_type=EffectType.TOPOLOGY_SPLIT,
                    scope=EffectScope(shape=Shape.GLOBAL),
                    duration=DurationType.THIS_TURN,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                    is_active=True,
                    split_axis="q",
                    split_value=0 if variant == "split-before" else 1,
                )
            )
        step = SelectStep(
            prompt="Select behind target",
            target_type=TargetType.HEX,
            filters=[predicate],
            output_key="behind",
            step_id="fixture/arien-line-edge",
        )
        state.execution_context["source_step"] = step
        state.execution_stack = [step]
        yield "arien-primary-line-edge-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]
