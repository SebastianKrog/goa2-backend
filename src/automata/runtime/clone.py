"""Independent search clones with shared immutable board geometry.

Mutable state is deep-copied, preserving aliases between decks, hands, planning
buffers and metadata. Normal Card/SpellCard scalar fields are copied in bulk;
their mutable containers remain independent. Unknown subclasses keep the general
Pydantic deepcopy path. Frozen Hex positions may be shared. Board shells and tile
occupancy are private to each clone, while immutable zones/lanes are shared.
"""

from __future__ import annotations

import copy

from automata.runtime.deepcopy import search_deepcopy
from automata.runtime.tile_map import SearchTileMap
from goa2.domain.models import Card
from goa2.domain.models.spell import SpellCard
from goa2.domain.state import GameState
from goa2.domain.tile import Tile


def _copy_card_mapping(value, memo, *, scalar=False):
    if type(value) is dict and (scalar or not value):
        identity = id(value)
        if identity not in memo:
            memo[identity] = value.copy()
        return memo[identity]
    return copy.deepcopy(value, memo)


def _seed_card_clones(state: GameState, memo: dict) -> None:
    """Copy normal card scalar fields in bulk, deep-copy mutable containers.

    Standard Card/SpellCard definitions contain only scalar fields plus these
    two containers. Unknown subclasses keep Pydantic's general deepcopy path.
    Seed every shell first so metadata cycles and collection aliases survive.
    """
    cards = {}
    for team in state.teams.values():
        for hero in team.heroes:
            for card in (
                *hero.deck,
                *hero.spells,
                *hero.hand,
                *hero.played_cards,
                *hero.discard_pile,
                hero.current_turn_card,
                hero.extra_turn_card,
                hero.ultimate_card,
            ):
                if type(card) in (Card, SpellCard):
                    cards[id(card)] = card
    for identity, card in cards.items():
        clone = object.__new__(type(card))
        object.__setattr__(clone, "__dict__", card.__dict__.copy())
        object.__setattr__(clone, "__pydantic_fields_set__", card.__pydantic_fields_set__.copy())
        object.__setattr__(clone, "__pydantic_extra__", None)
        object.__setattr__(clone, "__pydantic_private__", None)
        memo[identity] = clone
    for identity, card in cards.items():
        clone = memo[identity]
        actions = card.secondary_actions
        clone.__dict__["secondary_actions"] = _copy_card_mapping(
            actions,
            memo,
            scalar=type(actions) is dict and all(type(v) is int for v in actions.values()),
        )
        clone.__dict__["metadata"] = _copy_card_mapping(card.metadata, memo)
        object.__setattr__(
            clone, "__pydantic_extra__", copy.deepcopy(card.__pydantic_extra__, memo)
        )
        private = card.__pydantic_private__
        scalar_private = (
            type(private) is dict
            and set(private) == {"_definition_revision"}
            and type(private["_definition_revision"]) is int
        )
        object.__setattr__(
            clone, "__pydantic_private__", _copy_card_mapping(private, memo, scalar=scalar_private)
        )


def _clone_tile(tile):
    if type(tile) is not Tile:
        return tile.model_copy(deep=True)
    clone = object.__new__(Tile)
    object.__setattr__(clone, "__dict__", tile.__dict__.copy())
    object.__setattr__(clone, "__pydantic_fields_set__", tile.__pydantic_fields_set__.copy())
    object.__setattr__(clone, "__pydantic_extra__", None)
    object.__setattr__(clone, "__pydantic_private__", None)
    return clone


def clone_state(state: GameState) -> GameState:
    """Return an independent deep copy of ``state`` sharing only static board
    geometry. Safe to play forward without affecting ``state``."""
    board = state.board
    # Deep-copy all mutable state; share the static board object (memo) so the
    # expensive geometry isn't copied.
    memo = {id(board): board}
    _seed_card_clones(state, memo)
    clone = search_deepcopy(state, memo)
    # Give the clone its own board shell + tiles so the mutable occupancy cache
    # (`tile.occupant_id`) is independent of the original.
    new_board = board.model_copy()
    if isinstance(board.tiles, SearchTileMap):
        new_board.tiles = board.tiles.fork()
    elif all(type(tile) is Tile for tile in board.tiles.values()):
        new_board.tiles = SearchTileMap(board.tiles)
    else:
        new_board.tiles = {key: _clone_tile(tile) for key, tile in board.tiles.items()}
        new_board._search_geometry_safe = False
    clone.board = new_board
    return clone


def prepare_search_state(state: GameState) -> GameState:
    """Independent read-only search source with a persistent tile snapshot.

    This single initial clone also isolates hero/card data from callers. Each
    simulation then forks its immutable board snapshot and copies mutable data.
    """
    return clone_state(state)
