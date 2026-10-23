"""Persistent tile snapshots keep writable boards independent and serializable."""

import copy

from automata.runtime.clone import clone_state, prepare_search_state
from automata.runtime.tile_map import SearchTileMap
from goa2.domain.hex import Hex
from goa2.domain.state import GameState
from goa2.domain.tile import Tile
from goa2.engine.setup import GameSetup


def _state():
    return GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json", ["Arien"], ["Wasp"], seed=9
    )


def test_forks_are_lazy_and_writes_are_private_in_both_directions():
    original = _state()
    left = clone_state(original)
    right = clone_state(left)
    assert isinstance(left.board.tiles, SearchTileMap)
    assert left.board.tiles._views == right.board.tiles._views == {}
    h = next(iter(original.board.tiles))
    before = original.board.tiles[h].model_dump()
    left.board.tiles[h].occupant_id = "left"
    assert original.board.tiles[h].model_dump() == before
    assert right.board.tiles[h].model_dump() == before
    child = clone_state(left)
    assert child.board.tiles[h].occupant_id == "left"
    left.board.tiles[h].occupant_id = "later"
    original.board.tiles[h].occupant_id = "original changed"
    assert child.board.tiles[h].occupant_id == "left"
    assert right.board.tiles[h].model_dump() == before
    child.board.tiles[h].is_terrain = not before["is_terrain"]
    assert right.board.tiles[h].is_terrain == before["is_terrain"]


def test_mutations_through_values_and_items_are_private():
    original = _state()
    cloned = clone_state(original)
    for tile in cloned.board.tiles.values():
        tile.occupant_id = "value"
        break
    h, tile = next(iter(cloned.board.tiles.items()))
    tile.zone_id = "changed zone"
    assert original.board.tiles[h].occupant_id != "value"
    assert original.board.tiles[h].zone_id != "changed zone"
    assert clone_state(cloned).board.tiles[h].zone_id == "changed zone"


def test_serialization_round_trip_and_shallow_copy_keep_normal_tile_shape():
    original = _state()
    cloned = clone_state(original)
    assert cloned.model_dump(mode="json") == original.model_dump(mode="json")
    restored = GameState.model_validate_json(cloned.model_dump_json())
    assert restored.model_dump(exclude={"board"}) == cloned.model_dump(exclude={"board"})
    assert restored.board.zones == cloned.board.zones
    assert (
        restored.board.model_dump(mode="json")["tiles"]
        == cloned.board.model_dump(mode="json")["tiles"]
    )
    copied_board = cloned.board.model_copy(update={"tiles": cloned.board.tiles.copy()})
    assert copied_board.model_dump(mode="json") == cloned.board.model_dump(mode="json")
    assert copied_board.model_dump(mode="json", exclude={"tiles"}) == cloned.board.model_dump(
        mode="json", exclude={"tiles"}
    )


def test_tile_add_remove_and_external_assigned_model_mutations_survive_forks():
    cloned = clone_state(_state())
    h = Hex(q=99, r=-99, s=0)
    tile = Tile(hex=h, zone_id="new")
    cloned.board.tiles[h] = tile
    first = clone_state(cloned)
    tile.occupant_id = "assigned changed"
    second = clone_state(cloned)
    assert first.board.tiles[h].occupant_id is None
    assert second.board.tiles[h].occupant_id == "assigned changed"
    del cloned.board.tiles[h]
    assert h not in cloned.board.tiles
    assert h in first.board.tiles and h in second.board.tiles


def test_board_snapshot_key_changes_on_write_and_is_shared_when_unchanged():
    state = clone_state(_state())
    initial = state.board.tiles._search_key()
    child = clone_state(state)
    assert child.board.tiles._search_key() is initial
    h = next(iter(child.board.tiles))
    child.board.tiles[h].occupant_id = "changed"
    assert child.board.tiles._search_key() != initial
    assert state.board.tiles._search_key() is initial
    occupied = child.board.tiles._search_key()
    child.board.tiles[h].is_terrain = not child.board.tiles[h].is_terrain
    assert child.board.tiles._search_key() != occupied


def test_preparing_source_does_not_replace_or_mutate_original_tiles():
    original = _state()
    tiles = original.board.tiles
    prepared = prepare_search_state(original)
    assert original.board.tiles is tiles
    assert isinstance(prepared.board.tiles, SearchTileMap)
    child = clone_state(prepared)
    h = next(iter(tiles))
    child.board.tiles[h].occupant_id = "child"
    assert tiles[h].occupant_id != "child"


def test_deepcopy_of_tile_view_and_map_is_independent():
    state = clone_state(_state())
    h = next(iter(state.board.tiles))
    tile = copy.deepcopy(state.board.tiles[h])
    tile.occupant_id = "separate"
    assert state.board.tiles[h].occupant_id != "separate"
    tiles = copy.deepcopy(state.board.tiles)
    tiles[h].occupant_id = "separate map"
    assert state.board.tiles[h].occupant_id != "separate map"


def test_prepared_source_is_independent_of_later_caller_hero_mutations():
    original = _state()
    prepared = prepare_search_state(original)
    original.teams[next(iter(original.teams))].heroes[0].gold = 99
    assert prepared.teams[next(iter(prepared.teams))].heroes[0].gold != 99


def test_detached_tile_views_can_be_mutated_without_corrupting_the_map():
    state = clone_state(_state())
    h = next(iter(state.board.tiles))
    detached = state.board.tiles[h]
    del state.board.tiles[h]
    detached.occupant_id = "detached"
    assert h not in clone_state(state).board.tiles
    state.board.tiles[h] = Tile(hex=h)
    old = state.board.tiles[h]
    state.board.tiles[h] = Tile(hex=h, occupant_id="replacement")
    old.occupant_id = "old value changed"
    assert clone_state(state).board.tiles[h].occupant_id == "replacement"


def test_custom_tile_assignment_falls_back_without_losing_extra_mutable_fields():
    from pydantic import Field

    class CustomTile(Tile):
        details: list[int] = Field(default_factory=list)

    state = clone_state(_state())
    h = next(iter(state.board.tiles))
    custom = CustomTile(hex=h, details=[1])
    state.board.tiles[h] = custom
    child = clone_state(state)
    assert type(child.board.tiles[h]) is CustomTile
    child.board.tiles[h].details.append(2)
    assert custom.details == [1]
