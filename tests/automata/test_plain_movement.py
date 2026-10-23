"""Bitset movement must equal the full engine, including its flag behavior."""

import random

import pytest

from automata.runtime.clone import clone_state
from goa2.domain.board import Board
from goa2.domain.hex import Hex
from goa2.domain.models import Team, TeamColor, Token, TokenType
from goa2.domain.models.effect import EffectType
from goa2.domain.state import GameState
from goa2.domain.tile import Tile
from goa2.engine import rules


def _state(seed):
    rng = random.Random(seed)
    tiles = {}
    for q in range(-4, 5):
        for r in range(-4, 5):
            if abs(q + r) <= 4:
                h = Hex(q=q, r=r, s=-q - r)
                tiles[h] = Tile(hex=h, is_terrain=rng.random() < 0.15)
    state = GameState(
        board=Board(tiles=tiles),
        teams={
            TeamColor.RED: Team(color=TeamColor.RED, heroes=[], minions=[]),
            TeamColor.BLUE: Team(color=TeamColor.BLUE, heroes=[], minions=[]),
        },
    )
    for index, tile in enumerate(state.board.tiles.values()):
        if rng.random() < 0.15:
            tile.occupant_id = f"obstacle_{index}"
    return clone_state(state)


def _reference(state):
    return state.model_copy(
        update={
            "board": state.board.model_copy(update={"tiles": state.board.tiles._as_plain_tiles()})
        }
    )


@pytest.mark.parametrize("seed", range(8))
@pytest.mark.parametrize(
    "ignore,through", [(False, False), (True, False), (False, True), (True, True)]
)
def test_bitset_bfs_matches_full_engine_for_terrain_occupancy_and_flags(seed, ignore, through):
    state = _state(seed)
    reference = _reference(state)
    for start in list(state.board.tiles)[::11]:
        for steps in (0, 1, 2, 4, 9):
            kwargs = dict(ignore_obstacles=ignore, pass_through_obstacles=through)
            actual = rules.find_reachable_hexes(state.board, start, steps, state=state, **kwargs)
            expected = rules.find_reachable_hexes(
                reference.board, start, steps, state=reference, **kwargs
            )
            assert actual == expected


def test_updated_occupancy_and_terrain_masks_remain_correct_after_fork():
    state = _state(3)
    start = next(iter(state.board.tiles))
    state.board.tiles._reachable_plain(start, 3)
    for index, (_h, tile) in enumerate(state.board.tiles.items()):
        if index % 5 == 0:
            tile.occupant_id = None if tile.occupant_id else "changed"
            tile.is_terrain = not tile.is_terrain
    child = clone_state(state)
    reference = _reference(child)
    for start in list(child.board.tiles)[::7]:
        assert rules.find_reachable_hexes(child.board, start, 4, state=child) == (
            rules.find_reachable_hexes(reference.board, start, 4, state=reference)
        )


def test_passable_token_uses_general_path_even_without_an_entity_location():
    state = _state(1)
    h = next(iter(state.board.tiles))
    token = Token(id="mine", name="Mine", token_type=TokenType.MINE_DUD, is_passable=True)
    state.misc_entities[token.id] = token
    state.token_pool[TokenType.MINE_DUD] = [token]
    state.board.tiles[h].occupant_id = token.id
    state.board.tiles[h].is_terrain = False
    assert rules._plain_reachable(state.board, h, 3, state, False) is None
    reference = _reference(state)
    assert rules.find_reachable_hexes(state.board, h, 4, state=state) == (
        rules.find_reachable_hexes(reference.board, h, 4, state=reference)
    )


def test_custom_validator_and_off_map_origin_keep_general_path(monkeypatch):
    state = _state(1)
    outside = Hex(q=50, r=-50, s=0)
    assert rules._plain_reachable(state.board, outside, 3, state, False) is None
    monkeypatch.setattr(state.validator, "is_obstacle_for_actor", lambda *args, **kwargs: True)
    assert (
        rules._plain_reachable(state.board, next(iter(state.board.tiles)), 3, state, False) is None
    )


@pytest.mark.parametrize("kind", list(EffectType))
def test_active_effect_geometry_matches_general_engine(kind):
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.plain_geometry import plain_path_effects

    state = _state(3)
    state.active_effects.append(
        ActiveEffect(
            id="geometry-test",
            source_id="absent",
            effect_type=kind,
            scope=EffectScope(shape=Shape.GLOBAL),
            duration=DurationType.THIS_ROUND,
            created_at_turn=1,
            created_at_round=1,
            is_active=True,
        )
    )
    reference = _reference(state)
    blocking = {
        EffectType.TOPOLOGY_SPLIT,
        EffectType.TOPOLOGY_ISOLATION,
        EffectType.STATIC_BARRIER,
        EffectType.EMPTY_HEX_OBSTACLE,
        EffectType.PETRIFY,
    }
    assert plain_path_effects(state) == (kind not in blocking)
    for start in list(state.board.tiles)[::11]:
        assert rules.find_reachable_hexes(state.board, start, 4, state=state) == (
            rules.find_reachable_hexes(reference.board, start, 4, state=reference)
        )
