"""Source contracts for swap decisions, spatial identities and mutable operands."""

import copy

import pytest
from live_swap_traces import scenes
from object_execution_traces import reference

pytestmark = pytest.mark.filterwarnings(
    "ignore:(?s)Pydantic serializer warnings.*unit_a_id.*input_type=list:UserWarning"
)


@pytest.fixture(scope="module")
def swap_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(swap_scenes, name):
    state, ops = copy.deepcopy(swap_scenes[name])
    return state, reference(state, ops)


def events(records):
    return [
        event
        for record in records
        for event in record["events"]
        if event["event_type"] == "UNITS_SWAPPED"
    ]


def test_swap_reuses_original_hex_models_and_spatial_containers(swap_scenes):
    state, records = run(swap_scenes, "live-swap-matrix-plain")
    ctx = state.execution_context
    assert len(events(records)) == 1
    assert state.entity_locations is ctx["locations_alias"]
    assert state.board.tiles is ctx["tiles_alias"]
    assert state.entity_locations["hero_brogan"] is ctx["hex_b_alias"]
    assert state.entity_locations["hero_hanu"] is ctx["hex_a_alias"]
    assert list(state.entity_locations)[-2:] == ["hero_brogan", "hero_hanu"]


def test_same_unit_swap_is_real_delete_reinsert_and_emits_event(swap_scenes):
    state, records = run(swap_scenes, "live-swap-alias-self-repeated")
    assert len(events(records)) == 2
    assert list(state.entity_locations)[-1] == "hero_arien"
    assert state.entity_locations["hero_arien"] is state.execution_context["hex_a_alias"]
    assert events(records)[0]["actor_id"] == events(records)[0]["target_id"]


def test_reused_step_reads_changed_context_and_retains_identity(swap_scenes):
    state, records = run(swap_scenes, "live-swap-alias-live-keys")
    assert len(events(records)) == 2
    assert events(records)[0]["actor_id"] == "hero_arien"
    assert events(records)[1]["actor_id"] == "hero_brogan"
    assert state.execution_context["defeat_steps"][0] is state.execution_context["defeat_steps"][-1]
    assert state.entity_locations["hero_arien"] is state.execution_context["hex_a_alias"]
    assert state.entity_locations["hero_brogan"] is state.execution_context["hex_b_alias"]


def test_blockers_stop_swap_and_mandatory_abort_drops_tail(swap_scenes):
    state, records = run(swap_scenes, "live-swap-matrix-blocked")
    assert not events(records)
    assert state.execution_context["tail"]
    state, records = run(swap_scenes, "live-swap-matrix-mandatory")
    assert not events(records)
    assert "tail" not in state.execution_context


def test_missing_skipped_offboard_and_gated_swaps_do_not_mutate_positions(swap_scenes):
    for variant in (
        "missing-a",
        "missing-b",
        "skip-b",
        "empty-key",
        "offboard-a",
        "offboard-b",
        "skip-malformed",
        "inactive-malformed",
    ):
        name = "live-swap-alias-" + variant
        original = swap_scenes[name][0]
        state, records = run(swap_scenes, name)
        assert not events(records), variant
        assert state.entity_locations == original.entity_locations
        assert state.execution_context["tail"]


def test_snapshot_alias_remains_attached_to_swapped_locations(swap_scenes):
    state, records = run(swap_scenes, "live-swap-alias-snapshot-alias")
    assert len(events(records)) == 1
    assert state.last_turn_positions is state.entity_locations
