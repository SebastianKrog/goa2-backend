"""Frozen battle contracts assert mutation order and physical object identity."""

import copy

import pytest
from live_battle_traces import scenes
from object_execution_traces import reference

NAMES = {
    "live-battle-red-push-first",
    "live-battle-multi-both-thrones-same-hex-respawn-first",
    "live-battle-spawn-blocked-token-linked-effects-first",
    "live-battle-casualties-choice-pending-valid-first",
    "live-battle-casualties-choice-numeric-id-first",
    "live-battle-displace-two-red-duplicate-unit-first",
    "live-battle-casualties-force-over-pending-forced-first",
    "live-battle-displace-one-choice-pending-invalid-first",
    "live-battle-displace-one-choice-pending-hex-first",
    "live-battle-displace-two-teams-hero-default-lane-first",
    "live-battle-displace-two-teams-null-coin-first",
    "live-battle-spawn-blocked-one-non-minion-prefix-first",
    "live-battle-minions-tie-battle-exclusion-first",
}


@pytest.fixture(scope="module")
def inputs():
    values = {name: (state, ops) for name, state, ops in scenes(NAMES)}
    assert values.keys() == NAMES
    return values


def run(inputs, name):
    original, ops = inputs[name]
    state = copy.deepcopy(original)
    records = reference(state, ops)
    return state, records


def test_push_updates_aliased_maps_and_uses_the_original_spawn_hex(inputs):
    state, _ = run(inputs, "live-battle-red-push-first")
    assert state.execution_context["battle_zone_alias"] is state.battle_zones
    assert state.execution_context["wave_alias"] is state.wave_counters
    assert state.execution_context["location_alias"] is state.entity_locations
    assert state.battle_zones["lane_1"] == "lane_1-3" and state.wave_counters["lane_1"] == 4
    unit = state.teams["RED"].minions[0]
    spawn = next(sp for sp in state.board.zones["lane_1-3"].spawn_points if str(sp.team) == "RED")
    assert state.entity_locations[unit.id] is spawn.location


def test_same_hex_respawn_replaces_the_location_hex_after_wiping_the_unit(inputs):
    state, _ = run(inputs, "live-battle-multi-both-thrones-same-hex-respawn-first")
    for unit, previous in state.execution_context["old_position_aliases"]:
        current = state.entity_locations[unit]
        assert current == previous and current is not previous
        assert any(
            current is sp.location for z in state.board.zones.values() for sp in z.spawn_points
        )
    assert state.winner is None
    assert list(state.wave_counters.values()) == [4, 4]


def test_spawn_token_removal_replaces_only_the_active_effect_list_and_emits_no_events(inputs):
    state, records = run(inputs, "live-battle-spawn-blocked-token-linked-effects-first")
    old = state.execution_context["effects_alias"]
    assert len(old) == 4 and old is not state.active_effects
    assert [effect.id for effect in state.active_effects] == ["unrelated"]
    assert state.get_position("battle_token") is None
    assert not any(row["events"] for row in records)


def test_chosen_casualty_clears_pending_but_forced_casualties_retain_it(inputs):
    chosen, _ = run(inputs, "live-battle-casualties-choice-pending-valid-first")
    assert chosen.execution_context["battle_step_alias"].pending_input is None
    assert chosen.execution_context["pending_alias"]["selection"]
    forced, _ = run(inputs, "live-battle-casualties-force-over-pending-forced-first")
    assert (
        forced.execution_context["battle_step_alias"].pending_input
        is forced.execution_context["pending_alias"]
    )
    assert forced.execution_context["pending_alias"] == {"selection": "stale"}


def test_invalid_destination_is_cleared_before_forced_placement(inputs):
    state, _ = run(inputs, "live-battle-displace-one-choice-pending-invalid-first")
    assert state.execution_context["battle_step_alias"].pending_input is None
    assert state.execution_context["pending_alias"] == {"selection": "bad"}
    assert state.get_position(state.teams["RED"].minions[0].id).q == 1


def test_valid_hex_object_remains_shared_with_pending_input_and_final_location(inputs):
    state, _ = run(inputs, "live-battle-displace-one-choice-pending-hex-first")
    chosen = state.execution_context["selected_hex_alias"]
    assert (
        state.execution_context["battle_step_alias"].pending_input
        is state.execution_context["pending_alias"]
    )
    assert state.execution_context["pending_alias"]["selection"] is chosen
    assert state.entity_locations[state.teams["RED"].minions[0].id] is chosen


def test_hero_displacement_uses_the_default_lane_and_null_coin_prioritizes_blue(inputs):
    state, _ = run(inputs, "live-battle-displace-two-teams-hero-default-lane-first")
    assert (
        state.get_position("hero_arien")
        != state.execution_context["battle_step_alias"].displacements[0][1]
    )
    _, records = run(inputs, "live-battle-displace-two-teams-null-coin-first")
    first = next(row["request"] for row in records if row["request"])
    assert first["player_id"] == "team:BLUE"


def test_non_minion_spawn_prefix_is_skipped_without_losing_source_indices(inputs):
    state, _ = run(inputs, "live-battle-spawn-blocked-one-non-minion-prefix-first")
    assert sum(state.get_position(m.id) is not None for m in state.teams["RED"].minions) == 3
    assert sum(state.get_position(m.id) is not None for m in state.teams["BLUE"].minions) == 3


def test_active_battle_exclusion_changes_casualties_without_changing_effects(inputs):
    state, _ = run(inputs, "live-battle-minions-tie-battle-exclusion-first")
    assert len(state.active_effects) == 1 and state.active_effects[0].id == "battle-exclusion"
    assert sum(state.get_position(m.id) is not None for m in state.teams["BLUE"].minions) == 1


def test_numeric_casualty_selection_uses_source_string_conversion(inputs):
    state, _ = run(inputs, "live-battle-casualties-choice-numeric-id-first")
    assert state.get_position("1") is None
    assert state.execution_context["battle_step_alias"].pending_input is None
    assert state.execution_context["pending_alias"] == {"selection": 1}


def test_split_duplicate_displacements_preserves_each_equal_origin(inputs):
    state, _ = copy.deepcopy(inputs["live-battle-displace-two-red-duplicate-unit-first"])
    left, right = state.execution_context["duplicate_origin_aliases"]
    assert left == right and left is not right
    step = state.execution_stack[-1]
    step.pending_input = {"selection": step.displacements[0][0]}
    reference(state, [{"kind": "advance_prefix", "steps": 1}])
    chosen, remaining = state.execution_stack[-1], state.execution_stack[-2]
    assert chosen.displacements[0][1] is left
    assert remaining.displacements[0][1] is right
