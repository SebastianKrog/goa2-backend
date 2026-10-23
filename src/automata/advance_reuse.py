"""Exact search-local reuse between owned decisions, with planning RNG replay."""

import random
from dataclasses import dataclass, replace
from functools import wraps
from time import perf_counter

from automata.agents.heuristic_agent import HeuristicAgent
from automata.runtime.clone import clone_state
from automata.runtime.tile_map import SearchTileMap
from automata.search.contracts import CutoffUnit, LeafMode
from automata.search_reuse import (
    board_key,
    current_reuse,
    planning_rng_lock,
    record_planning_event,
    trace_planning,
)
from goa2.domain.types import HeroID
from goa2.engine.session import GameSession

_METHODS = {
    name: getattr(HeuristicAgent, name)
    for name in ("choose_input", "choose_planning", "score_card", "score_option")
}
_SESSION_ADVANCE = GameSession.advance
_COUNTERS = (
    "advance_steps",
    "_advance_calls",
    "_advance_transitions",
    "_session_advances",
    "_environment_planning",
    "_environment_inputs",
    "_continuation_inputs",
)


@dataclass
class AdvanceVariant:
    recipes: tuple
    state: object
    decision: object
    hero_id: str | None
    last_phase: object
    rollback_actor: object
    counters: tuple
    current_transitions: int
    after_fingerprint: bytes
    after_board_key: object


def read_only_heuristic(agent):
    return (
        type(agent) is HeuristicAgent
        and type(agent._rng) is random.Random
        and all(
            getattr(getattr(agent, name), "__func__", None) is method
            for name, method in _METHODS.items()
        )
    )


def _supported(sim):
    agent = sim.environment_policy
    return (
        read_only_heuristic(agent)
        and isinstance(sim.state.board.tiles, SearchTileMap)
        and not sim.state.board.tiles._foreign
        and sim.state.clock is None
        and sim.state.time_control is None
        and type(sim.session) is GameSession
        and getattr(sim.session.advance, "__func__", None) is _SESSION_ADVANCE
        and not sim.session._create_rollback_snapshots
        and sim.session._rollback_snapshot is None
        and sim.cfg.leaf_mode is LeafMode.BOUNDED_CONTINUATION
        and sim.cfg.cutoff_unit is CutoffUnit.TURNS
    )


def reuse_advance(advance):
    @wraps(advance)
    def wrapped(sim, pending=None, *, action_boundary=None):
        cache = current_reuse()
        if cache is None or action_boundary is not None or not _supported(sim):
            sim._own_state()
            sim._known_state_key = None
            return advance(sim, pending, action_boundary=action_boundary)
        with planning_rng_lock(sim.environment_policy):
            known = (
                getattr(sim, "_known_state_key", None)
                if getattr(sim, "_reuse_known_keys", False)
                else None
            )
            if known is not None and known[0] == id(sim.state):
                _, before, before_board = known
            else:
                before, before_board = cache.compact.fingerprint(sim.state), board_key(sim.state)
            key = (
                before,
                before_board,
                pending.model_dump_json() if pending is not None else None,
                sim.session._last_phase,
                sim.session._rollback_actor_id,
                sim.our_team,
                sim.owned_hero_ids,
            )
            variants = cache.get("advances", key)
            rng = sim.environment_policy._rng
            if variants:
                from automata.search.ismcts.engine import _check_deadline

                _check_deadline(sim.deadline)
                rng_before = rng.getstate()
                for saved in variants:
                    if sim.advance_steps + saved.counters[0] > sim.max_advance_steps:
                        continue
                    if saved.current_transitions > sim.cfg.max_advance_transitions:
                        continue
                    if not all(recipe.matches(rng) for recipe in saved.recipes):
                        rng.setstate(rng_before)
                        continue
                    for recipe in saved.recipes:
                        record_planning_event(sim.environment_policy, recipe)
                    # Replay has consumed exactly the real policy's draws. The
                    # saved result is usable only for that same choice transcript.
                    from automata.search.ismcts.engine import _check_deadline

                    _check_deadline(sim.deadline)
                    if saved.state is not None:
                        deferred = getattr(sim, "_defer_state_copy", False)
                        sim.state = saved.state if deferred else clone_state(saved.state)
                        sim.session.state = sim.state
                        sim._borrowed_state = deferred
                    sim.session._last_phase = saved.last_phase
                    sim.session._rollback_actor_id = saved.rollback_actor
                    for name, delta in zip(_COUNTERS, saved.counters, strict=True):
                        setattr(sim, name, getattr(sim, name) + delta)
                    sim._current_advance_transitions = saved.current_transitions
                    decision = replace(
                        saved.decision,
                        hero=sim.state.get_hero(HeroID(saved.hero_id)) if saved.hero_id else None,
                        request=(
                            saved.decision.request.model_copy(deep=True)
                            if saved.decision.request is not None
                            else None
                        ),
                    )
                    if getattr(sim, "_reuse_known_keys", False):
                        sim._known_state_key = (
                            id(sim.state),
                            saved.after_fingerprint,
                            saved.after_board_key,
                        )
                    _check_deadline(sim.deadline)
                    cache.hits["advance_variants"] += 1
                    return decision
                rng.setstate(rng_before)
            sim._own_state()
            counts_before = tuple(getattr(sim, name) for name in _COUNTERS)
            previous_entry = getattr(sim, "_advance_entry_fingerprint", None)
            sim._advance_entry_fingerprint = before
            sim._known_state_key = None
            started = perf_counter()
            try:
                with trace_planning(sim.environment_policy) as trace:
                    decision = advance(sim, pending, action_boundary=action_boundary)
            finally:
                sim._advance_entry_fingerprint = previous_entry
            elapsed = perf_counter() - started
            if trace.valid:
                # Keep normal state copying semantics; neither worlds nor returned
                # prompt models may be mutated through a cached snapshot.
                after = cache.compact.fingerprint(sim.state)
                after_board = board_key(sim.state)
                changed = before != after or before_board != after_board
                deferred = getattr(sim, "_defer_state_copy", False)
                snapshot = (sim.state if deferred else clone_state(sim.state)) if changed else None
                if changed and deferred:
                    sim._borrowed_state = True
                saved = AdvanceVariant(
                    tuple(trace.recipes),
                    snapshot,
                    replace(
                        decision,
                        hero=None,
                        request=(
                            decision.request.model_copy(deep=True)
                            if decision.request is not None
                            else None
                        ),
                    ),
                    str(decision.hero.id) if decision.hero is not None else None,
                    sim.session._last_phase,
                    sim.session._rollback_actor_id,
                    tuple(
                        getattr(sim, name) - previous
                        for name, previous in zip(_COUNTERS, counts_before, strict=True)
                    ),
                    sim._current_advance_transitions,
                    after,
                    after_board,
                )
                existing = list(variants or ())
                existing.append(saved)
                cache.put("advances", key, tuple(existing[-8:]))
                cache.advance_engine_seconds += elapsed
                if getattr(sim, "_reuse_known_keys", False):
                    sim._known_state_key = (id(sim.state), after, after_board)
            return decision

    return wrapped
