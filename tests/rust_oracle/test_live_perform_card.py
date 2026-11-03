"""Frozen-source contracts for copied foreign-card actions and scoped state."""

from live_perform_card_traces import base, flow_scenes
from object_execution_traces import reference


def test_previous_slot_uses_game_turn_and_preserves_physical_card():
    from goa2.engine.steps import PerformCardActionStep

    state, actor, owner, card = base("Tigerclaw", "poisoned_dagger")
    state.turn = 4
    owner.played_cards = [None, None, card]
    owner.resolved_turn_count = 1
    step = PerformCardActionStep(card_owner_key="foreign_owner", previous_slot=True)
    prompt = step.resolve(state, state.execution_context).input_request
    assert prompt.decision_context.source_card_id == card.id
    step.pending_input = {"selection": "HOLD"}
    result = step.resolve(state, state.execution_context)
    assert result.is_finished
    assert state.execution_context["performing_card_owner_id"] == owner.id
    assert state.execution_context["performing_card_id"] == card.id
    assert owner.played_cards[2] is card
    assert actor.current_turn_card is not card
    assert result.new_steps[-1].type.value == "restore_action_context"


def test_invalid_choice_does_not_open_a_scope_or_change_outer_context():
    from goa2.engine.steps import PerformCardActionStep

    state, _, _, _ = base("Arien", "noble_blade")
    before = dict(state.execution_context)
    step = PerformCardActionStep(
        card_owner_key="foreign_owner", pending_input={"selection": "DEFENSE"}
    )
    result = step.resolve(state, state.execution_context)
    assert result.requires_input and step.pending_input is None
    assert "action_context_stack" not in state.execution_context
    for key, value in before.items():
        assert state.execution_context[key] is value
    assert all(option.id != "DEFENSE" for option in result.input_request.options)


def test_copied_menu_keeps_negative_attack_and_normalizes_defense_skill():
    from goa2.domain.models import ActionType, StatType
    from goa2.engine.steps import PerformCardActionStep

    state, actor, _, card = base("Tigerclaw", "poisoned_dagger")
    actor.items = {StatType.ATTACK: -9}
    card.primary_action = ActionType.ATTACK
    card.primary_action_value = 2
    menu = (
        PerformCardActionStep(card_owner_key="foreign_owner")
        .resolve(state, state.execution_context)
        .input_request
    )
    assert next(option for option in menu.options if option.id == "ATTACK").metadata["value"] == -7
    card.primary_action = ActionType.DEFENSE_SKILL
    menu = (
        PerformCardActionStep(card_owner_key="foreign_owner")
        .resolve(state, state.execution_context)
        .input_request
    )
    assert menu.options[0].id == "SKILL"
    assert menu.options[0].text.startswith("Primary: SKILL")


def test_complete_copied_actions_restore_outer_scope_and_keep_cards_in_place():
    checked = set()
    for name, state, ops in flow_scenes():
        if not name.endswith("-primary-first") and not name.endswith("-HOLD-first"):
            continue
        before = {
            key: state.execution_context[key]
            for key in (
                "current_card_id",
                "current_action_type",
                "performing_card_id",
                "performing_card_owner_id",
                "token_type_override",
                "skip_markers",
                "substitution_actor_id",
            )
        }
        actor = state.execution_context["actor_alias"]
        owner = state.execution_context["owner_alias"]
        copied = state.execution_context["card_alias"]
        actor_card = actor.current_turn_card
        reference(state, ops)
        assert state.execution_context["copied_tail"]
        assert "action_context_stack" not in state.execution_context
        for key, value in before.items():
            assert state.execution_context[key] is value
        assert actor.current_turn_card is actor_card
        assert any(card is copied for card in owner.deck)
        checked.add(name)
    assert len(checked) == 12


def test_substitution_skips_markers_and_can_suppress_after_resolve():
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PerformCardActionStep

    state, actor, _, card = base("Tigerclaw", "poisoned_dagger")
    step = PerformCardActionStep(
        card_owner_key="foreign_owner",
        token_type_override=TokenType.ILLUSION,
        skip_markers=True,
        suppress_after_resolve_card=True,
        pending_input={"selection": card.current_primary_action.value},
    )
    result = step.resolve(state, state.execution_context)
    assert state.execution_context["substitution_actor_id"] == actor.id
    assert state.execution_context["token_type_override"] == "illusion"
    assert state.execution_context["skip_markers"] is True
    assert not any(
        getattr(child, "trigger", None) == "after_resolve_card" for child in result.new_steps
    )


def test_piece_performer_binds_primary_body_to_owner_but_secondary_to_piece():
    from live_perform_card_traces import piece_scenes

    for name, state, ops in piece_scenes():
        stack = reference(state, ops)[0]["semantic_state"]["execution_stack"]
        if name.endswith("-primary"):
            assert (
                next(step for step in stack if step["type"] == "place_unit")["unit_id"]
                == "hero_razzle"
            )
        else:
            assert (
                next(step for step in stack if step["type"] == "move_sequence")["unit_id"]
                == "hero_razzle_piece_1"
            )
