"""Source contracts for reviewed Xargatha recipes and direct board adjacency."""

import pytest
from live_xargatha_primary_traces import DELAYED, MINION_MOVES, action_scene, count_scene


def test_all_eighteen_xargatha_faces_are_reviewed_including_empty_ultimate():
    from primary_builders import catalog
    from primary_templates import XARGATHA_EFFECTS

    data = catalog()
    assert len(XARGATHA_EFFECTS) == 18
    assert set(XARGATHA_EFFECTS) <= data["recipes"].keys()
    assert data["recipes"]["metamorphosis"] == []


@pytest.mark.parametrize(
    "variant,expected",
    [
        ("ordinary", 6),
        ("no-actor", 0),
        ("unknown-actor", 0),
        ("off-board", 0),
        ("tiles-only", 6),
        ("locations-only", 5),
        ("owner-with-acting-piece", 0),
        ("piece-actor", 1),
        ("stale-occupant", 5),
    ],
)
def test_counter_uses_direct_actor_location_and_cached_tile_occupants(variant, expected):
    _, _, final = count_scene(variant)
    assert final.execution_context["adjacency_probe"] == expected


@pytest.mark.parametrize(
    "multiplier,subtract,expected", [(2, 1, 10), (3, 1, 15), (-2, 1, -10), (2, 8, 0), (2, -1, 14)]
)
def test_bonus_subtracts_before_clamping_and_preserves_negative_multiplier(
    multiplier, subtract, expected
):
    _, _, final = count_scene(multiplier=multiplier, subtract=subtract)
    assert final.execution_context["adjacency_probe"] == expected


def test_rapid_thrusts_recounts_after_first_target_is_defeated():
    from object_execution_traces import reference

    state, ops, final = action_scene("rapid_thrusts")
    records = reference(state, ops)
    counts = [
        r["semantic_state"]["execution_context"]["adj_rng_bonus"]
        for r in records
        if "adj_rng_bonus" in r["semantic_state"]["execution_context"]
    ]
    assert 6 in counts and 5 in counts
    assert final.execution_context["adj_rng_bonus"] == 5


@pytest.mark.parametrize("effect", DELAYED)
def test_real_round_end_executes_delayed_defeat_before_cleanup(effect):
    from object_execution_traces import reference

    state, ops, final = action_scene(effect, "round")
    records = reference(state, ops)
    events = [e for row in records for e in row["events"]]
    assert any(e["event_type"] == "UNIT_DEFEATED" for e in events)
    assert not final.active_effects
    assert final.phase.value == "PLANNING"


@pytest.mark.parametrize("effect", MINION_MOVES)
def test_only_dominate_can_offer_heavy_minion(effect):
    from object_execution_traces import reference

    state, ops, _ = action_scene(effect, variant="heavy-only")
    records = reference(state, ops)
    offered = [
        r["request"]
        for r in records
        if r["request"] and r["request"]["request_type"] == "SELECT_UNIT"
    ]
    assert bool(offered) == (effect == "dominate")
