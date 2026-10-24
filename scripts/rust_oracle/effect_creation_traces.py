"""Real EffectManager creation, deduplication, origin and activation cases."""

from __future__ import annotations


def add_creation_cases(add):
    from model_traces import setup

    from goa2.domain.hex import Hex
    from goa2.domain.models import ActionType, CardTier
    from goa2.domain.models.effect import (
        ActiveEffect,
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )
    from goa2.engine.steps import SetContextFlagStep

    def build():
        state = setup(["Razzle", "Arien"], ["Brogan", "Hanu"])
        state.round, state.turn = 3, 2
        state.current_actor_id = "hero_arien"
        state.acting_piece_id = state.get_piece_ids("hero_razzle")[0]
        state.next_entity_id = 117
        return state

    def template(**kwargs):
        values = dict(
            id="template",
            source_id="hero_arien",
            effect_type=EffectType.DELAYED_TRIGGER,
            scope=EffectScope(shape=Shape.GLOBAL),
            duration=DurationType.THIS_ROUND,
            created_at_turn=0,
            created_at_round=0,
            is_active=False,
            max_value=3,
            finishing_steps=[
                SetContextFlagStep(key="expiry", value=True, step_id="fixture/creation-finishing")
            ],
        )
        values.update(kwargs)
        return ActiveEffect(**values).model_dump(mode="json")

    def run(name, state, ops):
        add("creation-" + name, lambda: state, ops)

    def create(value):
        return {"kind": "create", "effect": value}

    restore = {"kind": "restore"}
    active = {"kind": "active"}
    for mode in (
        "hand",
        "unresolved",
        "resolved",
        "deck",
        "discard",
        "extra-only",
        "missing",
        "cardless",
        "empty",
        "foreign",
        "global-shadow",
    ):
        for enabled in (False, True):
            state = build()
            hero = state.get_hero("hero_arien")
            card = hero.hand[0]
            if mode in ("unresolved", "resolved"):
                hero.play_card(card)
                if mode == "resolved":
                    hero.resolve_current_card(turn_number=1)
            if mode == "deck":
                card = hero.deck[0]
            if mode == "discard":
                hero.discard_card(card)
            if mode == "extra-only":
                hero.hand = [c for c in hero.hand if c is not card]
                hero.deck = [c for c in hero.deck if c is not card]
                hero.extra_turn_card = card
            if mode == "foreign":
                card = state.get_hero("hero_hanu").hand[0]
            if mode == "global-shadow":
                state.get_hero("hero_razzle").deck.append(card.model_copy(deep=True))
            printed = (
                None
                if mode == "cardless"
                else "" if mode == "empty" else "absent" if mode == "missing" else str(card.id)
            )
            spec = template(source_card_id=printed, is_active=enabled)
            run(f"card-{mode}-{enabled}", state, [create(spec), restore, active])
    for mode in (
        "same",
        "dormant",
        "different-affects",
        "different-type",
        "cardless",
        "empty",
        "recreate",
    ):
        state = build()
        card = state.get_hero("hero_arien").hand[0]
        printed = None if mode == "cardless" else "" if mode == "empty" else str(card.id)
        spec = template(source_card_id=printed)
        changed = dict(
            spec,
            source_id="hero_hanu",
            max_value=99,
            is_active=True,
            duration="PASSIVE",
            stat_value=-5,
            scope={**spec["scope"], "origin_id": "hero_hanu", "range": 7},
            finishing_steps=[],
        )
        if mode == "different-affects":
            changed["scope"]["affects"] = "friendly_units"
        if mode == "different-type":
            changed["effect_type"] = "area_stat_modifier"
        ops = [
            create(spec),
            restore,
            {"kind": "clock", "round": 3, "turn": 3},
            create(changed),
            restore,
        ]
        if mode == "dormant":
            from goa2.engine.effect_manager import EffectManager

            fields = {
                k: v
                for k, v in spec.items()
                if k not in ("id", "created_at_turn", "created_at_round")
            }
            fields["scope"] = EffectScope.model_validate(fields["scope"])
            EffectManager.create_effect(state, **fields)
            for effect in state.active_effects:
                effect.is_active = False
            card.is_active_base = False
            ops = [create(changed), restore, active]
        if mode == "recreate":
            ops = [
                create(spec),
                {"kind": "expire_card", "card": printed},
                restore,
                create(changed),
                restore,
            ]
        run("dedup-" + mode, state, ops)
    for shape in Shape:
        for fixed in (False, True):
            for origin in (None, "hero_razzle", "hero_hanu"):
                state = build()
                scope = EffectScope(
                    shape=shape,
                    range=2,
                    origin_id=origin,
                    origin_hex=Hex(q=0, r=0, s=0) if fixed else None,
                )
                spec = template(source_id="hero_razzle", scope=scope)
                run(f"origin-{shape}-{fixed}-{origin}", state, [create(spec), restore, active])
    for origin in (None, "", "hero_razzle", "hero_hanu"):
        state = build()
        spec = template(
            source_id="hero_razzle",
            effect_type=EffectType.STATIC_BARRIER,
            scope=EffectScope(shape=Shape.GLOBAL),
            barrier_origin_id=origin,
            barrier_radius=2,
        )
        run("barrier-" + str(origin), state, [create(spec), restore])
    for mode in (
        "basic",
        "facedown",
        "tiered",
        "nested",
        "nested-extra",
        "missing-actor",
        "unknown-actor",
        "piece-actor",
        "minion-actor",
        "stale-nested",
        "skill",
        "none",
    ):
        state = build()
        hero = state.get_hero("hero_arien")
        card = hero.hand[0]
        hero.play_card(card)
        if mode == "tiered":
            card.tier = CardTier.I
        if mode == "facedown":
            card.is_facedown = True
        if mode.startswith("nested"):
            other = state.get_hero("hero_hanu")
            nested = other.hand[0]
            nested.tier = CardTier.I
            state.execution_context.update(
                performing_card_id=str(nested.id), performing_card_owner_id=str(other.id)
            )
            if mode == "nested-extra":
                other.hand = [c for c in other.hand if c is not nested]
                other.deck = [c for c in other.deck if c is not nested]
                other.extra_turn_card = nested
        if mode == "stale-nested":
            state.execution_context["performing_card_id"] = "absent"
        if mode == "missing-actor":
            state.current_actor_id = None
        if mode == "unknown-actor":
            state.current_actor_id = "absent"
        if mode == "piece-actor":
            state.current_actor_id = state.acting_piece_id
        if mode == "minion-actor":
            state.current_actor_id = next(iter(state.teams.values())).minions[0].id
        action = (
            ActionType.SKILL if mode == "skill" else None if mode == "none" else ActionType.ATTACK
        )
        spec = template(origin_action_type=action, origin_attack_is_basic=True)
        run("attack-" + mode, state, [create(spec), restore, active])
    state = build()
    ops = []
    for i, kind in enumerate(EffectType):
        spec = template(
            effect_type=kind,
            duration=list(DurationType)[i % 4],
            source_id="hero_razzle",
            scope=EffectScope(shape=Shape.RADIUS, range=3, affects=AffectsFilter.ALL_UNITS),
            token_id="token_binding" if i % 2 else None,
            subject_id="hero_hanu",
            passive_uses_this_turn=2,
            granted_passive_effect_id="big_sodding_gun",
            max_value=i + 1,
            stat_value=-i,
        )
        ops.append(create(spec))
    run("all-types-payloads", state, [*ops, restore, active, {"kind": "expire_turn"}, restore])
