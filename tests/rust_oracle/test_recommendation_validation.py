import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))

from goa2.domain.input import InputRequestType
from goa2.domain.models import GamePhase
from goa2.engine.steps.movement import MinePathChoiceStep
import planning_validation
from planning_validation import validate_commitments
from resolution_validation import validate_resolution


def test_incomplete_planning_commitment_is_rejected_before_submission():
    player = "hero_arien"
    hero = SimpleNamespace(id=player)
    state = SimpleNamespace(phase=GamePhase.PLANNING, get_hero=lambda _: hero)
    envelope = {"state": {"teams": {}}}
    result = {"score": 1, "commitments": [{"player_id": player, "first_card": 0}]}

    with pytest.raises(ValueError, match="malformed planning commitment"):
        validate_commitments(
            state, envelope, result, player_id=player, controlled_ids={player},
            searched_revision=1, live_revision=1,
        )


def test_planning_result_covers_each_controlled_open_slot(monkeypatch):
    player, ally = "hero_arien", "hero_brogan"
    heroes = {hero_id: SimpleNamespace(id=hero_id, team="RED") for hero_id in (player, ally)}
    state = SimpleNamespace(
        phase=GamePhase.PLANNING, get_hero=heroes.get, pending_inputs={},
    )
    envelope = {"state": {"teams": {}}}
    result = {"score": 1, "commitments": [
        {"player_id": player, "first_card": None, "second_card": None},
    ]}
    scope = {
        "player_id": player,
        "controlled_ids": {player, ally},
        "searched_revision": 1,
        "live_revision": 1,
    }
    submissions = []

    def check_sequences(_state, sequences):
        submissions.extend(sequences)
        return SimpleNamespace(results=[SimpleNamespace(legal=True)])

    monkeypatch.setattr(planning_validation, "check_sequences", check_sequences)
    with pytest.raises(ValueError, match="controlled open slots"):
        validate_commitments(state, envelope, result, **scope)
    assert submissions == []

    state.pending_inputs[ally] = None
    moves = validate_commitments(state, envelope, result, **scope)
    assert len(moves) == 1 and moves[0].player_id == player


@pytest.mark.parametrize(
    ("score", "shallow_score", "error"),
    [
        (None, 0, None),
        (None, None, "no completed score"),
        (None, True, "malformed planning result"),
        (1, 0, "malformed planning result"),
    ],
)
def test_completed_shallow_planning_score_is_validated(monkeypatch, score, shallow_score, error):
    player = "hero_arien"
    hero = SimpleNamespace(id=player, team="RED")
    state = SimpleNamespace(
        phase=GamePhase.PLANNING, get_hero=lambda _: hero, pending_inputs={},
    )
    envelope = {"state": {"teams": {}}}
    result = {
        "score": score,
        "shallow_score": shallow_score,
        "commitments": [{"player_id": player, "first_card": None, "second_card": None}],
    }
    scope = {
        "player_id": player,
        "controlled_ids": {player},
        "searched_revision": 1,
        "live_revision": 1,
    }
    seen = []

    def check_sequences(_state, sequences):
        seen.extend(sequences)
        return SimpleNamespace(results=[SimpleNamespace(legal=True)])

    monkeypatch.setattr(planning_validation, "check_sequences", check_sequences)
    if error:
        with pytest.raises(ValueError, match=error):
            validate_commitments(state, envelope, result, **scope)
        assert not seen
    else:
        (move,) = validate_commitments(state, envelope, result, **scope)
        assert move.kind == "PASS" and move.player_id == player
        assert seen == [[move]]


def test_incomplete_resolution_decision_is_rejected_before_submission():
    player = "hero_arien"
    request = SimpleNamespace(id="request-1")
    result = {"score": 1, "decision": {
        "kind": "input", "player_id": player, "request_id": request.id,
    }}

    with pytest.raises(ValueError, match="malformed resolution decision"):
        validate_resolution(
            None, request, result, player_id=player, controlled_ids={player},
            searched_revision=1, live_revision=1,
        )


@pytest.mark.parametrize(
    ("live_revision", "request_id", "message"),
    [
        (2, "request-1", "stale resolution result"),
        (1, "request-2", "does not match the live prompt"),
    ],
)
def test_resolution_rejects_a_changed_revision_or_request(live_revision, request_id, message):
    player = "hero_arien"
    request = SimpleNamespace(id=request_id)
    result = {"score": 1, "decision": {
        "kind": "input", "player_id": player, "request_id": "request-1", "selection": None,
    }}

    with pytest.raises(ValueError, match=message):
        validate_resolution(
            None, request, result, player_id=player, controlled_ids={player},
            searched_revision=1, live_revision=live_revision,
        )


def test_mine_path_resolution_preserves_issued_string_option_id():
    player = "hero_rowenna"
    hero = SimpleNamespace(id=player)
    state = SimpleNamespace(
        get_hero=lambda _: hero,
        execution_stack=[MinePathChoiceStep()],
    )
    request = SimpleNamespace(
        id="request-1",
        player_id=player,
        request_type=InputRequestType.SELECT_OPTION,
        options=[SimpleNamespace(id="0")],
    )
    decision = {
        "kind": "input",
        "player_id": player,
        "request_id": request.id,
        "selection": "0",
    }
    result = {"score": 1, "decision": decision}
    scope = {
        "player_id": player,
        "controlled_ids": {player},
        "searched_revision": 1,
        "live_revision": 1,
    }

    move = validate_resolution(state, request, result, **scope)

    assert move.selection == "0"
    decision["selection"] = 0
    with pytest.raises(ValueError, match="Python rejected the resolution move"):
        validate_resolution(state, request, result, **scope)
