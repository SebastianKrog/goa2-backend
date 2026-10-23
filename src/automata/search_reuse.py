"""Bounded, context-local reuse for one search; never a live-game cache."""

import hashlib
import marshal
import weakref
from collections import OrderedDict
from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass, field
from functools import cache, wraps
from operator import attrgetter
from threading import RLock

from pydantic import BaseModel
from pydantic_core import to_json, to_jsonable_python

from goa2.domain.models import Card, CardState, Token
from goa2.domain.models.spell import SpellCard
from goa2.engine.plain_geometry import plain_path_effects

_CARD_MUTABLE = Card._search_dynamic_fields


@cache
def _model_schema(cls):
    return (cls.__module__, cls.__qualname__), tuple(
        name for name, info in cls.model_fields.items() if info.exclude is not True
    )


@cache
def _card_layout(cls):
    kind, names = _model_schema(cls)
    static = tuple(name for name in names if name not in _CARD_MUTABLE)
    mutable = tuple(name for name in names if name in _CARD_MUTABLE)
    return kind, static, attrgetter(*static), attrgetter(*mutable)


def _definition_value(value):
    if isinstance(value, dict):
        return tuple((key, _definition_value(item)) for key, item in value.items())
    if isinstance(value, (list, tuple)):
        return tuple(_definition_value(item) for item in value)
    if isinstance(value, (set, frozenset)):
        return frozenset(_definition_value(item) for item in value)
    if isinstance(value, BaseModel):
        return to_json(value)
    return value


@cache
def _entity_layout(cls):
    kind, names = _model_schema(cls)
    # Intern by value, never assume these properties cannot be changed by a
    # scenario/effect. A different definition always obtains a different ID.
    fixed = {
        "id",
        "name",
        "team",
        "title",
        "piece_supply",
        "type",
        "lane_id",
        "entity_kind",
        "token_type",
        "owner_hero_id",
    }
    static = tuple(name for name in names if name in fixed)
    dynamic = tuple(name for name in names if name not in fixed)
    return kind, attrgetter(*static), dynamic


_CARD_STATES = {state: index for index, state in enumerate(CardState)}
_TOKEN_FLAGS = ("is_passable", "is_facedown", "persists_end_of_round", "is_immune_to_enemy_actions")


class CompactState:
    """Search-local definitions plus complete mutable records and references.

    The live engine still uses full models and their shared object references.
    Only hashing uses this encoding; API/save formats remain unchanged. Printed
    definitions are interned by value, including custom scenario overrides.
    References preserve order, None slots and independent copies of a card.
    """

    def __init__(self):
        self.definitions = {}
        self.bindings = OrderedDict()
        self._values = {}
        self._cards = []
        self._tokens = []
        self._token_refs = {}
        self.last_bytes = 0

    def _definition(self, definition):
        index = self.definitions.get(definition)
        if index is None:
            if len(self.definitions) < 4096:
                index = len(self.definitions) + 1
                self.definitions[definition] = index
            else:
                # Bounded catalog: overflow retains the complete definition.
                index = definition
        return index

    def pack_card(self, value):
        reference = self._values.get(id(value))
        if reference is not None:
            return reference
        kind, _names, static_values, _ = _card_layout(type(value))
        revision = value.__pydantic_private__["_definition_revision"]
        actions = tuple(value.secondary_actions.items())
        binding = self.bindings.get(id(value))
        if (
            type(value) in (Card, SpellCard)
            and binding is not None
            and binding[0]() is value
            and binding[1] == revision
            and binding[2] == actions
        ):
            definition = binding[3]
        else:
            definition = self._definition(
                (kind, tuple(_definition_value(item) for item in static_values(value)))
            )
            self.bindings[id(value)] = (weakref.ref(value), revision, actions, definition)
            self.bindings.move_to_end(id(value))
            while len(self.bindings) > 4096:
                self.bindings.popitem(last=False)
        reference = len(self._cards)
        self._values[id(value)] = reference
        flags = (
            int(value.is_facedown)
            | (int(value.played_this_round) << 1)
            | (int(value.is_active_base) << 2)
            | (int(value.enraged_active_override) << 3)
        )
        # Optional fields are sparse, but None metadata remains distinct from {}.
        extras = []
        for bit, item, default in (
            (4, value.initiative_override, None),
            (5, value.passive_uses_this_turn, 0),
            (6, value.metadata, {}),
        ):
            if item != default:
                flags |= 1 << bit
                extras.append(item)
        record = (definition, _CARD_STATES[value.state], flags)
        self._cards.append((*record, tuple(extras)) if extras else record)
        return reference

    def pack_entity(self, value):
        kind, static_values, dynamic_names = _entity_layout(type(value))
        definition = self._definition(
            (kind, tuple(_definition_value(item) for item in static_values(value)))
        )
        if isinstance(value, Token):
            reference = self._token_refs.get(id(value))
            if reference is not None:
                return reference
            reference = len(self._tokens)
            self._token_refs[id(value)] = reference
            flags = sum(int(getattr(value, name)) << bit for bit, name in enumerate(_TOKEN_FLAGS))
            # Preserve future/subclass fields instead of silently dropping them.
            extra = tuple(
                getattr(value, name)
                for name in dynamic_names
                if name not in _TOKEN_FLAGS and name != "owner_id"
            )
            self._tokens.append((definition, value.owner_id, flags, extra))
            return reference
        return definition, tuple(getattr(value, name) for name in dynamic_names)

    def encode(self, state):
        self._values.clear()
        self._cards.clear()
        self._token_refs.clear()
        self._tokens.clear()
        context = {"_compact_card_encoder": self}
        mutable = state.__pydantic_serializer__.to_python(
            state, mode="json", exclude={"board"}, context=context
        )
        records = to_jsonable_python((self._cards, self._tokens), context=context)
        # Metadata may itself contain another card/token. Serialization can
        # discover new referenced objects, so drain those tables to completion.
        while len(records[0]) < len(self._cards) or len(records[1]) < len(self._tokens):
            for index, source in enumerate((self._cards, self._tokens)):
                start = len(records[index])
                if start < len(source):
                    records[index].extend(to_jsonable_python(source[start:], context=context))
        # Version 2 deliberately disables object-identity memoization: equal
        # values must hash identically after cloning or a checkpoint round trip.
        payload = marshal.dumps((mutable, records), 2)
        self.last_bytes = len(payload)
        return payload

    def fingerprint(self, state):
        return hashlib.blake2b(self.encode(state), digest_size=16).digest()


_PLANNING_LOCKS = weakref.WeakKeyDictionary()
_PLANNING_LOCKS_GUARD = RLock()


def planning_rng_lock(agent):
    with _PLANNING_LOCKS_GUARD:
        lock = _PLANNING_LOCKS.get(agent)
        if lock is None:
            lock = RLock()
            _PLANNING_LOCKS[agent] = lock
        return lock


def serialized_planning(choose):
    @wraps(choose)
    def wrapped(agent, *args, **kwargs):
        with planning_rng_lock(agent):
            return choose(agent, *args, **kwargs)

    return wrapped


@dataclass(frozen=True)
class PlanningDrawRecipe:
    count: int
    eligible: tuple[int, ...]
    selected: int

    def choose_index(self, rng):
        draws = [rng.random() for _ in range(self.count)]
        return max(self.eligible, key=draws.__getitem__)

    def matches(self, rng):
        return self.choose_index(rng) == self.selected


@dataclass
class PlanningEvents:
    events: list = field(default_factory=list)
    valid: bool = True


_PLANNING_EVENTS: ContextVar[PlanningEvents | None] = ContextVar("planning_events", default=None)


def planning_events_active():
    return _PLANNING_EVENTS.get() is not None


def record_planning_event(agent, recipe):
    events = _PLANNING_EVENTS.get()
    if events is not None:
        events.events.append((agent, recipe))


def invalidate_planning_events():
    events = _PLANNING_EVENTS.get()
    if events is not None:
        events.valid = False


@contextmanager
def capture_planning_events():
    events = PlanningEvents()
    token = _PLANNING_EVENTS.set(events)
    try:
        yield events
    finally:
        _PLANNING_EVENTS.reset(token)


@dataclass
class PlanningTrace:
    agent: object
    recipes: list = field(default_factory=list)
    valid: bool = True


_PLANNING_TRACE: ContextVar[PlanningTrace | None] = ContextVar("planning_trace", default=None)


@contextmanager
def trace_planning(agent):
    trace = PlanningTrace(agent)
    token = _PLANNING_TRACE.set(trace)
    try:
        yield trace
    finally:
        _PLANNING_TRACE.reset(token)


def current_planning_trace(agent):
    trace = _PLANNING_TRACE.get()
    return trace if trace is not None and trace.agent is agent else None


@dataclass
class SearchReuse:
    compact: CompactState = field(default_factory=CompactState)
    static_boards: dict = field(default_factory=dict)
    positions: OrderedDict = field(default_factory=OrderedDict)
    destinations: OrderedDict = field(default_factory=OrderedDict)
    transitions: OrderedDict = field(default_factory=OrderedDict)
    advances: OrderedDict = field(default_factory=OrderedDict)
    policies: OrderedDict = field(default_factory=OrderedDict)
    continuations: OrderedDict = field(default_factory=OrderedDict)
    rollouts: OrderedDict = field(default_factory=OrderedDict)
    transition_seen: OrderedDict = field(default_factory=OrderedDict)
    transition_costs: OrderedDict = field(default_factory=OrderedDict)
    initial_state_keys: dict = field(default_factory=dict)
    advance_engine_seconds: float = 0.0
    hits: dict = field(
        default_factory=lambda: {
            "positions": 0,
            "destinations": 0,
            "transitions": 0,
            "advances": 0,
            "advance_variants": 0,
            "policies": 0,
            "continuations": 0,
            "rollouts": 0,
            "rollout_variants": 0,
        }
    )

    def get(self, name, key):
        values = getattr(self, name)
        if key not in values:
            return None
        value = values[key]
        values.move_to_end(key)
        self.hits[name] += 1
        return value

    def put(self, name, key, value):
        values = getattr(self, name)
        values[key] = value
        values.move_to_end(key)
        limit = (
            32
            if name == "positions"
            else 128 if name == "advances" else 4096 if name == "transition_seen" else 512
        )
        while len(values) > limit:
            values.popitem(last=False)


_CURRENT: ContextVar[SearchReuse | None] = ContextVar("search_reuse", default=None)
_ENABLED: ContextVar[bool] = ContextVar("search_reuse_enabled", default=True)


def current_reuse():
    return _CURRENT.get()


@contextmanager
def search_reuse():
    cache = SearchReuse() if _ENABLED.get() else None
    token = _CURRENT.set(cache)
    try:
        yield cache
    finally:
        _CURRENT.reset(token)


@contextmanager
def reuse_disabled():
    token = _ENABLED.set(False)
    current = _CURRENT.set(None)
    try:
        yield
    finally:
        _CURRENT.reset(current)
        _ENABLED.reset(token)


class _GeometryKey:
    """Value equality with a precomputed hash for shared immutable geometry."""

    __slots__ = ("_hash", "value")

    def __init__(self, value):
        self.value = value
        self._hash = hash(value)

    def __hash__(self):
        return self._hash

    def __eq__(self, other):
        return self is other or (isinstance(other, _GeometryKey) and self.value == other.value)


def position_board_key(state):
    # clone_state explicitly shares immutable zones/lanes throughout a search.
    # Keep strong references so identity keys cannot be recycled within scope.
    board = state.board
    cache = current_reuse()
    identity = (id(board.zones), id(board.lanes))
    if cache is not None and identity in cache.static_boards:
        return cache.static_boards[identity][2]
    value = _GeometryKey(
        (
            tuple((zid, frozenset(z.hexes), tuple(z.neighbors)) for zid, z in board.zones.items()),
            tuple((lid, tuple(zones)) for lid, zones in board.lanes.items()),
        )
    )
    if cache is not None and len(cache.static_boards) < 32:
        cache.static_boards[identity] = (board.zones, board.lanes, value)
    return value


def board_key(state, *, occupancy=True):
    tiles = state.board.tiles
    shared_key = getattr(tiles, "_search_key", None)
    mutable = (
        shared_key(occupancy)
        if shared_key is not None
        else marshal.dumps(
            tuple(
                (h.q, h.r, h.s, t.zone_id, t.is_terrain, t.occupant_id if occupancy else None)
                for h, t in tiles.items()
            ),
            2,
        )
    )
    return position_board_key(state), mutable


def plain_movement_key(state):
    # Be conservative: special effects/tokens use the uncached engine path.
    # Card-specific action permission is always checked separately before using
    # a fast-travel entry; movement values are part of each query key.
    if (
        not getattr(state.board, "_search_geometry_safe", True)
        or getattr(state.board.tiles, "_foreign", False)
        or not plain_path_effects(state)
        or any(uid in state.entity_locations for uid in state.misc_entities)
    ):
        return None
    teams = tuple(
        (str(u.id), u.team) for team in state.teams.values() for u in (*team.heroes, *team.minions)
    )
    return (board_key(state), tuple(state.entity_locations.items()), teams)
