from discard_effect_traces import state_for


def test_filtered_forced_discard_offers_a_nonbasic_shield_and_expires_it():
    from goa2.domain.input import InputResponse
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import ForceDiscardStep

    state, hero, shield = state_for("plain")
    assert not shield.is_basic
    hand_before = len(hero.hand)
    push_steps(state, [ForceDiscardStep(victim_id="hero_mrak", card_is_basic=True)])
    req = process_stack(state).input_request
    assert req is not None and shield.id in [o.id for o in req.options]
    submit_input(state, InputResponse(request_id=req.id, selection=shield.id))
    assert process_stack(state).input_request is None
    assert len(hero.hand) == hand_before
    assert any(c.id == shield.id for c in hero.discard_pile)
    assert all(e.source_card_id != shield.id for e in state.active_effects)
    assert "should_not_finish" not in state.execution_context


def test_effect_passive_provider_has_an_offer_only_for_its_matching_trigger():
    from goa2.engine.steps import CheckPassiveAbilitiesStep

    state, hero, _ = state_for("irrelevant-provider")
    empty = CheckPassiveAbilitiesStep(hero_id="hero_mrak", trigger="after_card_discard")
    assert not empty.resolve(state, state.execution_context).new_steps
    matching = CheckPassiveAbilitiesStep(hero_id="hero_mrak", trigger="after_push")
    offers = matching.resolve(state, state.execution_context).new_steps
    assert len(offers) == 1
    assert offers[0].active_effect_id == "grant"
    assert offers[0].card_id == hero.hand[0].id
