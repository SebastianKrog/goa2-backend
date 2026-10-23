"""Compact state keys must preserve rule data while removing repeated payloads."""

import marshal

import pytest

from automata.runtime.clone import clone_state
from automata.runtime.effects import register_all_effects
from automata.search_reuse import CompactState, current_reuse, search_reuse
from goa2.domain.models import ActionType, CardState, TokenType
from goa2.domain.state import GameState
from goa2.engine.setup import GameSetup


def _state():
    register_all_effects()
    return GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json", ["Min"], ["Gydion"], seed=7
    )


def _card(state):
    return state.teams[next(iter(state.teams))].heroes[0].hand[0]


def _token(state):
    return state.token_pool[TokenType.SMOKE_BOMB][0]


def test_keys_are_independent_of_clone_and_save_object_identity():
    state = _state()
    encoder = CompactState()
    baseline = encoder.fingerprint(state)
    assert encoder.fingerprint(clone_state(state)) == baseline
    assert encoder.fingerprint(GameState.model_validate_json(state.model_dump_json())) == baseline


def test_normal_serialization_is_unchanged_by_compact_encoding():
    state = _state()
    saved = state.model_dump_json()
    CompactState().fingerprint(state)
    assert state.model_dump_json() == saved
    decoded = state.model_dump(mode="json")
    hero = decoded["teams"]["RED"]["heroes"][0]
    assert isinstance(hero["hand"][0], dict)
    assert "effect_text" in hero["deck"][0]
    assert isinstance(decoded["token_pool"]["smoke_bomb"][0], dict)
    assert GameState.model_validate_json(saved).model_dump(exclude={"board"}) == state.model_dump(
        exclude={"board"}
    )


def test_cards_and_tokens_have_one_record_and_collections_have_references():
    state = _state()
    encoder = CompactState()
    mutable, (cards, tokens) = marshal.loads(encoder.encode(state))
    expected_cards = {
        id(card)
        for team in state.teams.values()
        for hero in team.heroes
        for card in [*hero.deck, *hero.spells, hero.ultimate_card]
        if card is not None
    }
    assert len(cards) == len(expected_cards)
    hero = mutable["teams"]["RED"]["heroes"][0]
    assert all(isinstance(reference, int) for reference in hero[1][1])  # deck
    assert len(tokens) == sum(len(pool) for pool in state.token_pool.values())
    for pool in mutable["token_pool"].values():
        for reference in pool:
            assert isinstance(reference, int)
            assert reference in mutable["misc_entities"].values()
    assert all(len(record) == 3 for record in cards)  # default extras omitted
    assert b"effect_text" not in encoder.encode(state)
    assert b"Smoke Bomb" not in encoder.encode(state)


@pytest.mark.parametrize(
    "field,value",
    [
        ("state", CardState.DISCARD),
        ("is_facedown", True),
        ("played_this_round", True),
        ("is_active_base", True),
        ("enraged_active_override", True),
        ("initiative_override", 99),
        ("passive_uses_this_turn", 2),
        ("metadata", {"test": [1, 2]}),
        ("metadata", None),
    ],
)
def test_every_mutable_card_field_changes_key(field, value):
    state = _state()
    card = _card(state)
    assert getattr(card, field) != value
    encoder = CompactState()
    before = encoder.fingerprint(state)
    setattr(card, field, value)
    assert encoder.fingerprint(state) != before


@pytest.mark.parametrize(
    "field,value",
    [
        ("range_value", 99),
        ("effect_text", "Custom scenario"),
        ("name", "Changed name"),
        ("initiative", 99),
    ],
)
def test_definition_overrides_cannot_reuse_an_old_key(field, value):
    state = _state()
    encoder = CompactState()
    before = encoder.fingerprint(state)
    setattr(_card(state), field, value)
    assert encoder.fingerprint(state) != before


def test_nested_mutable_card_data_is_checked_every_time():
    state = _state()
    encoder = CompactState()
    before = encoder.fingerprint(state)
    _card(state).secondary_actions[ActionType.MOVEMENT] = 99
    after_actions = encoder.fingerprint(state)
    assert after_actions != before
    _card(state).metadata["nested"] = {"value": 1}
    before_nested = encoder.fingerprint(state)
    _card(state).metadata["nested"]["value"] = 2
    assert encoder.fingerprint(state) != before_nested


@pytest.mark.parametrize(
    "field,value",
    [
        ("owner_id", "hero_min"),
        ("is_facedown", True),
        ("is_passable", True),
        ("persists_end_of_round", True),
        ("is_immune_to_enemy_actions", True),
        ("token_type", TokenType.GRENADE),
        ("name", "Custom token"),
    ],
)
def test_token_flags_owner_and_definition_are_preserved(field, value):
    state = _state()
    encoder = CompactState()
    before = encoder.fingerprint(state)
    setattr(_token(state), field, value)
    assert encoder.fingerprint(state) != before


def test_token_registry_references_and_supply_order_are_preserved():
    state = _state()
    encoder = CompactState()
    before = encoder.fingerprint(state)
    pool = state.token_pool[TokenType.MINE_BLAST]
    pool.reverse()
    assert encoder.fingerprint(state) != before
    pool.reverse()
    assert encoder.fingerprint(state) == before
    state.misc_entities.pop(_token(state).id)
    assert encoder.fingerprint(state) != before


def test_distinct_copies_of_same_card_are_not_silently_collapsed():
    state = _state()
    encoder = CompactState()
    before = encoder.fingerprint(state)
    hero = state.teams[next(iter(state.teams))].heroes[0]
    hero.hand[0] = hero.hand[0].model_copy(deep=True)
    hero.hand[0].metadata["independent"] = True
    assert encoder.fingerprint(state) != before


@pytest.mark.parametrize(
    "target,field,value",
    [
        ("hero", "gold", 9),
        ("hero", "level", 5),
        ("hero", "name", "Custom hero"),
        ("minion", "name", "Custom minion"),
        ("minion", "lane_id", "other_lane"),
    ],
)
def test_hero_and_minion_mutations_are_not_hidden_by_shared_definitions(target, field, value):
    state = _state()
    encoder = CompactState()
    before = encoder.fingerprint(state)
    team = state.teams[next(iter(state.teams))]
    entity = team.heroes[0] if target == "hero" else team.minions[0]
    setattr(entity, field, value)
    assert encoder.fingerprint(state) != before


def test_shared_definitions_do_not_accumulate_for_equivalent_clones():
    state = _state()
    encoder = CompactState()
    encoder.fingerprint(state)
    count = len(encoder.definitions)
    for _ in range(4):
        encoder.fingerprint(clone_state(state))
    assert len(encoder.definitions) == count


def test_compact_catalog_is_local_to_one_search():
    assert current_reuse() is None
    with search_reuse() as first:
        first.compact.fingerprint(_state())
        assert first.compact.definitions
    with search_reuse() as second:
        assert second is not first
        assert not second.compact.definitions
    assert current_reuse() is None


@pytest.mark.parametrize(
    "field,value",
    [
        ("rng_seed", 999),
        ("turn", 3),
        ("execution_context", {"selected": "enemy"}),
        ("planning_done", ["hero_min"]),
        ("next_entity_id", 999),
    ],
)
def test_non_entity_rule_state_is_still_included(field, value):
    state = _state()
    encoder = CompactState()
    before = encoder.fingerprint(state)
    setattr(state, field, value)
    assert encoder.fingerprint(state) != before


def test_hidden_hand_membership_and_empty_turn_slots_are_preserved():
    state = _state()
    encoder = CompactState()
    hero = state.teams[next(iter(state.teams))].heroes[0]
    before = encoder.fingerprint(state)
    hero.hand.reverse()
    assert encoder.fingerprint(state) != before
    hero.hand.reverse()
    assert encoder.fingerprint(state) == before
    hero.played_cards.append(None)
    assert encoder.fingerprint(state) != before


def test_catalog_overflow_retains_complete_definitions():
    state = _state()
    encoder = CompactState()
    encoder.definitions = {("padding", i): i + 1 for i in range(4096)}
    before = encoder.fingerprint(state)
    _card(state).range_value = 99
    assert encoder.fingerprint(state) != before
    assert len(encoder.definitions) == 4096


def test_future_token_subclass_fields_are_not_dropped():
    from goa2.domain.models import Token

    class CooldownToken(Token):
        cooldown: int = 0

    state = _state()
    original = _token(state)
    token = CooldownToken(**original.model_dump())
    state.token_pool[TokenType.SMOKE_BOMB][0] = token
    state.misc_entities[token.id] = token
    encoder = CompactState()
    before = encoder.fingerprint(state)
    token.cooldown = 2
    assert encoder.fingerprint(state) != before


def test_future_card_subclass_mutable_definition_containers_are_checked():
    from pydantic import Field

    from goa2.domain.models import Card

    class BonusCard(Card):
        bonuses: dict[str, int] = Field(default_factory=dict)

    state = _state()
    hero = state.teams[next(iter(state.teams))].heroes[0]
    hero.hand[0] = BonusCard(**hero.hand[0].model_dump(), bonuses={"attack": 1})
    encoder = CompactState()
    before = encoder.fingerprint(state)
    hero.hand[0].bonuses["attack"] = 2
    assert encoder.fingerprint(state) != before


def test_nested_metadata_references_also_receive_records():
    state = _state()
    nested = _card(state).model_copy(deep=True)
    nested.metadata["nested_only"] = True
    _card(state).metadata["another_card"] = nested
    encoder = CompactState()
    payload = encoder.encode(state)
    _, (cards, tokens) = marshal.loads(payload)
    assert len(cards) == len(encoder._cards)
    assert len(tokens) == len(encoder._tokens)
    before = encoder.fingerprint(state)
    nested.metadata["nested_only"] = False
    assert encoder.fingerprint(state) != before


def test_board_keys_compare_by_value_and_observe_mutable_tiles():
    from automata.search_reuse import board_key, position_board_key

    state = _state()
    with search_reuse():
        original = board_key(state)
        geometry = position_board_key(state)
        assert position_board_key(state) is geometry
        assert board_key(clone_state(state)) == original
        reload = GameState.model_validate_json(state.model_dump_json())
        assert board_key(reload) == original
        assert hash(position_board_key(reload)) == hash(geometry)
        tile = next(iter(state.board.tiles.values()))
        tile.occupant_id = "different_occupant"
        occupied = board_key(state)
        assert occupied != original
        tile.is_terrain = not tile.is_terrain
        assert board_key(state) != occupied
        terrain = board_key(state)
        tile.zone_id = "different_zone"
        assert board_key(state) != terrain


def test_admission_bypasses_board_key_for_cheap_transitions(monkeypatch):
    from automata.agents.heuristic_agent import HeuristicAgent
    from automata.search.ismcts import engine
    from goa2.domain.models import TeamColor
    from goa2.engine.session import GameSession, SessionResult, SessionResultType

    state = _state()

    def advance(session, response):
        session.state.execution_context["changed"] = True
        return SessionResult(
            result_type=SessionResultType.ACTION_COMPLETE, current_phase=session.state.phase
        )

    monkeypatch.setattr(GameSession, "advance", advance)
    monkeypatch.setattr(engine, "board_key", lambda _: pytest.fail("cheap path built a board key"))
    monkeypatch.setattr(engine, "_reuse_clock", lambda: 0.0)
    with search_reuse():
        for _ in range(4):
            simulator = engine._Simulator(
                clone_state(state),
                TeamColor.RED,
                HeuristicAgent(),
                owned_hero_ids=frozenset({"hero_min"}),
            )
            simulator._advance_session(None, engine._state_fingerprint(simulator.state))
            assert simulator.state.execution_context["changed"] is True


def test_expensive_cached_transitions_return_independent_state_and_result(monkeypatch):
    from automata.agents.heuristic_agent import HeuristicAgent
    from automata.search.ismcts import engine
    from goa2.domain.models import TeamColor
    from goa2.engine.session import GameSession, SessionResult, SessionResultType

    state = _state()
    calls = []

    def advance(session, response):
        calls.append(1)
        session.state.execution_context["nested"] = [1]
        return SessionResult(
            result_type=SessionResultType.ACTION_COMPLETE, current_phase=session.state.phase
        )

    monkeypatch.setattr(GameSession, "advance", advance)
    ticks = iter(i * 0.01 for i in range(40))
    monkeypatch.setattr(engine, "_reuse_clock", lambda: next(ticks))
    with search_reuse() as cache:
        simulators = []
        results = []
        for _ in range(6):
            simulator = engine._Simulator(
                clone_state(state),
                TeamColor.RED,
                HeuristicAgent(),
                owned_hero_ids=frozenset({"hero_min"}),
            )
            results.append(
                simulator._advance_session(None, engine._state_fingerprint(simulator.state))
            )
            simulators.append(simulator)
        assert len(calls) == 4  # two cost samples, admission, snapshot, then two hits
        assert cache.hits["transitions"] == 2
        simulators[-1].state.execution_context["nested"].append(2)
        assert simulators[-2].state.execution_context["nested"] == [1]
        results[-1].winner = "changed result"
        assert results[-2].winner is None


@pytest.mark.parametrize("custom_mutating_policy,expected_hashes", [(False, 3), (True, 4)])
def test_simulator_reuses_guard_keys_only_across_read_only_policy_calls(
    monkeypatch, custom_mutating_policy, expected_hashes
):
    from unittest.mock import Mock

    from automata.agents.heuristic_agent import HeuristicAgent
    from automata.search.ismcts import engine
    from goa2.domain.input import InputOption, InputRequest, InputRequestType
    from goa2.domain.models import GamePhase, TeamColor
    from goa2.engine.session import GameSession, SessionResult, SessionResultType

    state = _state()
    state.phase = GamePhase.RESOLUTION

    def request(owner):
        return InputRequest(
            request_type=InputRequestType.SELECT_OPTION,
            player_id=owner,
            prompt="Choose",
            options=[InputOption(id="A", text="A"), InputOption(id="B", text="B")],
        )

    foreign, owned = request("hero_gydion"), request("hero_min")
    calls = []

    def advance(session, response):
        calls.append(response)
        if len(calls) == 2:
            session.state.execution_context["progress"] = True
            return SessionResult(
                result_type=SessionResultType.ACTION_COMPLETE, current_phase=state.phase
            )
        return SessionResult(
            result_type=SessionResultType.INPUT_NEEDED,
            current_phase=state.phase,
            input_request=foreign if len(calls) == 1 else owned,
        )

    monkeypatch.setattr(GameSession, "advance", advance)
    original = engine._state_fingerprint
    fingerprint = Mock(wraps=original)
    monkeypatch.setattr(engine, "_state_fingerprint", fingerprint)
    policy = HeuristicAgent()
    if custom_mutating_policy:

        def choose_input(self, world, prompt, **kwargs):
            world.execution_context["policy_changed_state"] = True
            return "A"

        # Even a monkeypatched built-in must use the conservative path.
        monkeypatch.setattr(HeuristicAgent, "choose_input", choose_input)
    simulator = engine._Simulator(
        state, TeamColor.RED, policy, owned_hero_ids=frozenset({"hero_min"})
    )
    decision = simulator.advance()
    assert decision.request is owned
    assert fingerprint.call_count == expected_hashes
    if custom_mutating_policy:
        assert simulator.state.execution_context["policy_changed_state"] is True
