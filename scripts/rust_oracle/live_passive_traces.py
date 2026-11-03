"""Live passive primitive scenarios built from registered cards and source rules."""

from __future__ import annotations


def build(name):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import GamePhase, TeamColor

    others = [n for n in ("Arien", "Hanu", "Brogan", "Tigerclaw") if n != name]
    state = setup([name, others[0]], others[1:3])
    state.phase = GamePhase.RESOLUTION
    hero = state.teams[TeamColor.RED].heroes[0]
    state.current_actor_id = hero.id
    point = _board(state)(0, 0)
    state.place_entity(hero.id, point)
    return state, hero


def enable(state, hero, card, effect_id):
    from combat_effect_traces import effect

    from goa2.domain.models import CardState, RuneType, TeamColor
    from goa2.domain.models.effect import EffectType

    context = state.execution_context
    if effect_id == "legend_of_the_skies":
        state.turn = 3
        hero.played_cards = [None, hero.hand[-1]]
    if effect_id == "bullet_time":
        context["basic_action_card_id"] = "spent"
    if effect_id == "rune_mastery":
        hero.rune_slots = {1: RuneType.AXE}
    if effect_id == "the_archwizard":
        context["current_card_id"] = "prepare_spells"
        next(spell for spell in hero.spells if spell.id == "wish").state = CardState.SPELLBOOK
    if effect_id == "battle_fury":
        discarded = next(c for c in hero.hand if c.primary_action is not None)
        discarded.state, discarded.is_facedown = CardState.DISCARD, False
        hero.discard_pile = [discarded]
        context.update(
            discard_source="PLAYED",
            discarded_card_owner_id=str(hero.id),
            discarded_card_id=discarded.id,
        )
    if effect_id == "dragon_knight":
        from goa2.domain.models import ActionType

        skill = next(c for c in hero.deck if c.primary_action == ActionType.SKILL)
        skill.is_facedown = False
        hero.played_cards = [skill]
    if effect_id == "reign_of_winter":
        victim = state.teams[TeamColor.BLUE].minions[0]
        context.update(
            attack_card_id="ice_blast",
            ice_blast_victim=str(victim.id),
            defeat_marks={"ice_blast_victim": 1},
            defeat_log=[("old", str(hero.id)), (str(victim.id), None)],
        )
    if effect_id == "jinx":
        context["discarded_card_owner_id"] = str(hero.id)
        state.active_effects = [
            effect(
                state,
                "jinx-provider",
                EffectType.DELAYED_TRIGGER,
                source=str(hero.id),
                source_card_id=card.id,
            )
        ]


def retain(state, step):
    state.execution_stack = [step]
    state.execution_context.update(
        step_alias=step, pending_alias=step.pending_input, effects_alias=state.active_effects
    )
    return [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance_prefix", "steps": 0}]


def scenes():
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import ActionType, CardState
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.steps import CheckPassiveAbilitiesStep, MarkPassiveUsedStep, OfferPassiveStep

    setup()
    # Every configured registered passive, not just the custom predicates.
    matrix = []
    for name, hero in sorted(HeroRegistry._heroes.items()):
        for card in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]:
            effect = CardEffectRegistry.get(card.effect_id)
            if effect:
                for config in effect.get_passive_configs():
                    matrix.append((name, card.id, card.effect_id, config))
    for name, printed, effect_id, config in matrix:
        for variant in ("enabled", "empty-context", "facedown", "used", "alias"):
            state, hero = build(name)
            card = next(
                c for c in [*hero.deck, hero.ultimate_card] if c is not None and c.id == printed
            )
            hero.level = 8
            card.state, card.is_facedown = CardState.RESOLVED, False
            if card is not hero.ultimate_card:
                hero.played_cards = [card]
            enable(state, hero, card, effect_id)
            if variant == "empty-context":
                state.execution_context.clear()
                hero.rune_slots.clear()
            if variant == "facedown":
                card.is_facedown = True
            if variant == "used":
                card.passive_uses_this_turn = max(1, config.uses_per_turn)
            if variant == "alias":
                hero.played_cards = [card, card]
                hero.ultimate_card = None
            step = CheckPassiveAbilitiesStep(
                step_id="fixture/live-passive/scan", trigger=config.trigger.value
            )
            yield f"passive-scan-{effect_id}-{config.trigger.value}-{variant}", state, retain(
                state, step
            )
        # Direct offers preserve pending data on decline; they do not recheck
        # scan eligibility or usage limits.
        for choice in ("request", "NO", "missing-selection", "numeric"):
            state, hero = build(name)
            card = next(
                c for c in [*hero.deck, hero.ultimate_card] if c is not None and c.id == printed
            )
            card.state, card.is_facedown = CardState.RESOLVED, False
            if card is not hero.ultimate_card:
                hero.played_cards = [card]
            step = OfferPassiveStep(
                step_id="fixture/live-passive/offer",
                card_id=printed,
                trigger=config.trigger.value,
                prompt="Live passive?",
                is_optional=True,
            )
            if choice == "NO":
                step.pending_input = {"selection": "NO"}
            if choice == "missing-selection":
                step.pending_input = {"other": True}
            if choice == "numeric":
                step.pending_input = {"selection": 0}
            yield f"passive-offer-{effect_id}-{config.trigger.value}-{choice}", state, retain(
                state, step
            )

    # Targeted negative/legality branches for each custom runtime predicate.
    targets = [
        (
            "Cordelia",
            "jinx",
            "after_card_discard",
            ("wrong-owner", "inactive", "offboard", "capped", "surrounded"),
        ),
        (
            "Garrus",
            "battle_fury",
            "after_card_discard",
            ("wrong-source", "wrong-owner", "hidden", "blocked"),
        ),
        ("Widget", "dragon_knight", "after_movement", ("hidden", "blocked")),
        ("Snorri", "rune_mastery", "before_action", ("defense",)),
        (
            "Gydion",
            "the_archwizard",
            "before_prepare_spellbook",
            ("wrong-actor", "wrong-card", "spent-wish", "performing-overrides"),
        ),
        (
            "Tali",
            "reign_of_winter",
            "after_attack",
            ("wrong-card", "hero-target", "old-defeat", "no-mark", "negative-mark", "boolean-mark"),
        ),
    ]
    from combat_effect_traces import effect

    from goa2.domain.models.effect import EffectType

    for name, effect_id, trigger, variants in targets:
        for variant in variants:
            state, hero = build(name)
            card = next(
                c
                for c in [*hero.deck, hero.ultimate_card]
                if c is not None and c.effect_id == effect_id
            )
            hero.level = 8
            card.state, card.is_facedown = CardState.RESOLVED, False
            if card is not hero.ultimate_card:
                hero.played_cards = [card]
            enable(state, hero, card, effect_id)
            ctx = state.execution_context
            if variant == "wrong-owner":
                ctx["discarded_card_owner_id"] = "hero_missing"
            if variant == "wrong-source":
                ctx["discard_source"] = "HAND"
            if variant == "inactive":
                state.active_effects[0].is_active = False
            if variant == "offboard":
                state.remove_entity(hero.id)
            if variant == "capped":
                state.active_effects.append(
                    effect(state, "cap", EffectType.MOVEMENT_ZONE, max_value=0)
                )
            if variant == "surrounded":
                from goa2.domain.board import Board

                point = state.get_position(hero.id)
                state.board = Board(tiles={point: state.board.tiles[point]})
            if variant == "hidden":
                if effect_id == "battle_fury":
                    hero.discard_pile[0].is_facedown = True
                else:
                    for c in hero.played_cards:
                        if c:
                            c.is_facedown = True
            if variant == "blocked":
                state.active_effects.append(
                    effect(
                        state,
                        "block",
                        EffectType.TARGET_PREVENTION,
                        source="hero_hanu",
                        restrictions=[
                            ActionType.SKILL,
                            ActionType.ATTACK,
                            ActionType.MOVEMENT,
                            ActionType.DEFENSE,
                        ],
                    )
                )
            if variant == "defense":
                ctx["current_action_type"] = "DEFENSE"
            if variant == "wrong-actor":
                state.current_actor_id = "hero_arien"
            if variant == "wrong-card":
                ctx["current_card_id" if effect_id == "the_archwizard" else "attack_card_id"] = (
                    "wrong"
                )
            if variant == "performing-overrides":
                ctx["performing_card_id"] = "wrong"
            if variant == "spent-wish":
                next(c for c in hero.spells if c.id == "wish").state = CardState.OUTSIDE_SPELLBOOK
            if variant == "hero-target":
                ctx["ice_blast_victim"] = str(hero.id)
            if variant == "old-defeat":
                ctx["defeat_marks"]["ice_blast_victim"] = 2
            if variant == "no-mark":
                ctx["defeat_marks"].clear()
            if variant == "negative-mark":
                ctx["defeat_marks"]["ice_blast_victim"] = -1
            if variant == "boolean-mark":
                ctx["defeat_marks"]["ice_blast_victim"] = True
            step = CheckPassiveAbilitiesStep(step_id="fixture/live-passive/custom", trigger=trigger)
            yield f"passive-predicate-{effect_id}-{variant}", state, retain(state, step)

    from passive_provider_traces import state_for

    variants = (
        "plain",
        "inactive",
        "wrong-source",
        "missing-card",
        "unknown-grant",
        "empty-grant",
        "no-grant",
        "used",
        "negative-uses",
        "facedown-carrier",
        "duplicate-provider",
        "scope-ignored",
        "old-active",
        "empty-provider-id",
        "no-actor",
        "ordinary-dedup",
        "ordinary-facedown",
        "ordinary-low-level",
    )
    for variant in variants:
        for kind in ("scan", "offer", "mark"):
            state, _owner, card, provider = state_for(variant)
            if kind == "scan":
                step = CheckPassiveAbilitiesStep(
                    step_id="fixture/live-passive/provider", trigger="before_action"
                )
            elif kind == "offer":
                step = OfferPassiveStep(
                    step_id="fixture/live-passive/provider",
                    card_id=provider.source_card_id or card.id,
                    trigger="before_action",
                    active_effect_id=provider.id,
                    pending_input={"selection": "NO"},
                )
            else:
                step = MarkPassiveUsedStep(
                    step_id="fixture/live-passive/provider",
                    card_id=provider.source_card_id or card.id,
                    active_effect_id=provider.id,
                )
            yield f"passive-provider-{kind}-{variant}", state, retain(state, step)
    for alias in (False, True):
        state, _owner, card, provider = state_for("plain")
        state.active_effects.append(provider if alias else provider.model_copy(deep=True))
        step = MarkPassiveUsedStep(
            step_id="fixture/live-passive/mark", card_id=card.id, active_effect_id=provider.id
        )
        yield f"passive-mark-provider-alias-{alias}", state, retain(state, step)
    for variant in ("played", "ultimate", "missing", "hidden", "no-actor", "duplicate-card"):
        state, hero = build("Sabina")
        card = hero.ultimate_card
        if variant != "ultimate":
            hero.played_cards = [card]
        if variant == "hidden":
            card.is_facedown = True
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "duplicate-card":
            hero.played_cards = [card, card.model_copy(deep=True)]
        step = MarkPassiveUsedStep(
            step_id="fixture/live-passive/mark",
            card_id="missing" if variant == "missing" else card.id,
        )
        state.execution_context["card_alias"] = card
        yield f"passive-mark-{variant}", state, retain(state, step)
