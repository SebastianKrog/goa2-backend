"""Frozen live CreateEffectStep state and graph identity boundaries.

Raw primitives deliberately retain scopes, lists, steps and model-copy aliases.
Expected behavior is computed only by the frozen Python handler.
"""

from __future__ import annotations


def scenes():
    import copy
    from unittest.mock import patch

    import effect_step_traces
    from live_defeat_traces import retain
    from object_execution_traces import reference

    emitted = []

    def capture(state, ops):
        snapshot = copy.deepcopy(state)
        steps = list(reversed(snapshot.execution_stack))
        create = steps[0]
        snapshot.execution_context.update(
            scope_alias=create.scope,
            finishing_alias=create.finishing_steps,
            create_alias=create,
            restrictions_alias=create.restrictions,
            exceptions_alias=create.except_attacker_ids,
        )
        live_ops, _ = retain(snapshot, steps)
        emitted.append((snapshot, live_ops))
        return reference(state, ops)

    with patch("stack_traces.reference_trace", capture):
        cases = effect_step_traces.cases({"kind": "advance"}, None)
    assert len(cases) == len(emitted)
    for case, (state, ops) in zip(cases, emitted, strict=True):
        yield "live-" + case["name"], state, ops

    from combat_effect_traces import state_for

    from goa2.domain.models import CardColor
    from goa2.domain.models.effect import EffectScope, EffectType, Shape
    from goa2.engine.steps import (
        CreateEffectStep,
        ForceDiscardByColorStep,
        ForEachStep,
        SetContextFlagStep,
    )

    for variant in (
        "shared-roots",
        "nested-aliases",
        "nested-colors",
        "same-origin-copy",
        "repeat-cardless",
        "fixed-hex",
    ):
        state, h, _ = state_for()
        child = ForceDiscardByColorStep(victim_key="victim", color_key="color")
        scope = EffectScope(shape=Shape.RADIUS, range=3, origin_id="hero_swift")
        create = CreateEffectStep(
            effect_type=EffectType.DELAYED_TRIGGER,
            scope=scope,
            finishing_steps=[child],
            use_context_card=False,
        )
        child = create.finishing_steps[0]
        state.execution_context.update(
            color=CardColor.GREEN,
            child_alias=child,
            scope_alias=scope,
            finishing_alias=create.finishing_steps,
            create_alias=create,
        )
        if variant == "shared-roots":
            create.finishing_steps = [child, child]
        if variant in {"nested-aliases", "nested-colors"}:
            loop = ForEachStep(list_key="targets", item_key="victim", steps_template=[child])
            loop.steps_template = [child, child]
            create.finishing_steps = [loop, loop]
            state.execution_context.update(loop_alias=loop, loop_children_alias=loop.steps_template)
            if variant == "nested-aliases":
                child.color = CardColor.RED
        if variant == "same-origin-copy":
            create.origin_id_key = "origin"
            state.execution_context["origin"] = "hero_swift"
        if variant == "fixed-hex":
            scope.origin_hex = h(0, 0)
            create.isolated_hex = h(1, 0)
            state.execution_context.update(
                hex_alias=scope.origin_hex, isolated_alias=create.isolated_hex
            )
        steps = [create]
        if variant == "repeat-cardless":
            steps += [SetContextFlagStep(key="color", value=CardColor.RED), create]
        state.execution_context["finishing_alias"] = create.finishing_steps
        ops, _ = retain(state, steps)
        yield "live-effect-create-alias-" + variant, state, ops

    yield from manager_scenes()


def manager_scenes():
    from effect_creation_traces import add_creation_cases
    from live_defeat_traces import retain

    from goa2.domain.models import GamePhase
    from goa2.engine.steps import CreateEffectStep

    captured = []

    def collect(name, factory, operations):
        state = factory()
        state.phase = GamePhase.RESOLUTION
        specs = [operation["effect"] for operation in operations if operation["kind"] == "create"]
        if not name.startswith("creation-attack-"):
            state.current_actor_id = specs[0]["source_id"]
        steps = [
            CreateEffectStep(
                **{
                    key: value
                    for key, value in spec.items()
                    if key in CreateEffectStep.model_fields
                }
            )
            for spec in specs
        ]
        state.execution_context.update(
            create_steps_alias=steps, scopes_alias=[step.scope for step in steps]
        )
        ops, _ = retain(state, steps)
        captured.append(("live-effect-manager-" + name, state, ops))

    add_creation_cases(collect)
    yield from captured
