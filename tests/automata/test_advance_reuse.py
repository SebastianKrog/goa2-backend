"""Continuation replay must preserve policy draws, rules state and guard budgets."""

import random
from dataclasses import replace

import pytest

from automata.agents.heuristic_agent import HeuristicAgent
from automata.runtime.clone import clone_state
from automata.runtime.effects import register_all_effects
from automata.search.config import SearchConfig
from automata.search.contracts import CutoffUnit, LeafMode
from automata.search.ismcts.engine import (
    SearchAdvanceLimitExceeded,
    SearchDeadlineExceeded,
    _Simulator,
)
from automata.search_reuse import PlanningDrawRecipe, reuse_disabled, search_reuse
from goa2.domain.input import InputResponse, selection_value
from goa2.domain.models import TeamColor
from goa2.engine.session import GameSession
from goa2.engine.setup import GameSetup


def _state():
    register_all_effects()
    return GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json",
        ["Arien", "Hanu"],
        ["Brogan", "Tigerclaw"],
        seed=9,
    )


def _sim(state, agent, **kwargs):
    return _Simulator(
        clone_state(state),
        TeamColor.BLUE,
        agent,
        cfg=SearchConfig(cutoff_unit=CutoffUnit.TURNS, leaf_mode=LeafMode.BOUNDED_CONTINUATION),
        owned_hero_ids=frozenset({"hero_brogan"}),
        **kwargs,
    )


def _counters(sim):
    return tuple(
        getattr(sim, name)
        for name in (
            "advance_steps",
            "_advance_calls",
            "_advance_transitions",
            "_session_advances",
            "_environment_planning",
            "_environment_inputs",
            "_continuation_inputs",
            "_current_advance_transitions",
        )
    )


def test_replayed_advances_match_engine_choices_rng_state_and_counters():
    state = _state()
    actual_agent, reference_agent = HeuristicAgent(23), HeuristicAgent(23)
    with search_reuse() as cache:
        for _ in range(16):
            actual = _sim(state, actual_agent)
            decision = actual.advance()
            reference = _sim(state, reference_agent)
            with reuse_disabled():
                expected = reference.advance()
            assert decision.kind == expected.kind == "CARD"
            assert decision.hero is actual.state.get_hero("hero_brogan")
            assert actual.state.model_dump(exclude={"board"}) == reference.state.model_dump(
                exclude={"board"}
            )
            assert actual_agent._rng.getstate() == reference_agent._rng.getstate()
            assert _counters(actual) == _counters(reference)
        assert cache.hits["advance_variants"] > 0


def test_failed_variant_restores_rng_before_trying_next_variant():
    state = _state()
    with search_reuse() as cache:
        first = _sim(state, HeuristicAgent(23))
        first.advance()
        key, variants = next(iter(cache.advances.items()))
        # Impossible selected index forces a miss after consuming draws.
        bad = replace(variants[0], recipes=(PlanningDrawRecipe(5, (0,), 1),))
        cache.advances[key] = (bad, *variants)
        actual_agent, reference_agent = HeuristicAgent(23), HeuristicAgent(23)
        actual, reference = _sim(state, actual_agent), _sim(state, reference_agent)
        actual.advance()
        with reuse_disabled():
            reference.advance()
        assert actual_agent._rng.getstate() == reference_agent._rng.getstate()
        assert actual.state.model_dump(exclude={"board"}) == reference.state.model_dump(
            exclude={"board"}
        )
        assert cache.hits["advance_variants"] == 1


def test_cached_worlds_and_hero_references_remain_independent():
    state = _state()
    with search_reuse() as cache:
        first = _sim(state, HeuristicAgent(23))
        first.advance()
        first.state.get_hero("hero_brogan").gold = 99
        second = _sim(state, HeuristicAgent(23))
        result = second.advance()
        assert cache.hits["advance_variants"] == 1
        assert result.hero is second.state.get_hero("hero_brogan")
        assert result.hero.gold != 99


def test_cached_advance_cannot_bypass_step_budget_or_deadline():
    state = _state()
    with search_reuse():
        first = _sim(state, HeuristicAgent(23))
        first.advance()
        constrained = _sim(state, HeuristicAgent(23), max_advance_steps=first.advance_steps - 1)
        with pytest.raises(SearchAdvanceLimitExceeded):
            constrained.advance()
        agent = HeuristicAgent(23)
        rng_before = agent._rng.getstate()
        expired = _sim(state, agent, deadline=-1)
        with pytest.raises(SearchDeadlineExceeded):
            expired.advance()
        assert agent._rng.getstate() == rng_before


def test_prompt_copy_retains_stale_response_validation_and_isolation():
    state = _state()
    session = GameSession(state)
    agent = HeuristicAgent(7)
    for team in state.teams.values():
        for hero in team.heroes:
            session.commit_card(hero.id, agent.choose_planning(state, hero).card)
    cfg = SearchConfig(cutoff_unit=CutoffUnit.TURNS, leaf_mode=LeafMode.BOUNDED_CONTINUATION)
    owned = frozenset(str(hero.id) for team in state.teams.values() for hero in team.heroes)
    with search_reuse() as cache:
        first = _Simulator(
            clone_state(state), TeamColor.BLUE, HeuristicAgent(7), cfg=cfg, owned_hero_ids=owned
        )
        first_result = first.advance()
        assert first_result.request is not None
        first_result.request.prompt = "mutated prompt"
        second = _Simulator(
            clone_state(state), TeamColor.BLUE, HeuristicAgent(7), cfg=cfg, owned_hero_ids=owned
        )
        second_result = second.advance()
        assert cache.hits["advance_variants"] == 1
        request = second_result.request
        assert request is not None and request.prompt != "mutated prompt"
        with pytest.raises(ValueError, match="request_id"):
            second.session.advance(InputResponse(request_id="stale", selection="anything"))
        second.session.advance(
            InputResponse(request_id=request.id, selection=selection_value(request.options[0]))
        )


def test_planning_recipe_consumes_exactly_one_draw_per_candidate():
    recipe = PlanningDrawRecipe(5, (1, 3), 1)
    actual, expected = random.Random(3), random.Random(3)
    draws = [expected.random() for _ in range(5)]
    assert recipe.matches(actual) == (max((1, 3), key=draws.__getitem__) == 1)
    assert actual.getstate() == expected.getstate()


def _rule_data(value):
    if isinstance(value, list):
        return [_rule_data(item) for item in value]
    if isinstance(value, dict):
        return {
            key: _rule_data(item)
            for key, item in value.items()
            if key not in {"step_id", "pending_request_id"}
            and not (key == "id" and "request_type" in value)
        }
    return value


@pytest.mark.parametrize("deferred", [False, True])
def test_carried_keys_stay_current_across_owned_planning_and_input_actions(deferred):
    from automata.search.ismcts.engine import legal_keys
    from automata.search_reuse import board_key

    state = _state()
    actual_agent, reference_agent = HeuristicAgent(23), HeuristicAgent(23)
    with search_reuse() as cache:
        actual, reference = _sim(state, actual_agent), _sim(state, reference_agent)
        actual._reuse_known_keys = True
        actual._defer_state_copy = deferred
        decision = actual.advance()
        with reuse_disabled():
            expected = reference.advance()
        for _ in range(12):
            assert decision.kind == expected.kind
            assert _rule_data(actual.state.model_dump(mode="json", exclude={"board"})) == (
                _rule_data(reference.state.model_dump(mode="json", exclude={"board"}))
            )
            assert actual_agent._rng.getstate() == reference_agent._rng.getstate()
            key = actual._known_state_key
            assert key[0] == id(actual.state)
            assert key[1] == cache.compact.fingerprint(actual.state)
            assert key[2] == board_key(actual.state)
            if decision.is_terminal:
                break
            legal = legal_keys(decision)
            reference_legal = legal_keys(expected)
            assert legal == reference_legal
            chosen = legal[0] if legal else None
            decision = actual.apply_ours(decision, chosen)
            with reuse_disabled():
                expected = reference.apply_ours(expected, chosen)


@pytest.mark.parametrize("seed", [1, 7, 23])
def test_owned_planning_reuse_preserves_fresh_rng_choices(seed):
    from automata.search.continuation import AgentContinuationPolicy
    from automata.search.contracts import SearchContext
    from automata.search.ismcts.engine import _choose_continuation, legal_keys
    from automata.search_reuse import board_key

    state = _state()
    actual_agent, reference_agent = HeuristicAgent(seed), HeuristicAgent(seed)
    with search_reuse() as cache:
        sim = _sim(state, actual_agent)
        sim._reuse_known_keys = True
        decision = sim.advance()
        assert decision.kind == "CARD"
        legal = legal_keys(decision)
        context = SearchContext("hero_brogan", TeamColor.BLUE, "hero_brogan", decision)
        actual_policy = AgentContinuationPolicy(actual_agent)
        reference_policy = AgentContinuationPolicy(reference_agent)
        # Synchronize draws used to advance other heroes before our decision.
        reference_agent._rng.setstate(actual_agent._rng.getstate())
        for _ in range(40):
            actual = _choose_continuation(sim, actual_policy, context, decision, legal)
            with reuse_disabled():
                expected = reference_policy.choose(context, sim.state, decision, legal)
            assert actual == expected
            assert actual_agent._rng.getstate() == reference_agent._rng.getstate()
        assert cache.hits["continuations"] == 39
        # A changed hand must use a new verified world key, never the old recipe.
        decision.hero.hand.reverse()
        sim._known_state_key = (
            id(sim.state),
            cache.compact.fingerprint(sim.state),
            board_key(sim.state),
        )
        actual = _choose_continuation(sim, actual_policy, context, decision, legal_keys(decision))
        with reuse_disabled():
            expected = reference_policy.choose(context, sim.state, decision, legal_keys(decision))
        assert actual == expected
        assert actual_agent._rng.getstate() == reference_agent._rng.getstate()
        assert len(cache.continuations) == 2


def test_shared_planning_agent_consumes_serial_equivalent_draws_without_lock_fields():
    from concurrent.futures import ThreadPoolExecutor
    from copy import deepcopy

    state = _state()
    hero = state.get_hero("hero_brogan")
    actual, reference = HeuristicAgent(7), HeuristicAgent(7)
    with ThreadPoolExecutor(max_workers=4) as pool:
        choices = list(pool.map(lambda _: actual.choose_planning(state, hero), range(80)))
    for _ in choices:
        reference.choose_planning(state, hero)
    assert actual._rng.getstate() == reference._rng.getstate()
    copied = deepcopy(actual)
    assert copied._rng.getstate() == actual._rng.getstate()


def test_deferred_snapshot_detaches_before_commit_and_evaluation():
    from automata.search.continuation import AgentContinuationPolicy
    from automata.search.contracts import LeafEvaluation, SearchContext
    from automata.search.ismcts.engine import _rollout, legal_keys
    from automata.search_reuse import board_key

    state = _state()
    with search_reuse() as cache:
        sim = _sim(state, HeuristicAgent(23))
        sim._reuse_known_keys = True
        sim._defer_state_copy = True
        decision = sim.advance()
        assert sim._borrowed_state
        snapshot = sim.state
        before = cache.compact.fingerprint(snapshot), board_key(snapshot)
        decision = sim.apply_ours(decision, legal_keys(decision)[0])
        assert snapshot is not sim.state
        assert before == (cache.compact.fingerprint(snapshot), board_key(snapshot))

        if not sim._borrowed_state:
            pytest.fail("test must reach a saved snapshot before custom evaluation")
        snapshot = sim.state
        before = cache.compact.fingerprint(snapshot), board_key(snapshot)

        class MutatingEvaluator:
            def evaluate(self, context, world):
                assert world is sim.state and world is not snapshot
                if context.decision.hero is not None:
                    assert context.decision.hero is world.get_hero(context.decision.hero.id)
                world.get_hero("hero_brogan").gold = 999
                world.execution_context["evaluator_probe"] = [1]
                return LeafEvaluation(value=0.0)

        context = SearchContext("hero_brogan", TeamColor.BLUE, "hero_brogan", decision)
        _rollout(
            sim,
            decision,
            replace(sim.cfg, cutoff_limit=0),
            AgentContinuationPolicy(sim.environment_policy),
            MutatingEvaluator(),
            context,
        )
        assert sim.state.get_hero("hero_brogan").gold == 999
        assert before == (cache.compact.fingerprint(snapshot), board_key(snapshot))
