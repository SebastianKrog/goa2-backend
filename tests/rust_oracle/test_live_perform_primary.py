"""Independent contracts for selected-card scope restoration and exclusion."""

from live_perform_primary_traces import base, edge_scenes, flow_scenes, replacement_scenes
from object_execution_traces import reference


def test_primary_reperformance_restores_outer_values_and_shallow_aliases():
    checked = 0
    for name, state, operations in flow_scenes():
        record = reference(state, operations)[-1]
        context = record["semantic_state"]["execution_context"]
        assert context["current_card_id"] == "outer", name
        assert context["reperforming_card_id"] == {"retained": True}, name
        assert "action_context_stack" not in context, name
        alias = next(
            binding
            for binding in record["object_bindings"]
            if ["execution_context", "old_reperforming"] in binding["paths"]
        )
        assert ["execution_context", "reperforming_card_id"] in alias["paths"], name
        checked += 1
    assert checked == 28


def test_primary_lookup_uses_current_first_and_skips_missing_or_unregistered_cards():
    checked = 0
    for name, state, operations in edge_scenes():
        record = reference(state, operations)[0]
        context = record["semantic_state"]["execution_context"]
        variant = name.removeprefix("perform-primary-edge-")
        found = variant in ("played", "discard", "hand", "deck", "spells")
        assert ("action_context_stack" in context) == found, name
        assert (
            any(
                step["type"] == "restore_action_context"
                for step in record["semantic_state"]["execution_stack"]
            )
            == found
        ), name
        if not found:
            assert context["current_card_id"] == "outer-card", name
        checked += 1
    assert checked == 12


def test_reperformance_replacement_prompts_and_restoration():
    checked = 0
    for name, state, operations in replacement_scenes():
        records = reference(state, operations)
        if name == "perform-primary-replacement-liquid_leap-prompt":
            # Liquid Leap is a Skill, even though its effect places the hero.
            assert records[0]["request"] is None
            assert (
                records[0]["semantic_state"]["execution_context"]["current_card_id"]
                == "liquid_leap"
            )
        elif name.endswith("prompt"):
            request = records[0]["request"]
            assert request["request_type"] == "CHOOSE_ACTION"
            assert [option["id"] for option in request["options"]] == (
                ["MOVEMENT", "FAST_TRAVEL"] if "deluge" in name else ["ATTACK", "CLEAR"]
            )
            assert records[0]["semantic_state"]["execution_context"]["current_card_id"] == "outer"
        else:
            context = records[-1]["semantic_state"]["execution_context"]
            assert context["current_card_id"] == "outer", name
            assert "action_context_stack" not in context, name
        checked += 1
    assert checked == 12


def test_exclusion_reaches_nested_repeats_and_attack_filters():
    from goa2.domain.models import TargetType
    from goa2.engine.steps import PerformPrimaryActionStep
    from goa2.engine.steps.combat import AttackSequenceStep
    from goa2.engine.steps.effects import CreateEffectStep
    from goa2.engine.steps.selection import MultiSelectStep, SelectStep
    from goa2.engine.steps.utility import ForEachStep, MayRepeatNTimesStep

    for effect, hero in (
        ("noble_blade", "Arien"),
        ("bullet_hell", "Sabina"),
        ("gunslinger", "Sabina"),
    ):
        state, owner, _ = base(effect, hero)
        result = PerformPrimaryActionStep(
            hero_id=str(owner.id), exclude_target_key="blocked_target"
        ).resolve(state, state.execution_context)
        found = 0

        def check(steps):
            nonlocal found
            for step in steps:
                filters = (
                    step.target_filters
                    if isinstance(step, AttackSequenceStep)
                    else (
                        step.filters
                        if isinstance(step, (SelectStep, MultiSelectStep))
                        and step.target_type in (TargetType.UNIT, TargetType.UNIT_OR_TOKEN)
                        else None
                    )
                )
                if filters is not None:
                    assert any(
                        getattr(f, "exclude_keys", []) == ["blocked_target"] for f in filters
                    )
                    found += 1
                if isinstance(step, (MayRepeatNTimesStep, ForEachStep)):
                    check(step.steps_template)
                if isinstance(step, CreateEffectStep):
                    check(step.finishing_steps)

        check(result.new_steps)
        assert found > 0, effect


def test_nested_primary_inherits_exclusion_but_keeps_an_explicit_key():
    from goa2.engine.steps import ForEachStep, PerformPrimaryActionStep

    nested = ForEachStep(
        list_key="targets",
        item_key="target",
        steps_template=[
            PerformPrimaryActionStep(),
            PerformPrimaryActionStep(exclude_target_key="own-key"),
        ],
    )
    PerformPrimaryActionStep._inject_exclusion_filter([nested], "parent-key")
    assert [step.exclude_target_key for step in nested.steps_template] == ["parent-key", "own-key"]
