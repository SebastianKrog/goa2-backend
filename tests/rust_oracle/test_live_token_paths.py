"""Frozen token path generation, copied retries and placement-order contracts."""

import pytest
from live_token_path_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def path_scenes():
    return {name: (state, operations) for name, state, operations in scenes()}


def test_line_scans_past_obstacles_but_trail_leaves_checks_to_children(path_scenes):
    for prefix in ("line", "trail"):
        for variant in ("terrain", "occupied"):
            state, operations = path_scenes[f"token-{prefix}-{variant}"]
            records = reference(state, operations)
            ctx = state.execution_context
            if prefix == "line":
                assert [h["q"] for h in ctx["targets"]] == [2, 3, 4]
            else:
                assert [ctx[f"_trail_hex_{i}"]["q"] for i in range(3)] == [0, 1, 2]
            events = [e for r in records for e in r["events"] if e["event_type"] == "TOKEN_PLACED"]
            assert len(events) == (2 if prefix == "trail" and variant == "occupied" else 3)


def test_source_line_zero_and_negative_count_still_take_first_empty_space(path_scenes):
    for variant in ("zero", "negative"):
        state, operations = path_scenes["token-line-" + variant]
        reference(state, operations)
        assert [h["q"] for h in state.execution_context["targets"]] == [1]


def test_exclusions_compare_coordinates_without_changing_trail_endpoints(path_scenes):
    for variant, expected in (
        ("excluded", [0, 2]),
        ("duplicate-exclusions", [0, 2]),
        ("all-excluded", []),
    ):
        state, operations = path_scenes["token-trail-" + variant]
        reference(state, operations)
        ctx = state.execution_context
        assert [ctx[f"_trail_hex_{i}"]["q"] for i in range(len(expected))] == expected
        assert f"_trail_hex_{len(expected)}" not in ctx
        assert state.execution_stack == []


def test_shortfall_removes_old_supply_before_recomputed_placement(path_scenes):
    checked = 0
    for name, (state, operations) in path_scenes.items():
        if not name.startswith("token-path-removal-"):
            continue
        original = state.execution_context["path_step"]
        # Source retries are a deep model copy retaining the authored step ID.
        reference(state, [operations[0]])
        retry = state.execution_stack[0]
        assert retry is not original
        assert retry.step_id == original.step_id
        assert retry.model_dump() == original.model_dump()
        records = reference(state, operations[1:])
        events = [e["event_type"] for r in records for e in r["events"]]
        removed = [i for i, e in enumerate(events) if e == "TOKEN_REMOVED"]
        placed = [i for i, e in enumerate(events) if e == "TOKEN_PLACED"]
        assert removed and placed and max(removed) < min(placed)
        if name.startswith("token-path-removal-False-"):
            assert [h["q"] for h in state.execution_context["targets"]] == [1, 2, 3]
        assert state.execution_stack == []
        checked += 1
    assert checked == 12
