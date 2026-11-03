"""Frozen batch feasibility, opt-in, dead-end avoidance and filter alias contracts."""

import pytest
from live_token_batch_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def batch_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def test_declining_batch_skips_removal_placement_and_completion_flag(batch_scenes):
    for variant in ("optin", "zero"):
        state, ops = batch_scenes[f"token-batch-{variant}-last"]
        records = reference(state, ops)
        assert state.execution_context["b_go"] is None
        assert "placed" not in state.execution_context
        assert state.execution_context["tail"] is True
        assert "b_owner" in state.execution_context
        assert not [e for r in records for e in r["events"]]
        assert "_batch_freed_hexes" not in state.execution_context


def test_infeasible_batches_do_not_offer_optin_or_write_owner_context(batch_scenes):
    for variant in (
        "impossible",
        "mandatory-impossible",
        "oversupply",
        "mandatory-oversupply",
        "negative",
        "bad-slots",
    ):
        state, ops = batch_scenes[f"token-batch-{variant}-first"]
        records = reference(state, ops)
        assert all(r["request"] is None for r in records)
        assert "b_owner" not in state.execution_context
        assert "placed" not in state.execution_context
        assert state.execution_context.get("tail", False) == (not variant.startswith("mandatory-"))


def test_later_reserved_hex_cannot_be_taken_by_the_first_slot(batch_scenes):
    for mode in ("first", "last"):
        state, ops = batch_scenes[f"token-batch-reserve-last-{mode}"]
        reference(state, ops)
        ctx = state.execution_context
        assert ctx["b_hex_0"] != ctx["one"]
        assert ctx["b_hex_1"] == ctx["one"]
        assert ctx["placed"] is True and ctx["tail"] is True
        assert "_batch_freed_hexes" not in ctx


def test_removal_steering_frees_the_required_space_before_any_placement(batch_scenes):
    for variant in ("remove-needed", "all-removals"):
        state, ops = batch_scenes[f"token-batch-{variant}-invalid"]
        records = reference(state, ops)
        events = [e for r in records for e in r["events"]]
        removed = [i for i, e in enumerate(events) if e["event_type"] == "TOKEN_REMOVED"]
        placed = [i for i, e in enumerate(events) if e["event_type"] == "TOKEN_PLACED"]
        assert len(removed) == (1 if variant == "remove-needed" else 3)
        assert max(removed) < min(placed)
        assert state.execution_context["placed"] is True
        assert state.active_effects is not state.execution_context["old_effects"]
        assert state.entity_locations is state.execution_context["locations_alias"]


def test_generated_batch_containers_are_fresh_but_filter_models_are_shared(batch_scenes):
    from goa2.engine.steps import SelectStep

    state, ops = batch_scenes["token-batch-reserve-last-invalid"]
    original = state.execution_context["batch_alias"]
    reference(state, [ops[0]])
    first, second = [
        step for step in reversed(state.execution_stack) if isinstance(step, SelectStep)
    ]
    a, b = first.filters[-1], second.filters[-1]
    assert a.slot_keys == b.slot_keys and a.slot_keys is not b.slot_keys
    assert a.slot_filters is not b.slot_filters
    assert a.slot_filters[0] is not b.slot_filters[0]
    assert first.filters[0] is original.slot_filters[0][0]
    assert first.filters[0] is a.slot_filters[0][0] is b.slot_filters[0][0]
    assert first.filters[1] is a.slot_filters[0][-1] is b.slot_filters[0][-1]
    assert first.filters[1] is not second.filters[1]
    reference(state, ops[1:])
    assert state.execution_stack == []


def test_source_authored_glitch_batches_preserve_spacing_and_decline(batch_scenes):
    checked = 0
    for name, (state, ops) in batch_scenes.items():
        if not name.startswith("token-batch-card-"):
            continue
        step = state.execution_context["batch_alias"]
        records = reference(state, ops)
        placed = [e for r in records for e in r["events"] if e["event_type"] == "TOKEN_PLACED"]
        declined = name.endswith("-last") and ("-flashback-" in name or "-deja_vu-" in name)
        assert len(placed) == (0 if declined else step.count)
        if not declined and step.token_type.value == "glitch":
            points = [state.get_position(e["target_id"]) for e in placed]
            assert all(a.distance(b) >= 3 for i, a in enumerate(points) for b in points[i + 1 :])
        assert state.execution_stack == []
        assert "_batch_freed_hexes" not in state.execution_context
        checked += 1
    assert checked == 24
