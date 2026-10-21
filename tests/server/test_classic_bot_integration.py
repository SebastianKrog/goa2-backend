from __future__ import annotations

import json
import time

import pytest
from fastapi.testclient import TestClient

from goa2.server.app import create_app


@pytest.mark.parametrize("kind", ["random", "heuristic"])
def test_rest_creates_and_persists_a_classic_bot(tmp_path, monkeypatch, kind) -> None:
    monkeypatch.setenv("GOA2_SAVE_DIR", str(tmp_path))
    with TestClient(create_app()) as client:
        response = client.post(
            "/games",
            json={
                "map_name": "forgotten_island",
                "red_heroes": ["Arien"],
                "blue_heroes": ["Wasp"],
                "bots": {"hero_wasp": {"kind": kind}},
            },
        )
        assert response.status_code == 201, response.text
        game_id = response.json()["game_id"]
        game = client.app.state.registry.get(game_id)
        assert game.bot_specs["hero_wasp"].kind == kind
        deadline = time.monotonic() + 5
        while "hero_wasp" not in game.session.state.pending_inputs:
            assert time.monotonic() < deadline, "Bot did not commit its planning card"
            time.sleep(0.01)
        assert "hero_arien" not in game.session.state.pending_inputs

    saved = json.loads((tmp_path / f"{game_id}.json").read_text())
    assert saved["bot_specs"] == {"hero_wasp": {"kind": kind, "search": None}}

    with TestClient(create_app()) as restored_client:
        restored = restored_client.app.state.registry.get(game_id)
        assert restored.bot_specs["hero_wasp"].kind == kind


def test_rest_rejects_unknown_bot_hero() -> None:
    with TestClient(create_app()) as client:
        response = client.post(
            "/games",
            json={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Wasp"],
                "bots": {"hero_brogan": {"kind": "random"}},
            },
        )
    assert response.status_code == 400
    assert "not in the game roster" in response.json()["detail"]


def test_search_settings_are_bounded_at_request_validation() -> None:
    with TestClient(create_app()) as client:
        response = client.post(
            "/games",
            json={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Wasp"],
                "bots": {
                    "hero_wasp": {
                        "kind": "ismcts",
                        "search": {"iterations": 1001, "decision_timeout_seconds": 2},
                    }
                },
            },
        )
    assert response.status_code == 422
