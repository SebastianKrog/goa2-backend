"""Verify a web draft against a real hosted Rust service and disposable database."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
import sqlite3
from pathlib import Path
import tempfile
import time
from threading import Event
from unittest.mock import patch

from fastapi.testclient import TestClient

from goa2.server import bug_reports
from goa2.server.app import create_app
from goa2.server.rust_hosted import RustHostedGames

from verify_rust_game_server import backup_database, hero, packet, request, serve, sockets

HOSTED_TOKEN = "hosted-draft-smoke-token"


def auth(token):
    return {"Authorization": f"Bearer {token}"}


def claim_draft(client, *, lose_reply=False, cheats=False):
    def call(path, body=None, token=None, expect=200):
        response = client.post("/drafts" + path, json=body, headers=auth(token) if token else None)
        assert response.status_code == expect, response.text
        return response.json()

    host = call("", {"host_name": "Alice", "cheats_enabled": cheats}, expect=201)
    draft = "/" + host["draft_id"]
    bob = call(draft + "/join", {"display_name": "Bob"})
    players = {"p1": host["player_token"], bob["player_id"]: bob["player_token"]}
    call(draft + "/team", {"team": "RED"}, players["p1"])
    call(draft + "/team", {"team": "BLUE"}, bob["player_token"])
    running = call(draft + "/start", token=players["p1"])["draft"]
    captains = {p["team"]: p["id"] for p in running["players"] if p["is_captain"]}
    names = iter(client.get("/heroes").json())
    for step in running["sequence"]:
        running = call(draft + "/action", {"hero": next(names)}, players[captains[step["team"]]])["draft"]
    red = running["picks"]["RED"][0]
    blue = running["picks"]["BLUE"][0]
    call(draft + "/claim", {"hero": red}, players["p1"])
    final = {"hero": blue}
    if lose_reply:
        call(draft + "/claim", final, bob["player_token"], expect=503)
        pending = client.get("/drafts" + draft, headers=auth(players["p1"])).json()
        assert pending["draft"]["status"] == "CLAIMING"
        assert pending["game_id"] is None and pending["game_token"] is None
        last_player = next(p for p in pending["draft"]["players"] if p["id"] == bob["player_id"])
        assert last_player["claimed_hero"] is None
    complete = call(draft + "/claim", final, bob["player_token"])
    assert complete["draft"]["status"] == "COMPLETE"
    if lose_reply:
        assert call(draft + "/claim", final, bob["player_token"])["game_token"] == complete["game_token"]
    return {
        name: client.get("/drafts" + draft, headers=auth(token)).json()
        for name, token in {
            "Alice": players["p1"], "Bob": bob["player_token"], "spectator": host["spectator_token"],
        }.items()
    }

def verify_presence(player, spectator, actor, other_actor, card_id, hex_position):
    def send(socket, kind, **fields):
        socket.send(json.dumps({"type": kind, **fields}))

    def quiet(socket):
        try:
            message = socket.recv(timeout=0.12)
        except TimeoutError:
            return
        raise AssertionError(f"Unexpected table message: {message}")

    card_target = {"kind": "CARD", "zone": "CURRENT", "hero_id": actor}
    send(player, "PING", hero_id=other_actor, ping_id="forged",
         target={**card_target, "card_id": card_id, "index": 99})
    ping = packet(player, "PING")
    assert packet(spectator, "PING") == ping
    assert ping["hero_id"] == actor and ping["ping_id"] != "forged"
    assert ping["target"] == card_target

    send(player, "PING", target={"kind": "HEX", "hex": hex_position})
    quiet(spectator)
    time.sleep(0.46)
    send(player, "PING", target={"kind": "HEX", "hex": {**hex_position, "card_id": card_id}})
    ping = packet(player, "PING")
    assert packet(spectator, "PING") == ping
    assert ping["target"] == {"kind": "HEX", "hex": hex_position}
    time.sleep(0.46)
    send(player, "PING", target={"kind": "HEX", "hex": {"q": 2000, "r": -2000, "s": 0}})
    assert packet(player, "ERROR")["detail"] == "PING hex is not on the board"
    send(spectator, "PING", target=card_target)
    assert "Spectators" in packet(spectator, "ERROR")["detail"]

    send(player, "POINTER", target_hero_id=other_actor, zone="HAND", hero_id=other_actor,
         card_id=card_id)
    assert packet(spectator, "POINTER") == {
        "type": "POINTER", "hero_id": actor, "target_hero_id": other_actor, "zone": "HAND",
    }
    quiet(player)
    send(player, "POINTER", x=101, z=0)
    quiet(spectator)
    send(player, "POINTER", x=1.5, z=-0.5)
    assert packet(spectator, "POINTER") == {"type": "POINTER", "hero_id": actor, "x": 1.5, "z": -0.5}
    time.sleep(0.07)
    send(player, "POINTER", hidden=True, x=1000)
    assert packet(spectator, "POINTER") == {"type": "POINTER", "hero_id": actor, "hidden": True}
    send(spectator, "POINTER", hidden=True)
    assert "Spectators" in packet(spectator, "ERROR")["detail"]
    send(player, "GET_VIEW")
    packet(player, "STATE_UPDATE", 1)


def smoke(binary, pack, database, root):
    attempts = []
    real_provision = RustHostedGames._provision

    def lose_first_reply(self, game_id, setup):
        opened = real_provision(self, game_id, setup)
        attempts.append((game_id, setup, opened))
        if len(attempts) == 1:
            app.state.rust_new_drafts = False
            raise TimeoutError("simulate loss after Rust commits creation")
        return opened

    with serve(binary, pack, database, hosted_token=HOSTED_TOKEN) as rust:
        env = {
            "GOA2_RUST_DRAFT_ORIGIN": f"http://{rust}",
            "GOA2_RUST_NEW_DRAFTS": "1",
            "GOA2_GAME_PROVISION_TOKEN": HOSTED_TOKEN,
            "GOA2_SAVE_DIR": str(root / "python-games"),
            "GOA2_REPLAY_DIR": str(root / "python-replays"),
            "GOA2_PREWARM_WORKERS": "0",
        }
        with patch.dict(os.environ, env):
            app = create_app()
            with TestClient(app) as client:
                with patch.object(RustHostedGames, "_provision", lose_first_reply):
                    views = claim_draft(client, lose_reply=True)
                game_id = views["Alice"]["game_id"]
                schema = client.get("/overrides/schema")
                assert schema.status_code == 200
                assert request(rust, "/overrides/schema") == schema.json()
                assert request(rust, f"/games/{game_id}/overrides/schema") == schema.json()
                for suffix in ("", "?include_playtest=true"):
                    for path in ("/heroes", "/heroes/metadata"):
                        expected = client.get(path + suffix)
                        assert expected.status_code == 200
                        assert request(rust, path + suffix) == expected.json()
                maps = client.get("/drafts/maps")
                assert maps.status_code == 200 and maps.json()
                assert request(rust, "/maps") == maps.json()
                assert app.state.registry.all_games() == []
                assert app.state.rust_new_drafts is False
                legacy = claim_draft(client)
                assert not legacy["Alice"]["game_id"].startswith("rust-")
                assert len(app.state.registry.all_games()) == 1
                app.state.rust_new_drafts = True
                cheat_draft = claim_draft(client, cheats=True)
                assert cheat_draft["Alice"]["game_id"].startswith("rust-")
                cheat_game = request(
                    rust, "/games/" + cheat_draft["Alice"]["game_id"], token=cheat_draft["Alice"]["game_token"],
                )
                assert cheat_game["view"]["cheats_enabled"] is True
        assert len(attempts) == 2 and attempts[0] == attempts[1]
        game_id, setup, links = attempts[0]
        assert setup["tie_breaker_team"] in {"RED", "BLUE"}
        assert set(setup["player_names"].values()) == {"Alice", "Bob"}
        game = "/games/" + game_id
        provision = "/__hosted" + game
        request(rust, "/games", body=setup, expect=403)
        request(rust, "/__lab/health", lab=True, expect=404)
        request(rust, provision, body=setup, method="PUT", expect=401)
        request(
            rust, provision, body={**setup, "seed": setup["seed"] ^ 1},
            method="PUT", provision_token=HOSTED_TOKEN, expect=409,
        )
        assert all(view["game_id"] == game_id for view in views.values())
        alice = views["Alice"]["game_token"]
        bob = views["Bob"]["game_token"]
        spectator = views["spectator"]["game_token"]
        assert len({alice, bob, spectator}) == 3
        assert spectator == links.spectator_token
        initial = request(rust, game, token=alice)
        alice_hero = next(seat.hero_id for seat in links.player_tokens if seat.token == alice)
        assert initial["hero_names"][alice_hero] == "Alice"
        assert initial["view"]["tie_breaker_team"] == setup["tie_breaker_team"]
        own_hand = hero(initial, alice_hero)["hand"]
        assert own_hand and not hero(request(rust, game, token=bob), alice_hero)["hand"]
        history = game + "/overrides/history"
        request(rust, history, expect=401)
        assert request(rust, history, token=spectator) == {"total": 0, "decisions": []}
        request(rust, game + "/cards", token=spectator, body={"card_id": own_hand[0]["id"]}, expect=403)
        with sockets(rust, game, alice) as player, sockets(rust, game, spectator) as public:
            packet(player, "STATE_UPDATE", 0)
            packet(public, "STATE_UPDATE", 0)
            player.send(json.dumps({
                "type": "COMMIT_CARD", "card_id": own_hand[0]["id"],
                "client_action_id": "hosted-first-card", "expected_revision": 0,
            }))
            packet(player, "ACTION_RESULT", 1)
            packet(player, "STATE_UPDATE", 1)
            update = packet(public, "STATE_UPDATE", 1)
            assert not hero(update, alice_hero)["hand"]
            bob_hero = next(seat.hero_id for seat in links.player_tokens if seat.token == bob)
            verify_presence(player, public, alice_hero, bob_hero, own_hand[0]["id"],
                            initial["view"]["board"]["entity_locations"][alice_hero])

        private_row = request(rust, history, token=alice)
        assert private_row["total"] == 1
        assert private_row["decisions"] == [{
            "index": 0, "type": "commit", "round": 1, "turn": 1, "hero_id": alice_hero,
            "label": f"Alice committed {own_hand[0]['name']}", "superseded": False,
        }]
        public_row = request(rust, history, token=spectator)
        assert request(rust, history, token=bob) == public_row
        assert public_row["decisions"][0]["label"] == "Alice committed a card"
        for private in (own_hand[0]["id"], own_hand[0]["name"], "hosted-first-card", alice, bob):
            assert private not in json.dumps(public_row)

        # Concurrent retries for a new game commit one credential set.
        concurrent = "/__hosted/games/rust-" + "a" * 32
        def retry():
            return request(
                rust, concurrent, body=setup, method="PUT", provision_token=HOSTED_TOKEN, expect=201,
            )
        with ThreadPoolExecutor(max_workers=2) as executor:
            first, second = list(executor.map(lambda _: retry(), range(2)))
        assert first == second
        request(rust, game, token=first["player_tokens"][0]["token"], expect=401)
        request(rust, history, token=first["player_tokens"][0]["token"], expect=401)

        backup = database.with_name("restored-draft.sqlite")
        backup_database(binary, database, backup)

    with serve(binary, pack, backup, hosted_token=HOSTED_TOKEN) as restarted:
        assert request(restarted, game, token=alice)["revision"] == 1
        assert request(restarted, history, token=alice) == private_row
        assert request(restarted, history, token=spectator) == public_row
        recovered = request(
            restarted, provision, body=setup, method="PUT", provision_token=HOSTED_TOKEN, expect=201,
        )
        assert recovered["revision"] == 0
        assert recovered["spectator_token"] == spectator
        assert {seat["token"] for seat in recovered["player_tokens"]} == {alice, bob}
        assert request(restarted, game, token=alice)["revision"] == 1

        opponent = request(restarted, game, token=bob)
        own_hero = next(seat.hero_id for seat in links.player_tokens if seat.token == bob)
        next_card = hero(opponent, own_hero)["hand"][0]["id"]
        accepted = request(restarted, game + "/cards", token=bob, body={
            "card_id": next_card, "expected_revision": 1,
        })
        assert accepted["revision"] == 2


def verify_reporting(binary, pack, database, root):
    game_id = "rust-" + "b" * 32
    game = "/games/" + game_id
    replay = "/replays/" + game_id
    staff = "test-staff-report-token"
    admin = auth(staff)

    def environment(server):
        return {
            "GOA2_RUST_DRAFT_ORIGIN": f"http://{server}",
            "GOA2_RUST_NEW_DRAFTS": "0",
            "GOA2_GAME_PROVISION_TOKEN": HOSTED_TOKEN,
            "GOA2_ADMIN_TOKEN": staff,
            "GOA2_REPLAY_API": "0",
            "GOA2_BUG_REPORT_DIR": str(root / "staff-reports"),
            "GOA2_SAVE_DIR": str(root / "report-python-games"),
            "GOA2_REPLAY_DIR": str(root / "report-python-replays"),
            "GOA2_PREWARM_WORKERS": "0",
        }

    with serve(binary, pack, database, hosted_token=HOSTED_TOKEN) as server:
        provision = "/__hosted" + game
        opened = request(
            server, provision, method="PUT", provision_token=HOSTED_TOKEN, expect=201,
            body={"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29},
        )
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        player, spectator = seats["hero_arien"], opened["spectator_token"]
        path = game + "/bug-reports"
        request(server, provision + "/report-position", method="POST", token=player, expect=401)
        request(server, provision + "/report-position", method="POST",
                provision_token=HOSTED_TOKEN, expect=401)
        request(server, provision + "/replay/state?decision=0", token=player, expect=401)
        request(server, game + "/cards", token=player, body={"card_id": "aspiring_duelist"})
        with patch.dict(os.environ, environment(server)), patch.object(bug_reports, "MAX_REPORTS_PER_GAME", 2):
            with TestClient(create_app()) as app:
                forged = {
                    "title": "  Planning issue  ", "description": "At the first decision",
                    "reporter_hero": "hero_brogan", "decision_index": 999, "round": 9, "turn": 9,
                }
                assert app.post(path, json=forged).status_code == 401
                assert app.post(path, json=forged, headers=auth("wrong")).status_code == 401
                response = app.post(path, json=forged, headers=auth(player))
                assert response.status_code == 201, response.text
                first = response.json()
                assert first["title"] == "Planning issue" and first["reporter_hero"] == "hero_arien"
                assert (first["decision_index"], first["round"], first["turn"]) == (1, 1, 1)
                assert first["status"] == "open"

                request(server, game + "/uncommit", token=player, body={"expected_revision": 1})
                response = app.post(path, json={"title": "Spectator issue"}, headers=auth(spectator))
                assert response.status_code == 201, response.text
                second = response.json()
                assert second["reporter_hero"] is None and second["decision_index"] == 2
                assert app.post(path, json=forged, headers=auth(player)).status_code == 429

                request(server, game + "/cards", token=player, body={"card_id": "spell_break"})
                assert app.get("/bug-reports", headers=auth(player)).status_code == 401
                assert app.get(replay, headers=auth(player)).status_code == 401
                assert app.get(replay + "/state?decision=1", headers=auth(spectator)).status_code == 401
                listing = app.get("/bug-reports", headers=admin)
                assert listing.status_code == 200
                assert {row["id"] for row in listing.json()} == {first["id"], second["id"]}
                catalogue = app.get("/replays", headers=admin)
                assert catalogue.status_code == 200, catalogue.text
                summary, = catalogue.json()
                assert summary["game_id"] == game_id and summary["num_decisions"] == 3
                assert app.post(replay + "/share", headers=admin).status_code == 409
                meta = app.get(replay, headers=admin)
                assert meta.status_code == 200, meta.text
                assert meta.json()["setup"]["engine"] == "rust"
                assert len(meta.json()["decisions"]) == 3
                state = app.get(replay + "/state?decision=1", headers=admin)
                assert state.status_code == 200, state.text
                frame = state.json()
                assert frame["position"]["decision_index"] == 1
                assert hero(frame, "hero_arien")["current_turn_card"]["id"] == "aspiring_duelist"
                assert hero(frame, "hero_brogan")["hand"]
                assert all(secret not in state.text for secret in [*seats.values(), spectator, HOSTED_TOKEN])
                start = app.get(replay + "/state?round=1&turn=1", headers=admin)
                assert start.json()["position"]["decision_index"] == 0

    # Keep the pinned journal independently of live game retention.
    with sqlite3.connect(database) as db:
        db.execute("PRAGMA foreign_keys = ON")
        db.execute("DELETE FROM games WHERE id=?", (game_id,))
    with serve(binary, pack, database, hosted_token=HOSTED_TOKEN) as server:
        with patch.dict(os.environ, environment(server)):
            with TestClient(create_app()) as app:
                state = app.get(replay + "/state?decision=1", headers=admin)
                assert state.status_code == 200, state.text
                restored = state.json()
                assert restored["view"] == frame["view"]
                assert restored["position"]["decision_index"] == 1
                assert restored["position"]["total_decisions"] == 2
                summary, = app.get("/replays", headers=admin).json()
                assert summary["game_id"] == game_id and summary["num_decisions"] == 2
                resolved = app.patch("/bug-reports/" + first["id"], json={"status": "resolved"}, headers=admin)
                assert resolved.status_code == 200 and resolved.json()["status"] == "resolved"
                assert app.delete("/bug-reports/" + second["id"], headers=admin).status_code == 204
                assert [row["id"] for row in app.get("/bug-reports", headers=admin).json()] == [first["id"]]

def verify_gold_cheats(server, game, player_token, spectator_token):
    target = "hero_brogan"
    endpoint = game + "/cheats/gold"
    grant = {"hero_id": target, "amount": 2, "expected_revision": 0, "client_action_id": "test-grant"}
    start = request(server, game, token=player_token)
    starting_gold = hero(start, target)["gold"]
    assert start["view"]["cheats_enabled"] is True
    assert start["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"
    request(server, endpoint, body=grant, expect=401)
    request(server, endpoint, token=spectator_token, body=grant, expect=403)
    request(server, endpoint, token=player_token, body={**grant, "hero_id": "hero_min"}, expect=404)
    request(server, endpoint, token=player_token, body={**grant, "amount": -1}, expect=400)
    assert request(server, game, token=player_token)["revision"] == 0

    with sockets(server, game, player_token) as player, sockets(server, game, spectator_token) as spectator:
        packet(player, "STATE_UPDATE", 0)
        packet(spectator, "STATE_UPDATE", 0)
        result = request(server, endpoint, token=player_token, body=grant)
        assert result["revision"] == 1 and result["result_type"] == "ACTION_COMPLETE"
        event, = result["events"]
        assert event["actor_id"] == target and event["event_type"] == "GOLD_GAINED"
        assert event["metadata"] == {"amount": 2, "reason": "cheat"}
        assert request(server, endpoint, token=player_token, body=grant) == result
        for socket in (player, spectator):
            update = packet(socket, "STATE_UPDATE", 1)
            assert hero(update, target)["gold"] == starting_gold + 2
            assert update["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"

        player.send(json.dumps({
            "type": "CHEATS_GOLD", "hero_id": target, "amount": 3,
            "expected_revision": 1, "client_action_id": "test-grant-ws",
        }))
        assert packet(player, "ACTION_RESULT", 2)["events"][0]["metadata"]["amount"] == 3
        packet(player, "STATE_UPDATE", 2)
        assert hero(packet(spectator, "STATE_UPDATE", 2), target)["gold"] == starting_gold + 5

    request(server, endpoint, token=player_token, body={
        "hero_id": target, "amount": 2**63 - 1, "expected_revision": 2,
    }, expect=422)
    assert request(server, game, token=player_token)["revision"] == 2
    replay = "/__hosted" + game + "/replay"
    record = request(server, replay, provision_token=HOSTED_TOKEN)
    assert [row["type"] for row in record["decisions"]] == ["cheat_gold", "cheat_gold"]
    assert record["decisions"][1]["sel"] == {"hero_id": target, "amount": 3}
    for index, expected in enumerate([starting_gold, starting_gold + 2, starting_gold + 5]):
        frame = request(server, replay + f"/state?decision={index}", provision_token=HOSTED_TOKEN)
        assert hero(frame, target)["gold"] == expected
    return grant, result, starting_gold + 5


def verify_manual_creation(binary, pack, database, root):
    body = {
        "map_name": "forgotten_island", "red_heroes": ["Arien"], "blue_heroes": ["Brogan"],
        "game_type": "QUICK", "player_names": {"Arien": "  Alice  ", "Brogan": "Bob", "Min": "unused"},
        "time_control": {
            "planning_allowance_seconds": 60, "resolution_allowance_seconds": 30,
            "response_grant_seconds": 2, "initial_time_bank_seconds": 0,
            "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0, "upgrade_allowance_seconds": 15,
        },
    }
    calls = []
    committed, release_reply = Event(), Event()
    provision = RustHostedGames._provision

    def lost_reply(self, game_id, setup):
        result = provision(self, game_id, setup)
        calls.append((game_id, result))
        if len(calls) == 1:
            committed.set()
            assert release_reply.wait(timeout=10)
            raise TimeoutError("lost first creation reply after durable commit")
        return result

    with serve(binary, pack, database, hosted_token=HOSTED_TOKEN) as server:
        env = {
            "GOA2_RUST_DRAFT_ORIGIN": f"http://{server}",
            "GOA2_GAME_PROVISION_TOKEN": HOSTED_TOKEN,
            "GOA2_RUST_NEW_GAMES": "1", "GOA2_RUST_NEW_DRAFTS": "0",
            "GOA2_CREATE_LIMIT_PER_HOUR": "1", "GOA2_PREWARM_WORKERS": "0",
            "GOA2_SAVE_DIR": str(root / "manual-python-games"),
            "GOA2_REPLAY_DIR": str(root / "manual-python-replays"),
            "GOA2_LOG_DIR": str(root / "manual-python-logs"),
        }
        with patch.dict(os.environ, env):
            app = create_app()
            with TestClient(app) as client:
                assert client.post("/games", json={**body, "map_name": "missing"}).status_code == 404
                assert client.post("/games", json={**body, "game_type": "OTHER"}).status_code == 400
                assert client.post("/games", json={**body, "blue_heroes": ["Arien"]}).status_code == 422
                assert client.post("/games", json={**body, "red_heroes": []}).status_code == 422
                with patch.object(RustHostedGames, "_provision", lost_reply), ThreadPoolExecutor() as pool:
                    first = pool.submit(client.post, "/games", json=body)
                    try:
                        assert committed.wait(timeout=10)
                        limited = client.post("/games", json=body)
                        assert limited.status_code == 429 and limited.headers["Retry-After"]
                        assert client.post("/drafts", json={"host_name": "Alice"}).status_code == 429
                    finally:
                        release_reply.set()
                    response = first.result()
                assert response.status_code == 201, response.text
                assert len(calls) == 2 and calls[0] == calls[1]
                assert app.state.registry.all_games() == []
                created = response.json()
                assert created["game_id"].startswith("rust-") and created["game_id"] == calls[0][0]
                game = "/games/" + created["game_id"]
                tokens = {seat["hero_id"]: seat["token"] for seat in created["player_tokens"]}
                assert set(tokens) == {"hero_arien", "hero_brogan"}
                assert len({*tokens.values(), created["spectator_token"]}) == 3
                with sockets(server, game, tokens["hero_arien"]) as player:
                    update = packet(player, "STATE_UPDATE", 0)
                    assert update["hero_names"] == {"hero_arien": "Alice", "hero_brogan": "Bob"}
                    assert update["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"
                    assert not hero(update, "hero_brogan")["hand"]
                replay = request(server, "/__hosted" + game + "/replay", provision_token=HOSTED_TOKEN)
                assert replay["setup"]["game_type"] == "QUICK"
                assert not hero(request(server, game, token=created["spectator_token"]), "hero_arien")["hand"]
                request(server, game + "/cheats/gold", token=tokens["hero_arien"],
                        body={"hero_id": "hero_brogan", "amount": 1}, expect=403)

            with TestClient(create_app(), client=("203.0.113.2", 50000)) as client:
                bot = {"hero_brogan": {"kind": "rust"}}
                assert client.post("/games", json={
                    **body, "bots": {**bot, "hero_arien": {"kind": "random"}},
                }).status_code == 422
                assert client.post("/games", json={
                    **body, "bots": {"hero_min": {"kind": "rust"}},
                }).status_code == 422
                response = client.post("/games", json={**body, "bots": bot})
                assert response.status_code == 201, response.text
                created = response.json()
                token = next(seat["token"] for seat in created["player_tokens"] if seat["hero_id"] == "hero_brogan")
                request(server, f"/games/{created['game_id']}/ready", token=token, body={"ready": True}, expect=403)

            with TestClient(create_app(), client=("203.0.113.3", 50000)) as client:
                response = client.post("/games", json={**body, "cheats_enabled": True})
                assert response.status_code == 201, response.text
                created = response.json()
                assert created["game_id"].startswith("rust-")
                game = "/games/" + created["game_id"]
                token = next(seat["token"] for seat in created["player_tokens"] if seat["hero_id"] == "hero_arien")
                grant, receipt, final_gold = verify_gold_cheats(server, game, token, created["spectator_token"])

        with sqlite3.connect(database) as db:
            assert db.execute("SELECT count(*) FROM games").fetchone() == (3,)

    with serve(binary, pack, database, hosted_token=HOSTED_TOKEN) as restarted:
        assert request(restarted, game + "/cheats/gold", token=token, body=grant) == receipt
        current = request(restarted, game, token=token)
        assert current["revision"] == 2 and hero(current, "hero_brogan")["gold"] == final_gold


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("content", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="goa2-hosted-draft-") as path:
        root = Path(path)
        smoke(args.binary.resolve(), args.content.resolve(), root / "rust.sqlite", root)
        verify_reporting(args.binary.resolve(), args.content.resolve(), root / "reports.sqlite", root)
        verify_manual_creation(args.binary.resolve(), args.content.resolve(), root / "manual.sqlite", root)
    print("Hosted creation, draft, presence and reports passed provisioning, scoped auth, exact replay, triage and restart")


if __name__ == "__main__":
    main()
