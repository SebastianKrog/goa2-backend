"""Necessary candidate bounds retain the complete ordered legal set."""

from unittest.mock import patch

import pytest

from automata.runtime.clone import clone_state
from goa2.engine.filters import (
    FastTravelDestinationFilter,
    MovementPathFilter,
    RangeFilter,
)
from goa2.engine.setup import GameSetup
from goa2.engine.steps.selection import _narrow_hex_candidates


def _state(persistent):
    state = GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json", ["Wasp"], ["Arien"], seed=7
    )
    state.current_actor_id = "hero_wasp"
    return clone_state(state) if persistent else state


@pytest.mark.parametrize("persistent", [False, True])
@pytest.mark.parametrize("radius", [0, 1, 2, 4])
def test_range_bounds_keep_order_and_dynamic_context(persistent, radius):
    state = _state(persistent)
    context = {"limit": radius}
    filter_ = RangeFilter(max_range=9, max_range_key="limit")
    candidates = list(state.board.tiles)
    accepts = filter_.prepare(state, context)
    narrowed = _narrow_hex_candidates(candidates, [accepts], state)
    expected = [h for h in candidates if filter_.apply(h, state, context)]
    assert [h for h in narrowed if accepts(h)] == expected
    if radius <= 2:
        assert len(narrowed) <= 1 + 3 * radius * (radius + 1)
    context["limit"] = 1
    refreshed = filter_.prepare(state, context)
    assert [h for h in candidates if refreshed(h)] == [
        h for h in candidates if filter_.apply(h, state, context)
    ]


@pytest.mark.parametrize("persistent", [False, True])
def test_movement_and_fast_travel_bounds_keep_every_candidate_in_original_order(persistent):
    state = _state(persistent)
    candidates = list(state.board.tiles)
    for filter_ in [MovementPathFilter(range_val=2), FastTravelDestinationFilter()]:
        accepts = filter_.prepare(state, {})
        narrowed = _narrow_hex_candidates(candidates, [accepts], state)
        assert [h for h in narrowed if accepts(h)] == [
            h for h in candidates if filter_.apply(h, state, {})
        ]


def test_range_prepare_resolves_origin_once_per_scan():
    state = _state(True)
    filter_ = RangeFilter(max_range=2)
    original = type(state).get_position
    with patch.object(type(state), "get_position", autospec=True, side_effect=original) as position:
        accepts = filter_.prepare(state, {})
        for h in state.board.tiles:
            accepts(h)
        assert position.call_count == 1
