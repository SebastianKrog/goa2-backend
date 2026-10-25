"""Freeze real token-removal steps with token/source/scope-linked effects."""

from __future__ import annotations


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.models import GamePhase, TokenType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.domain.models.token import Token
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import CreateEffectStep, RemoveTokenStep, SetContextFlagStep

    variants = [
        "token",
        "source",
        "scope",
        "all-links",
        "unrelated",
        "barrier-only",
        "offboard",
        "twice",
        "key",
        "key-precedence",
        "missing-key",
        "unknown",
        "hero",
        "no-actor",
        "card-flags",
        "duplicate-id",
        "prefix",
        "create-remove",
        "dormant",
        "expired",
        "fixed-scope",
        "gate-off",
    ]
    variants += ["type-" + kind.value for kind in EffectType]
    result = []
    for variant in variants:
        state = setup()
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = None if variant == "no-actor" else "hero_arien"
        hero = state.get_hero("hero_arien")
        token = Token(
            id="effect_token", name="Effect token", token_type=TokenType.TOTEM, owner_id=hero.id
        )
        state.token_pool.setdefault(TokenType.TOTEM, []).append(token)
        state.misc_entities[token.id] = token
        point = next(
            h
            for h, tile in state.board.tiles.items()
            if not tile.is_obstacle and h not in state.entity_locations.values()
        )
        if variant != "offboard":
            state.place_entity(token.id, point)
        state.execution_context = {"token": str(token.id)}
        card = hero.hand[0]
        card.is_active_base = True
        kind = (
            EffectType(variant[5:]) if variant.startswith("type-") else EffectType.DELAYED_TRIGGER
        )
        link = (
            variant
            if variant in ("source", "scope", "all-links", "unrelated", "barrier-only")
            else "token"
        )
        effect = ActiveEffect(
            id="linked-effect",
            source_id=str(token.id) if link in ("source", "all-links") else str(hero.id),
            token_id=str(token.id) if link in ("token", "all-links") else None,
            source_card_id=str(card.id),
            effect_type=kind,
            scope=EffectScope(
                shape=Shape.GLOBAL,
                origin_id=str(token.id) if link in ("scope", "all-links") else None,
                origin_hex=point if variant == "fixed-scope" else None,
            ),
            duration=DurationType.THIS_TURN,
            created_at_turn=state.turn - (1 if variant == "expired" else 0),
            created_at_round=state.round,
            is_active=variant != "dormant",
            barrier_origin_id=str(token.id) if link == "barrier-only" else None,
            finishing_steps=[
                SetContextFlagStep(
                    key="must_not_finish", value=True, step_id="fixture/token-effect/finish"
                )
            ],
        )
        survivor = effect.model_copy(
            deep=True,
            update={
                "id": effect.id if variant == "duplicate-id" else "unrelated-effect",
                "token_id": "other-token",
                "source_id": "hero_hanu",
                "scope": EffectScope(shape=Shape.GLOBAL),
                "barrier_origin_id": None,
                "source_card_id": None,
            },
        )
        state.active_effects = [effect, survivor]
        kwargs = dict(token_id=str(token.id), step_id="fixture/token-effect/remove")
        if variant in ("key", "key-precedence", "missing-key"):
            kwargs["token_key"] = "absent" if variant == "missing-key" else "token"
            kwargs["token_id"] = "unknown" if variant == "key-precedence" else str(token.id)
        if variant in ("unknown", "hero"):
            kwargs["token_id"] = "unknown" if variant == "unknown" else str(hero.id)
        if variant == "gate-off":
            kwargs["active_if_key"] = "absent"
        steps = [RemoveTokenStep(**kwargs)]
        if variant in ("twice", "prefix"):
            steps.append(
                RemoveTokenStep(token_id=str(token.id), step_id="fixture/token-effect/repeat")
            )
        if variant == "create-remove":
            state.active_effects = []
            state.execution_context["origin"] = str(token.id)
            steps.insert(
                0,
                CreateEffectStep(
                    effect_type=EffectType.DELAYED_TRIGGER,
                    scope=EffectScope(shape=Shape.GLOBAL),
                    duration=DurationType.PASSIVE,
                    is_token_effect=True,
                    origin_id_key="origin",
                    step_id="fixture/token-effect/create",
                ),
            )
        push_steps(state, steps)
        envelope = export_state(state)
        ops = (
            [{"kind": "advance_prefix", "steps": 1}, advance]
            if variant in ("prefix", "create-remove")
            else [advance]
        )
        expected = reference_trace(state, ops)
        events = [
            e for record in expected for e in record["events"] if e["event_type"] == "TOKEN_REMOVED"
        ]
        noop = variant in ("offboard", "missing-key", "unknown", "hero", "gate-off")
        assert len(events) == (0 if noop else 1), variant
        if events:
            assert events[0]["metadata"]["effects_removed"] == (
                0 if link in ("unrelated", "barrier-only") else 1
            ), variant
        assert "must_not_finish" not in state.execution_context
        assert card.is_active_base  # Token removal deliberately leaves this flag alone.
        if variant == "duplicate-id":
            assert (
                len(state.active_effects) == 1 and state.active_effects[0].source_id == "hero_hanu"
            )
        result.append(
            {
                "name": "token-effect-" + variant,
                "envelope": envelope,
                "operations": ops,
                "expected": expected,
            }
        )
    return result
