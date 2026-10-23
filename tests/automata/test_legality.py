"""Batch legality must preserve real engine acceptance and prefix state changes."""

from dataclasses import replace

import pytest

from automata.runtime.clone import clone_state
from automata.runtime.effects import register_all_effects
from automata.runtime.legality import DecisionMove, check_moves, check_sequences
from goa2.domain.input import selection_value
from goa2.domain.models import GamePhase, TargetType
from goa2.engine.session import GameSession
from goa2.engine.setup import GameSetup
from goa2.engine.steps.selection import SelectStep


def _state():
    register_all_effects()
    return GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json",
        ["Arien", "Hanu"],
        ["Brogan", "Tigerclaw"],
        seed=9,
    )


def _numbers():
    state = _state()
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_arien"
    state.execution_stack = [
        SelectStep(
            target_type=TargetType.NUMBER,
            prompt="Second",
            number_options=[3, 4],
            output_key="second",
        ),
        SelectStep(
            target_type=TargetType.NUMBER, prompt="First", number_options=[1, 2], output_key="first"
        ),
    ]
    return state


def _real_trace(state, count=16):
    from goa2.domain.input import InputResponse

    session = GameSession(clone_state(state), create_rollback_snapshots=False)
    trace = []
    result = None
    for _ in range(count):
        if session.state.phase is GamePhase.PLANNING:
            hero = next(
                h
                for team in session.state.teams.values()
                for h in team.heroes
                if h.id not in session.state.pending_inputs
            )
            if hero.hand:
                card = hero.hand[0]
                trace.append(DecisionMove("CARD", hero.id, card.id))
                result = session.commit_card(hero.id, card)
            else:
                trace.append(DecisionMove("PASS", hero.id))
                result = session.pass_turn(hero.id)
        else:
            if result is None or result.input_request is None:
                result = session.advance()
            if result.input_request is None:
                break
            request = result.input_request
            assert request.options
            value = selection_value(request.options[0])
            trace.append(DecisionMove("INPUT", request.player_id, value))
            result = session.advance(InputResponse(request_id=request.id, selection=value))
    return trace


def test_batch_input_checks_index_once_without_applying_or_mutating_the_source():
    state = _numbers()
    before = state.model_dump(mode="json")
    moves = [DecisionMove("INPUT", "hero_arien", value) for value in [1, "2", 3, None, "wrong"]]
    batch = check_moves(state, moves * 20)
    assert [r.legal for r in batch.results] == [True, True, False, False, False] * 20
    assert batch.applied_decisions == 0
    assert batch.state_clones == 1
    assert state.model_dump(mode="json") == before


def test_sequence_choices_are_checked_against_the_new_prompt_not_the_root_prompt():
    state = _numbers()
    first = DecisionMove("INPUT", "hero_arien", 1)
    batch = check_sequences(
        state,
        [
            [],
            [first],
            [first, replace(first, selection=3)],
            [first, first],
            [replace(first, player_id="hero_hanu")],
        ],
    )
    assert [r.legal for r in batch.results] == [True, True, True, False, False]
    assert batch.results[3].failed_at == 1
    assert batch.results[3].checked == 1
    assert batch.results[4].failed_at == 0


def test_repeated_and_branching_sequences_share_prefixes_and_keep_siblings_independent():
    state = _numbers()
    before = state.model_dump(mode="json")
    a = DecisionMove("INPUT", "hero_arien", 1)
    b = replace(a, selection=3)
    batch = check_sequences(state, [[a, b], [a, replace(b, selection=4)], [a, b], [a, b, b]])
    assert [r.legal for r in batch.results] == [True, True, True, False]
    assert batch.applied_decisions == 2  # only prefixes with further decisions need execution
    assert batch.state_clones == 1
    assert state.model_dump(mode="json") == before


def test_explicit_stale_request_ids_are_rejected_but_hypothetical_ids_can_be_omitted():
    state = _numbers()
    move = DecisionMove("INPUT", "hero_arien", 1)
    assert check_moves(state, [move]).results[0].legal
    assert not check_moves(state, [replace(move, request_id="stale")]).results[0].legal
    assert (
        not check_sequences(state, [[move, replace(move, selection=3, request_id="stale")]])
        .results[0]
        .legal
    )


def test_planning_rejects_wrong_players_cards_and_repeated_commits():
    state = _state()
    hero = state.get_hero("hero_arien")
    move = DecisionMove("CARD", hero.id, hero.hand[0].id)
    invalid = [
        replace(move, player_id="hero_hanu"),
        replace(move, selection="absent"),
        replace(move, kind="PASS", selection=None),
        replace(move, player_id="absent"),
    ]
    batch = check_moves(state, [move, *invalid])
    assert [r.legal for r in batch.results] == [True, False, False, False, False]
    result = check_sequences(state, [[move, move]]).results[0]
    assert not result.legal and result.failed_at == 1


def test_real_game_decision_trace_matches_individual_validation_and_keeps_original_unchanged():
    state = _state()
    trace = _real_trace(state)
    assert len(trace) >= 10
    before = state.model_dump(mode="json")
    sequences = [trace[:length] for length in range(1, len(trace) + 1)]
    combined = check_sequences(state, sequences)
    assert all(result.legal for result in combined.results)
    assert combined.applied_decisions == len(trace) - (trace[-1].kind == "INPUT")
    assert combined.state_clones == 1
    for sequence in sequences:
        assert check_sequences(state, [sequence]).results[0].legal
    assert state.model_dump(mode="json") == before


def test_no_legality_result_survives_a_source_state_change():
    state = _numbers()
    move = DecisionMove("INPUT", "hero_arien", 1)
    assert check_moves(state, [move]).results[0].legal
    state.execution_stack[-1].number_options = [9]
    assert not check_moves(state, [move]).results[0].legal


def test_branching_nonterminal_prefixes_fork_before_mutation():
    state = _numbers()
    a = DecisionMove("INPUT", "hero_arien", 1)
    b = replace(a, selection=3)
    sequences = [[a, b, b], [replace(a, selection=2), replace(b, selection=4), b]]
    batch = check_sequences(state, sequences)
    assert [r.legal for r in batch.results] == [False, False]
    assert [r.failed_at for r in batch.results] == [2, 2]
    assert batch.applied_decisions == 4
    assert batch.state_clones == 2


def test_structured_rune_batches_use_engine_validation_not_scalar_option_membership():
    from goa2.domain.models import RuneType
    from goa2.engine.steps.markers import PlaceRunesStep

    state = _numbers()
    state.execution_stack = [PlaceRunesStep(hero_id="hero_arien")]
    values = [rune.value for rune in RuneType]
    good = DecisionMove("INPUT", "hero_arien", values[:3])
    bad = replace(good, selection=[values[0], values[0], values[1]])
    assert [r.legal for r in check_moves(state, [good, bad]).results] == [True, False]
    assert check_sequences(state, [[good]]).results[0].legal
    assert state.get_hero("hero_arien").rune_slots == {}


def test_simultaneous_upgrades_validate_payload_and_player_ownership():
    from goa2.engine.steps.cards import ResolveUpgradesStep

    state = _state()
    state.phase = GamePhase.LEVEL_UP
    state.pending_upgrades = {"hero_arien": 1}
    state.execution_stack = [ResolveUpgradesStep()]
    choices = ResolveUpgradesStep()._get_upgrade_options(state, "hero_arien")
    assert choices
    card_id = choices[0]["pair"][0]
    move = DecisionMove("INPUT", "hero_arien", {"hero_id": "hero_arien", "card_id": card_id})
    batch = check_moves(
        state,
        [
            move,
            replace(move, player_id="hero_hanu"),
            replace(move, selection={"hero_id": "hero_arien", "card_id": "missing"}),
        ],
    )
    assert [r.legal for r in batch.results] == [True, False, False]
    assert check_sequences(state, [[move]]).results[0].legal
    assert (
        check_sequences(state, [[move, DecisionMove("CARD", "hero_arien", card_id)]])
        .results[0]
        .legal
    )


def test_two_card_hero_and_finish_planning_use_actual_planning_rules():
    state = GameSetup.create_game("src/goa2/data/maps/forgotten_island.json", ["Emmitt"], ["Wasp"])
    hero = state.get_hero("hero_emmitt")
    hero.level = 8
    a, b, c = [DecisionMove("CARD", hero.id, card.id) for card in hero.hand[:3]]
    done = DecisionMove("FINISH_PLANNING", hero.id)
    batch = check_sequences(state, [[a, b], [a, b, c], [a, done], [a, done, b], [done]])
    assert [r.legal for r in batch.results] == [True, False, True, False, False]
    assert batch.results[1].failed_at == 2
    assert batch.results[3].failed_at == 2
    assert not state.pending_inputs


def test_sequence_movement_checks_use_positions_after_each_actual_move():
    from goa2.engine.filters_hex import MovementPathFilter, ObstacleFilter
    from goa2.engine.steps.movement import MoveUnitStep

    state = _numbers()
    start = state.get_position("hero_arien")

    def empty(h):
        return h in state.board.tiles and not state.board.tiles[h].is_obstacle

    first, second = next(
        (a, b)
        for a in start.neighbors()
        if empty(a)
        for b in a.neighbors()
        if empty(b) and start.distance(b) == 2
    )
    # Share the actual filter object deliberately: prepared results must belong
    # to a single unchanged scan, not persist across the intervening move.
    path = MovementPathFilter(range_val=1, unit_id="hero_arien")

    def selector(prompt):
        return SelectStep(
            target_type=TargetType.HEX,
            prompt=prompt,
            output_key="target_hex",
            filters=[path, ObstacleFilter()],
        )

    state.execution_stack = [
        selector("Second move"),
        MoveUnitStep(unit_id="hero_arien"),
        selector("First move"),
    ]
    a = DecisionMove("INPUT", "hero_arien", first.model_dump())
    b = replace(a, selection=second.model_dump())
    assert not check_moves(state, [b]).results[0].legal
    assert check_sequences(state, [[a, b]]).results[0].legal
    assert state.get_position("hero_arien") == start


def test_movement_filter_direct_calls_and_preparation_do_not_reuse_stale_ranges():
    from goa2.engine.filters_hex import MovementPathFilter

    state = _numbers()
    start = state.get_position("hero_arien")
    target = next(
        h
        for h in start.neighbors()
        if h in state.board.tiles and not state.board.tiles[h].is_obstacle
    )
    path = MovementPathFilter(range_val=1, unit_id="hero_arien")
    assert path.apply(target, state, {})
    assert path.prepare(state, {})(target)
    path.range_val = 0
    assert not path.apply(target, state, {})
    assert not path.prepare(state, {})(target)


def test_unenumerated_input_without_a_hook_is_unknown_not_silently_legal(monkeypatch):
    original = SelectStep.resolve

    def without_options(step, state, context):
        result = original(step, state, context)
        if result.input_request is not None:
            result.input_request.options = []
        return result

    monkeypatch.setattr(SelectStep, "resolve", without_options)
    state = _numbers()
    move = DecisionMove("INPUT", "hero_arien", 1)
    assert check_moves(state, [move]).results[0].legal is None
    assert check_sequences(state, [[move]]).results[0].legal is None


def test_unexpected_engine_errors_are_not_converted_to_unknown_or_illegal(monkeypatch):
    original = SelectStep.resolve

    def broken_on_submission(step, state, context):
        if step.pending_input:
            raise NotImplementedError("broken engine step")
        return original(step, state, context)

    monkeypatch.setattr(SelectStep, "resolve", broken_on_submission)
    state = _numbers()
    first = DecisionMove("INPUT", "hero_arien", 1)
    with pytest.raises(NotImplementedError, match="broken engine step"):
        check_sequences(state, [[first, replace(first, selection=3)]])
