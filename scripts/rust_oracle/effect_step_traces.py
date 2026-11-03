"""Freeze ActiveEffect defaults and real CreateEffectStep handler traces."""

from __future__ import annotations

from fixture_cache import write_text

import json

from oracle import ROOT, digest, verify_sources


def defaults():
    from goa2.domain.models import ActionType, CardColor
    from goa2.domain.models.effect import (
        ActiveEffect,
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )

    prototype = ActiveEffect(
        id="native-effect-template",
        source_id="system",
        effect_type=EffectType.DELAYED_TRIGGER,
        scope=EffectScope(shape=Shape.GLOBAL),
        duration=DurationType.THIS_TURN,
        created_at_turn=0,
        created_at_round=0,
    )
    data = {
        "schema_version": 1,
        "source_sha256": verify_sources()["source_sha256"],
        "prototype": prototype.model_dump(mode="json"),
        "effect_types": [e.value for e in EffectType],
        "affects": [e.value for e in AffectsFilter],
        "actions": [e.value for e in ActionType],
        "colors": [e.value for e in CardColor],
    }
    return {**data, "catalog_sha256": digest(data)}


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.models import CardColor, GamePhase
    from goa2.domain.models.effect import DurationType, EffectScope, EffectType, Shape
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        CreateEffectStep,
        ForceDiscardByColorStep,
        ForEachStep,
        SetContextFlagStep,
    )

    result = []
    variants = [
        "plain",
        "literal-card",
        "context-card",
        "no-infer",
        "empty-card",
        "literal-precedence",
        "no-actor",
        "piece-actor",
        "dynamic-origin",
        "integer-origin",
        "falsy-origin",
        "token",
        "token-substitution",
        "foreign-substitution",
        "unbound-substitution",
        "except",
        "except-duplicate",
        "except-falsy",
        "named-color",
        "missing-color",
        "finishing-color",
        "finishing-fixed",
        "nested-color",
        "token-piece",
        "split",
        "duplicate",
        "duplicate-dormant",
        "id-collision",
        "gate-off",
        "skip-null",
        "skip-false",
        "freeze-colors",
        "prefix",
        "attack",
        "skill",
    ]
    variants += ["type-" + kind.value for kind in EffectType]
    for variant in variants:
        state = setup(["Razzle", "Arien"], ["Brogan", "Hanu"])
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        state.acting_piece_id = state.get_piece_ids("hero_razzle")[0]
        hero = state.get_hero("hero_arien")
        card = hero.hand[0]
        state.execution_context = {"color": "GREEN", "origin": "hero_hanu", "except": "hero_hanu"}
        kwargs = dict(
            effect_type=EffectType.DELAYED_TRIGGER,
            scope=EffectScope(shape=Shape.RADIUS, range=2),
            duration=DurationType.THIS_ROUND,
            step_id="fixture/effect-step/main",
        )
        if variant.startswith("type-"):
            kwargs["effect_type"] = EffectType(variant[5:])
        if variant in (
            "literal-card",
            "literal-precedence",
            "duplicate",
            "duplicate-dormant",
            "id-collision",
        ):
            kwargs["source_card_id"] = str(card.id)
        if variant in ("context-card", "literal-precedence", "no-infer", "empty-card"):
            state.execution_context["current_card_id"] = (
                17 if variant == "literal-precedence" else str(card.id)
            )
        if variant == "no-infer":
            kwargs["use_context_card"] = False
        if variant == "empty-card":
            kwargs["source_card_id"] = ""
        if variant == "no-actor":
            state.current_actor_id = None
        if variant in ("piece-actor", "token-piece"):
            state.current_actor_id = state.acting_piece_id
        if variant in (
            "dynamic-origin",
            "integer-origin",
            "falsy-origin",
            "token",
            "token-substitution",
            "foreign-substitution",
            "unbound-substitution",
            "token-piece",
        ):
            kwargs["origin_id_key"] = "origin"
            if variant == "integer-origin":
                state.execution_context["origin"] = 17
            if variant == "falsy-origin":
                state.execution_context["origin"] = False
        if variant.startswith("token") or "substitution" in variant:
            kwargs["is_token_effect"] = True
            state.execution_context["current_card_id"] = str(card.id)
            if variant == "token-piece":
                state.execution_context["origin"] = "hero_razzle"
        if "substitution" in variant:
            state.execution_context["token_type_override"] = "ILLUSION"
            if variant != "unbound-substitution":
                state.execution_context["substitution_actor_id"] = (
                    "hero_hanu" if variant == "foreign-substitution" else "hero_arien"
                )
        if variant.startswith("except"):
            kwargs.update(
                except_attacker_key="except",
                except_attacker_ids=(
                    ["hero_hanu"] if variant == "except-duplicate" else ["hero_brogan"]
                ),
            )
            if variant == "except-falsy":
                state.execution_context["except"] = None
        if variant in ("named-color", "missing-color", "freeze-colors"):
            kwargs["named_color_key"] = "color"
        if variant == "missing-color":
            state.execution_context.pop("color")
        if variant in ("finishing-color", "finishing-fixed", "nested-color", "freeze-colors"):
            child = ForceDiscardByColorStep(
                victim_key="victim",
                color_key="color",
                color=CardColor.RED if variant == "finishing-fixed" else None,
                step_id="fixture/effect-step/finish",
            )
            if variant == "nested-color":
                child = ForEachStep(
                    list_key="targets",
                    item_key="victim",
                    steps_template=[child],
                    step_id="fixture/effect-step/loop",
                )
            kwargs["finishing_steps"] = [child]
        if variant == "split":
            kwargs.update(
                effect_type=EffectType.TOPOLOGY_SPLIT,
                split_axis="q",
                split_value=-2,
                named_color_key="color",
            )
        if variant == "gate-off":
            kwargs["active_if_key"] = "absent"
        if variant in ("skip-null", "skip-false"):
            kwargs["skip_if_key"] = "skip"
            state.execution_context["skip"] = None if variant == "skip-null" else False
        if variant in ("attack", "skill"):
            state.execution_context["current_action_type"] = (
                "ATTACK" if variant == "attack" else "SKILL"
            )
        step = CreateEffectStep(**kwargs)
        steps = [step]
        if variant in ("duplicate", "duplicate-dormant", "id-collision"):
            from goa2.engine.effect_manager import EffectManager

            if variant != "duplicate":
                EffectManager.create_effect(
                    state,
                    source_id="hero_arien",
                    source_card_id=str(card.id),
                    effect_type=EffectType.DELAYED_TRIGGER,
                    scope=kwargs["scope"],
                    duration=DurationType.THIS_ROUND,
                )
                if variant == "duplicate-dormant":
                    for e in state.active_effects:
                        e.is_active = False
                    card.is_active_base = False
                else:
                    state.active_effects[0].source_card_id = None
                    state.next_entity_id -= 1
            else:
                steps.append(
                    step.model_copy(deep=True, update={"step_id": "fixture/effect-step/repeat"})
                )
        if variant == "freeze-colors":
            steps.append(
                SetContextFlagStep(key="color", value="RED", step_id="fixture/effect-step/change")
            )
        push_steps(state, steps)
        envelope = export_state(state)
        ops = (
            [{"kind": "advance_prefix", "steps": 1}, advance]
            if variant in ("prefix", "freeze-colors", "duplicate")
            else [advance]
        )
        expected = reference_trace(state, ops)
        events = [e for r in expected for e in r["events"] if e["event_type"] == "EFFECT_CREATED"]
        assert len(events) == (
            0
            if variant
            in ("token-substitution", "gate-off", "skip-false", "duplicate-dormant", "id-collision")
            else 1
        ), variant
        if variant == "freeze-colors":
            assert (
                state.active_effects[0].named_color == CardColor.GREEN
                and state.active_effects[0].finishing_steps[0].color == CardColor.GREEN
            )
        result.append(
            {
                "name": "effect-step-" + variant,
                "envelope": envelope,
                "operations": ops,
                "expected": expected,
            }
        )
    return result


if __name__ == "__main__":
    data = defaults()
    write_text(ROOT / "rust/tests/fixtures/effect-defaults.json", json.dumps(data, indent=2) + "\n")
    print(data["catalog_sha256"])
