"""Brogan bodies and live query boundaries against the frozen source.

Raw setup retains card, collection and step aliases for object identity checks.
"""

from __future__ import annotations

EFFECTS = (
    "bullrush",
    "furious_charge",
    "throwing_axe",
    "throwing_spear",
    "shield",
    "bolster",
    "fortify",
    "war_drummer",
    "master_skald",
    "bulwark",
    "one_man_army",
)


def base(effect, variant="ordinary"):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, StatType

    state = setup(["Brogan", "Hanu"], ["Arien", "Tigerclaw"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.battle_zones = {}
    state.current_actor_id = "hero_brogan"
    hero = state.get_hero("hero_brogan")
    card = next(c for c in [*hero.deck, hero.ultimate_card] if c and c.effect_id == effect)
    hero.current_turn_card = card
    card.state = CardState.UNRESOLVED
    card.is_facedown = False
    if card in hero.hand:
        hero.hand.remove(card)
    for name, point in (
        (hero.id, h(0, 0)),
        ("hero_hanu", h(0, 1)),
        ("hero_arien", h(3, 0)),
        ("hero_tigerclaw", h(1, -1)),
    ):
        state.place_entity(name, point)
    state.place_entity(state.teams["BLUE"].minions[0].id, h(3, -1))
    spare = next(c for c in hero.hand if c is not card)
    if variant in ("existing-discard", "retrieve"):
        hero.hand.remove(spare)
        hero.discard_pile.append(spare)
    if variant == "defeated":
        state.heroes_defeated_this_round.append("hero_arien")
    if variant == "empty-defeat-mark":
        state.heroes_defeated_this_round.append("")
    if variant.startswith("radius-"):
        card.radius_value = {"radius-none": None, "radius-zero": 0, "radius-negative": -1}[variant]
    if variant == "negative-attack":
        hero.items = {StatType.ATTACK: -9}
    if variant == "blocked":
        state.place_entity("hero_hanu", h(1, 0))
        state.remove_entity("hero_tigerclaw")
        state.remove_entity(state.teams["BLUE"].minions[0].id)
    if variant == "no-enemy":
        for entity in list(state.entity_locations):
            if entity not in (hero.id, "hero_hanu"):
                state.remove_entity(entity)
    if variant == "long-charge":
        state.place_entity("hero_arien", h(4, 0))
        state.remove_entity("hero_tigerclaw")
        state.remove_entity(state.teams["BLUE"].minions[0].id)
    state.execution_context.update(
        hero_alias=hero,
        card_alias=card,
        hand_alias=hero.hand,
        discard_alias=hero.discard_pile,
        effects_alias=state.active_effects,
        defeat_alias=state.heroes_defeated_this_round,
        heroes_defeated_this_round={"stale": 99},
    )
    return state, hero, card


def action_scene(effect, entry, mode, variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
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
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/brogan-primary"
    state.execution_stack = [
        SetContextFlagStep(key="brogan_tail", value=True, step_id="fixture/brogan-tail"),
        step,
    ]
    state.execution_context.update(selected_card=card.id, brogan_step=step)
    bad = False

    def select(request, _mode):
        nonlocal bad
        if mode == "invalid" and not bad:
            bad = True
            return "not-offered"
        kind = request.request_type.value
        if kind == "CHOOSE_ACTION":
            return card.current_primary_action.value
        if kind == "SELECT_CARD_OR_PASS":
            return "PASS"
        if (
            kind == "SELECT_NUMBER"
            and effect in ("throwing_axe", "throwing_spear")
            and "mode" in request.prompt
        ):
            return 2 if mode in ("last", "skip", "ranged") else 1
        if request.can_skip and (mode == "skip" or variant == "existing-discard"):
            return "SKIP"
        if kind == "SELECT_UNIT":
            for target in ("hero_hanu", "hero_arien", "hero_tigerclaw"):
                if any(option.id == target for option in request.options):
                    return target
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, final = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}], final


def scenes():
    for effect in EFFECTS:
        for entry in ("text", "action", "perform"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode)
                yield f"brogan-primary-{effect}-{entry}-{mode}", state, ops
    for effect in EFFECTS:
        variants = []
        if effect in ("bullrush", "furious_charge"):
            variants += ["blocked", "no-enemy", "long-charge", "negative-attack"]
        if effect in ("throwing_axe", "throwing_spear"):
            variants += ["existing-discard", "negative-attack"]
        if effect in ("shield", "bolster", "fortify", "bulwark"):
            variants += ["radius-none", "radius-zero", "radius-negative"]
        if effect in ("war_drummer", "master_skald"):
            variants += ["defeated", "empty-defeat-mark", "no-enemy"]
        if effect == "bulwark":
            variants += ["retrieve"]
        for variant in variants:
            for mode in ("first", "last", "skip"):
                state, ops, _ = action_scene(effect, "text", mode, variant)
                yield f"brogan-primary-{effect}-{variant}-{mode}", state, ops


def query_scenes():
    from live_defeat_traces import retain

    from goa2.domain.models.enums import CardContainerType
    from goa2.engine.steps import CheckHeroDefeatedThisRoundStep, CountCardsStep

    for container in CardContainerType:
        for variant in (
            "actor",
            "literal",
            "key",
            "empty-key",
            "unknown",
            "no-actor",
            "skip",
            "inactive",
        ):
            state, hero, card = base("throwing_spear", "existing-discard")
            hero.played_cards[:] = [card, None, card, None]
            step = CountCardsStep(card_container=container, output_key="query_result")
            state.execution_context.update(
                query_result="retained", chosen_hero="hero_hanu", played_alias=hero.played_cards
            )
            if variant == "literal":
                step.hero_id = hero.id
                step.hero_key = "chosen_hero"
            if variant in ("key", "empty-key"):
                step.hero_key = "chosen_hero"
                if variant == "empty-key":
                    state.execution_context["chosen_hero"] = ""
            if variant == "unknown":
                step.hero_id = "absent"
            if variant == "no-actor":
                state.current_actor_id = None
            if variant == "skip":
                step.skip_if_key = "chosen_hero"
            if variant == "inactive":
                step.active_if_key = "absent"
            ops, _ = retain(state, [step])
            yield f"brogan-query-count-{container.value}-{variant}", state, ops
    for variant in ("empty", "nonempty", "zero", "skip", "inactive"):
        state, _, _ = base("war_drummer")
        step = CheckHeroDefeatedThisRoundStep(output_key="query_result")
        state.execution_context.update(query_result="retained", enabled=True)
        if variant in ("nonempty", "zero"):
            state.heroes_defeated_this_round.append("hero_arien" if variant == "nonempty" else "")
        if variant == "skip":
            step.skip_if_key = "enabled"
        if variant == "inactive":
            step.active_if_key = "absent"
        ops, _ = retain(state, [step])
        yield f"brogan-query-defeat-{variant}", state, ops


def protection_scenes():
    from live_defeat_traces import retain

    from goa2.domain.hex import Hex
    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
        CheckMinionProtectionStep,
        PushUnitStep,
        ResolveCardTextStep,
        SetActorStep,
    )

    for effect in ("shield", "bolster", "fortify"):
        for mode in ("protect", "skip", "gold-only", "no-cards"):
            state, hero, card = base(effect)
            minion = state.teams[hero.team].minions[0]
            state.place_entity(minion.id, Hex(q=0, r=-1, s=1))
            if mode == "gold-only":
                hero.hand[:] = [c for c in hero.hand if c.color and c.color.value == "GOLD"]
            if mode == "no-cards":
                hero.hand.clear()
            steps = [
                ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
                CheckMinionProtectionStep(minion_id=minion.id, killer_id="hero_arien"),
            ]

            def select(request, _mode, mode=mode):
                return (
                    "SKIP"
                    if mode == "skip" and request.can_skip
                    else selection_value(request.options[0])
                )

            ops, _ = retain(state, steps, select=select)
            yield f"brogan-protection-{effect}-{mode}", state, ops
    for active in (False, True):
        for actor in ("hero_arien", "hero_hanu", "hero_brogan"):
            for target in ("hero_brogan", "hero_hanu"):
                state, hero, card = base("bulwark")
                steps = [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)]
                if active:
                    from goa2.engine.effect_manager import EffectManager

                    _, _, state = action_scene("bulwark", "text", "first")
                    hero = state.get_hero("hero_brogan")
                    card = hero.current_turn_card
                    hero.resolve_current_card(turn_number=state.turn)
                    EffectManager.activate_effects_by_card(state, card.id)
                    steps = []
                steps += [
                    SetActorStep(actor_id=actor),
                    PushUnitStep(target_id=target, source_hex=Hex(q=1, r=0, s=-1), distance=1),
                ]
                ops, _ = retain(state, steps)
                phase = "active" if active else "dormant"
                yield f"brogan-protection-bulwark-{phase}-{actor}-{target}", state, ops
