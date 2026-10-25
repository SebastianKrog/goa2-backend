"""Actual adjacency handler traces through directional topology and immunity."""

from __future__ import annotations


def cases(advance, answer, *, rejected=False):
    from aura_traces import _board
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.models import GamePhase
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import CheckAdjacencyStep, SetContextFlagStep

    variants = [
        "plain",
        "split",
        "isolation-in",
        "isolation-out",
        "zero-in",
        "zero-out",
        "isolation-fallback",
        "isolation-dormant",
        "isolation-past",
        "shield",
        "shield-dormant",
        "shield-except",
        "shield-friendly",
        "shield-radius",
        "keys",
        "unknown",
        "offboard",
        "prefix",
    ]
    result = []
    for variant in variants:
        state = setup()
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        state.place_entity("hero_arien", h(1, -1))
        state.place_entity("hero_brogan", h(1, 0))
        state.place_entity("hero_hanu", h(0, 0))

        def effect(kind, state=state, **kwargs):
            fields = dict(
                id="topology",
                source_id="hero_brogan",
                effect_type=kind,
                scope=EffectScope(shape=Shape.GLOBAL),
                duration=DurationType.PASSIVE,
                created_at_round=state.round,
                created_at_turn=state.turn,
                is_active=True,
                split_axis="q",
                split_value=0,
            )
            fields.update(kwargs)
            return ActiveEffect(**fields)

        if variant != "plain":
            e = effect(
                EffectType.TOPOLOGY_SPLIT if variant == "split" else EffectType.TOPOLOGY_ISOLATION
            )
            if variant == "isolation-fallback":
                e.source_id = "unknown"
                e.isolated_hex = h(1, 0)
            if variant == "isolation-dormant":
                e.is_active = False
            if variant == "isolation-past":
                e.duration = DurationType.THIS_TURN
                e.created_at_round = state.round - 1
            if variant.startswith("shield"):
                e.source_id = "hero_hanu"
                shield = effect(EffectType.ATTACK_IMMUNITY, id="shield", source_id="hero_arien")
                e.origin_attack_is_basic = True
                if variant == "shield-dormant":
                    shield.is_active = False
                if variant == "shield-except":
                    shield.except_attacker_ids = ["hero_hanu"]
                if variant in ("shield-friendly", "shield-radius"):
                    shield.effect_type = EffectType.IMMUNITY_ENEMY_ACTIONS
                    shield.blocks_friendly_actors = True
                if variant == "shield-radius":
                    shield.scope = EffectScope(shape=Shape.RADIUS, range=2)
                state.active_effects.append(shield)
            state.active_effects.append(e)
        a, b = "hero_arien", "hero_brogan"
        if variant.startswith("shield"):
            b = "hero_hanu"
        if variant in ("isolation-out", "zero-out"):
            a, b = b, a
        if variant == "zero-in":
            a = "hero_hanu"
        if variant == "zero-out":
            b = "hero_hanu"
        if variant == "unknown":
            a = "unknown"
        if variant == "offboard":
            state.remove_entity(a)
        kwargs = dict(
            unit_a_id=a,
            unit_b_id=b,
            output_key="adjacent",
            step_id="fixture/effect-adjacency/check",
        )
        if variant == "keys":
            kwargs.update(unit_a_id=None, unit_b_id=None, unit_a_key="a", unit_b_key="b")
            state.execution_context = {"a": a, "b": b}
        steps = [CheckAdjacencyStep(**kwargs)]
        if variant == "prefix":
            steps.append(
                SetContextFlagStep(
                    key="after", value=True, step_id="fixture/effect-adjacency/after"
                )
            )
        push_steps(state, steps)
        envelope = export_state(state)
        ops = (
            [{"kind": "advance_prefix", "steps": 1}, advance] if variant == "prefix" else [advance]
        )
        try:
            expected = reference_trace(state, ops)
        except RecursionError:
            if variant != "shield-radius":
                raise
            if rejected:
                result.append(
                    {
                        "name": "effect-adjacency-" + variant,
                        "envelope": envelope,
                        "operations": ops,
                        "reference_error": "RecursionError",
                    }
                )
            continue
        if rejected:
            continue
        assert state.execution_context["adjacent"] == (
            variant
            in [
                "plain",
                "split",
                "isolation-out",
                "zero-in",
                "zero-out",
                "shield",
                "shield-friendly",
                "shield-radius",
            ]
        ), variant
        result.append(
            {
                "name": "effect-adjacency-" + variant,
                "envelope": envelope,
                "operations": ops,
                "expected": expected,
            }
        )
    return result
