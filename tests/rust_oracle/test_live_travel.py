"""Independent contracts for fast travel and retained physical destination models."""

from live_travel_traces import edge_scenes
from object_execution_traces import reference


def test_travel_restrictions_card_precedence_and_action_lifecycles():
    successful = {
        "literal-hex",
        "context-hex",
        "context-dict",
        "literal-wins",
        "snapshot-shared",
        "except-current",
        "except-source",
        "except-empty-source",
        "except-physical",
        "placement-only",
        "barrier",
        "action-gold",
        "action-red",
    }
    checked = 0
    for name, state, operations in edge_scenes():
        records = reference(state, operations)
        variant = name.removeprefix("arena-travel-edge-")
        events = [event for record in records for event in record["events"]]
        assert bool(events) == (variant in successful), variant
        assert all(event["event_type"] == "UNIT_PLACED" for event in events), variant
        if variant in successful and not variant.startswith("action-"):
            assert records[-1]["semantic_state"]["entity_locations"]["hero_arien"] == {
                "q": 2,
                "r": -1,
                "s": -1,
            }
        if variant in (
            "blocked-movement",
            "physical-null",
            "physical-dict",
            "physical-text",
            "facedown",
            "missing-grant",
            "empty-destination",
            "skip-destination",
        ):
            assert records[-1]["semantic_state"]["execution_context"]["travel_continued"] is True
        if variant in ("blocked-travel", "topology-split", "bad-destination"):
            assert "travel_continued" not in records[-1]["semantic_state"]["execution_context"]
        checked += 1
    assert checked == 24


def test_travel_reuses_frozen_hexes_but_constructs_a_new_hex_from_dicts():
    checked = 0
    for name, state, operations in edge_scenes():
        variant = name.removeprefix("arena-travel-edge-")
        if variant not in ("literal-hex", "context-hex", "context-dict", "snapshot-shared"):
            continue
        record = reference(state, operations)[-1]
        bindings = record["object_bindings"]
        position = next(
            binding
            for binding in bindings
            if ["entity_locations", "hero_arien"] in binding["paths"]
        )
        assert (["execution_context", "destination_alias"] in position["paths"]) == (
            variant != "context-dict"
        )
        if variant == "snapshot-shared":
            locations = next(
                binding for binding in bindings if ["entity_locations"] in binding["paths"]
            )
            assert ["last_turn_positions"] in locations["paths"]
            assert ["execution_context", "snapshot_alias"] in locations["paths"]
        checked += 1
    assert checked == 4


def test_crossing_a_static_barrier_uses_its_barrier_radius_not_scope_range():
    for name, state, operations in edge_scenes():
        if name != "arena-travel-edge-barrier":
            continue
        state.active_effects[0].barrier_radius = 2
        records = reference(state, operations)
        assert not any(record["events"] for record in records)
        assert "travel_continued" not in records[-1]["semantic_state"]["execution_context"]
        assert records[-1]["semantic_state"]["entity_locations"]["hero_arien"] == {
            "q": 0,
            "r": 0,
            "s": 0,
        }
        return
    raise AssertionError("barrier scenario missing")
