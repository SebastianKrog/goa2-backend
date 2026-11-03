"""Gydion's ordinary spell-school cards and preparation/Archwizard flows.

Raw setup retains physical card, spellbook, effect-list and step identities.
Every entry point executes the frozen source; copied access cards use a foreign
performer while the physical ordinary card and spellbook remain Gydion's.
"""

from __future__ import annotations

from primary_templates import GYDION_EFFECTS as EFFECTS


def base(effect, variant="ordinary"):
    from live_spell_body_traces import base as spell_base

    from goa2.domain.models import CardState

    state, hero, _ = spell_base("magic_missile")
    card = next(c for c in hero.deck if c.effect_id == effect)
    hero.current_turn_card = card
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    if card in hero.hand:
        hero.hand.remove(card)
    if effect == "prepare_spells" and variant != "archwizard-all-prepared":
        for spell in hero.spells[:2]:
            spell.state, spell.is_facedown = CardState.OUTSIDE_SPELLBOOK, False
    if variant == "spent":
        for spell in hero.spells:
            spell.state, spell.is_facedown = CardState.OUTSIDE_SPELLBOOK, False
    if variant == "no-owner":
        hero.spells.clear()
    if variant.startswith("archwizard-"):
        hero.level = 8
        hero.ultimate_card.state = CardState.RESOLVED
        hero.ultimate_card.is_facedown = False
        if variant == "archwizard-spent-wish":
            next(s for s in hero.spells if s.id == "wish").state = CardState.OUTSIDE_SPELLBOOK
        if variant in ("archwizard-third", "archwizard-third-fizzle", "archwizard-third-no-other"):
            hero.wish_cast_count = 2
        if variant in ("archwizard-third-fizzle", "archwizard-third-no-other"):
            for spell in hero.spells:
                if spell.id != "wish" and not (
                    variant == "archwizard-third-fizzle" and spell.id == "magic_missile"
                ):
                    spell.state, spell.is_facedown = CardState.OUTSIDE_SPELLBOOK, False
            if variant == "archwizard-third-fizzle":
                for entity in [*state.teams["BLUE"].heroes, *state.teams["BLUE"].minions]:
                    state.remove_entity(entity.id)
        if variant == "archwizard-wrong-actor":
            state.current_actor_id = "hero_hanu"
    state.execution_context.update(
        ordinary_alias=card,
        ordinary_hand_alias=hero.hand,
        ordinary_ultimate_alias=hero.ultimate_card,
        selected_card=card.id,
    )
    return state, hero, card


def action_scene(effect, entry="text", mode="first", variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
        PerformCardActionStep,
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SetContextFlagStep,
    )

    state, hero, card = base(effect, variant)
    if entry == "text":
        step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
    elif entry == "action":
        step = ResolveCardStep(hero_id=hero.id)
    elif entry == "copy":
        state.current_actor_id = "hero_arien"
        state.execution_context.update(copied_ordinary=card.id, copied_owner=hero.id)
        step = PerformCardActionStep(
            card_key="copied_ordinary",
            card_owner_key="copied_owner",
            hero_id="hero_arien",
            suppress_after_resolve_card=True,
        )
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/gydion-primary"
    state.execution_stack = [
        SetContextFlagStep(key="gydion_tail", value=True, step_id="fixture/gydion-tail"),
        step,
    ]
    state.execution_context["gydion_step"] = step
    bad = False
    selected_spell = None

    def select(request, _mode):
        nonlocal bad, selected_spell
        if mode == "invalid" and not bad:
            bad = True
            return "not-offered"
        kind = request.request_type.value
        if kind == "CONFIRM_PASSIVE":
            return "NO" if variant == "archwizard-no" else "YES"
        if kind == "SELECT_CARD" and request.prompt == "Choose a prepared spell to cast":
            options = request.options
            if variant.startswith("archwizard-"):
                for wanted in ("wish", "expeditious_retreat", "magic_missile"):
                    if any(o.id == wanted for o in options):
                        selected_spell = wanted
                        return wanted
            selected_spell = options[-1].id if mode == "last" else options[0].id
            return selected_spell
        if kind == "CHOOSE_ACTION":
            chosen = next((s for s in hero.spells if s.id == selected_spell), card)
            primary = chosen.primary_action.value
            return "SKILL" if primary == "DEFENSE_SKILL" else primary
        if kind == "SELECT_CARD_OR_PASS":
            return "PASS"
        if request.can_skip and mode == "skip":
            return "SKIP"
        if kind in ("SELECT_UNIT", "SELECT_UNIT_OR_TOKEN"):
            for target in ("hero_arien", "hero_tigerclaw", "hero_hanu", "hero_gydion"):
                if any(o.id == target for o in request.options):
                    return target
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, final = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}], final


def scenes():
    for effect in EFFECTS:
        for entry in ("text", "action", "perform", "copy"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode)
                yield f"gydion-primary-{effect}-{entry}-{mode}", state, ops
    for effect in EFFECTS:
        for variant in ("spent", "no-owner"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"gydion-primary-{effect}-{variant}", state, ops
    for entry in ("text", "action", "perform", "copy"):
        for variant in (
            "archwizard-yes",
            "archwizard-no",
            "archwizard-all-prepared",
            "archwizard-spent-wish",
            "archwizard-wrong-actor",
            "archwizard-third",
            "archwizard-third-fizzle",
            "archwizard-third-no-other",
        ):
            state, ops, _ = action_scene("prepare_spells", entry, variant=variant)
            yield f"gydion-primary-prepare-{entry}-{variant}", state, ops
