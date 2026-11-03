"""Frozen combat arithmetic and attack-tracking boundary contracts.

Raw source fixtures are used deliberately for context aliases, scalar variants
and independent internal steps, rather than normal effect-author helpers.
"""

import copy

import pytest
from live_combat_resolution_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def combat_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(combat_scenes, name):
    state, ops = copy.deepcopy(combat_scenes[name])
    return state, reference(state, ops)


def test_negative_attacks_compare_without_zero_clamping(combat_scenes):
    for damage in (-16, -1, 0):
        for defense in (None, -17, -16, 0, 3):
            state, records = run(combat_scenes, f"combat-live-values-{damage}-{defense}-0")
            expected = defense is not None and defense >= damage
            assert state.execution_context["block_succeeded"] == expected
            event = next(
                e for row in records for e in row["events"] if e["event_type"] == "COMBAT_RESOLVED"
            )
            assert event["metadata"]["attack_value"] == damage
            assert event["metadata"]["outcome"] == ("BLOCKED" if expected else "DEFEATED")


def test_flags_precedence_cancellation_and_unused_values(combat_scenes):
    for variant, blocked in (
        ("invalid", False),
        ("auto", True),
        ("both", False),
        ("ignore", True),
        ("initiative", False),
        ("unused-defense", True),
    ):
        state, _ = run(combat_scenes, "combat-live-flags-" + variant)
        assert state.execution_context["block_succeeded"] == blocked
    for variant in ("missing-target", "offboard"):
        state, records = run(combat_scenes, "combat-live-flags-" + variant)
        assert state.execution_context["block_succeeded"] is False
        assert state.execution_context["last_combat_target"] == "old"
        assert not [e for row in records for e in row["events"]]
    for variant in ("skip", "active"):
        state, _ = run(combat_scenes, "combat-live-flags-" + variant)
        assert state.execution_context["block_succeeded"] == "old"


def test_fractional_values_and_exact_large_comparisons(combat_scenes):
    for index, blocked in enumerate((False, True, True, True, True, True, False, True, False)):
        state, _ = run(combat_scenes, f"combat-live-precision-{index}")
        assert state.execution_context["block_succeeded"] == blocked
    state, records = run(combat_scenes, "combat-live-values-4-2.5-0.5")
    assert state.execution_context["block_succeeded"] is False
    event = records[0]["events"][0]
    assert event["metadata"]["defense_value"] == 2.5
    assert event["metadata"]["modifier_value"] == 0.5


def test_bonus_coercion_and_custom_target_snapshot(combat_scenes):
    for variant, blocked in (("bonus", False), ("bonus-bool", False), ("bonus-string", True)):
        state, _ = run(combat_scenes, "combat-live-flags-" + variant)
        assert state.execution_context["block_succeeded"] == blocked
    state, _ = run(combat_scenes, "combat-live-flags-alias-block")
    assert state.execution_context["last_combat_target"] == "hero_brogan"
    assert state.execution_context["block_succeeded"] is True


def test_snapshot_replaces_the_old_list_and_is_not_a_target_selection(combat_scenes):
    state, records = run(combat_scenes, "combat-snapshot-targeting-independent")
    candidate = state.execution_context["denied_candidate"]
    assert state.execution_context["neighbors"] == [candidate]
    assert not state.validator.can_be_targeted(
        state, str(state.current_actor_id), candidate, state.execution_context
    ).allowed
    assert not [e for row in records for e in row["events"]]
    for relation in ("ENEMY", "FRIENDLY", "ANY"):
        state, _ = run(combat_scenes, "combat-snapshot-plain-" + relation)
        assert state.execution_context["old_neighbors"] == ["old"]
        assert state.execution_context["neighbors"] is not state.execution_context["old_neighbors"]


def test_snapshot_without_an_actor_team_returns_a_fresh_empty_list(combat_scenes):
    for name in ("no-actor", "empty-actor", "unknown-actor"):
        state, _ = run(combat_scenes, f"combat-snapshot-{name}-ANY")
        assert state.execution_context["neighbors"] == []
        assert state.execution_context["neighbors"] is not state.execution_context["old_neighbors"]
    state, _ = run(combat_scenes, "combat-snapshot-token-actor")
    assert state.execution_context["neighbors"] == []


def test_defeat_marks_limit_history_and_by_actor_filters_kill_credit(combat_scenes):
    history = [["hero_brogan", "hero_hanu"], ["hero_brogan", "hero_swift"]]
    for mark, expected in (
        (None, False),
        (-1, True),
        (0, True),
        (1, True),
        (2, False),
        (100, False),
        (False, True),
        (True, True),
    ):
        name = f"combat-track-{mark}-True-{len(history)}-{history}"
        state, _ = run(combat_scenes, name)
        assert state.execution_context["chosen"] == expected
        assert state.execution_context["marks_alias"] is state.execution_context["defeat_marks"]
        assert state.execution_context["log_alias"] is state.execution_context["defeat_log"]
    history = [["hero_brogan", "hero_hanu"], ["hero_brogan", "hero_swift"]]
    state, _ = run(combat_scenes, f"combat-track-0-False-2-{history}")
    assert state.execution_context["chosen"] is True


def test_piece_killers_normalize_but_victim_ids_remain_exact(combat_scenes):
    for variant, expected in (
        ("piece-killer", True),
        ("owner-killer", True),
        ("wrong-victim", False),
        ("no-actor", False),
        ("unknown-actor", True),
        ("numeric-target", True),
    ):
        state, _ = run(combat_scenes, "combat-track-special-" + variant)
        assert state.execution_context["chosen"] == expected


def test_missing_history_and_tuple_history_preserve_source_behavior(combat_scenes):
    for variant, expected in (
        ("missing-target", False),
        ("missing-mark", False),
        ("missing-log", False),
        ("empty-log-aggregate-target", False),
        ("tuple-log", True),
    ):
        state, _ = run(combat_scenes, "combat-track-special-" + variant)
        assert state.execution_context["chosen"] == expected


def test_tracking_can_replace_its_marks_output_without_mutating_the_old_mapping(combat_scenes):
    state, ops = copy.deepcopy(combat_scenes["combat-track-special-output-marks"])
    old_marks = state.execution_context["defeat_marks"]
    reference(state, ops)
    assert state.execution_context["defeat_marks"] is True
    assert old_marks == {"attack_target": 0}
