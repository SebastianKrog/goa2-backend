"""Independent source assertions for shared resource effect primitives."""

import copy

import pytest
from live_resource_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def resource_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(resource_scenes, name):
    state, ops = copy.deepcopy(resource_scenes["resource-" + name])
    records = reference(state, ops)
    events = [event for row in records for event in row["events"]]
    return state, events


def test_gain_is_signed_and_dynamic_amount_overrides_static(resource_scenes):
    for variant, total in (
        ("plain", 8),
        ("negative", -3),
        ("dynamic", 14),
        ("missing-amount", 8),
        ("dynamic-bool", 6),
        ("repeat", 11),
    ):
        state, events = run(resource_scenes, "gain-" + variant)
        assert state.get_hero("hero_arien").gold == total
        assert all(event["event_type"] == "GOLD_GAINED" for event in events)
        assert len(events) == (2 if variant == "repeat" else 1)
    _, events = run(resource_scenes, "gain-dynamic-bool")
    assert events[0]["metadata"]["amount"] is True


def test_loss_coerces_amount_and_records_actual_loss(resource_scenes):
    for variant, actual in (
        ("plain", 3),
        ("negative", 0),
        ("dynamic", 5),
        ("float", 2),
        ("text", 5),
        ("negative-float", 0),
    ):
        state, events = run(resource_scenes, "lose-" + variant)
        assert state.get_hero("hero_arien").gold == 5 - actual
        assert state.execution_context["actual"] == actual
        assert events[0]["metadata"] == {
            "amount": actual,
            "owner_id": "hero_arien",
            "reason": "effect",
        }
        assert events[0]["actor_id"] == "hero_swift"
        assert events[0]["target_id"] == "hero_arien"


def test_life_cap_and_noop_do_not_emit_events(resource_scenes):
    for variant, total, count in (
        ("plain", 9, 1),
        ("cap", 10, 1),
        ("full", 10, 0),
        ("overfull", 12, 0),
        ("negative", 6, 0),
        ("zero", 6, 0),
        ("repeat", 10, 2),
        ("starting-none", 6, 0),
    ):
        state, events = run(resource_scenes, "life-" + variant)
        assert state.teams[state.get_hero("hero_arien").team].life_counters == total
        assert len(events) == count
        assert all(event["metadata"]["reason"] == "life_counter_restored" for event in events)


def test_missing_heroes_and_guards_leave_gold_unchanged(resource_scenes):
    for kind in ("gain", "lose"):
        for variant in (
            "missing-hero",
            "empty-hero",
            "unknown-hero",
            "numeric-hero",
            "skip",
            "inactive",
        ):
            state, events = run(resource_scenes, kind + "-" + variant)
            assert state.get_hero("hero_arien").gold == 5
            assert not events
            assert "actual" not in state.execution_context


def test_piece_event_identity_and_owner_identity_are_distinct(resource_scenes):
    for kind, total in (("gain", 7), ("lose", 3)):
        state, events = run(resource_scenes, kind + "-piece")
        assert state.get_hero("hero_razzle").gold == total
        event = events[0]
        assert event["actor_id" if kind == "gain" else "target_id"] == "hero_razzle_piece_1"
        if kind == "lose":
            assert event["metadata"]["owner_id"] == "hero_razzle"
    _, events = run(resource_scenes, "life-piece")
    assert events[0]["actor_id"] == "hero_razzle"
