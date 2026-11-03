"""Frozen-source behavioral contracts for Mind Grip and previous-slot selection."""

from live_defeat_traces import operations
from live_mind_grip_traces import base, filter_scenes
from object_execution_traces import reference


def run(branch, action="SKILL"):
    from goa2.domain.input import selection_value
    from goa2.engine.steps import ResolveCardStep, SetContextFlagStep

    state, hero, card, copied, owner = base()
    state.execution_stack = [
        SetContextFlagStep(key="tail", value=True),
        ResolveCardStep(hero_id=hero.id),
    ]

    def select(request, _mode):
        if request.request_type.value == "SELECT_NUMBER":
            return branch
        if request.request_type.value == "CHOOSE_ACTION":
            return "SKILL" if "Choose action for card" in request.prompt else action
        if "previous turn slot" in request.prompt:
            return owner.id
        return selection_value(request.options[0])

    ops, _ = operations(state, select=select)
    records = reference(state, ops)
    return state, hero, card, copied, owner, records


def test_mind_grip_copies_previous_card_and_suppresses_poison_marker():
    state, hero, card, copied, owner, records = run(1)
    assert state.execution_context["tail"]
    assert owner.played_cards[0] is copied
    assert owner.current_turn_card is not copied
    assert hero.current_turn_card is card
    assert not state.markers
    assert state.execution_context["current_card_id"] == card.id
    assert state.execution_context["current_action_type"] == "SKILL"
    assert "action_context_stack" not in state.execution_context
    assert "token_type_override" not in state.execution_context
    assert any(
        row["semantic_state"]["execution_context"].get("token_type_override") == "illusion"
        for row in records
    )
    assert any(
        row["request"] and "Choose an action to perform on" in row["request"]["prompt"]
        for row in records
    )


def test_mind_grip_alternate_choice_defeats_an_adjacent_enemy_minion():
    state, _hero, _card, copied, owner, records = run(2)
    assert state.execution_context["tail"]
    events = [event for row in records for event in row["events"]]
    defeated = [event for event in events if event["event_type"] == "UNIT_DEFEATED"]
    assert len(defeated) == 1
    assert defeated[0]["target_id"] not in state.entity_locations
    assert owner.played_cards[0] is copied
    assert not any(
        row["request"] and "Choose an action to perform on" in row["request"]["prompt"]
        for row in records
    )


def test_previous_slot_filter_uses_game_turn_and_piece_owner():
    checked = 0
    for name, state, ops in filter_scenes():
        record = reference(state, ops)[0]
        turn = state.turn
        if name.endswith("-HEX") or turn == 1:
            assert record["request"] is None
        else:
            ids = {option["id"] for option in record["request"]["options"]}
            assert ids == (
                {"hero_tigerclaw"}
                if turn in (2, 4)
                else {"hero_razzle_piece_1", "hero_razzle_piece_2", "hero_razzle_piece_3"}
            )
        checked += 1
    assert checked == 15
