"""Batch destination checks must equal the original target-specific BFS."""

import pytest
from test_plain_movement import _reference, _state

from goa2.domain.hex import Hex
from goa2.engine.rules import (
    _validate_movement_path_general,
    validate_movement_path,
    validate_movement_paths,
)


@pytest.mark.parametrize(
    "ignore,through", [(False, False), (True, False), (False, True), (True, True)]
)
@pytest.mark.parametrize("seed", [1, 3, 7])
def test_batch_and_single_movement_match_original_bfs_for_all_flag_combinations(
    ignore, through, seed
):
    state = _state(seed)
    ends = [*state.board.tiles, Hex(q=80, r=-80, s=0)]
    for start in list(state.board.tiles)[::19]:
        for steps in [0, 1, 2, 4]:
            kwargs = dict(
                board=state.board,
                start=start,
                max_steps=steps,
                ignore_obstacles=ignore,
                pass_through_obstacles=through,
                state=state,
            )
            expected = [_validate_movement_path_general(end=end, **kwargs) for end in ends]
            assert validate_movement_paths(ends=iter(ends), **kwargs) == expected
            assert [validate_movement_path(end=end, **kwargs) for end in ends[::11]] == expected[
                ::11
            ]


def test_plain_dict_board_and_custom_validator_keep_original_rules(monkeypatch):
    state = _reference(_state(1))
    ends = list(state.board.tiles)[::5]
    start = ends[0]
    assert validate_movement_paths(state.board, start, ends, 3, state=state) == [
        _validate_movement_path_general(state.board, start, end, 3, state=state) for end in ends
    ]
    monkeypatch.setattr(state.validator, "is_obstacle_for_actor", lambda state, h, *args: h.q > 0)
    assert validate_movement_paths(state.board, start, ends, 3, state=state) == [
        _validate_movement_path_general(state.board, start, end, 3, state=state) for end in ends
    ]


def test_mutation_is_rechecked_and_empty_batch_does_no_work():
    state = _state(1)
    start = next(iter(state.board.tiles))
    end = next(h for h in start.neighbors() if h in state.board.tiles)
    state.board.tiles[end].is_terrain = False
    state.board.tiles[end].occupant_id = None
    assert validate_movement_paths(state.board, start, [end], 1, state=state) == [True]
    state.board.tiles[end].occupant_id = "blocker"
    assert validate_movement_paths(state.board, start, [end], 1, state=state) == [False]
    assert validate_movement_paths(state.board, start, [], 1, state=state) == []
