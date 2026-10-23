"""Whole-continuation replay is exact rule reuse, including RNG and budgets."""

from dataclasses import replace

import pytest
from test_advance_reuse import _counters, _rule_data, _sim, _state

from automata.agents.heuristic_agent import HeuristicAgent
from automata.search.continuation import AgentContinuationPolicy
from automata.search.contracts import SearchContext
from automata.search.ismcts.engine import (
    SearchAdvanceLimitExceeded,
    SearchDeadlineExceeded,
    _bounded_continuation,
)
from automata.search_reuse import PlanningDrawRecipe, reuse_disabled, search_reuse


def _start(state, agent):
    sim = _sim(state, agent)
    sim._reuse_known_keys = True
    sim._defer_state_copy = True
    decision = sim.advance()
    context = SearchContext("hero_brogan", sim.our_team, "hero_brogan", decision)
    return sim, decision, context


def _continue(sim, decision, context):
    return _bounded_continuation(
        sim,
        decision,
        sim.cfg,
        AgentContinuationPolicy(sim.environment_policy),
        context,
        sim.state.round,
        sim.state.round,
        sim.state.turn,
    )


def _occupancy(state):
    return {h: tile.occupant_id for h, tile in state.board.tiles.items()}


def test_replayed_trajectories_match_rules_choices_rng_counters_and_independent_worlds():
    state = _state()
    actual_agent, reference_agent = HeuristicAgent(7), HeuristicAgent(7)
    with search_reuse() as cache:
        for seed in [7, 7, 11, 7, 23, 11, 7, 23]:
            actual_agent._rng.seed(seed)
            reference_agent._rng.seed(seed)
            actual, decision, context = _start(state, actual_agent)
            result = _continue(actual, decision, context)
            with reuse_disabled():
                reference, ref_decision, ref_context = _start(state, reference_agent)
                expected = _continue(reference, ref_decision, ref_context)
            assert result[0].kind == expected[0].kind
            assert result[2:] == expected[2:]
            assert actual_agent._rng.getstate() == reference_agent._rng.getstate()
            assert _counters(actual) == _counters(reference)
            assert _rule_data(actual.state.model_dump(mode="json", exclude={"board"})) == (
                _rule_data(reference.state.model_dump(mode="json", exclude={"board"}))
            )
            assert _occupancy(actual.state) == _occupancy(reference.state)
            snapshot = actual.state
            copied_decision = actual._own_state(result[0])
            assert actual.state is not snapshot
            if copied_decision.hero is not None:
                assert copied_decision.hero is actual.state.get_hero(copied_decision.hero.id)
            actual.state.get_hero("hero_brogan").gold = 999
            assert snapshot.get_hero("hero_brogan").gold != 999
        assert cache.hits["rollout_variants"] > 0


def test_failed_trajectory_variant_rewinds_all_consumed_draws():
    state = _state()
    actual_agent, reference_agent = HeuristicAgent(7), HeuristicAgent(7)
    with search_reuse() as cache:
        sim, decision, context = _start(state, actual_agent)
        _continue(sim, decision, context)
        key, variants = next(iter(cache.rollouts.items()))
        good = variants[0]
        assert good.events
        agent, recipe = good.events[0]
        bad = replace(
            good, events=((agent, PlanningDrawRecipe(recipe.count, recipe.eligible, -1)),)
        )
        cache.rollouts[key] = (bad, good)
        actual_agent._rng.seed(7)
        actual, decision, context = _start(state, actual_agent)
        _continue(actual, decision, context)
        with reuse_disabled():
            reference, decision, context = _start(state, reference_agent)
            _continue(reference, decision, context)
        assert actual_agent._rng.getstate() == reference_agent._rng.getstate()
        assert _rule_data(actual.state.model_dump(mode="json", exclude={"board"})) == (
            _rule_data(reference.state.model_dump(mode="json", exclude={"board"}))
        )
        assert cache.hits["rollout_variants"] == 1


def test_trajectory_cache_respects_deadline_and_total_advancement_budget():
    state, agent = _state(), HeuristicAgent(7)
    with search_reuse():
        first, decision, context = _start(state, agent)
        _continue(first, decision, context)
        agent._rng.seed(7)
        expired, decision, context = _start(state, agent)
        expired.deadline = -1
        before = agent._rng.getstate()
        with pytest.raises(SearchDeadlineExceeded):
            _continue(expired, decision, context)
        assert agent._rng.getstate() == before
        agent._rng.seed(7)
        limited, decision, context = _start(state, agent)
        limited.max_advance_steps = first.advance_steps - 1
        with pytest.raises(SearchAdvanceLimitExceeded):
            _continue(limited, decision, context)
