"""Exact reuse of completed bounded-continuation trajectories.

The tree still selects and expands each iteration normally. Only an identical
continuation, with identical fresh planning choices, may reuse its rules result.
Leaf evaluators and observers always run normally on an independent world.
"""

from contextlib import ExitStack
from dataclasses import dataclass, replace
from functools import wraps

from automata.advance_reuse import _COUNTERS, read_only_heuristic
from automata.search.continuation import AgentContinuationPolicy
from automata.search_reuse import (
    capture_planning_events,
    current_reuse,
    planning_rng_lock,
    record_planning_event,
)
from goa2.domain.types import HeroID

_CONTINUATION_CHOOSE = AgentContinuationPolicy.choose


@dataclass
class BoundedReplay:
    events: tuple
    world: object
    decision: object
    hero_id: str | None
    context: object
    decisions: int
    forced_decisions: int
    seen_forced: frozenset
    counters: tuple
    current_transitions: int
    last_phase: object
    rollback_actor: object
    known: tuple


def reuse_bounded(continuation_loop):
    @wraps(continuation_loop)
    def wrapped(
        sim, decision, cfg, continuation, context, start_round, start_turn_round, start_turn
    ):
        cache = current_reuse()
        known = getattr(sim, "_known_state_key", None)
        if (
            cache is None
            or not getattr(sim, "_defer_state_copy", False)
            or known is None
            or known[0] != id(sim.state)
            or type(continuation) is not AgentContinuationPolicy
            or getattr(continuation.choose, "__func__", None) is not _CONTINUATION_CHOOSE
            or not read_only_heuristic(continuation.agent)
            or not read_only_heuristic(sim.environment_policy)
        ):
            return continuation_loop(
                sim, decision, cfg, continuation, context, start_round, start_turn_round, start_turn
            )
        agents = tuple(sorted({sim.environment_policy, continuation.agent}, key=id))
        key = (
            known[1],
            known[2],
            id(sim.environment_policy),
            id(continuation.agent),
            context.root_viewer_id,
            context.perspective_team,
            context.current_owner_id,
            decision.kind,
            decision.request.id if decision.request else None,
            sim.session._last_phase,
            sim.session._rollback_actor_id,
            cfg.cutoff_unit,
            cfg.cutoff_limit,
            cfg.max_forced_decisions,
            cfg.max_advance_transitions,
            sim.our_team,
            sim.owned_hero_ids,
            start_round,
            start_turn_round,
            start_turn,
        )
        with ExitStack() as locks:
            for agent in agents:
                locks.enter_context(planning_rng_lock(agent))
            variants = cache.get("rollouts", key)
            if variants:
                from automata.search.ismcts.engine import _check_deadline

                _check_deadline(sim.deadline)
                rng_before = {agent: agent._rng.getstate() for agent in agents}
                for saved in variants:
                    if sim.advance_steps + saved.counters[0] > sim.max_advance_steps:
                        continue
                    if not all(recipe.matches(agent._rng) for agent, recipe in saved.events):
                        for agent, state in rng_before.items():
                            agent._rng.setstate(state)
                        continue
                    _check_deadline(sim.deadline)
                    for agent, recipe in saved.events:
                        record_planning_event(agent, recipe)
                    sim.state = saved.world
                    sim.session.state = sim.state
                    sim._borrowed_state = True
                    sim._known_state_key = (id(sim.state), *saved.known)
                    sim.session._last_phase = saved.last_phase
                    sim.session._rollback_actor_id = saved.rollback_actor
                    sim._current_advance_transitions = saved.current_transitions
                    for name, delta in zip(_COUNTERS, saved.counters, strict=True):
                        setattr(sim, name, getattr(sim, name) + delta)
                    result_decision = replace(
                        saved.decision,
                        hero=sim.state.get_hero(HeroID(saved.hero_id)) if saved.hero_id else None,
                        request=(
                            saved.decision.request.model_copy(deep=True)
                            if saved.decision.request is not None
                            else None
                        ),
                    )
                    result_context = replace(saved.context, decision=result_decision)
                    cache.hits["rollout_variants"] += 1
                    return (
                        result_decision,
                        result_context,
                        saved.decisions,
                        saved.forced_decisions,
                        set(saved.seen_forced),
                    )
                for agent, state in rng_before.items():
                    agent._rng.setstate(state)
            before = tuple(getattr(sim, name) for name in _COUNTERS)
            with capture_planning_events() as transcript:
                result = continuation_loop(
                    sim,
                    decision,
                    cfg,
                    continuation,
                    context,
                    start_round,
                    start_turn_round,
                    start_turn,
                )
            final_decision, final_context, decisions, forced, seen = result
            after = getattr(sim, "_known_state_key", None)
            if (
                transcript.valid
                and after is not None
                and after[0] == id(sim.state)
                and all(agent in agents for agent, _ in transcript.events)
            ):
                sim._borrowed_state = True
                stored_decision = replace(
                    final_decision,
                    hero=None,
                    request=(
                        final_decision.request.model_copy(deep=True)
                        if final_decision.request is not None
                        else None
                    ),
                )
                saved = BoundedReplay(
                    tuple(transcript.events),
                    sim.state,
                    stored_decision,
                    str(final_decision.hero.id) if final_decision.hero else None,
                    replace(final_context, decision=stored_decision),
                    decisions,
                    forced,
                    frozenset(seen),
                    tuple(
                        getattr(sim, name) - old
                        for name, old in zip(_COUNTERS, before, strict=True)
                    ),
                    sim._current_advance_transitions,
                    sim.session._last_phase,
                    sim.session._rollback_actor_id,
                    after[1:],
                )
                cache.put("rollouts", key, (*tuple(variants or ())[-7:], saved))
            return result

    return wrapped
