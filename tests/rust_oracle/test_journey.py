"""Frozen Journey lifecycle and delayed-program contracts."""

import pytest
from journey_traces import build


@pytest.mark.parametrize("move_after", [False, True])
def test_literal_return_program_and_manager_defaults(move_after):
    state, card, step = build("context-card", move_after)
    result = step.resolve(state, state.execution_context)
    immunity, delayed = state.active_effects
    assert len(result.events) == 1
    assert result.events[0].metadata == {"effect_type": "immunity_enemy_actions"}
    assert immunity.subject_id == "hero_brogan"
    assert immunity.blocks_friendly_actors and immunity.blocks_enemy_actors
    assert not immunity.blocks_self
    for effect in (immunity, delayed):
        assert effect.source_id == "hero_hanu"
        assert effect.source_card_id == card.id
        assert effect.duration.value == "THIS_TURN"
        assert effect.is_active
        assert effect.origin_action_type is None
    assert len(delayed.finishing_steps) == (3 if move_after else 1)
    swap = delayed.finishing_steps[0]
    assert swap.unit_a_id == "hero_hanu" and swap.unit_b_id == "hero_brogan"
    state.execution_context.clear()
    before = dict(state.entity_locations)
    result = swap.resolve(state, state.execution_context)
    assert len(result.events) == 1
    assert state.entity_locations["hero_hanu"] == before["hero_brogan"]
    assert state.entity_locations["hero_brogan"] == before["hero_hanu"]
    if move_after:
        select, move = delayed.finishing_steps[1:]
        assert select.filters[0].unit_id == move.unit_id == "hero_hanu"
        assert select.output_key == move.destination_key == "journey_move_dest"
        assert not move.is_movement_action


@pytest.mark.parametrize(
    "variant,added",
    [
        ("plain", 2),
        ("existing-immunity", 1),
        ("existing-delayed", 1),
        ("existing-both", 0),
        ("existing-inactive", 0),
    ],
)
def test_card_kind_scope_dedup_preserves_existing_objects(variant, added):
    state, _, step = build(variant, True)
    before = list(state.active_effects)
    counter = state.next_entity_id
    result = step.resolve(state, state.execution_context)
    assert state.next_entity_id == counter + added
    assert all(state.active_effects[i] is effect for i, effect in enumerate(before))
    assert len(result.events) == int(added != 0)


def test_unbound_repetition_retains_private_programs():
    state, _, step = build("unbound", True)
    step.resolve(state, state.execution_context)
    step.resolve(state, state.execution_context)
    a, b = state.active_effects[1::2]
    assert a.finishing_steps is not b.finishing_steps
    assert all(
        left is not right for left, right in zip(a.finishing_steps, b.finishing_steps, strict=True)
    )


def test_card_discard_cancels_return_without_executing_it():
    from goa2.engine.steps import DiscardCardStep

    state, card, step = build("plain", True)
    step.resolve(state, state.execution_context)
    before = dict(state.entity_locations)
    DiscardCardStep(hero_id="hero_hanu", card_id=card.id, source="PLAYED").resolve(
        state, state.execution_context
    )
    assert not state.active_effects
    assert state.entity_locations == before
    assert not card.is_active


def test_duplicate_effect_ids_do_not_emit_creation_event():
    state, _, step = build("counter-collision")
    result = step.resolve(state, state.execution_context)
    assert len(state.active_effects) == 4
    assert not result.events
