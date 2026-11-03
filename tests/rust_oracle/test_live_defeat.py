"""Gold-standard defeat/protection outcomes and authoritative alias contracts.

Raw boundaries intentionally isolate lifecycle primitives and pointer identity;
normal card-effect author helpers do not expose these retained object handles.
"""

import copy

import pytest
from live_defeat_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def defeat_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(defeat_scenes, name):
    state, ops = copy.deepcopy(defeat_scenes[name])
    return state, reference(state, ops)


def test_hero_rewards_assists_life_and_event_order(defeat_scenes):
    from goa2.domain.models import TeamColor

    state, records = run(defeat_scenes, "defeat-rewards-4-hero_swift")
    assert state.get_hero("hero_swift").gold == 4
    assert state.get_hero("hero_hanu").gold == 6
    assert state.teams[TeamColor.BLUE].life_counters == 4
    assert state.execution_context["defeat_log"] == [["hero_brogan", "hero_swift"]]
    assert state.get_position("hero_brogan") is None
    assert [e["event_type"] for row in records for e in row["events"]] == [
        "UNIT_DEFEATED",
        "GOLD_GAINED",
        "GOLD_GAINED",
        "LIFE_COUNTER_CHANGED",
        "UNIT_REMOVED",
    ]


def test_defeat_resolves_the_physical_card_and_consumes_retrieved_slots_once(defeat_scenes):
    state, ops = copy.deepcopy(defeat_scenes["defeat-hero-current"])
    hero = state.get_hero("hero_brogan")
    card = hero.current_turn_card
    played = hero.played_cards
    reference(state, ops)
    assert hero.current_turn_card is None
    assert hero.played_cards is played
    assert hero.played_cards[0] is card
    assert card.state.value == "RESOLVED"
    assert card.initiative_override is None
    assert hero.resolved_turn_count == 1
    state, _ = run(defeat_scenes, "defeat-hero-retrieved")
    assert state.get_hero("hero_brogan").played_cards == [None, None]
    assert state.get_hero("hero_brogan").resolved_turn_count == 2
    state, _ = run(defeat_scenes, "defeat-hero-repeat")
    assert state.get_hero("hero_brogan").resolved_turn_count == 1
    assert len(state.execution_context["defeat_log"]) == 2


def test_tracking_and_log_lists_keep_their_original_identity(defeat_scenes):
    state, _ = run(defeat_scenes, "defeat-hero-shared-tracking")
    assert state.unresolved_hero_ids is state.heroes_defeated_this_round
    assert state.unresolved_hero_ids is state.execution_context["unresolved_alias"]
    assert list(state.unresolved_hero_ids) == ["hero_hanu", "hero_brogan"]
    state, _ = run(defeat_scenes, "defeat-hero-existing-log")
    assert state.execution_context["defeat_log"] is state.execution_context["old_log"]
    assert state.execution_context["defeat_log"][0] == ["before", None]
    assert state.active_effects is not state.execution_context["effects_alias"]


def test_source_expiry_uses_card_owner_and_token_exemptions_without_finishings(defeat_scenes):
    for variant, retained in (
        ("source", False),
        ("foreign-card", True),
        ("owned-card", False),
        ("token", True),
        ("empty-token", True),
        ("inactive", False),
    ):
        state, _ = run(defeat_scenes, "live-combat-effect-expiry-" + variant)
        assert bool(state.active_effects) == retained
        assert state.active_effects is not state.execution_context["effects_alias"]
        assert "should_not_finish" not in state.execution_context
    state, _ = run(defeat_scenes, "defeat-remove-token-False")
    assert state.active_effects is state.execution_context["effects_alias"]
    assert len(state.active_effects) == 1


def test_card_protection_scores_the_defeat_but_totem_sacrifice_does_not(defeat_scenes):
    for variant, alive in (
        ("card", True),
        ("no-protection", False),
        ("totem", True),
    ):
        state, records = run(defeat_scenes, f"live-combat-effect-protection-{variant}-False")
        defeated = [
            e for row in records for e in row["events"] if e["event_type"] == "UNIT_DEFEATED"
        ]
        if variant == "totem":
            assert state.get_hero("hero_swift").gold == 0
            assert not defeated
            assert "defeat_log" not in state.execution_context
            assert state.get_position("protect-totem") is None
        else:
            target = defeated[0]["target_id"]
            expected_gold = state.get_unit(target).value + int(variant == "no-protection")
            assert state.get_hero("hero_swift").gold == expected_gold
            assert (state.get_position(target) is not None) == alive
            assert state.execution_context["last_defeated_minion_id"] == target


def test_retry_constructs_a_new_checker_and_tried_list(defeat_scenes):
    state, ops = copy.deepcopy(defeat_scenes["live-combat-effect-protection-two-skip-True"])
    original = state.execution_context["defeat_steps"][0]
    reference(state, ops[:3])
    retry = state.execution_stack[-1]
    assert retry is not original
    assert retry.tried_effect_ids == ["protect"]
    assert original.tried_effect_ids == []
    assert original.pending_input == {"selection": "SKIP"}


def test_game_over_clears_shared_stacks_and_terminal_reentry_is_inert(defeat_scenes):
    state, records = run(defeat_scenes, "defeat-terminal-RED-RESOLUTION")
    assert state.phase.value == "GAME_OVER"
    assert state.winner.value == "RED"
    assert state.victory_condition == "test-condition"
    assert state.execution_stack is state.execution_context["stack_alias"]
    assert state.execution_stack == []
    assert state.input_stack is state.execution_context["input_alias"]
    assert state.input_stack == []
    assert state.execution_context["queued_model"].id == "fixture/queued-input"
    assert "tail" not in state.execution_context
    assert sum(e["event_type"] == "GAME_OVER" for row in records for e in row["events"]) == 1
    state, records = run(defeat_scenes, "defeat-terminal-RED-GAME_OVER")
    assert state.execution_stack
    assert state.input_stack
    assert not [event for row in records for event in row["events"]]


def test_explicit_kill_credit_and_fallbacks_are_preserved(defeat_scenes):
    for variant, killer in (
        ("literal", "hero_brogan"),
        ("key", "hero_hanu"),
        ("empty-key", "hero_swift"),
        ("no-actor", None),
        ("empty-literal", "hero_hanu"),
        ("missing-key", "hero_swift"),
    ):
        state, _ = run(defeat_scenes, "defeat-credit-" + variant)
        assert state.execution_context["defeat_log"] == [["hero_brogan", killer]]


def test_actual_team_and_roster_have_distinct_reward_roles(defeat_scenes):
    from goa2.domain.models import TeamColor

    state, _ = run(defeat_scenes, "defeat-roster-beneficiary-team")
    minion = state.get_unit(state.execution_context["last_defeated_minion_id"])
    assert state.get_hero("hero_swift").gold == minion.value
    assert state.active_effects[0].max_value == 2
    state, _ = run(defeat_scenes, "defeat-roster-killer-team")
    assert state.get_hero("hero_brogan").gold == 1
    assert state.get_hero("hero_arien").gold == 1
    assert state.get_hero("hero_hanu").gold == 0
    state, _ = run(defeat_scenes, "defeat-roster-victim-team")
    assert state.teams[TeamColor.RED].life_counters == 5
    assert state.teams[TeamColor.BLUE].life_counters == 6


def test_real_empty_hand_turret_bodies_and_annihilation(defeat_scenes):
    from goa2.domain.models import TeamColor

    state, _ = run(defeat_scenes, "defeat-card-emergency_protocol-both-False-first")
    assert len(state.execution_context["self_destruct_victims"]) == 2
    assert state.get_position("hero_hanu") is None
    assert state.get_position("hero_brogan") is None
    assert state.teams[TeamColor.BLUE].life_counters == 4
    assert state.get_position("trinkets_turret") is None
    assert state.execution_context["tail"] is True
    state, _ = run(defeat_scenes, "defeat-card-self_destruct-both-False-first")
    assert state.teams[TeamColor.BLUE].life_counters == 6
    assert state.get_position("hero_hanu") is not None
    assert state.get_position("hero_brogan") is not None
    state, _ = run(defeat_scenes, "defeat-card-emergency_protocol-both-True-first")
    assert state.phase.value == "GAME_OVER"
    assert state.winner.value == "RED"
    assert len(state.execution_context["defeat_log"]) == 1
    assert state.get_position("trinkets_turret") is not None
    assert "tail" not in state.execution_context


def test_repeated_model_references_share_a_budget_but_equal_ids_do_not(defeat_scenes):
    for budget, count, shared in (
        (1, 2, True),
        (2, 2, True),
        (1, 3, True),
        (2, 3, True),
        (1, 2, False),
        (2, 2, False),
    ):
        state, records = run(defeat_scenes, f"defeat-bounty-model-{budget}-{count}-{shared}")
        paid = min(budget, count) if shared else count
        events = [
            e
            for row in records
            for e in row["events"]
            if e["metadata"].get("reason") == "minion_defeat_bounty"
        ]
        assert len(events) == paid
        minion = state.get_unit(state.execution_context["last_defeated_minion_id"])
        assert state.get_hero("hero_swift").gold == minion.value + paid
        assert all(e.max_value == budget - (paid if shared else 1) for e in state.active_effects)
        assert (state.active_effects[0] is state.active_effects[1]) == shared
        assert state.active_effects[0] is state.execution_context["bounty_alias"]
