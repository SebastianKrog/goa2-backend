"""Tests for fast state cloning (the MCTS substrate)."""

from __future__ import annotations

import copy

import pytest

from automata.agents.random_agent import RandomAgent
from automata.runtime.clone import clone_state
from automata.runtime.effects import register_all_effects
from goa2.domain.input import InputResponse
from goa2.domain.models import GamePhase, TeamColor
from goa2.domain.state import GameState
from goa2.domain.types import HeroID
from goa2.engine.session import GameSession, SessionResultType
from goa2.engine.setup import GameSetup


def _fresh() -> GameState:
    register_all_effects()
    return GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json",
        ["Wasp", "Xargatha"],
        ["Arien", "Brogan"],
        game_type="QUICK",
        seed=1,
    )


def _play_one_round(state: GameState) -> None:
    agent = RandomAgent(3)
    session = GameSession(state)
    resp = None
    for _ in range(2000):
        if session.current_phase == GamePhase.PLANNING:
            for team in state.teams.values():
                for hero in team.heroes:
                    if session.current_phase != GamePhase.PLANNING:
                        break
                    if hero.id in state.pending_inputs:
                        continue
                    planning = agent.choose_planning(state, hero)
                    if planning.card is None:
                        session.pass_turn(HeroID(hero.id))
                    else:
                        session.commit_card(HeroID(hero.id), planning.card)
            resp = None
            if state.round > 1:
                return
            continue
        result = session.advance(resp)
        resp = None
        if result.result_type == SessionResultType.GAME_OVER:
            return
        if result.result_type == SessionResultType.INPUT_NEEDED:
            req = result.input_request
            assert req is not None
            resp = InputResponse(request_id=req.id, selection=agent.choose_input(state, req))


def _positions(state: GameState) -> dict[str, tuple[int, int, int]]:
    return {k: (v.q, v.r, v.s) for k, v in state.unit_locations.items()}


def _occupancy(state: GameState) -> dict[str, str]:
    return {str(tid): str(t.occupant_id) for tid, t in state.board.tiles.items() if t.occupant_id}


def test_clone_copies_mutable_board_tiles() -> None:
    state = _fresh()
    clone = clone_state(state)
    assert clone.board is not state.board
    assert clone.board.tiles is not state.board.tiles


def test_playing_clone_does_not_mutate_original() -> None:
    state = _fresh()
    pos0, occ0 = _positions(state), _occupancy(state)
    round0, turn0 = state.round, state.turn

    clone = clone_state(state)
    _play_one_round(clone)

    # The clone advanced; the original is untouched (positions AND occupancy).
    assert clone.round > round0 or clone.winner is not None
    assert _positions(state) == pos0
    assert _occupancy(state) == occ0
    assert (state.round, state.turn) == (round0, turn0)


def test_mutating_clone_state_is_independent() -> None:
    state = _fresh()
    clone = clone_state(state)
    before = state.teams[TeamColor.BLUE].life_counters
    clone.teams[TeamColor.BLUE].life_counters -= 3
    assert state.teams[TeamColor.BLUE].life_counters == before


def test_fast_card_clones_preserve_aliases_and_isolate_every_mutable_container():
    from goa2.domain.models import ActionType

    state = _fresh()
    hero = state.teams[TeamColor.RED].heroes[0]
    original = hero.hand[0]
    shared_metadata = {"nested": [1, {"value": 2}]}
    original.metadata = shared_metadata
    hero.hand[1].metadata = shared_metadata
    state.execution_context["card"] = original
    clone = clone_state(state)
    cloned_hero = clone.teams[TeamColor.RED].heroes[0]
    card = cloned_hero.hand[0]
    assert card is not original
    assert card is next(c for c in cloned_hero.deck if c.id == card.id)
    assert card is clone.execution_context["card"]
    assert card.metadata is cloned_hero.hand[1].metadata
    assert card.metadata is not shared_metadata
    card.metadata["nested"][1]["value"] = 99
    assert original.metadata["nested"][1]["value"] == 2
    original_actions = original.secondary_actions.copy()
    card.secondary_actions[ActionType.MOVEMENT] = 99
    assert original.secondary_actions == original_actions
    card.__pydantic_fields_set__.add("clone_only")
    assert "clone_only" not in original.__pydantic_fields_set__
    card._definition_revision += 1
    assert card._definition_revision != original._definition_revision


def test_fast_clone_supports_card_metadata_cycles():
    state = _fresh()
    hero = state.teams[TeamColor.RED].heroes[0]
    first, second = hero.hand[:2]
    first.metadata["other"] = second
    second.metadata["other"] = first
    clone = clone_state(state)
    cloned = clone.teams[TeamColor.RED].heroes[0]
    assert cloned.hand[0].metadata["other"] is cloned.hand[1]
    assert cloned.hand[1].metadata["other"] is cloned.hand[0]


def test_unknown_card_subclass_uses_general_independent_deepcopy():
    from pydantic import Field

    from goa2.domain.models import Card

    class CustomCard(Card):
        extra_rules: dict[str, list[int]] = Field(default_factory=dict)

    state = _fresh()
    hero = state.teams[TeamColor.RED].heroes[0]
    custom = CustomCard(**hero.hand[0].model_dump(), extra_rules={"values": [1]})
    hero.hand[0] = custom
    clone = clone_state(state)
    copied = clone.teams[TeamColor.RED].heroes[0].hand[0]
    copied.extra_rules["values"].append(2)
    assert custom.extra_rules == {"values": [1]}


def test_spell_card_clone_is_independent():
    state = GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json", ["Gydion"], ["Min"], seed=1
    )
    hero = state.teams[TeamColor.RED].heroes[0]
    assert hero.spells
    hero.spells[0].metadata["nested"] = [1]
    clone = clone_state(state)
    copied = clone.teams[TeamColor.RED].heroes[0].spells[0]
    copied.metadata["nested"].append(2)
    assert hero.spells[0].metadata["nested"] == [1]


def test_same_id_independent_cards_remain_independent():
    state = _fresh()
    hero = state.teams[TeamColor.RED].heroes[0]
    hero.hand[0] = hero.hand[0].model_copy(deep=True)
    clone = clone_state(state)
    copied = clone.teams[TeamColor.RED].heroes[0]
    hand_card = copied.hand[0]
    deck_card = next(card for card in copied.deck if card.id == hand_card.id)
    assert hand_card is not deck_card
    hand_card.metadata["independent"] = True
    assert "independent" not in deck_card.metadata


def _without_transport_labels(value):
    if isinstance(value, list):
        return [_without_transport_labels(item) for item in value]
    if isinstance(value, dict):
        return {
            key: _without_transport_labels(item)
            for key, item in value.items()
            if key not in {"step_id", "pending_request_id"}
            and not (key == "id" and "request_type" in value)
        }
    return value


@pytest.mark.parametrize(
    "reds,blues",
    [
        (["Wasp", "Xargatha"], ["Arien", "Brogan"]),
        (["Min", "Gydion"], ["Widget", "Trinkets"]),
        (["Wuk", "Mrak"], ["Tali", "Ignatia"]),
        (["Emmitt", "NebKher"], ["Arien", "Hanu"]),
    ],
)
def test_fast_clone_matches_reference_rules_after_playing_a_round(reds, blues):
    register_all_effects()
    state = GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json", reds, blues, game_type="QUICK", seed=1
    )
    # Original search clone implementation, with the same geometry sharing.
    reference = copy.deepcopy(state, {id(state.board): state.board})
    reference.board = state.board.model_copy()
    reference.board.tiles = {key: tile.model_copy() for key, tile in state.board.tiles.items()}
    accelerated = clone_state(state)
    original = state.model_dump(mode="json", exclude={"board"})
    _play_one_round(reference)
    _play_one_round(accelerated)
    assert reference.round > 1 or reference.winner is not None
    assert accelerated.round > 1 or accelerated.winner is not None
    assert _without_transport_labels(accelerated.model_dump(mode="json", exclude={"board"})) == (
        _without_transport_labels(reference.model_dump(mode="json", exclude={"board"}))
    )
    assert _occupancy(accelerated) == _occupancy(reference)
    assert state.model_dump(mode="json", exclude={"board"}) == original


def test_custom_tile_initial_clone_keeps_mutable_extension_fields_independent():
    from pydantic import Field

    from goa2.domain.tile import Tile

    class ScenarioTile(Tile):
        annotations: dict = Field(default_factory=dict)

    state = _fresh()
    position = next(iter(state.board.tiles))
    state.board.tiles[position] = ScenarioTile(
        **state.board.tiles[position].model_dump(), annotations={"notes": [1]}
    )
    copied = clone_state(state)
    copied.board.tiles[position].annotations["notes"].append(2)
    assert state.board.tiles[position].annotations == {"notes": [1]}
