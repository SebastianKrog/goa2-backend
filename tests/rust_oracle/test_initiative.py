"""Frozen source contracts for Hurry Up!'s temporary physical-card override."""

import pytest
from initiative_traces import build


@pytest.mark.parametrize("value", [0, -3, 1, 11, 99, 2**53 + 1])
def test_override_preserves_printed_initiative_and_effect_list(value):
    state, _, card, step = build("no-override", value)
    printed = card.initiative
    effects = state.active_effects
    result = step.resolve(state, state.execution_context)
    assert card.initiative == printed
    assert card.initiative_override == value
    assert card.current_initiative == value
    assert state.active_effects is effects
    assert not effects
    assert result.events[0].metadata == {
        "effect": "initiative_override",
        "card_id": card.id,
        "value": value,
    }


def test_shared_current_card_changes_once_and_independent_copy_stays_private():
    state, _, card, step = build("shared-current")
    step.resolve(state, state.execution_context)
    assert state.get_hero("hero_brogan").current_turn_card is card
    assert card.initiative_override == 11
    state, _, card, step = build("independent-current")
    other = state.get_hero("hero_brogan").current_turn_card
    step.resolve(state, state.execution_context)
    assert card.initiative_override == 11
    assert other.initiative_override == 3


def test_changing_card_state_clears_override_but_unresolved_assignment_retains_it():
    from goa2.domain.models import CardState

    state, _, card, step = build()
    step.resolve(state, state.execution_context)
    card.state = CardState.UNRESOLVED
    assert card.initiative_override == 11
    card.state = CardState.RESOLVED
    assert card.initiative_override is None
    card.initiative_override = 7
    card.state = CardState.RESOLVED
    assert card.initiative_override is None


def test_piece_target_mutates_owner_card_and_retains_piece_event_target():
    state, hero, card, step = build("piece-offboard")
    piece = state.execution_context["who"]
    result = step.resolve(state, state.execution_context)
    assert piece != hero.id
    assert hero.current_turn_card is card
    assert card.initiative_override == 11
    assert result.events[0].target_id == piece
