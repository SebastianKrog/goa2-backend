"""M6 previews use live engine legality without consuming a game decision."""

import pytest
from fastapi.testclient import TestClient

from goa2.server.app import create_app
from tests.server.test_batch_lifecycle import _game, _view


@pytest.fixture
def client(tmp_path, monkeypatch):
    for key in ["SAVE", "REPLAY", "LOG"]:
        monkeypatch.setenv(f"GOA2_{key}_DIR", str(tmp_path / key.lower()))
    with TestClient(create_app()) as client:
        yield client


def _preview(client, game, token, request, ids):
    return client.post(
        f"/games/{game.game_id}/upgrades/preview",
        headers={"Authorization": f"Bearer {token}"},
        json={"request_id": request["request_id"], "card_ids": ids},
    )


def test_preview_prefix_and_full_batch_leave_live_game_request_history_and_save_unchanged(client):
    game, hero, tokens = _game(client, "upgrades")
    request = _view(client, game, tokens[hero])["input_request"]
    assert request["supports_preview"] is True
    ids = [o["pair"][0] for o in request["players"][hero]["options"]]
    client.app.state.registry.save_game(game.game_id)
    from pathlib import Path

    path = Path(client.app.state.registry._save_dir) / f"{game.game_id}.json"
    saved = path.read_bytes()
    state = game.session.state.model_dump(mode="json")
    records = list(game.replay_recorder.records)
    for prefix in [[], ids[:1], ids[:2], ids]:
        response = _preview(client, game, tokens[hero], request, prefix)
        assert response.status_code == 200, response.text
        preview = response.json()
        assert preview["card_ids"] == prefix
        assert preview["remaining"] == 3 - len(prefix)
        assert preview["request_id"] == request["request_id"]
        own = next(
            h for t in preview["view"]["teams"].values() for h in t["heroes"] if h["id"] == hero
        )
        assert set(prefix).issubset({c["id"] for c in own["hand"]})
        opponent = preview["view"]["teams"]["BLUE"]["heroes"][0]
        assert isinstance(opponent["deck"], dict)
        assert game.session.state.model_dump(mode="json") == state
        assert _view(client, game, tokens[hero])["input_request"] == request
        assert game.replay_recorder.records == records
        assert path.read_bytes() == saved


def test_preview_uses_engine_tier_progression_and_allows_prefix_that_is_not_a_commit(client):
    game, hero, tokens = _game(client, "upgrades")
    game.session.state.pending_upgrades[hero] = 4
    game.last_result = game.session.advance()
    request = _view(client, game, tokens[hero])["input_request"]
    ids = [o["pair"][0] for o in request["players"][hero]["options"]]
    response = _preview(client, game, tokens[hero], request, ids)
    assert response.status_code == 200
    assert response.json()["remaining"] == 1
    assert all(o["tier"] == "III" for o in response.json()["options"])
    next_id = response.json()["options"][0]["pair"][0]
    assert _preview(client, game, tokens[hero], request, [next_id]).status_code == 400
    assert _preview(client, game, tokens[hero], request, [*ids, next_id]).status_code == 200
    # Three of four is only a preview prefix, never a legal mutation payload.
    response = client.post(
        f"/games/{game.game_id}/input",
        headers={"Authorization": f"Bearer {tokens[hero]}"},
        json={"request_id": request["request_id"], "selection": {"hero_id": hero, "card_ids": ids}},
    )
    assert response.status_code == 400
    assert game.session.state.pending_upgrades[hero] == 4


def test_preview_is_correlated_scoped_and_rejects_invalid_prefixes(client):
    game, hero, tokens = _game(client, "upgrades")
    request = _view(client, game, tokens[hero])["input_request"]
    card = request["players"][hero]["options"][0]["pair"][0]
    before = game.session.state.model_dump(mode="json")
    for ids in [["unknown"], [card, card], [card] * 4]:
        assert _preview(client, game, tokens[hero], request, ids).status_code == 400
    assert _preview(client, game, tokens[hero], {"request_id": "old"}, []).status_code == 409
    assert _preview(client, game, tokens["hero_wasp"], request, [card]).status_code == 400
    assert _preview(client, game, "invalid-token", request, []).status_code == 401
    assert _preview(client, game, game.spectator_token, request, []).status_code == 403
    assert _preview(client, game, tokens[hero], request, [card] * 7).status_code == 422
    game.session.state.pending_upgrades.pop("hero_wasp")
    assert _preview(client, game, tokens["hero_wasp"], request, []).status_code == 400
    game.session.state.pending_upgrades["hero_wasp"] = 3
    assert game.session.state.model_dump(mode="json") == before


def test_rune_request_exposes_acknowledged_slots_after_reload(client):
    game, hero, tokens = _game(client, "runes")
    request = _view(client, game, tokens[hero])["input_request"]
    assert request["placed_runes"] == {}
    response = client.post(
        f"/games/{game.game_id}/input",
        headers={"Authorization": f"Bearer {tokens[hero]}"},
        json={"request_id": request["request_id"], "selection": "axe"},
    )
    assert response.status_code == 200
    fresh = _view(client, game, tokens[hero])["input_request"]
    assert fresh["placed_runes"] == {"1": "axe"}
    from goa2.domain.state import GameState
    from goa2.engine.session import GameSession

    resumed = GameSession(
        GameState.model_validate_json(game.session.state.model_dump_json())
    ).advance()
    assert resumed.input_request.to_dict()["placed_runes"] == {"1": "axe"}
