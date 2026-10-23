"""Scoring reuse is local to a read-only batch and preserves search guards."""
import hashlib
from concurrent.futures import ThreadPoolExecutor
from threading import Barrier
from unittest.mock import patch

import pytest
from automata.agents.heuristic_agent import HeuristicAgent
from automata.runtime.clone import clone_state
from automata.runtime.effects import register_all_effects
from automata.search.ismcts.engine import _state_fingerprint
from goa2.domain.models import TeamColor
from goa2.engine.setup import GameSetup


@pytest.fixture
def state():
    register_all_effects()
    return GameSetup.create_game("src/goa2/data/maps/forgotten_island.json",["Wasp"],["Arien"],seed=7)


def test_scoring_reuses_same_destination_but_new_batch_observes_mutation(state):
    agent = HeuristicAgent()
    destination = state.get_position("hero_wasp")
    with patch.object(agent,"_uncached_position_score",wraps=agent._uncached_position_score) as calculate:
        with agent.position_scoring(state):
            before = agent._position_score(state,destination,TeamColor.RED)
            assert agent._position_score(state,destination,TeamColor.RED) == before
        assert calculate.call_count == 1
        enemy = state.teams[TeamColor.BLUE].heroes[0]
        nearby = next(h for h in destination.neighbors() if h in state.board.tiles and not state.board.tiles[h].is_obstacle and not state.board.tiles[h].is_occupied)
        state.move_unit(str(enemy.id),nearby)
        with agent.position_scoring(state):
            after = agent._position_score(state,destination,TeamColor.RED)
        assert calculate.call_count == 2
        assert after > before


def test_nested_exception_restores_outer_scoring_batch(state):
    agent = HeuristicAgent()
    other = clone_state(state)
    destination = state.get_position("hero_wasp")
    with patch.object(agent,"_uncached_position_score",wraps=agent._uncached_position_score) as calculate:
        with agent.position_scoring(state):
            before = agent._position_score(state,destination,TeamColor.RED)
            with pytest.raises(RuntimeError):
                with agent.position_scoring(other):
                    agent._position_score(other,destination,TeamColor.RED)
                    raise RuntimeError("policy failure")
            assert agent._position_score(state,destination,TeamColor.RED) == before
        assert calculate.call_count == 2
        agent._position_score(state,destination,TeamColor.RED)
        assert calculate.call_count == 3


def test_shared_agent_scoring_batches_are_isolated_between_threads(state):
    agent = HeuristicAgent()
    other = clone_state(state)
    destination = state.get_position("hero_wasp")
    other.remove_entity("hero_arien")
    barrier = Barrier(2)
    def score(world):
        expected = agent._position_score(world,destination,TeamColor.RED)
        with agent.position_scoring(world):
            first = agent._position_score(world,destination,TeamColor.RED)
            barrier.wait(timeout=5)
            assert agent._position_score(world,destination,TeamColor.RED) == first == expected
        return first
    with ThreadPoolExecutor(max_workers=2) as pool:
        results = list(pool.map(score,[state,other]))
    assert all(isinstance(value,float) for value in results)


def test_direct_fingerprint_bytes_match_previous_json_encoding_and_detect_mutations(state):
    def previous(world):
        return hashlib.blake2b(world.model_dump_json(exclude={"board"}).encode("utf-8"),digest_size=16).digest()
    assert _state_fingerprint(state) == previous(state)
    original = _state_fingerprint(state)
    state.execution_context["nested"] = {"selection":["MOVE",1,None]}
    assert _state_fingerprint(state) == previous(state) != original
    original = _state_fingerprint(state)
    state.teams[TeamColor.RED].heroes[0].gold += 1
    assert _state_fingerprint(state) == previous(state) != original


def test_allocation_free_hex_distances_match_previous_cube_subtraction(state):
    hexes = list(state.board.tiles)
    for first in hexes:
        for second in hexes[::13]:
            assert first.distance(second) == (first-second).length()
