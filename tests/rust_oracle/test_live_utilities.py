"""Source contracts for coordinate model identities and repeat continuations."""

import copy

import pytest
from live_utility_traces import scenes
from object_execution_traces import reference

from goa2.domain.hex import Hex


@pytest.fixture(scope="module")
def utility_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(utility_scenes, name):
    state, ops = copy.deepcopy(utility_scenes["utility-" + name])
    return state, reference(state, ops)


def test_record_creates_fresh_mutable_dicts_not_hex_models(utility_scenes):
    state, _ = run(utility_scenes, "record-fresh")
    first, second = state.execution_context["history"]
    assert type(first) is dict and type(second) is dict
    assert first == second == {"q": 1, "r": 0, "s": -1}
    assert first is not second
    assert state.execution_context["old_output"] == {"before": True}
    assert state.execution_context["position_alias"] is state.entity_locations["hero_brogan"]


def test_computed_hex_is_fresh_and_preserves_hex_deepcopy_contract(utility_scenes):
    state, _ = run(utility_scenes, "compute-fresh")
    first, second = state.execution_context["history"]
    assert isinstance(first, Hex) and isinstance(second, Hex)
    assert first == second == Hex(q=2, r=0, s=-2)
    assert first is not second
    assert copy.deepcopy(first) is first
    state, _ = run(utility_scenes, "compute-zero")
    assert state.execution_context["output"] == state.entity_locations["hero_brogan"]
    assert state.execution_context["output"] is not state.entity_locations["hero_brogan"]


def test_coordinate_missing_values_and_literal_precedence(utility_scenes):
    for kind in ("record", "compute"):
        for variant in (
            "missing-key",
            "empty-key",
            "unknown",
            "offboard",
            "numeric",
            "skip",
            "inactive",
        ):
            state, _ = run(utility_scenes, kind + "-" + variant)
            assert state.execution_context["output"] == {"before": True}
        state, _ = run(utility_scenes, kind + "-literal-wins")
        assert state.execution_context["output"] != {"before": True}


def test_adjacency_uses_topology_and_missing_units_are_false(utility_scenes):
    for variant in ("missing-key", "empty-key", "unknown", "offboard", "numeric", "same", "far"):
        state, _ = run(utility_scenes, "adjacency-" + variant)
        assert state.execution_context["output"] is False
    state, _ = run(utility_scenes, "adjacency-topology-plain")
    assert state.execution_context["output"] is True
    state, _ = run(utility_scenes, "adjacency-topology-split")
    assert state.execution_context["output"] is True  # ZERO bridges both sides.
    state, _ = run(utility_scenes, "adjacency-topology-isolated")
    assert state.execution_context["output"] is False
    state, _ = run(utility_scenes, "adjacency-topology-reverse")
    assert state.execution_context["output"] is True


def test_repeats_copy_each_root_and_keep_original_templates_unchanged(utility_scenes):
    state, records = run(utility_scenes, "repeat-three")
    assert state.execution_context["counter"] == 3
    assert state.execution_context["repeat_alias"].repeats_done == 3
    history = state.execution_context["history"]
    assert len(history) == 6
    assert len({id(value) for value in history}) == 6
    assert all(value is not state.execution_context["template_value"] for value in history)
    assert len([row for row in records if row["request"]]) == 3
    assert state.execution_context["template_value"] == [{"nested": []}]


def test_declines_blocked_limits_and_missing_actors_do_not_execute_templates(utility_scenes):
    for variant in (
        "no",
        "invalid",
        "limit",
        "zero-limit",
        "negative-limit",
        "no-actor",
        "empty-actor",
        "skip",
        "inactive",
        "blocked",
        "pending-other",
    ):
        state, _ = run(utility_scenes, "repeat-" + variant)
        assert "counter" not in state.execution_context
        assert state.execution_context["tail"]
    state, _ = run(utility_scenes, "repeat-pending-other")
    assert state.execution_context["repeat_alias"].pending_input is None


def test_live_template_alias_edits_and_nested_counters_are_private(utility_scenes):
    state, _ = run(utility_scenes, "repeat-aliases")
    assert state.execution_context["template_value"][-1] == {"injected": True}
    assert all(value[-1] == {"injected": True} for value in state.execution_context["history"])
    state, _ = run(utility_scenes, "repeat-nested")
    assert state.execution_context["counter"] == 2
    assert state.execution_context["templates_alias"][0].repeats_done == 0


def test_lane_check_preserves_the_live_repeat_and_precedes_requests(utility_scenes):
    state, records = run(utility_scenes, "repeat-lane")
    assert state.execution_stack[-1].type.value == "check_lane_push"
    assert state.execution_stack[-2] is state.execution_context["repeat_alias"]
    assert state.execution_context["repeat_alias"].lane_push_checked
    assert not any(row["request"] for row in records)


def test_direct_coordinate_lookups_do_not_resolve_razzle_owner_to_acting_piece(utility_scenes):
    for kind in ("record", "compute", "adjacency"):
        state, _ = run(utility_scenes, "multipiece-" + kind + "-owner")
        assert (
            state.execution_context["output"] is False
            if kind == "adjacency"
            else state.execution_context["output"] == "unchanged"
        )
        state, _ = run(utility_scenes, "multipiece-" + kind + "-piece")
        assert state.execution_context["output"] != "unchanged"
        if kind == "adjacency":
            assert state.execution_context["output"] is True


def test_complete_cards_reach_real_combat_and_repeat_movement(utility_scenes):
    for effect in ("onslaught", "blink_strike"):
        state, records = run(utility_scenes, "card-" + effect + "-first")
        assert not state.execution_stack
        assert any(
            event["event_type"] in {"UNIT_DEFEATED", "UNIT_MOVED", "UNIT_PLACED"}
            for row in records
            for event in row["events"]
        )
    for effect in ("troop_movement", "marching_orders", "path_to_victory"):
        _, records = run(utility_scenes, "card-" + effect + "-first")
        assert (
            len(
                [
                    event
                    for row in records
                    for event in row["events"]
                    if event["event_type"] == "UNIT_MOVED"
                ]
            )
            >= 2
        )


def test_foreach_return_revalidates_nested_repeat_templates(utility_scenes):
    state, _ = run(utility_scenes, "repeat-in-foreach")
    assert state.execution_context["counter"] == 1
    assert state.execution_context["repeat_alias"].repeats_done == 0
    a, b = state.execution_context["history"]
    assert a == b == state.execution_context["template_value"]
    assert a is not b
    assert a is not state.execution_context["template_value"]
