"""Whole-game policies choose offered actions without altering the rules state."""

import random

from live_game_traces import choose_hanu_input, choose_input
from live_hanu_primary_traces import base


def action_request(state, hero_id):
    from goa2.engine.handler import process_stack
    from goa2.engine.steps import ResolveCardStep

    state.current_actor_id = state.resolution_owner_id = hero_id
    state.execution_stack = [ResolveCardStep(hero_id=hero_id)]
    return process_stack(state).input_request


def test_hanu_game_policy_exercises_an_offered_primary_skill_without_mutation():
    state, hero, _ = base("unexpected_journey")
    request = action_request(state, hero.id)
    before = state.model_dump(mode="json")
    choice = choose_hanu_input(state, request, random.Random(0))
    assert choice == "SKILL" and choice in [option.id for option in request.options]
    assert state.model_dump(mode="json") == before


def test_hanu_policy_keeps_baseline_choices_for_other_acting_heroes():
    state, _, _ = base("hurry_up")
    request = action_request(state, "hero_brogan")
    before = state.model_dump(mode="json")
    assert choose_hanu_input(state, request, random.Random(0)) == choose_input(
        state, request, random.Random(0)
    )
    assert state.model_dump(mode="json") == before
