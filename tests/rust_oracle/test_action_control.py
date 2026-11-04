"""Frozen creation and handler contracts for Hanu's action-control window."""

import pytest
from action_control_traces import build


@pytest.mark.parametrize(
    "variant,created",
    [
        ("plain", True),
        ("level-7", False),
        ("level-8", True),
        ("no-ultimate", False),
        ("dormant-ultimate", True),
        ("no-actor", False),
        ("missing-card", False),
        ("target-state-RESOLVED", False),
        ("actor-piece", True),
    ],
)
def test_creation_gates_and_literal_manager_defaults(variant, created):
    state, _, card, step = build(variant)
    before = state.next_entity_id
    active = card.is_active
    result = step.resolve(state, state.execution_context)
    assert len(result.events) == int(created)
    assert state.next_entity_id == before + int(created)
    assert card.is_active is active
    if created:
        effect = state.active_effects[-1]
        assert effect.source_card_id is None
        assert effect.controlled_card_id == card.id
        assert effect.origin_action_type is None
        assert effect.is_active
        assert effect.duration.value == "THIS_ROUND"


def test_context_card_and_action_are_not_inherited_by_manager_call():
    state, _, _, step = build("context-card")
    step.resolve(state, state.execution_context)
    effect = state.active_effects[-1]
    assert effect.source_card_id is None
    assert effect.origin_action_type is None
    assert effect.origin_attack_is_basic is None


def test_repeated_scheduling_creates_distinct_unlinked_effects_and_scopes():
    state, _, _, step = build()
    first = step.resolve(state, state.execution_context)
    second = step.resolve(state, state.execution_context)
    a, b = state.active_effects
    assert len(first.events) == len(second.events) == 1
    assert a.id != b.id
    assert a.scope is not b.scope


def test_control_covers_confirmation_but_requires_the_owned_turn_and_current_card():
    from goa2.domain.models import CardState
    from goa2.engine.handler import _action_controller

    state, target, card, step = build()
    step.resolve(state, state.execution_context)
    state.current_actor_id = target.id
    state.resolution_owner_id = target.id
    assert _action_controller(state, target.id) == "hero_hanu"
    card.state = CardState.RESOLVED
    assert _action_controller(state, target.id) == "hero_hanu"
    state.resolution_owner_id = "hero_brogan"
    assert _action_controller(state, target.id) is None
    state.resolution_owner_id = None
    assert _action_controller(state, target.id) is None
    state.resolution_owner_id = target.id
    target.current_turn_card = None
    assert _action_controller(state, target.id) is None


def test_handler_uses_first_matching_active_flag_without_rechecking_timestamps():
    from goa2.engine.handler import _action_controller

    state, target, _, step = build("existing-expired")
    step.resolve(state, state.execution_context)
    state.current_actor_id = target.id
    state.resolution_owner_id = target.id
    assert _action_controller(state, target.id) == "hero_brogan"


def test_counter_collision_still_appends_and_emits_schedule_event():
    state, _, _, step = build("counter-collision")
    result = step.resolve(state, state.execution_context)
    assert len(state.active_effects) == 2
    assert state.active_effects[0].id == state.active_effects[1].id
    assert len(result.events) == 1
    assert result.events[0].metadata["effect"] == "action_control"
