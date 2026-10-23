"""Persistent tile snapshots with private lazy writable views for search clones."""

import copy
import marshal
from collections.abc import MutableMapping
from typing import NamedTuple

from goa2.domain.tile import Tile


class _TileData(NamedTuple):
    hex: object
    zone_id: object
    spawn_point: object
    occupant_id: object
    is_terrain: bool
    fields_set: frozenset

    @classmethod
    def from_tile(cls, tile):
        if isinstance(tile, SearchTile):
            return tile._data
        return cls(
            tile.hex,
            tile.zone_id,
            tile.spawn_point,
            tile.occupant_id,
            tile.is_terrain,
            frozenset(tile.model_fields_set),
        )

    def model(self):
        return Tile.model_construct(
            _fields_set=set(self.fields_set),
            hex=self.hex,
            zone_id=self.zone_id,
            spawn_point=self.spawn_point,
            occupant_id=self.occupant_id,
            is_terrain=self.is_terrain,
        )


class _GeometryIndex:
    def __init__(self, data):
        self.source = data
        self.order = None
        self.zones = None
        self.adjacency = None

    def graph(self):
        if self.adjacency is None:
            keys = tuple(self.source)
            order = {key: index for index, key in enumerate(keys)}
            masks = tuple(sum(1 << order[n] for n in h.neighbors() if n in order) for h in keys)
            self.order = order
            self.adjacency = keys, order, masks, (1 << len(keys)) - 1
        return self.adjacency

    def ordered_subset(self, candidates):
        if self.order is None:
            self.order = {key: index for index, key in enumerate(self.source)}
        return sorted((key for key in candidates if key in self.order), key=self.order.__getitem__)

    def zone_candidates(self, zones):
        if self.zones is None:
            self.zones = {}
            for key, tile in self.source.items():
                self.zones.setdefault(tile.zone_id, []).append(key)
        return self.ordered_subset({key for zone in zones for key in self.zones.get(zone, ())})


class _Snapshot:
    __slots__ = ("data", "geometry", "keys", "occupants", "walk_mask")

    def __init__(self, data, geometry=None, walk_mask=None, occupants=None):
        self.data = data
        self.keys = {}
        self.geometry = geometry if geometry is not None else _GeometryIndex(data)
        self.walk_mask = walk_mask
        self.occupants = occupants

    def movement_masks(self):
        if self.walk_mask is None:
            keys, _order, _, _ = self.geometry.graph()
            mask, occupants = 0, {}
            for index, h in enumerate(keys):
                tile = self.data[h]
                if not tile.is_terrain and tile.occupant_id is None:
                    mask |= 1 << index
                if tile.occupant_id is not None:
                    occupants[tile.occupant_id] = occupants.get(tile.occupant_id, 0) + 1
            self.occupants = occupants
            self.walk_mask = mask
        return self.walk_mask, self.occupants

    def key(self, occupancy):
        if occupancy not in self.keys:
            self.keys[occupancy] = marshal.dumps(
                tuple(
                    (h.q, h.r, h.s, t.zone_id, t.is_terrain, t.occupant_id if occupancy else None)
                    for h, t in self.data.items()
                ),
                2,
            )
        return self.keys[occupancy]


def _tile_property(name):
    def get(tile):
        return getattr(tile._data, name)

    def set_(tile, value):
        tile._data = tile._data._replace(
            **{name: value, "fields_set": tile._data.fields_set | {name}}
        )
        tile._owner._changed(tile._key, tile=tile, geometry=name == "zone_id")

    return property(get, set_)


class SearchTile:
    """Mutable facade belonging to exactly one map; its backing record is frozen."""

    __slots__ = ("_data", "_key", "_owner")

    def __init__(self, owner, key, data):
        self._owner, self._key, self._data = owner, key, data

    hex = _tile_property("hex")
    zone_id = _tile_property("zone_id")
    spawn_point = _tile_property("spawn_point")
    occupant_id = _tile_property("occupant_id")
    is_terrain = _tile_property("is_terrain")

    @property
    def is_occupied(self):
        return self._data.occupant_id is not None

    @property
    def is_obstacle(self):
        return self._data.is_terrain or self._data.occupant_id is not None

    @property
    def model_fields_set(self):
        return set(self._data.fields_set)

    def _as_search_tile_model(self):
        return self._data.model()

    def model_copy(self, *, update=None, deep=False):
        return self._data.model().model_copy(update=update, deep=deep)

    def model_dump(self, **kwargs):
        return self._data.model().model_dump(**kwargs)

    def model_dump_json(self, **kwargs):
        return self._data.model().model_dump_json(**kwargs)

    def __deepcopy__(self, memo):
        copied = self.model_copy(deep=True)
        memo[id(self)] = copied
        return copied

    def __eq__(self, other):
        if isinstance(other, SearchTile):
            return self._data[:5] == other._data[:5]
        if isinstance(other, Tile):
            return self._data.model() == other
        return NotImplemented


class SearchTileMap(MutableMapping):
    """Fork in constant time; each map owns its writable views and changes."""

    def __init__(self, tiles=None, *, snapshot=None):
        self._base = (
            snapshot
            if snapshot is not None
            else _Snapshot({key: _TileData.from_tile(tile) for key, tile in tiles.items()})
        )
        self._views = {}
        self._assigned = (
            {
                key: tile
                for key, tile in tiles.items()
                if type(tile) is not Tile and not isinstance(tile, SearchTile)
            }
            if tiles is not None
            else {}
        )
        self._foreign = bool(self._assigned)
        self._dirty = set()
        self._zones_dirty = False
        self._snapshot = self._base

    def __getitem__(self, key):
        if key in self._assigned:
            return self._assigned[key]
        value = self._views.get(key)
        if value is None:
            value = SearchTile(self, key, self._base.data[key])
            self._views[key] = value
        return value

    def __setitem__(self, key, value):
        if type(value) is not Tile and not isinstance(value, SearchTile):
            self._foreign = True
        if key not in self._base.data:
            data = self._base.data.copy()
            data[key] = _TileData.from_tile(value)
            self._base = _Snapshot(data)
        self._assigned[key] = value
        self._views.pop(key, None)
        self._changed(key, geometry=True)

    def __delitem__(self, key):
        if key not in self._base.data:
            raise KeyError(key)
        data = self._base.data.copy()
        del data[key]
        self._base = _Snapshot(data)
        self._views.pop(key, None)
        self._assigned.pop(key, None)
        self._dirty.discard(key)
        self._snapshot = None

    def __iter__(self):
        return iter(self._base.data)

    def __len__(self):
        return len(self._base.data)

    def __contains__(self, key):
        return key in self._base.data

    def _changed(self, key, *, tile=None, geometry=False):
        if tile is not None and self._views.get(key) is not tile:
            return  # A removed/replaced view no longer belongs to this map.
        self._dirty.add(key)
        self._zones_dirty |= geometry
        self._snapshot = None

    def _freeze(self):
        # Assigned model values retain dict assignment's live-reference
        # semantics, so their external mutations must be read on every freeze.
        if self._snapshot is not None and not self._assigned:
            return self._snapshot
        data = self._base.data.copy()
        for key in self._dirty | self._assigned.keys():
            value = self._assigned.get(key)
            if value is None:
                value = self._views[key]
            data[key] = _TileData.from_tile(value)
        geometry = None if self._zones_dirty or self._assigned else self._base.geometry
        mask, occupants = None, None
        if geometry is not None and self._base.walk_mask is not None:
            mask, occupants = self._base.walk_mask, self._base.occupants.copy()
            order = geometry.graph()[1]
            for key in self._dirty:
                old, new = self._base.data[key], data[key]
                bit = 1 << order[key]
                mask = mask & ~bit if new.is_terrain or new.occupant_id is not None else mask | bit
                if old.occupant_id is not None:
                    count = occupants[old.occupant_id] - 1
                    if count:
                        occupants[old.occupant_id] = count
                    else:
                        del occupants[old.occupant_id]
                if new.occupant_id is not None:
                    occupants[new.occupant_id] = occupants.get(new.occupant_id, 0) + 1
        self._snapshot = _Snapshot(data, geometry, mask, occupants)
        self._base = self._snapshot
        self._dirty.clear()
        self._zones_dirty = False
        return self._snapshot

    def fork(self):
        if self._foreign:
            return {key: copy.deepcopy(tile) for key, tile in self.items()}
        return SearchTileMap(snapshot=self._freeze())

    def _ordered_subset(self, candidates):
        return self._freeze().geometry.ordered_subset(candidates)

    def _zone_candidates(self, zones):
        return self._freeze().geometry.zone_candidates(zones)

    def _occupied_ids(self):
        return self._freeze().movement_masks()[1].keys()

    def _reachable_plain(self, start, steps, pass_through_obstacles=False):
        if self._foreign:
            return None
        snapshot = self._freeze()
        keys, index, adjacency, all_bits = snapshot.geometry.graph()
        start_index = index.get(start)
        if start_index is None:
            return None
        allowed = all_bits if pass_through_obstacles else snapshot.movement_masks()[0]
        visited = frontier = 1 << start_index
        for _ in range(steps):
            expansion = 0
            remaining = frontier
            while remaining:
                bit = remaining & -remaining
                expansion |= adjacency[bit.bit_length() - 1]
                remaining ^= bit
            frontier = expansion & allowed & ~visited
            if not frontier:
                break
            visited |= frontier
        result = set()
        while visited:
            bit = visited & -visited
            result.add(keys[bit.bit_length() - 1])
            visited ^= bit
        return result

    def _search_key(self, occupancy=True):
        return self._freeze().key(occupancy)

    def _as_plain_tiles(self):
        if self._foreign:
            return {
                key: tile._as_search_tile_model() if isinstance(tile, SearchTile) else tile
                for key, tile in self.items()
            }
        return {key: value.model() for key, value in self._freeze().data.items()}

    def copy(self):
        return dict(self.items())

    def __deepcopy__(self, memo):
        copied = self.fork()
        memo[id(self)] = copied
        return copied
