"""Source contracts for live marker identity, context and triggered passives."""

import copy

import pytest
from live_marker_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def marker_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(marker_scenes, name):
    state, ops = copy.deepcopy(marker_scenes[name])
    return state, reference(state, ops)


def events(records, event_type):
    return [
        event
        for record in records
        for event in record["events"]
        if event["event_type"] == event_type
    ]


def test_existing_marker_and_dictionary_aliases_survive_placement(marker_scenes):
    for kind in ("venom", "poison", "bounty"):
        state, records = run(marker_scenes, f"live-marker-{kind}-existing")
        ctx = state.execution_context
        assert state.markers is ctx["markers_alias"]
        assert state.markers[kind] is ctx["marker_alias"]
        assert state.markers[kind].target_id == "hero_brogan"
        assert state.markers[kind].source_id == "hero_swift"
        assert state.markers[kind].value == -2
        assert ctx["marker_target_id"] == "hero_brogan"
        assert len(events(records, "MARKER_PLACED")) == 1


def test_shared_marker_under_different_type_keys_keeps_actual_model_type(marker_scenes):
    for kind in ("venom", "poison", "bounty"):
        state, _ = run(marker_scenes, f"live-marker-{kind}-shared-types")
        assert len(state.markers) == 2
        assert all(
            marker is state.execution_context["marker_alias"] for marker in state.markers.values()
        )
        assert all(
            marker.type.value == kind and marker.value == -2 for marker in state.markers.values()
        )


def test_removal_retains_model_and_emits_even_when_already_in_supply(marker_scenes):
    for variant in ("remove", "remove-supply", "remove-repeat"):
        state, records = run(marker_scenes, "live-marker-bounty-" + variant)
        marker = state.markers["bounty"]
        assert marker is state.execution_context["marker_alias"]
        assert marker.target_id is marker.source_id is None
        assert marker.value == 0
        assert len(events(records, "MARKER_REMOVED")) == (2 if variant == "remove-repeat" else 1)
    state, records = run(marker_scenes, "live-marker-bounty-remove-absent")
    assert not state.markers
    assert not events(records, "MARKER_REMOVED")


def test_repeated_steps_read_live_target_and_actor(marker_scenes):
    state, records = run(marker_scenes, "live-marker-venom-live-target")
    placed = events(records, "MARKER_PLACED")
    assert [event["target_id"] for event in placed] == ["hero_brogan", "hero_arien"]
    assert state.markers["venom"].target_id == "hero_arien"
    state, records = run(marker_scenes, "live-marker-venom-live-actor")
    assert [event["actor_id"] for event in events(records, "MARKER_PLACED")] == [
        "hero_swift",
        "hero_hanu",
    ]
    assert state.markers["venom"].source_id == "hero_hanu"


def test_marker_suppression_belongs_to_performer_and_system_is_fallback(marker_scenes):
    for variant in ("skip-performer", "inactive", "skip-gate", "missing", "empty-key"):
        _state, records = run(marker_scenes, "live-marker-poison-" + variant)
        assert not events(records, "MARKER_PLACED"), variant
    for variant in ("skip-reactor", "no-skip-performer"):
        _state, records = run(marker_scenes, "live-marker-poison-" + variant)
        assert len(events(records, "MARKER_PLACED")) == 1
    for variant in ("no-actor", "empty-actor", "skip-no-actor"):
        state, records = run(marker_scenes, "live-marker-poison-" + variant)
        assert events(records, "MARKER_PLACED")[0]["actor_id"] == "system"
        assert state.markers["poison"].source_id == "system"


def test_literal_skip_and_unknown_targets_are_still_marker_targets(marker_scenes):
    for variant, target in (("skip-target", "SKIP"), ("unknown", "unknown")):
        state, records = run(marker_scenes, "live-marker-bounty-" + variant)
        assert state.markers["bounty"].target_id == target
        assert events(records, "MARKER_PLACED")[0]["target_id"] == target


def test_bain_passive_runs_for_each_marker_type_without_forcing_empty_hand(marker_scenes):
    for kind in ("venom", "poison", "bounty"):
        state, records = run(marker_scenes, f"live-marker-bain-{kind}-hand")
        original = marker_scenes[f"live-marker-bain-{kind}-hand"][0]
        assert (
            len(state.get_hero("hero_brogan").hand)
            == len(original.get_hero("hero_brogan").hand) - 1
        )
        assert sum(row["request"] is not None for row in records) >= 1
        assert [event["event_type"] for row in records for event in row["events"]] == [
            "MARKER_PLACED"
        ]
        state, records = run(marker_scenes, f"live-marker-bain-{kind}-empty")
        assert not state.get_hero("hero_brogan").hand
        assert not any(row["request"] is not None for row in records)
