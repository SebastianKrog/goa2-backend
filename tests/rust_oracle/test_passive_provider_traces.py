from passive_provider_traces import state_for


def test_granted_passive_uses_the_creator_and_keeps_foreign_card_usage_separate():
    from goa2.domain.input import InputResponse, selection_value
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import CheckPassiveAbilitiesStep

    state, owner, carrier, provider = state_for("plain")
    push_steps(state, [CheckPassiveAbilitiesStep(hero_id=owner, trigger="before_action")])
    req = process_stack(state).input_request
    assert req is not None and req.player_id == owner
    submit_input(state, InputResponse(request_id=req.id, selection="YES"))
    req = process_stack(state).input_request
    assert req is not None and req.request_type.value == "SELECT_HEX"
    target = selection_value(req.options[0])
    submit_input(state, InputResponse(request_id=req.id, selection=target))
    assert process_stack(state).input_request is None
    assert provider.passive_uses_this_turn == 1
    assert carrier.passive_uses_this_turn == 7


def test_marking_an_inactive_provider_does_not_require_a_live_carrier():
    from goa2.engine.steps import MarkPassiveUsedStep

    state, _, _, provider = state_for("missing-card")
    provider.is_active = False
    assert state.get_card_by_id(provider.source_card_id) is None
    MarkPassiveUsedStep(card_id=provider.source_card_id, active_effect_id=provider.id).resolve(
        state, state.execution_context
    )
    assert provider.passive_uses_this_turn == 1
