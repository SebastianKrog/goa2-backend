"""Source contracts for live pre-action movement and reentrant disruptors.

Raw boundary fixtures intentionally preserve step/list aliases. Registered real
cards seed the providers; the frozen handler supplies all expected outcomes.
"""

import copy

import pytest
from live_pre_action_traces import scenes
from object_execution_traces import reference

pytestmark = pytest.mark.filterwarnings(
    "ignore:(?s)Pydantic serializer warnings.*processed_effect_ids.*input_type=tuple:UserWarning"
)


@pytest.fixture(scope="module")
def pre_action_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(pre_action_scenes, name):
    state, ops = copy.deepcopy(pre_action_scenes[name])
    return state, reference(state, ops)


def prompts(records):
    return [row["request"] for row in records if row["request"] is not None]


def test_movement_uses_first_active_provider_and_source_range_defaults(pre_action_scenes):
    for variant, range_value in (
        ("first-active", 2),
        ("after-inactive", 3),
        ("wrong-source", 3),
        ("zero", 1),
    ):
        state, records = run(pre_action_scenes, "pre-action-move-order-" + variant)
        assert (
            prompts(records)[0]["prompt"]
            == f"Pre-action movement (up to {range_value} space{'s' if range_value>1 else ''})"
        )
        assert len(state.active_effects) == 2
    _state, records = run(pre_action_scenes, "pre-action-move-order-numeric")
    assert not prompts(records)
    state, records = run(pre_action_scenes, "pre-action-move-order-negative")
    assert prompts(records)[0]["prompt"] == "Pre-action movement (up to -2 space)"
    assert (
        state.entity_locations
        == pre_action_scenes["pre-action-move-order-negative"][0].entity_locations
    )


def test_optional_movement_skip_and_stale_selection_keep_actor_and_board(pre_action_scenes):
    name = "live-targeting-pre-move-two-skip"
    original = pre_action_scenes[name][0]
    state, records = run(pre_action_scenes, name)
    assert state.entity_locations == original.entity_locations
    assert state.current_actor_id == original.current_actor_id
    assert "pre_action_move_hex" not in state.execution_context
    _state, records = run(pre_action_scenes, "live-targeting-pre-move-two-invalid")
    requests = prompts(records)
    assert len(requests) >= 2 and requests[0]["id"] != requests[1]["id"]


def test_real_misa_cards_generate_correct_ranges_and_respect_activity(pre_action_scenes):
    for effect_id, range_value in (("focus", 1), ("discipline", 2), ("mastery", 3)):
        state, records = run(pre_action_scenes, f"pre-action-card-{effect_id}-first")
        assert (
            prompts(records)[0]["prompt"]
            == f"Pre-action movement (up to {range_value} space{'s' if range_value>1 else ''})"
        )
        assert state.active_effects[0].source_card_id == state.execution_context["carrier_alias"].id
        for variant in ("inactive", "expired"):
            _state, records = run(pre_action_scenes, f"pre-action-card-{effect_id}-{variant}")
            assert not prompts(records)


def test_disruptor_updates_original_processed_list_and_detaches_effect_list(pre_action_scenes):
    state, _ = run(pre_action_scenes, "live-targeting-pre-discard-plain-first")
    step = state.execution_context["pre_action_alias"]
    assert step.processed_effect_ids is state.execution_context["processed_alias"]
    assert step.processed_effect_ids == ["disrupt"]
    assert state.active_effects is not state.execution_context["effects_alias"]
    assert any(row.id == "disrupt" for row in state.execution_context["effects_alias"])
    assert not any(row.id == "disrupt" for row in state.active_effects)
    state, _ = run(pre_action_scenes, "live-targeting-pre-discard-finishing-first")
    assert "should_not_finish" not in state.execution_context


def test_source_activity_and_immunity_gates_are_distinct(pre_action_scenes):
    state, _ = run(pre_action_scenes, "live-targeting-pre-discard-passive-dormant-first")
    assert "disrupt" in state.execution_context["processed_alias"]
    for variant in ("card-dormant", "expired", "off-scope", "immune", "origin-attack"):
        state, records = run(pre_action_scenes, f"live-targeting-pre-discard-{variant}-first")
        assert state.execution_context["processed_alias"] == []
        assert not prompts(records)
        assert any(row.id == "disrupt" for row in state.active_effects)


def test_duplicate_ids_and_safe_empty_hands_are_processed_once(pre_action_scenes):
    state, _ = run(pre_action_scenes, "live-targeting-pre-discard-duplicate-first")
    assert state.execution_context["processed_alias"] == ["disrupt"]
    assert not any(row.id == "disrupt" for row in state.active_effects)
    state, _ = run(pre_action_scenes, "pre-action-discard-alias-empty-safe-many")
    assert state.execution_context["processed_alias"] == ["disrupt", "disrupt-2", "disrupt-3"]
    assert len(state.active_effects) == 3
    state, _ = run(pre_action_scenes, "pre-action-discard-alias-duplicate-safe")
    assert state.execution_context["processed_alias"] == ["disrupt"]
    assert len(state.active_effects) == 3


def test_reused_step_and_shared_processed_list_preserve_identity(pre_action_scenes):
    state, _ = run(pre_action_scenes, "pre-action-discard-alias-shared-step")
    assert state.execution_context["defeat_steps"][0] is state.execution_context["defeat_steps"][1]
    assert state.execution_context["processed_alias"] == ["disrupt"]
    state, _ = run(pre_action_scenes, "pre-action-discard-alias-shared-processed")
    left, right = state.execution_context["defeat_steps"]
    assert (
        left.processed_effect_ids
        is right.processed_effect_ids
        is state.execution_context["processed_alias"]
    )
    state, _ = run(pre_action_scenes, "pre-action-discard-alias-no-effects-null")
    assert state.execution_context["pre_action_alias"].processed_effect_ids is None
    state, _ = run(pre_action_scenes, "pre-action-discard-alias-processed-tuple")
    assert state.execution_context["pre_action_alias"].processed_effect_ids == ("disrupt",)


def test_real_disruptor_cards_consume_only_for_a_discard(pre_action_scenes):
    for name in ("disruptor_jolt", "disruptor_pulse", "disruptor_grid"):
        state, _ = run(pre_action_scenes, f"pre-action-card-{name}-hand")
        assert not any(row.id.startswith("fixture/" + name) for row in state.active_effects)
        assert state.execution_context["carrier_alias"].is_active is False
        state, _ = run(pre_action_scenes, f"pre-action-card-{name}-out-of-radius")
        assert state.execution_context["processed_alias"] == []
        assert state.execution_context["carrier_alias"].is_active is True
    for name in ("disruptor_jolt", "disruptor_pulse"):
        state, _ = run(pre_action_scenes, f"pre-action-card-{name}-empty")
        assert state.has_board_presence("hero_brogan")
        assert any(row.id.startswith("fixture/" + name) for row in state.active_effects)


def test_defeat_credits_creator_and_defender_mode_preserves_attacker(pre_action_scenes):
    for variant in ("empty", "defender"):
        state, _ = run(pre_action_scenes, "pre-action-card-disruptor_grid-" + variant)
        assert not state.has_board_presence("hero_brogan")
        assert state.execution_context["defeat_log"] == [["hero_brogan", "hero_trinkets"]]
        assert state.get_hero("hero_trinkets").gold > 0
        assert any(row.id.startswith("fixture/disruptor_grid") for row in state.active_effects)
        assert ("tail" in state.execution_context) is (variant == "defender")
    state, _ = run(pre_action_scenes, "live-targeting-pre-discard-offboard-continue-first")
    assert state.execution_context["after"] is True
    state, _ = run(pre_action_scenes, "live-targeting-pre-discard-offboard-abort-first")
    assert "after" not in state.execution_context


def test_attack_integration_rechecks_scope_expiry_and_restores_attacker(pre_action_scenes):
    for variant in (
        "move",
        "discard",
        "defeat",
        "both",
        "leave-radius",
        "multiple",
        "linked-defense",
        "mine-disrupt",
    ):
        for ranged in (False, True):
            for mode in ("first", "skip"):
                state, _ = run(
                    pre_action_scenes, f"live-targeting-attack-pre-{variant}-{ranged}-{mode}"
                )
                assert state.execution_stack[-1].type.value == "resolve_defense_text"
                assert str(state.current_actor_id) == "hero_arien"
                if variant == "defeat":
                    assert not state.has_board_presence("hero_wasp")
                    assert state.execution_context["defeat_log"][-1] == [
                        "hero_wasp",
                        "hero_trinkets",
                    ]
