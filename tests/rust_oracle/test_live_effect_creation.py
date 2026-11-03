"""Frozen source contracts for live effect construction and isolation."""

import copy

import pytest
from live_effect_creation_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def creation_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(creation_scenes, name):
    state, ops = copy.deepcopy(creation_scenes[name])
    return state, reference(state, ops)


def events(records):
    return [
        event
        for row in records
        for event in row["events"]
        if event["event_type"] == "EFFECT_CREATED"
    ]


def test_scope_is_retained_and_constructor_lists_are_fresh(creation_scenes):
    state, records = run(creation_scenes, "live-effect-step-plain")
    effect = state.active_effects[0]
    assert len(events(records)) == 1
    assert state.active_effects is state.execution_context["effects_alias"]
    assert effect.scope is state.execution_context["scope_alias"]
    assert effect.finishing_steps is not state.execution_context["finishing_alias"]
    assert effect.restrictions is not state.execution_context["restrictions_alias"]
    assert effect.except_attacker_ids is not state.execution_context["exceptions_alias"]


def test_dynamic_scope_copy_and_token_identity_precede_board_actor_binding(creation_scenes):
    state, _ = run(creation_scenes, "live-effect-step-integer-origin")
    assert state.active_effects[0].scope.origin_id == "17"
    assert state.active_effects[0].scope is not state.execution_context["scope_alias"]
    state, _ = run(creation_scenes, "live-effect-step-token-piece")
    assert state.active_effects[0].token_id == "hero_razzle"
    assert state.active_effects[0].scope.origin_id == state.acting_piece_id
    state, _ = run(creation_scenes, "live-effect-create-alias-same-origin-copy")
    assert state.active_effects[0].scope is not state.execution_context["scope_alias"]


def test_duplicate_dormant_instance_is_pure_noop(creation_scenes):
    state, records = run(creation_scenes, "live-effect-step-duplicate-dormant")
    assert len(state.active_effects) == 1
    assert not state.active_effects[0].is_active
    assert not events(records)
    card = state.get_card_by_id(state.active_effects[0].source_card_id)
    assert not card.is_active_base


def test_new_row_with_colliding_effect_id_suppresses_only_event(creation_scenes):
    state, records = run(creation_scenes, "live-effect-step-id-collision")
    assert len(state.active_effects) == 2
    assert state.active_effects[0].id == state.active_effects[1].id
    assert state.active_effects[0] is not state.active_effects[1]
    assert not events(records)


def test_finishing_color_is_bound_without_mutating_original_template(creation_scenes):
    state, _ = run(creation_scenes, "live-effect-step-freeze-colors")
    assert state.execution_context["color"] == "RED"
    assert state.active_effects[0].named_color.value == "GREEN"
    assert state.active_effects[0].finishing_steps[0].color.value == "GREEN"
    assert state.execution_context["finishing_alias"][0].color is None


def test_effect_constructor_splits_nested_aliases_again(creation_scenes):
    state, _ = run(creation_scenes, "live-effect-create-alias-nested-aliases")
    original = state.execution_context["loop_alias"]
    assert original.steps_template[0] is original.steps_template[1]
    roots = state.active_effects[0].finishing_steps
    assert roots[0] is not roots[1]
    assert roots[0] is not original
    assert roots[0].steps_template[0] is not roots[0].steps_template[1]
    assert roots[0].steps_template[0] is not roots[1].steps_template[0]
    assert all(child.color.value == "RED" for root in roots for child in root.steps_template)


def test_resolved_and_unresolved_card_activation_are_distinct(creation_scenes):
    state, _ = run(creation_scenes, "live-effect-manager-creation-card-unresolved-False")
    assert not state.active_effects[0].is_active
    assert state.get_card_by_id(state.active_effects[0].source_card_id).is_active_base
    state, _ = run(creation_scenes, "live-effect-manager-creation-card-resolved-False")
    assert state.active_effects[0].is_active


def test_origin_attack_uses_live_nested_performing_card(creation_scenes):
    for variant, expected in (
        ("basic", True),
        ("tiered", False),
        ("nested", False),
        ("unknown-actor", False),
        ("skill", None),
    ):
        state, _ = run(creation_scenes, "live-effect-manager-creation-attack-" + variant)
        assert state.active_effects[0].origin_attack_is_basic is expected
