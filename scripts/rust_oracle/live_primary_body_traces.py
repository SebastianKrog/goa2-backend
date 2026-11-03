"""Live primary construction, nested identities, and ordinary complete flows.

Raw world construction is deliberate: these traces assert dispatch boundaries
and retained aliases independent of any particular card-effect test helper.
"""

from __future__ import annotations


def scenes(positive_only=False, *, effects=None, include_common=True):
    from aura_traces import _board
    from live_defeat_traces import retain
    from model_traces import setup
    from primary_templates import ALL_EFFECTS as EFFECTS

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import ActionType, CardState, GamePhase, StatType
    from goa2.engine.effects import CardEffect, CardEffectRegistry
    from goa2.engine.steps import ResolveCardTextStep, SetContextFlagStep

    def base(effect, name):
        foe = "Brogan" if name != "Brogan" else "Arien"
        ally = "Swift" if name != "Swift" else "Arien"
        other = "Hanu" if name != "Hanu" else "Tigerclaw"
        state = setup([name, ally], [foe, other])
        h = _board(state)
        hero = state.get_hero("hero_" + name.lower())
        card = next(
            card
            for card in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
            if card.effect_id == effect
        )
        hero.current_turn_card = card
        if card in hero.hand:
            hero.hand.remove(card)
        card.is_facedown = False
        card.state = CardState.UNRESOLVED
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = str(hero.id)
        state.place_entity("hero_razzle_piece_1" if name == "Razzle" else hero.id, h(0, 0))
        state.place_entity("hero_" + foe.lower(), h(1, 0))
        state.place_entity("hero_" + ally.lower(), h(0, 1))
        state.place_entity("hero_" + other.lower(), h(-1, 1))
        state.execution_context.update(
            hero_alias=hero, current_alias=card, deck_alias=hero.deck, current_card_id="old-card"
        )
        return state, hero, card

    def construction(state, hero, variant):
        step = ResolveCardTextStep(
            hero_id=str(hero.id), card_id="ignored-literal", step_id="fixture/primary-text"
        )
        if variant == "missing-hero":
            step.hero_id = "absent"
        if variant == "skip":
            step.skip_if_key = "skip"
            state.execution_context["skip"] = True
        if variant == "inactive":
            step.active_if_key = "enabled"
        tail = SetContextFlagStep(key="tail", value=True, step_id="fixture/primary-tail")
        state.execution_stack = [tail, step]
        state.execution_context["stored_primary_step"] = step
        return [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance_prefix", "steps": 0}]

    if not positive_only:
        for effect, name in (EFFECTS if effects is None else effects).items():
            for variant in ("ordinary", "items", "ranged", "facedown", "skip"):
                state, hero, card = base(effect, name)
                if variant == "items":
                    hero.items = {
                        StatType.ATTACK: -7,
                        StatType.MOVEMENT: 2,
                        StatType.RANGE: 3,
                        StatType.RADIUS: 1,
                    }
                if variant == "ranged":
                    card.is_ranged = not card.is_ranged
                if variant == "facedown":
                    card.is_facedown = True
                ops = construction(state, hero, variant)
                yield "primary-text-" + effect + "-" + variant, state, ops

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
                    effect is None
                    or type(effect).build_steps is not CardEffect.build_steps
                    or original.effect_id in emitted
                ):
                    continue
                emitted.add(original.effect_id)
                state, hero, _ = base(original.effect_id, name)
                yield "primary-text-empty-" + original.effect_id, state, construction(
                    state, hero, "ordinary"
                )

        for variant in (
            "missing-hero",
            "missing-current",
            "inactive",
            "no-effect",
            "empty-effect",
            "movement",
            "attack",
            "defense",
            "defense-skill",
            "skill",
            "no-primary",
        ):
            state, hero, card = base("liquid_leap", "Arien")
            if variant == "missing-current":
                hero.current_turn_card = None
            if variant == "no-effect":
                card.effect_id = None
            if variant == "empty-effect":
                card.effect_id = ""
            if variant in {"movement", "attack", "defense", "defense-skill", "skill", "no-primary"}:
                card.effect_id = "unregistered-primary"
                card.primary_action = (
                    None
                    if variant == "no-primary"
                    else ActionType(variant.replace("-", "_").upper())
                )
                card.primary_action_value = -3
                card.range_value = 0
            yield "primary-text-fallback-" + variant, state, construction(state, hero, variant)

    complete = {
        "arcane_whirlpool",
        "liquid_leap",
        "magical_current",
        "stranger_tide",
        "poisoned_dagger",
        "poisoned_dart",
        "back_to_back",
        "listen_up",
        "roger_roger",
        "ready_and_waiting",
        "close_support",
        "covering_fire",
        "shield_bash",
        "counterattack",
        "steady_advance",
        "unwavering_resolve",
        "devoted_followers",
        "fresh_converts",
    }
    if not positive_only:
        for effect in sorted(complete):
            for mode in ("first", "last", "skip", "invalid"):
                state, hero, _ = base(effect, EFFECTS[effect])
                step = ResolveCardTextStep(hero_id=str(hero.id), card_id="ignored-literal")
                ops, _ = retain(state, [step], mode)
                yield "primary-text-flow-" + effect + "-" + mode, state, ops

    for effect in sorted(complete):
        state, hero, card = base(effect, EFFECTS[effect])
        from goa2.domain.hex import Hex
        from goa2.domain.models import TeamColor

        def h(q, r):
            return Hex(q=q, r=r, s=-q - r)

        friendly = state.teams[hero.team]
        enemy = state.teams[TeamColor.BLUE]
        for minion, point in zip(friendly.minions, [h(-1, 0), h(0, -1), h(-2, 0)], strict=False):
            state.place_entity(minion.id, point)
        state.place_entity(enemy.minions[0].id, h(1, -1))
        for team in state.teams.values():
            for other in team.heroes:
                other.hand = [c for c in other.deck if c.is_basic and c is not card]
                for c in other.hand:
                    c.state = CardState.HAND
                    c.is_facedown = False
        retrieved = next(c for c in hero.deck if c is not card and not c.is_basic)
        retrieved.state = CardState.DISCARD
        retrieved.is_facedown = False
        hero.discard_pile.append(retrieved)
        step = ResolveCardTextStep(hero_id=str(hero.id), card_id="ignored-literal")
        ops, _ = retain(state, [step], "first")
        yield "primary-text-positive-" + effect, state, ops
