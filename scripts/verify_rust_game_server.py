"""Run the Rust service against a temporary database and exercise the frontend protocol."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import json
import os
import signal
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from websockets.exceptions import ConnectionClosed
from websockets.sync.client import connect


LAB_TOKEN = "smoke-test-lab-token"


@contextmanager
def serve(binary, pack, database, *, override_timeout=120, hosted_token=None, process_handle=False):
    process = subprocess.Popen(
        [binary],
        env={
            **os.environ,
            "GOA2_GAME_CONTENT": str(pack),
            "GOA2_GAME_DATABASE": str(database),
            "GOA2_GAME_BIND": "127.0.0.1:0",
            "GOA2_LAB_TOKEN": "" if hosted_token else LAB_TOKEN,
            "GOA2_GAME_PROVISION_TOKEN": hosted_token or "",
            "GOA2_OVERRIDE_TIMEOUT_SECONDS": str(override_timeout),
        },
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        ready = process.stderr.readline()
        prefix = "goa2-game-server listening on "
        assert ready.startswith(prefix), ready
        address = ready.removeprefix(prefix).strip()
        yield (address, process) if process_handle else address
    finally:
        process.kill()
        process.wait(timeout=10)
        process.stderr.close()


def backup_database(binary, database, destination):
    env = {**os.environ, "GOA2_GAME_DATABASE": str(database)}
    env.pop("GOA2_GAME_CONTENT", None)
    subprocess.run([binary, "--backup", destination], env=env, check=True, timeout=30)


def request(
    server, path, *, token=None, lab=False, body=None, expect=200, method=None, provision_token=None, timeout=10
):
    headers = {"Content-Type": "application/json"} if body is not None else {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if lab:
        headers["X-Lab-Token"] = LAB_TOKEN
    if provision_token is not None:
        headers["X-Game-Provision-Token"] = provision_token
    req = Request(
        f"http://{server}{path}",
        data=json.dumps(body).encode() if body is not None else None,
        headers=headers,
        method=method,
    )
    try:
        response = urlopen(req, timeout=timeout)
    except HTTPError as error:
        response = error
    with response:
        result = json.load(response)
        statuses = expect if isinstance(expect, tuple) else (expect,)
        assert response.status in statuses, (path, response.status, result.get("detail"))
        return result


def packet(socket, kind, revision=None):
    value = json.loads(socket.recv(timeout=10))
    assert value["type"] == kind, value["type"]
    if revision is not None:
        assert value["revision"] == revision, value["revision"]
    return value


def sockets(server, game, token):
    return connect(f"ws://{server}{game}/ws?token={token}", open_timeout=10, close_timeout=1)


def hero(response, hero_id):
    return next(
        hero for team in response["view"]["teams"].values()
        for hero in team["heroes"] if hero["id"] == hero_id
    )


def verify_replay(server, game, *, credentials=(), operations=(), tamper=False):
    lab_game = "/__lab" + game
    exported = request(server, lab_game + "/replay", lab=True)
    archive_text = exported["replay_json"]
    assert all(token not in archive_text for token in credentials)
    archive = json.loads(archive_text)
    assert archive["kind"] == "rust-live-replay"
    start = archive["start_revision"]
    end = archive["revision"]
    assert (start, end) == (exported["start_revision"], exported["revision"])
    assert [entry["revision"] for entry in archive["entries"]] == list(range(start + 1, end + 1))
    kinds = {entry["decision"]["operation"]["kind"] for entry in archive["entries"]}
    assert set(operations) <= kinds, (operations, kinds)
    latest = request(server, "/__lab/replay", lab=True, body={"replay_json": archive_text})
    current = request(server, lab_game + "/checkpoint", lab=True)
    assert latest["revision"] == end
    assert json.loads(latest["checkpoint"]["native_checkpoint_json"]) == current["checkpoint"]
    opening = request(server, "/__lab/replay", lab=True, body={
        "replay_json": archive_text, "revision": start,
    })
    assert opening["revision"] == start
    assert json.loads(opening["checkpoint"]["native_checkpoint_json"]) == archive["start"]
    if tamper:
        request(server, lab_game + "/replay", token=credentials[0], expect=401)
        request(server, "/__lab/replay", body={"replay_json": archive_text}, expect=401)
        request(server, "/__lab/replay", lab=True, body={
            "replay_json": archive_text, "revision": end + 1,
        }, expect=422)
        assert "client_action_id" not in archive_text
        archive["entries"][-1]["checkpoint_sha256"] = "0" * 64
        invalid = request(server, "/__lab/replay", lab=True, body={
            "replay_json": json.dumps(archive),
        }, expect=422)
        assert str(end) in invalid["detail"] and "reconstructed checkpoint differs" in invalid["detail"], invalid
        archive["entries"].pop()
        invalid = request(server, "/__lab/replay", lab=True, body={
            "replay_json": json.dumps(archive),
        }, expect=422)
        assert "not contiguous" in invalid["detail"]
    return archive

def verify_lab_gold_cheat(server):
    opened = request(server, "/games", body={
        "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "cheats_enabled": True, "seed": 29,
    }, expect=201)
    game = "/games/" + opened["game_id"]
    lab_game = "/__lab" + game
    seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
    actor, target = "hero_arien", "hero_brogan"
    current = request(server, game, token=seats[actor])
    gold = hero(current, target)["gold"]
    command = {
        "actor": actor, "expected_revision": 0,
        "message": {"type": "CHEATS_GOLD", "hero_id": target, "amount": 7},
    }
    preview = request(server, lab_game + "/comparison", lab=True, body=command)
    assert preview["revision"] == 0
    assert hero(preview["views"]["spectator"], target)["gold"] == gold + 7
    assert hero(request(server, game, token=seats[actor]), target)["gold"] == gold
    assert request(server, lab_game + "/command", lab=True, body=command)["revision"] == 1
    for revision, (seat, card) in enumerate([(actor, "aspiring_duelist"), (target, "mad_dash")], 1):
        request(server, game + "/cards", token=seats[seat],
                body={"card_id": card, "expected_revision": revision})
    current = request(server, game, token=seats[actor])
    assert current["view"]["phase"] == "RESOLUTION"
    request(server, game + "/cheats/gold", token=seats[actor], body={
        "hero_id": target, "amount": 7, "expected_revision": 3,
    }, expect=422)
    assert request(server, game, token=seats[actor])["revision"] == 3


def smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        identity = json.loads(subprocess.check_output([binary, "--check-content", pack], text=True))
        assert identity["engine"] == "rust"
        assert request(server, "/health") == {"ok": True, **identity}
        request(server, "/__lab/health", expect=401)
        assert "Arien" in request(server, "/heroes")
        verify_lab_gold_cheat(server)
        setup = {
            "red_heroes": ["Arien", "Brogan"], "blue_heroes": ["Tigerclaw", "Sabina"],
            "seed": 29, "player_names": {"Arien": "  Lab Arien  "},
        }
        opened = request(server, "/games", body=setup, expect=201)
        game = f"/games/{opened['game_id']}"
        lab_game = "/__lab" + game
        assert request(server, game + "/health") == {"ok": True, **identity}
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        arien = seats["hero_arien"]
        spectator = opened["spectator_token"]
        request(server, game, expect=401)
        request(server, lab_game + "/checkpoint", token=spectator, expect=401)
        request(server, game + "/cards", token=spectator, body={"card_id": "aspiring_duelist"}, expect=403)
        request(server, game + "/advance", token=spectator, method="POST", expect=403)
        request(server, game + "/advance", token=arien, method="POST", expect=400)

        first = {"card_id": "aspiring_duelist", "expected_revision": 0, "client_action_id": "arien-commit"}
        with sockets(server, game, arien) as player, sockets(server, game, spectator) as public:
            player_start = packet(player, "STATE_UPDATE", 0)
            assert player_start["hero_names"]["hero_arien"] == "Lab Arien"
            assert hero(player_start, "hero_arien")["hand"]
            assert not hero(packet(public, "STATE_UPDATE", 0), "hero_arien")["hand"]
            player.send(json.dumps({"type": "COMMIT_CARD", **first}))
            assert packet(player, "ACTION_RESULT", 1)["client_action_id"] == "arien-commit"
            own_update = packet(player, "STATE_UPDATE", 1)
            other_update = packet(public, "STATE_UPDATE", 1)
            assert own_update["client_action_id"] == "arien-commit"
            assert "client_action_id" not in other_update
            assert hero(own_update, "hero_arien")["current_turn_card"]["id"] == "aspiring_duelist"
            hidden = hero(other_update, "hero_arien")["current_turn_card"]
            assert hidden["is_facedown"] and "id" not in hidden

        original = request(server, game + "/cards", token=arien, body=first)
        assert original["revision"] == 1
        request(server, game + "/cards", token=arien, body={**first, "card_id": "spell_break"}, expect=409)
        request(server, game + "/uncommit", token=arien, body={"expected_revision": 0}, expect=409)

        with sqlite3.connect(database) as db:
            db.execute("CREATE TRIGGER reject_update BEFORE INSERT ON replay_entries "
                       "BEGIN SELECT RAISE(ABORT, 'injected storage error'); END")
        uncommit = {"expected_revision": 1, "client_action_id": "uncommit-after-failure"}
        request(server, game + "/uncommit", token=arien, body=uncommit, expect=500)
        assert request(server, game, token=arien)["revision"] == 1
        history = game + "/overrides/history"
        assert request(server, history, token=spectator)["total"] == 1
        with sqlite3.connect(database) as db:
            db.execute("DROP TRIGGER reject_update")
        assert request(server, game + "/uncommit", token=arien, body=uncommit)["revision"] == 2
        uncommitted = request(server, history, token=spectator)
        assert uncommitted["total"] == 2
        assert [row["type"] for row in uncommitted["decisions"]] == ["commit", "uncommit"]
        assert uncommitted["decisions"][0]["label"] == "Lab Arien committed a card"

        for revision, (actor, card) in enumerate([
            ("hero_arien", "aspiring_duelist"), ("hero_brogan", "mad_dash"),
            ("hero_tigerclaw", "hit_and_run"), ("hero_sabina", "listen_up"),
        ], start=2):
            last = request(server, game + "/cards", token=seats[actor],
                           body={"card_id": card, "expected_revision": revision})
        assert last["revision"] == 6
        revealed = request(server, history, token=spectator)["decisions"]
        assert len(revealed) == 6 and [row["index"] for row in revealed] == list(range(6))
        assert revealed[2]["label"] == "Lab Arien committed Aspiring Duelist"
        responder, = request(server, lab_game + "/responders", lab=True)["players"]
        pending = request(server, game, token=seats[responder])
        assert pending["input_request"]["type"] == "CHOOSE_ACTION"
        assert pending["input_request"]["can_rollback"] is True
        assert request(server, game, token=spectator)["input_request"] is None
        boundary = request(server, game + "/advance", token=seats[responder], method="POST")
        assert boundary["revision"] == 6 and boundary["events"] == []
        assert boundary["result_type"] == "INPUT_NEEDED"
        assert boundary["input_request"] == pending["input_request"]
        other = next(hero for hero in seats if hero != responder)
        scoped = request(server, game + "/advance", token=seats[other], method="POST")
        assert scoped["input_request"] is None and scoped["awaiting_input"] == [responder]
        captured = request(server, lab_game + "/checkpoint", lab=True)
        assert captured["revision"] == 6
        assert not {"player_tokens", "spectator_token"} & captured.keys()
        request(server, lab_game + "/snapshot", expect=401)
        snapshot = request(server, lab_game + "/snapshot", lab=True)["checkpoint"]
        assert snapshot["engine"] == "rust"
        assert snapshot["summary"]["decision"] == "CHOOSE_ACTION"
        assert snapshot["summary"]["responder"] == responder
        assert json.loads(snapshot["native_checkpoint_json"]) == captured["checkpoint"]
        assert {hero["id"] for team in snapshot["state"]["teams"].values()
                for hero in team["heroes"]} == seats.keys()
        assert request(server, "/__lab/validate-snapshot", lab=True,
                       body={"checkpoint_json": snapshot["native_checkpoint_json"]}) == {"ok": True}
        request(server, "/__lab/validate-snapshot", lab=True, expect=422,
                body={"checkpoint_json": "{}"})
        cloned = request(server, "/__lab/restore", lab=True, expect=201,
                         body={"checkpoint_json": snapshot["native_checkpoint_json"],
                               "hero_names": snapshot["hero_names"]})
        clone_seats = {row["hero_id"]: row["token"] for row in cloned["player_tokens"]}
        assert clone_seats[responder] != seats[responder]
        clone_game = f"/games/{cloned['game_id']}"
        request(server, clone_game, token=seats[responder], expect=401)
        assert request(server, clone_game, token=clone_seats[responder]) == pending
        unanchored = {key: value for key, value in captured["checkpoint"].items() if key != "rollback"}
        early = request(server, "/__lab/restore", lab=True, expect=201,
                        body={"checkpoint": unanchored})
        early_game = f"/games/{early['game_id']}"
        early_seats = {row["hero_id"]: row["token"] for row in early["player_tokens"]}
        other = next(token for actor, token in early_seats.items() if actor != responder)
        assert not request(server, early_game, token=early_seats[responder])["input_request"].get("can_rollback")
        request(server, early_game + "/starting-position", token=other, body={"op": "cancel"})
        assert not request(server, early_game, token=early_seats[responder])["input_request"].get("can_rollback")

        def concurrent_input(action_id):
            return request(server, clone_game + "/input", token=clone_seats[responder],
                           expect=(200, 409), body={
                               "request_id": pending["input_request"]["request_id"],
                               "selection": "FAST_TRAVEL", "expected_revision": 6,
                               "client_action_id": action_id,
                           })

        with ThreadPoolExecutor(max_workers=2) as requests:
            results = list(requests.map(concurrent_input, ["race-a", "race-b"]))
        assert sum(reply.get("revision") == 7 for reply in results) == 1
        assert sum(reply.get("detail") == "Stale game revision" for reply in results) == 1
        assert request(server, game, token=seats[responder]) == pending

    with serve(binary, pack, database) as restarted:
        before_undo = request(restarted, clone_game, token=clone_seats[responder])
        assert before_undo["revision"] == 7
        move_prompt = before_undo["input_request"]
        assert move_prompt["type"] == "SELECT_HEX" and move_prompt["can_rollback"]
        other = next(token for actor, token in clone_seats.items() if actor != responder)
        request(restarted, clone_game + "/rollback", token=other,
                body={"expected_revision": 7}, expect=422)
        with sockets(restarted, clone_game, clone_seats[responder]) as player:
            assert packet(player, "STATE_UPDATE", 7)["input_request"] == move_prompt
            player.send(json.dumps({"type": "ROLLBACK", "expected_revision": 7,
                                    "client_action_id": "undo-after-restart"}))
            result = packet(player, "ACTION_RESULT", 8)
            assert result["client_action_id"] == "undo-after-restart"
            rewound = packet(player, "STATE_UPDATE", 8)
            undo_prompt = rewound["input_request"]
            assert rewound["view"] == pending["view"]
            assert undo_prompt["request_id"] != pending["input_request"]["request_id"]
            assert undo_prompt["request_id"] != move_prompt["request_id"]
            assert {k: v for k, v in undo_prompt.items() if k != "request_id"} == {
                k: v for k, v in pending["input_request"].items() if k != "request_id"
            }
        request(restarted, clone_game + "/input", token=clone_seats[responder], expect=422,
                body={"request_id": pending["input_request"]["request_id"], "selection": "FAST_TRAVEL"})
        assert request(restarted, clone_game + "/rollback", token=clone_seats[responder],
                       body={"expected_revision": 7, "client_action_id": "undo-after-restart"}) == {
            k: v for k, v in result.items() if k not in ("type", "client_action_id")
        }
        assert request(restarted, game, token=seats[responder]) == pending
        assert request(restarted, game + "/cards", token=arien, body=first) == original
        with (
            sockets(restarted, game, spectator) as public,
            sockets(restarted, game, seats[responder]) as player,
        ):
            assert packet(public, "STATE_UPDATE", 6)["awaiting_input"] == [responder]
            assert packet(player, "STATE_UPDATE", 6)["input_request"] == pending["input_request"]
            result = request(restarted, lab_game + "/command", lab=True, body={
                "actor": responder, "expected_revision": 6,
                "message": {"type": "SUBMIT_INPUT", "request_id": pending["input_request"]["request_id"],
                            "selection": "FAST_TRAVEL"},
            })
            assert result["revision"] == 7
            assert packet(public, "STATE_UPDATE", 7)["input_request"] is None
            move = packet(player, "STATE_UPDATE", 7)["input_request"]
            assert move["type"] == "SELECT_HEX"
            destination = move["valid_hexes"][0]
            player.send(json.dumps({
                "type": "SUBMIT_INPUT", "request_id": move["request_id"],
                "selection": destination, "expected_revision": 7, "client_action_id": "move-after-restart",
            }))
            assert packet(player, "ACTION_RESULT", 8)["client_action_id"] == "move-after-restart"
            own = packet(player, "STATE_UPDATE", 8)
            observed = packet(public, "STATE_UPDATE", 8)
            assert own["client_action_id"] == "move-after-restart"
            assert "client_action_id" not in observed
            assert own["view"]["board"]["entity_locations"][responder] == destination
            assert observed["view"]["board"]["entity_locations"][responder] == destination
            assert any(event["to_hex"] == destination for event in observed["events"])
        request(restarted, lab_game + "/command", lab=True, expect=409, body={
            "actor": responder, "expected_revision": 6,
            "message": {"type": "SUBMIT_INPUT", "request_id": pending["input_request"]["request_id"],
                        "selection": "FAST_TRAVEL"},
        })
        assert request(restarted, game, token=spectator)["revision"] == 8
        undo = request(restarted, game + "/rollback", token=seats[responder],
                       body={"expected_revision": 8})
        assert undo["revision"] == 9
        assert request(restarted, game, token=seats[responder])["view"] == pending["view"]
        replay = verify_replay(restarted, game, credentials=(spectator, *seats.values()),
                               operations=("command",), tamper=True)
        assert replay["start_revision"] == 0 and replay["revision"] == 9
        replay = verify_replay(restarted, clone_game, operations=("command",))
        assert replay["start_revision"] == 6 and replay["revision"] == 8
        choices = request(restarted, history, token=spectator)["decisions"]
        assert len(choices) == 9 and choices[-1]["type"] == "rollback"
        assert choices[6]["type"] == "input" and "FAST_TRAVEL" not in choices[6]["label"]
        assert pending["input_request"]["request_id"] not in json.dumps(choices)

    with sqlite3.connect(database) as db:
        db.execute("ALTER TABLE replay_entries DROP COLUMN history")
    with serve(binary, pack, database) as migrated:
        old = request(migrated, history, token=spectator)["decisions"]
        assert [row["type"] for row in old] == [row["type"] for row in choices]
        assert all(row["round"] is None and row["turn"] is None for row in old)

    # Simulate a database written by the server before live journals existed.
    with sqlite3.connect(database) as db:
        db.execute("DROP TABLE replay_entries")
        db.execute("DROP TABLE replay_starts")
    with serve(binary, pack, database) as migrated:
        replay = verify_replay(migrated, game)
        assert replay["start_revision"] == replay["revision"] == 9
        prompt = request(migrated, game, token=seats[responder])["input_request"]
        request(migrated, game + "/input", token=seats[responder], body={
            "request_id": prompt["request_id"], "selection": "FAST_TRAVEL", "expected_revision": 9,
        })
        replay = verify_replay(migrated, game, operations=("command",))
        assert replay["start_revision"] == 9 and replay["revision"] == 10



def deployment_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien", "Hanu"],
            "blue_heroes": ["Brogan", "Tigerclaw"],
            "seed": 20261017,
        })
        game = f"/games/{opened['game_id']}"
        route = game + "/starting-position"
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        arien, hanu, brogan = (seats[f"hero_{name}"] for name in ("arien", "hanu", "brogan"))
        spectator = opened["spectator_token"]
        beginning = request(server, game, token=arien)
        position = beginning["view"]["starting_position"]
        assert position["swap_targets"] == ["hero_hanu"]
        assert not position["requests"]
        assert request(server, game, token=spectator)["view"]["starting_position"] is None
        locations = beginning["view"]["board"]["entity_locations"]
        request(server, route, token=arien, expect=422,
                body={"op": "move", "destination": locations["hero_brogan"]})
        request(server, route, token=spectator, expect=403, body={"op": "cancel"})
        destination = position["destinations"][0]
        moved = request(server, route, token=arien, body={
            "op": "move", "destination": destination, "expected_revision": 0,
        })
        assert moved["revision"] == 1
        snapshot = request(server, "/__lab" + game + "/checkpoint", lab=True)
        assert snapshot["checkpoint"]["envelope"]["state"]["last_turn_positions"]["hero_arien"] == destination
        with sockets(server, game, arien) as player:
            assert packet(player, "STATE_UPDATE", 1)["view"]["board"]["entity_locations"]["hero_arien"] == destination
            player.send(json.dumps({"type": "STARTING_POSITION", "op": "request_swap",
                                    "target": "hero_hanu", "expected_revision": 1}))
            packet(player, "STARTING_POSITION_UPDATED", 2)
            row, = packet(player, "STATE_UPDATE", 2)["view"]["starting_position"]["requests"]
        assert row["from"] == "hero_arien" and row["to"] == "hero_hanu"
        assert not request(server, game, token=brogan)["view"]["starting_position"]["requests"]
        accept = {"op": "respond_swap", "request_id": row["id"], "accept": True}
        request(server, route, token=brogan, expect=422, body=accept)

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=hanu)["view"]["starting_position"]["requests"] == [row]
        result = request(restarted, route, token=hanu, body={**accept, "expected_revision": 2})
        assert result["revision"] == 3
        swapped = request(restarted, game, token=arien)["view"]
        assert swapped["board"]["entity_locations"]["hero_hanu"] == destination
        assert swapped["board"]["entity_locations"]["hero_arien"] == locations["hero_hanu"]
        assert not swapped["starting_position"]["requests"]
        request(restarted, route, token=hanu, expect=422, body=accept)
        for respond in ({"op": "respond_swap", "accept": False}, {"op": "cancel"}):
            request(restarted, route, token=arien, body={"op": "request_swap", "target": "hero_hanu"})
            item, = request(restarted, game, token=hanu)["view"]["starting_position"]["requests"]
            assert item["id"] != row["id"]
            request(restarted, route, token=hanu, body={**respond, "request_id": item["id"]})
            assert not request(restarted, game, token=arien)["view"]["starting_position"]["requests"]
        request(restarted, route, token=arien, body={"op": "request_swap", "target": "hero_hanu"})
        item, = request(restarted, game, token=hanu)["view"]["starting_position"]["requests"]
        request(restarted, game + "/cards", token=arien, body={"card_id": "aspiring_duelist"})
        assert request(restarted, game, token=arien)["view"]["starting_position"] is None
        assert not request(restarted, game, token=hanu)["view"]["starting_position"]["requests"]
        request(restarted, route, token=hanu, expect=422, body={**accept, "request_id": item["id"]})


def wait_view(server, game, token, predicate, timeout=25):
    deadline = time.monotonic() + timeout
    while True:
        value = request(server, game, token=token)
        if predicate(value):
            return value
        if time.monotonic() >= deadline:
            raise AssertionError((value["revision"], value["view"]["phase"], value["awaiting_input"]))
        time.sleep(0.08)


def bot_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        setup = {"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29}
        request(server, "/games", body={**setup, "bots": {"hero_hanu": {"kind": "rust"}}}, expect=422)
        with sqlite3.connect(database) as db:
            db.execute("CREATE TRIGGER bot_restart BEFORE UPDATE ON games "
                       "BEGIN SELECT RAISE(ABORT, 'bot pending across restart'); END")
        bots = {"hero_brogan": {"kind": "rust"}}
        opened = request(server, "/games", body={**setup, "bots": bots}, expect=201)
        game = f"/games/{opened['game_id']}"
        lab_game = "/__lab" + game
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        arien = seats["hero_arien"]
        brogan = seats["hero_brogan"]
        assert request(server, game, token=arien)["revision"] == 0
        assert request(server, lab_game + "/checkpoint", lab=True)["bots"] == bots

    with sqlite3.connect(database) as db:
        db.execute("DROP TRIGGER bot_restart")

    with serve(binary, pack, database) as server:
        first = wait_view(server, game, arien, lambda view: view["revision"] == 1)
        assert first["view"]["phase"] == "PLANNING"
        assert hero(first, "hero_arien")["current_turn_card"] is None
        public_commitment = hero(first, "hero_brogan")["current_turn_card"]
        assert public_commitment["is_facedown"] and "id" not in public_commitment
        request(server, game + "/uncommit", token=brogan, body={}, expect=403)
        request(server, game + "/cards", token=arien,
                body={"card_id": "liquid_leap", "expected_revision": 1})
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            state = request(server, game, token=arien)
            if state["view"]["turn"] > 1:
                assert state["revision"] >= 4
                break
            prompt = state["input_request"]
            if prompt is None:
                time.sleep(0.08)
                continue
            options = prompt.get("options", [])
            ids = [option["id"] if isinstance(option, dict) else option for option in options]
            choice = next((item for item in ("HOLD", "PASS", "NO") if item in ids), None)
            if choice is None:
                choice = prompt.get("player_ids", ids)[0]
            request(server, game + "/input", token=arien, expect=(200, 409),
                    body={"request_id": prompt["request_id"], "selection": choice,
                          "expected_revision": state["revision"]})
        else:
            raise AssertionError("Mixed game did not complete its first turn")


def clock_smoke(binary, pack, database):
    advisory = {
        "planning_allowance_seconds": 5, "resolution_allowance_seconds": 1,
        "initiative_bonus_seconds": 2, "response_grant_seconds": 1,
        "initial_time_bank_seconds": 0, "time_bank_increment_seconds": 0,
        "max_time_bank_seconds": 10, "upgrade_allowance_seconds": 1,
        "enforce_timeouts": False,
    }
    setup = {"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29}
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", body={**setup, "time_control": advisory}, expect=201)
        game = f"/games/{opened['game_id']}"
        lab_game = "/__lab" + game
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        arien, brogan = seats["hero_arien"], seats["hero_brogan"]
        assert request(server, game, token=arien)["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"
        request(server, game + "/cards", token=arien, expect=422, body={"card_id": "aspiring_duelist"})
        with sockets(server, game, arien) as player:
            packet(player, "STATE_UPDATE", 0)
            player.send(json.dumps({"type": "SET_READY", "ready": True}))
            assert packet(player, "READY_UPDATED", 1)["hero_id"] == "hero_arien"
            assert packet(player, "STATE_UPDATE", 1)["view"]["clock"]["ready_hero_ids"] == ["hero_arien"]
        running = request(server, game + "/ready", token=brogan, body={"ready": True})["view"]["clock"]
        assert running["status"] == "RUNNING"
        assert running["active"]["running_hero_ids"] == ["hero_arien", "hero_brogan"]
        before = request(server, lab_game + "/checkpoint", lab=True)
        time.sleep(0.08)
        after = request(server, game, token=arien)["view"]["clock"]
        assert after["server_now_ms"] > running["server_now_ms"]
        assert after["players"]["hero_arien"]["planning_allowance_ms"] < running["players"]["hero_arien"]["planning_allowance_ms"]
        assert request(server, lab_game + "/checkpoint", lab=True) == before
        snapshot = request(server, lab_game + "/snapshot", lab=True)["checkpoint"]
        saved_clock = json.loads(snapshot["native_checkpoint_json"])["envelope"]["state"]["clock"]
        assert saved_clock["last_settled_at_ms"] >= after["server_now_ms"]
        assert saved_clock["players"]["hero_arien"]["planning_allowance_ms"] <= after["players"]["hero_arien"]["planning_allowance_ms"]
        frozen = request(server, "/__lab/restore", lab=True, expect=201, body={
            "checkpoint_json": snapshot["native_checkpoint_json"], "freeze_clock": True,
        })
        frozen_view = lambda: request(server, "/games/" + frozen["game_id"],
                                      token=frozen["spectator_token"])["view"]["clock"]
        frozen_start = frozen_view()
        time.sleep(0.05)
        assert frozen_view() == frozen_start
        request(server, game + "/cards", token=arien, body={"card_id": "aspiring_duelist"})
        clock = request(server, game, token=arien)["view"]["clock"]
        assert clock["active"]["running_hero_ids"] == ["hero_brogan"]
        assert clock["players"]["hero_arien"]["planning_complete"]
        request(server, game + "/cards", token=brogan, body={"card_id": "mad_dash"})
        resolution = request(server, game, token=arien)["view"]["clock"]
        assert resolution["active"]["kind"] == "RESOLUTION"
        actor, = resolution["active"]["hero_ids"]
        assert resolution["players"][actor]["initiative_bonus_ms"] > 0

        enforcing = {**advisory, "planning_allowance_seconds": 0,
                     "resolution_allowance_seconds": 0, "initiative_bonus_seconds": 0,
                     "response_grant_seconds": 0, "upgrade_allowance_seconds": 0,
                     "enforce_timeouts": True, "automatic_turn_limit": 1}
        timed = request(server, "/games", body={**setup, "time_control": enforcing}, expect=201)
        timed_game = f"/games/{timed['game_id']}"
        spectator = timed["spectator_token"]
        with sockets(server, timed_game, spectator) as public:
            packet(public, "STATE_UPDATE", 0)
            for row in timed["player_tokens"]:
                request(server, timed_game + "/ready", token=row["token"], body={"ready": True})
            events = []
            for _ in range(30):
                value = json.loads(public.recv(timeout=5))
                events.extend(value.get("events", []))
                if value["view"]["clock"]["status"] == "SUSPENDED_FOR_INACTIVITY":
                    assert value["view"]["turn"] == 2
                    break
            else:
                raise AssertionError("Automatic turn did not suspend")
            timeouts = [event for event in events if event["event_type"] == "TIMER_EXPIRED"]
            assert {event["actor_id"] for event in timeouts if event["metadata"]["clock_kind"] == "PLANNING"} == {"hero_arien", "hero_brogan"}
            assert all("card_id" not in event["metadata"] for event in timeouts)
        suspended = request(server, timed_game, token=spectator)
        assert suspended["view"]["clock"]["ready_hero_ids"] == []
        restarting = request(server, "/games", body={**setup, "time_control": {
            **enforcing, "planning_allowance_seconds": 2,
        }}, expect=201)
        restarting_game = f"/games/{restarting['game_id']}"
        for row in restarting["player_tokens"]:
            request(server, restarting_game + "/ready", token=row["token"], body={"ready": True})
        assert request(server, restarting_game, token=restarting["spectator_token"])["revision"] == 2

    with serve(binary, pack, database) as restarted:
        automatic = wait_view(restarted, restarting_game, restarting["spectator_token"],
                              lambda value: value["view"]["clock"]["status"] == "SUSPENDED_FOR_INACTIVITY",
                              timeout=10)
        assert automatic["view"]["turn"] == 2
        verify_replay(restarted, restarting_game, operations=("command", "timeout"))
        assert request(restarted, timed_game, token=spectator)["view"]["clock"]["status"] == "SUSPENDED_FOR_INACTIVITY"
        resumed = request(restarted, game, token=arien)["view"]["clock"]
        assert resumed["status"] == "RUNNING"
        assert resumed["active"]["request_id"] == resolution["active"]["request_id"]


def timed_bot_smoke(binary, pack, database):
    setup = {
        "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
        "bots": {"hero_brogan": {"kind": "rust"}},
        "time_control": {
            "planning_allowance_seconds": 12, "resolution_allowance_seconds": 0,
            "response_grant_seconds": 0, "initial_time_bank_seconds": 0,
            "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0,
            "upgrade_allowance_seconds": 0, "automatic_turn_limit": 1,
        },
    }
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", lab=True, body=setup, expect=201)
        game = "/games/" + opened["game_id"]
        lab_game = "/__lab" + game
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        spectator = opened["spectator_token"]
        waiting = request(server, game, token=spectator)
        assert waiting["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"
        assert waiting["view"]["clock"]["ready_hero_ids"] == ["hero_brogan"]
        assert request(server, lab_game + "/responders", lab=True)["players"] == ["hero_arien"]
        request(server, game + "/ready", token=seats["hero_brogan"], body={"ready": True}, expect=403)
        snapshot = request(server, lab_game + "/snapshot", lab=True)["checkpoint"]
        request(server, "/__lab/restore", lab=True, expect=422, body={
            "checkpoint_json": snapshot["native_checkpoint_json"], "freeze_clock": True,
            "bots": setup["bots"],
        })
        time.sleep(0.15)
        assert request(server, game, token=spectator)["revision"] == waiting["revision"]

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=spectator)["view"]["clock"]["ready_hero_ids"] == ["hero_brogan"]
        with sockets(restarted, game, spectator) as public:
            packet(public, "STATE_UPDATE", waiting["revision"])
            ready = request(restarted, game + "/ready", token=seats["hero_arien"], body={"ready": True})
            assert ready["view"]["clock"]["status"] == "RUNNING"
            saw_bot_plan = False
            for _ in range(36):
                update = json.loads(public.recv(timeout=17))
                if update.get("type") != "STATE_UPDATE":
                    continue
                clock = update["view"]["clock"]
                brogan = clock["players"]["hero_brogan"]
                if update["view"]["phase"] == "PLANNING" and update["view"]["turn"] == 1:
                    saw_bot_plan |= brogan["planning_complete"] and not brogan["planning_locked_by_timeout"]
                if clock["status"] == "SUSPENDED_FOR_INACTIVITY":
                    assert update["view"]["turn"] == 2
                    break
            else:
                raise AssertionError("Timed bot match did not suspend after the unattended turn")
            assert saw_bot_plan, "Brogan's own choice should finish planning before the clock fallback"
        assert request(restarted, lab_game + "/responders", lab=True)["players"] == ["hero_arien"]
        replay = verify_replay(restarted, game, operations=("command", "timeout"))
        assert replay["bot_seats"] == ["hero_brogan"]
        resumed = request(restarted, game + "/ready", token=seats["hero_arien"], body={"ready": True})
        clock = resumed["view"]["clock"]
        assert clock["status"] == "RUNNING"
        assert clock["ready_hero_ids"] == ["hero_arien", "hero_brogan"]

        resolution = request(restarted, "/games", lab=True, expect=201, body={
            **setup, "time_control": {
                **setup["time_control"], "resolution_allowance_seconds": 30,
            },
        })
        game = "/games/" + resolution["game_id"]
        human = next(row["token"] for row in resolution["player_tokens"] if row["hero_id"] == "hero_arien")
        with sockets(restarted, game, resolution["spectator_token"]) as public:
            packet(public, "STATE_UPDATE", 0)
            request(restarted, game + "/ready", token=human, body={"ready": True})
            request(restarted, game + "/cards", token=human, body={"card_id": "aspiring_duelist"})
            bot_prompt = None
            for _ in range(12):
                update = json.loads(public.recv(timeout=17))
                if update.get("type") != "STATE_UPDATE":
                    continue
                assert all(event["event_type"] != "TIMER_EXPIRED" for event in update.get("events", []))
                if bot_prompt is not None and update["revision"] > bot_prompt:
                    break
                if update["awaiting_input"] == ["hero_brogan"]:
                    assert update["view"]["clock"]["active"]["kind"] == "RESOLUTION"
                    bot_prompt = update["revision"]
                elif update["awaiting_input"] == ["hero_arien"]:
                    offered = request(restarted, game, token=human)["input_request"]
                    options = [option["id"] for option in offered["options"]]
                    selection = "HOLD" if "HOLD" in options else "CONFIRM"
                    assert selection in options
                    request(restarted, game + "/input", token=human, body={
                        "request_id": offered["request_id"], "selection": selection,
                    })
            else:
                raise AssertionError("The bot did not answer its timed resolution prompt")


def reopened_bot_smoke(binary, pack, database):
    setup = {
        "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
        "bots": {"hero_brogan": {"kind": "rust"}},
        "time_control": {
            "planning_allowance_seconds": 30, "resolution_allowance_seconds": 30,
            "response_grant_seconds": 0, "initial_time_bank_seconds": 0,
            "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0,
            "upgrade_allowance_seconds": 30,
        },
    }

    def patch_lives(player, amount, revision):
        player.send(json.dumps({
            "type": "PROPOSE_OVERRIDE", "family": "patch", "op": "set_life_counters",
            "args": {"team": "RED", "value": amount},
        }))
        packet(player, "OVERRIDE_PROPOSED")
        assert packet(player, "OVERRIDE_RESOLVED")["outcome"] == "applied"
        return packet(player, "STATE_UPDATE", revision)

    with serve(binary, pack, database) as server:
        opened = request(server, "/games", lab=True, body=setup, expect=201)
        game = "/games/" + opened["game_id"]
        human = next(seat["token"] for seat in opened["player_tokens"] if seat["hero_id"] == "hero_arien")
        with sockets(server, game, human) as player:
            assert packet(player, "STATE_UPDATE", 0)["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"
            finished = patch_lives(player, 0, 1)
            assert finished["view"]["phase"] == "GAME_OVER" and finished["winner"] == "BLUE"

    with serve(binary, pack, database) as server:
        with sockets(server, game, human) as player:
            assert packet(player, "STATE_UPDATE", 1)["view"]["phase"] == "GAME_OVER"
            reopened = patch_lives(player, 2, 2)
            assert reopened["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"
            assert reopened["winner"] is None
            request(server, game + "/ready", token=human, body={"ready": True})
            assert packet(player, "STATE_UPDATE", 3)["view"]["clock"]["status"] == "RUNNING"
            planned = packet(player, "STATE_UPDATE", 4)
            brogan = planned["view"]["clock"]["players"]["hero_brogan"]
            assert brogan["planning_complete"] and not brogan["planning_locked_by_timeout"]
        replay = verify_replay(server, game, operations=("patch", "command"))
        assert replay["entries"][-1]["decision"]["operation"]["actor"] == "hero_brogan"


def pause_vote_smoke(binary, pack, database):
    setup = {
        "red_heroes": ["Arien", "Hanu"], "blue_heroes": ["Brogan", "Tigerclaw"], "seed": 29,
        "time_control": {
            "planning_allowance_seconds": 60, "resolution_allowance_seconds": 60,
            "response_grant_seconds": 20, "initial_time_bank_seconds": 0,
            "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0,
            "upgrade_allowance_seconds": 60,
        },
    }

    def start(server):
        opened = request(server, "/games", body=setup, expect=201)
        game = "/games/" + opened["game_id"]
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        for token in seats.values():
            request(server, game + "/ready", token=token, body={"ready": True})
        return game, seats, opened["spectator_token"]

    def send(socket, kind, **body):
        socket.send(json.dumps({"type": kind, **body}))

    def receive(socket, kind, **fields):
        for _ in range(24):
            value = json.loads(socket.recv(timeout=10))
            if value["type"] == kind and all(value.get(key) == expected for key, expected in fields.items()):
                return value
        raise AssertionError(f"Missing {kind} {fields}")

    def propose(socket):
        send(socket, "PROPOSE_OVERRIDE", family="pause")
        return receive(socket, "OVERRIDE_PROPOSED")["proposal_id"]

    def resolved(socket, identifier, outcome):
        return receive(socket, "OVERRIDE_RESOLVED", proposal_id=identifier, outcome=outcome)

    def reveal(socket, target):
        send(socket, "PROPOSE_OVERRIDE", family="reveal_player", args={"hero_id": target})
        proposal = receive(socket, "OVERRIDE_PROPOSED")
        assert proposal["family"] == "reveal_player" and proposal["args"] == {"hero_id": target}
        return proposal["proposal_id"]

    def link(socket, target, token):
        assert packet(socket, "PLAYER_LINK_REVEALED") == {
            "type": "PLAYER_LINK_REVEALED", "hero_id": target, "token": token,
        }

    with serve(binary, pack, database, override_timeout=3) as server:
        running_game, running_seats, running_spectator = start(server)
        target = "hero_brogan"
        with (
            sockets(server, running_game, running_seats["hero_arien"]) as first,
            sockets(server, running_game, running_seats["hero_hanu"]) as second,
            sockets(server, running_game, running_spectator) as public,
        ):
            for socket in (first, second, public):
                packet(socket, "STATE_UPDATE", 4)
            send(first, "PROPOSE_OVERRIDE", family="reveal_player", args={"hero_id": "hero_min"})
            assert "human seat" in packet(first, "ERROR")["detail"]
            vote = reveal(first, target)
            for socket in (second, public):
                assert packet(socket, "OVERRIDE_PROPOSED")["proposal_id"] == vote
            for socket in (first, second, public):
                assert packet(socket, "STATE_UPDATE", 5)["view"]["clock"]["active"] is None
            send(public, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            assert "Spectators" in packet(public, "ERROR")["detail"]
            send(first, "PROPOSE_OVERRIDE", family="pause")
            assert "already open" in packet(first, "ERROR")["detail"]
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            for socket in (first, second, public):
                assert packet(socket, "OVERRIDE_RESOLVED")["outcome"] == "applied"
            for socket in (first, second):
                link(socket, target, running_seats[target])
            for socket in (first, second, public):
                assert packet(socket, "STATE_UPDATE", 6)["view"]["clock"]["active"]["kind"] == "PLANNING"
        archive = verify_replay(
            server, running_game,
            credentials=[*running_seats.values(), running_spectator],
            operations=("pause_opened", "pause_released"),
        )
        assert [entry["decision"]["operation"]["kind"] for entry in archive["entries"]][-2:] == [
            "pause_opened", "pause_released",
        ]

        game, seats, spectator = start(server)
        with (
            sockets(server, game, seats["hero_arien"]) as first,
            sockets(server, game, seats["hero_arien"]) as duplicate,
            sockets(server, game, seats["hero_hanu"]) as second,
            sockets(server, game, spectator) as public,
        ):
            for socket in (first, duplicate, second, public):
                packet(socket, "STATE_UPDATE", 4)
            vote = propose(first)
            for socket in (duplicate, second, public):
                proposal = receive(socket, "OVERRIDE_PROPOSED", proposal_id=vote)
                assert proposal["eligible_voters"] == ["hero_arien", "hero_hanu"]
                assert proposal["threshold"] == 2 and proposal["tally"]["yes"] == ["hero_arien"]
            held = packet(first, "STATE_UPDATE", 5)["view"]["clock"]
            assert held["active"] is None and held["status"] == "RUNNING"
            assert request(server, "/__lab" + game + "/responders", lab=True)["players"] == []
            time.sleep(0.15)
            assert request(server, game, token=spectator)["view"]["clock"]["players"] == held["players"]
            request(server, game + "/cards", token=seats["hero_arien"], expect=422,
                    body={"card_id": "aspiring_duelist"})
            send(public, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            assert "Spectators" in receive(public, "ERROR")["detail"]
            with sockets(server, game, seats["hero_brogan"]) as late:
                packet(late, "STATE_UPDATE", 5)
                receive(late, "OVERRIDE_PROPOSED", proposal_id=vote)
                send(late, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
                assert "proposal time" in receive(late, "ERROR")["detail"]
            send(second, "CANCEL_OVERRIDE", proposal_id=vote)
            assert "proposer" in receive(second, "ERROR")["detail"]
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=False)
            resolved(first, vote, "rejected")
            resumed = packet(first, "STATE_UPDATE", 6)["view"]["clock"]
            assert resumed["active"]["kind"] == "PLANNING"
            for name in seats:
                assert 0 <= held["players"][name]["planning_allowance_ms"] - resumed["players"][name]["planning_allowance_ms"] < 140

            vote = propose(first)
            send(first, "CANCEL_OVERRIDE", proposal_id=vote)
            resolved(first, vote, "cancelled")
            vote = propose(first)
            resolved(first, vote, "expired")
            assert request(server, game, token=spectator)["view"]["clock"]["active"]["kind"] == "PLANNING"

            for name, token in seats.items():
                card = hero(request(server, game, token=token), name)["hand"][0]["id"]
                request(server, game + "/cards", token=token, body={"card_id": card})
            before = request(server, game, token=spectator)
            assert before["view"]["phase"] == "RESOLUTION"
            active = before["view"]["clock"]["active"]
            vote = propose(first)
            with sqlite3.connect(database) as db:
                db.execute("CREATE TRIGGER pause_failure BEFORE UPDATE ON games "
                           "BEGIN SELECT RAISE(ABORT, 'failed pause transaction'); END")
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            receive(second, "ERROR")
            assert request(server, game, token=spectator)["view"]["clock"]["active"] is None
            with sqlite3.connect(database) as db:
                db.execute("DROP TRIGGER pause_failure")
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            resolved(first, vote, "applied")
            paused = receive(first, "STATE_UPDATE", revision=before["revision"] + 2)["view"]["clock"]
            assert paused["status"] == "PAUSED" and paused["ready_hero_ids"] == []
            assert paused["pause"]["requested_by"] == "hero_arien"
            send(first, "PROPOSE_OVERRIDE", family="rewind", to=0)
            assert "paused" in packet(first, "ERROR")["detail"]


            target = "hero_brogan"
            vote = reveal(first, target)
            receive(second, "OVERRIDE_PROPOSED", proposal_id=vote)
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            for socket in (first, second):
                resolved(socket, vote, "applied")
                link(socket, target, seats[target])
            assert request(server, game, token=spectator)["revision"] == before["revision"] + 2
            with sockets(server, game, seats[target]) as reconnected:
                packet(reconnected, "STATE_UPDATE", before["revision"] + 2)
                resolved(reconnected, vote, "applied")
                send(reconnected, "GET_VIEW")
                packet(reconnected, "STATE_UPDATE", before["revision"] + 2)

            vote = reveal(first, target)
            receive(second, "OVERRIDE_PROPOSED", proposal_id=vote)
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=False)
            for socket in (first, second):
                resolved(socket, vote, "rejected")
                send(socket, "GET_VIEW")
                packet(socket, "STATE_UPDATE", before["revision"] + 2)

        other, other_seats, other_spectator = start(server)
        with (
            sockets(server, other, other_seats["hero_arien"]) as first,
            sockets(server, other, other_seats["hero_hanu"]) as second,
        ):
            packet(first, "STATE_UPDATE", 4)
            packet(second, "STATE_UPDATE", 4)
            interrupted = propose(first)
            packet(first, "STATE_UPDATE", 5)

    with serve(binary, pack, database, override_timeout=3) as restarted:
        with sockets(restarted, other, other_spectator) as public:
            recovered = packet(public, "STATE_UPDATE", 6)
            assert recovered["view"]["clock"]["active"]["kind"] == "PLANNING"
            resolved(public, interrupted, "cancelled")
        verify_replay(restarted, other, operations=("pause_opened", "pause_released"))
        verify_replay(restarted, game, operations=("pause_opened", "pause_approved"))
        current = request(restarted, game, token=spectator)["view"]["clock"]
        assert current["status"] == "PAUSED" and current["players"] == paused["players"]
        for token in seats.values():
            request(restarted, game + "/ready", token=token, body={"ready": True})
        resumed = request(restarted, game, token=spectator)["view"]["clock"]
        assert resumed["status"] == "RUNNING" and resumed["active"] == active

        opened = request(restarted, "/games", body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
        }, expect=201)
        untimed = "/games/" + opened["game_id"]
        manual = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
        with sockets(restarted, untimed, manual["hero_arien"]) as sole:
            packet(sole, "STATE_UPDATE", 0)
            vote = reveal(sole, "hero_brogan")
            assert packet(sole, "OVERRIDE_RESOLVED")["proposal_id"] == vote
            link(sole, "hero_brogan", manual["hero_brogan"])
            assert request(restarted, untimed, token=manual["hero_brogan"])["revision"] == 0

        one, one_seats, _ = start(restarted)
        with sockets(restarted, one, one_seats["hero_arien"]) as sole:
            packet(sole, "STATE_UPDATE", 4)
            vote = propose(sole)
            resolved(sole, vote, "applied")
            assert packet(sole, "STATE_UPDATE", 5)["view"]["clock"]["status"] == "PAUSED"


def rewind_vote_smoke(binary, pack, database):
    def send(socket, kind, **fields):
        socket.send(json.dumps({"type": kind, **fields}))

    with serve(binary, pack, database) as server:
        opened = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
        })
        game = "/games/" + opened["game_id"]
        seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
        arien, brogan = seats["hero_arien"], seats["hero_brogan"]
        spectator = opened["spectator_token"]
        request(server, game + "/cards", token=arien, body={"card_id": "aspiring_duelist"})
        request(server, game + "/cards", token=brogan, body={"card_id": "mad_dash"})
        actor = request(server, game, token=spectator)["awaiting_input"][0]
        original = request(server, game, token=seats[actor])["input_request"]["request_id"]

        with sockets(server, game, arien) as first, sockets(server, game, brogan) as second:
            packet(first, "STATE_UPDATE", 2)
            packet(second, "STATE_UPDATE", 2)
            send(first, "PROPOSE_OVERRIDE", family="rewind", to=9)
            assert "out of range" in packet(first, "ERROR")["detail"]

            def rewind(to, revision, *, fail_once=False):
                send(first, "PROPOSE_OVERRIDE", family="rewind", to=to)
                offer = packet(first, "OVERRIDE_PROPOSED")
                assert offer["family"] == "rewind" and offer["to"] == to and offer["threshold"] == 2
                assert packet(second, "OVERRIDE_PROPOSED") == offer
                if fail_once:
                    with sqlite3.connect(database) as db:
                        db.execute("CREATE TRIGGER rewind_failure BEFORE UPDATE ON games "
                                   "BEGIN SELECT RAISE(ABORT, 'injected rewind failure'); END")
                    send(second, "VOTE_OVERRIDE", proposal_id=offer["proposal_id"], approve=True)
                    packet(second, "ERROR")
                    assert request(server, game, token=spectator)["revision"] == revision - 1
                    with sqlite3.connect(database) as db:
                        db.execute("DROP TRIGGER rewind_failure")
                send(second, "VOTE_OVERRIDE", proposal_id=offer["proposal_id"], approve=True)
                for socket in (first, second):
                    assert packet(socket, "OVERRIDE_RESOLVED")["outcome"] == "applied"
                    packet(socket, "STATE_UPDATE", revision)

            rewind(2, 3)
            refreshed = request(server, game, token=seats[actor])["input_request"]["request_id"]
            assert refreshed != original
            stale = request(server, game + "/input", token=seats[actor], expect=422, body={
                "request_id": original, "selection": "HOLD",
            })
            assert "stale or unissued" in stale["detail"]

            rewind(1, 4, fail_once=True)
            state = request(server, game, token=arien)
            assert state["view"]["phase"] == "PLANNING"
            assert hero(state, "hero_arien")["current_turn_card"]["id"] == "aspiring_duelist"
            assert not hero(state, "hero_brogan")["current_turn_card"]
            history = request(server, game + "/overrides/history", token=spectator)["decisions"]
            assert [(row["type"], row["superseded"]) for row in history] == [
                ("commit", False), ("commit", True), ("ov_rewind", False), ("ov_rewind", False),
            ]
            assert "mad_dash" not in json.dumps(history)
            request(server, game + "/cards", token=brogan, body={
                "card_id": "mad_dash", "expected_revision": 4,
            })
            packet(first, "STATE_UPDATE", 5)
            packet(second, "STATE_UPDATE", 5)
            rewind(3, 6)
            final_prompt = request(server, game, token=seats[actor])["input_request"]
            assert final_prompt["request_id"] not in {original, refreshed}

        archive = verify_replay(server, game, credentials=[arien, brogan, spectator], operations=("rewind",))
        assert [row["decision"]["operation"]["to"] for row in archive["entries"]
                if row["decision"]["operation"]["kind"] == "rewind"] == [2, 1, 3]

        timed = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
            "time_control": {
                "planning_allowance_seconds": 60, "resolution_allowance_seconds": 60,
                "response_grant_seconds": 0, "initial_time_bank_seconds": 0,
                "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0,
                "upgrade_allowance_seconds": 60,
            },
        })
        timed_game = "/games/" + timed["game_id"]
        red, blue = (seat["token"] for seat in timed["player_tokens"])
        for token in (red, blue):
            request(server, timed_game + "/ready", token=token, body={"ready": True})
        request(server, timed_game + "/cards", token=red, body={"card_id": "aspiring_duelist"})
        with sockets(server, timed_game, red) as first, sockets(server, timed_game, blue) as second:
            for socket in (first, second):
                packet(socket, "STATE_UPDATE", 3)
            send(first, "PROPOSE_OVERRIDE", family="rewind", to=2)
            vote = packet(first, "OVERRIDE_PROPOSED")["proposal_id"]
            packet(second, "OVERRIDE_PROPOSED")
            for socket in (first, second):
                assert packet(socket, "STATE_UPDATE", 4)["view"]["clock"]["active"] is None
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            for socket in (first, second):
                assert packet(socket, "OVERRIDE_RESOLVED")["outcome"] == "applied"
                state = packet(socket, "STATE_UPDATE", 5)
                assert not hero(state, "hero_arien")["current_turn_card"]
                clock = state["view"]["clock"]
                assert clock["status"] == "RUNNING"
                assert set(clock["active"]["running_hero_ids"]) == {"hero_arien", "hero_brogan"}
        verify_replay(server, timed_game, operations=("pause_opened", "rewind"))

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=seats[actor])["input_request"] == final_prompt
        result = request(restarted, game + "/input", token=seats[actor], body={
            "request_id": final_prompt["request_id"],
            "selection": final_prompt["options"][0]["id"], "expected_revision": 6,
        })
        assert result["revision"] == 7
        verify_replay(restarted, game, operations=("rewind",))


def unstick_vote_smoke(binary, pack, database):
    cases = []
    with serve(binary, pack, database) as server:
        for op in ("skip_input", "abort_action", "end_turn", "force_actor"):
            opened = request(server, "/games", expect=201, body={
                "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
            })
            game = "/games/" + opened["game_id"]
            seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
            arien, brogan = seats["hero_arien"], seats["hero_brogan"]
            spectator = opened["spectator_token"]
            request(server, game + "/cards", token=arien, body={"card_id": "aspiring_duelist"})
            request(server, game + "/cards", token=brogan, body={"card_id": "mad_dash"})
            before = request(server, game, token=arien)
            assert before["input_request"]["type"] == "CHOOSE_ACTION"
            if op in ("skip_input", "abort_action"):
                request(server, game + "/input", token=arien, body={
                    "request_id": before["input_request"]["request_id"], "selection": "FAST_TRAVEL",
                })
                before = request(server, game, token=arien)
                assert before["input_request"]["can_skip"] is True
            old_request = before["input_request"]["request_id"]
            args = {"hero_id": "hero_brogan"} if op == "force_actor" else {}
            with sockets(server, game, arien) as first, sockets(server, game, brogan) as second:
                for socket in (first, second):
                    packet(socket, "STATE_UPDATE", before["revision"])

                def vote(command, arguments, outcome):
                    first.send(json.dumps({
                        "type": "PROPOSE_OVERRIDE", "family": "unstick", "op": command, "args": arguments,
                    }))
                    proposed = packet(first, "OVERRIDE_PROPOSED")
                    assert proposed["family"] == "unstick" and proposed["op"] == command
                    assert proposed["args"] == arguments and proposed["threshold"] == 2
                    assert packet(second, "OVERRIDE_PROPOSED") == proposed
                    second.send(json.dumps({
                        "type": "VOTE_OVERRIDE", "proposal_id": proposed["proposal_id"], "approve": True,
                    }))
                    resolved = packet(first, "OVERRIDE_RESOLVED")
                    assert packet(second, "OVERRIDE_RESOLVED") == resolved
                    assert resolved["outcome"] == outcome, resolved
                    return resolved

                if op == "force_actor":
                    rejected = vote("skip_input", {}, "rejected")
                    assert "does not accept SKIP" in rejected["reason"]["message"]
                    assert request(server, game, token=arien)["revision"] == before["revision"]

                vote(op, args, "applied")
                revision = before["revision"] + 1
                for socket in (first, second):
                    packet(socket, "STATE_UPDATE", revision)

            state = request(server, game, token=spectator)
            if op in ("skip_input", "abort_action"):
                positions = before["view"]["board"]["entity_locations"]
                assert state["view"]["board"]["entity_locations"]["hero_arien"] == positions["hero_arien"]
            if op == "end_turn":
                assert hero(state, "hero_arien")["played_cards"][0]["id"] == "aspiring_duelist"
                assert state["view"]["current_actor_id"] == "hero_brogan"
            if op == "force_actor":
                assert state["view"]["current_actor_id"] == "hero_brogan"
            rejected = request(server, game + "/input", token=arien, expect=422, body={
                "request_id": old_request, "selection": "HOLD",
            })
            assert "stale or unissued" in rejected["detail"]
            if op == "end_turn":
                with sockets(server, game, arien) as sole:
                    packet(sole, "STATE_UPDATE", revision)
                    packet(sole, "OVERRIDE_RESOLVED")
                    sole.send(json.dumps({"type": "PROPOSE_OVERRIDE", "family": "unstick", "op": op}))
                    assert packet(sole, "OVERRIDE_PROPOSED")["threshold"] == 1
                    assert packet(sole, "OVERRIDE_RESOLVED")["outcome"] == "applied"
                    revision += 1
                    assert packet(sole, "STATE_UPDATE", revision)["view"]["phase"] == "PLANNING"
            archive = verify_replay(server, game, operations=("unstick",))
            assert archive["entries"][-1]["decision"]["operation"]["command"] == {"op": op, **args}
            history = request(server, game + "/overrides/history", token=spectator)
            assert history["decisions"][-1]["type"] == "ov_unstick"
            cases.append((game, spectator, revision))

        opened = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
            "time_control": {
                "planning_allowance_seconds": 60, "resolution_allowance_seconds": 60,
                "response_grant_seconds": 0, "initial_time_bank_seconds": 0,
                "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0,
                "upgrade_allowance_seconds": 60,
            },
        })
        game = "/games/" + opened["game_id"]
        arien, brogan = (seat["token"] for seat in opened["player_tokens"])
        spectator = opened["spectator_token"]
        for token in (arien, brogan):
            request(server, game + "/ready", token=token, body={"ready": True})
        request(server, game + "/cards", token=arien, body={"card_id": "aspiring_duelist"})
        request(server, game + "/cards", token=brogan, body={"card_id": "mad_dash"})
        with sockets(server, game, arien) as first, sockets(server, game, brogan) as second:
            old = packet(first, "STATE_UPDATE", 4)["input_request"]["request_id"]
            packet(second, "STATE_UPDATE", 4)
            for revision, (op, expected) in enumerate([("skip_input", "rejected"), ("end_turn", "applied")]):
                first.send(json.dumps({"type": "PROPOSE_OVERRIDE", "family": "unstick", "op": op, "args": {}}))
                vote = packet(first, "OVERRIDE_PROPOSED")
                packet(second, "OVERRIDE_PROPOSED")
                for socket in (first, second):
                    assert packet(socket, "STATE_UPDATE", 5 + revision * 2)["view"]["clock"]["active"] is None
                second.send(json.dumps({"type": "VOTE_OVERRIDE", "proposal_id": vote["proposal_id"], "approve": True}))
                for socket in (first, second):
                    assert packet(socket, "OVERRIDE_RESOLVED")["outcome"] == expected
                    state = packet(socket, "STATE_UPDATE", 6 + revision * 2)
                    assert state["view"]["clock"]["status"] == "RUNNING"
                active = state["view"]["clock"]["active"]
                assert active["kind"] == "RESOLUTION"
                if op == "skip_input":
                    assert active["request_id"] == old
                else:
                    assert active["request_id"] != old
                    assert active["running_hero_ids"] == ["hero_brogan"]
        verify_replay(server, game, operations=("pause_opened", "pause_released", "unstick"))
        cases.append((game, spectator, 8))

    with serve(binary, pack, database) as restarted:
        for game, spectator, revision in cases:
            assert request(restarted, game, token=spectator)["revision"] == revision
            verify_replay(restarted, game, operations=("unstick",))


def patch_vote_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
        })
        game = "/games/" + opened["game_id"]
        seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
        arien, brogan = seats["hero_arien"], seats["hero_brogan"]
        spectator = opened["spectator_token"]
        opening = request(server, game, token=arien)["view"]
        lane = next(iter(opening["wave_counters"]))
        tie = "BLUE" if opening["tie_breaker_team"] == "RED" else "RED"
        with sockets(server, game, arien) as first, sockets(server, game, brogan) as second:
            revision = 0
            for socket in (first, second):
                packet(socket, "STATE_UPDATE", revision)

            def propose(op, args):
                first.send(json.dumps({
                    "type": "PROPOSE_OVERRIDE", "family": "patch", "op": op, "args": args,
                }))

            def patch(op, **args):
                nonlocal revision
                propose(op, args)
                offered = packet(first, "OVERRIDE_PROPOSED")
                assert offered["family"] == "patch" and offered["op"] == op and offered["args"] == args
                assert packet(second, "OVERRIDE_PROPOSED") == offered
                second.send(json.dumps({
                    "type": "VOTE_OVERRIDE", "proposal_id": offered["proposal_id"], "approve": True,
                }))
                revision += 1
                assert packet(first, "OVERRIDE_RESOLVED")["outcome"] == "applied"
                assert packet(second, "OVERRIDE_RESOLVED")["outcome"] == "applied"
                state = packet(first, "STATE_UPDATE", revision)
                packet(second, "STATE_UPDATE", revision)
                return state

            for op, args in [
                ("set_gold", {"hero_id": "hero_arien", "value": -1}),
                ("set_level", {"hero_id": "hero_arien", "value": 9}),
                ("set_wave_counter", {"lane_id": "unknown", "value": 1}),
            ]:
                propose(op, args)
                packet(first, "ERROR")
            assert request(server, game, token=arien)["revision"] == 0

            state = patch("set_gold", hero_id="hero_arien", value=5)
            assert hero(state, "hero_arien")["gold"] == 5
            state = patch("set_level", hero_id="hero_arien", value=2)
            assert hero(state, "hero_arien")["level"] == 2
            state = patch("set_item", hero_id="hero_arien", stat="RANGE", value=2)
            assert hero(state, "hero_arien")["items"]["RANGE"] == 2
            state = patch("set_item", hero_id="hero_arien", stat="RANGE", value=0)
            assert "RANGE" not in hero(state, "hero_arien")["items"]
            state = patch("set_wave_counter", lane_id=lane, value=2)
            assert state["view"]["wave_counters"][lane] == 2
            state = patch("set_tie_breaker_team", team=tie)
            assert state["view"]["tie_breaker_team"] == tie
            state = patch("set_life_counters", team="RED", value=3)
            assert state["view"]["teams"]["RED"]["life_counters"] == 3
            assert state["view"]["teams"]["RED"]["starting_life_counters"] == opening["teams"]["RED"]["starting_life_counters"]

            for token, card in [(arien, "aspiring_duelist"), (brogan, "mad_dash")]:
                request(server, game + "/cards", token=token, body={
                    "card_id": card, "expected_revision": revision,
                })
                revision += 1
                state = packet(first, "STATE_UPDATE", revision)
                packet(second, "STATE_UPDATE", revision)
            old = state["input_request"]["request_id"]
            state = patch("set_gold", hero_id="hero_brogan", value=2)
            assert hero(state, "hero_brogan")["gold"] == 2
            assert state["input_request"]["request_id"] != old
            request(server, game + "/input", token=arien, expect=422, body={
                "request_id": old, "selection": "HOLD",
            })

            state = patch("set_life_counters", team="RED", value=0)
            assert state["view"]["phase"] == "GAME_OVER" and state["winner"] == "BLUE"
            assert any(event["event_type"] == "GAME_OVER" for event in state["events"])
            state = patch("set_life_counters", team="RED", value=2)
            assert state["view"]["phase"] != "GAME_OVER" and state["winner"] is None
            assert state["view"]["teams"]["RED"]["life_counters"] == 2

        final = request(server, game, token=spectator)
        archive = verify_replay(server, game, operations=("patch",))
        assert archive["entries"][-1]["decision"]["operation"]["command"] == {
            "op": "set_life_counters", "team": "RED", "value": 2,
        }
        history = request(server, game + "/overrides/history", token=spectator)
        assert sum(row["type"] == "ov_patch" for row in history["decisions"]) == 10

        timed = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
            "time_control": {
                "planning_allowance_seconds": 60, "resolution_allowance_seconds": 60,
                "response_grant_seconds": 0, "initial_time_bank_seconds": 0,
                "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0,
                "upgrade_allowance_seconds": 60,
            },
        })
        timed_game = "/games/" + timed["game_id"]
        red, blue = (seat["token"] for seat in timed["player_tokens"])
        for token in (red, blue):
            request(server, timed_game + "/ready", token=token, body={"ready": True})
        with sockets(server, timed_game, red) as first, sockets(server, timed_game, blue) as second:
            for socket in (first, second):
                packet(socket, "STATE_UPDATE", 2)
            for amount, revision, status in [(0, 4, "FINISHED"), (2, 5, "WAITING_FOR_PLAYERS")]:
                first.send(json.dumps({
                    "type": "PROPOSE_OVERRIDE", "family": "patch", "op": "set_life_counters",
                    "args": {"team": "RED", "value": amount},
                }))
                offer = packet(first, "OVERRIDE_PROPOSED")
                packet(second, "OVERRIDE_PROPOSED")
                if amount == 0:
                    for socket in (first, second):
                        assert packet(socket, "STATE_UPDATE", 3)["view"]["clock"]["active"] is None
                second.send(json.dumps({
                    "type": "VOTE_OVERRIDE", "proposal_id": offer["proposal_id"], "approve": True,
                }))
                for socket in (first, second):
                    assert packet(socket, "OVERRIDE_RESOLVED")["outcome"] == "applied"
                    frame = packet(socket, "STATE_UPDATE", revision)
                    assert frame["view"]["clock"]["status"] == status
        for token in (red, blue):
            request(server, timed_game + "/ready", token=token, body={"ready": True})
        state = request(server, timed_game, token=red)
        assert state["view"]["clock"]["status"] == "RUNNING" and state["revision"] == 7
        verify_replay(server, timed_game, operations=("patch", "pause_opened"))

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=spectator) == final
        verify_replay(restarted, game, operations=("patch",))


def board_patch_vote_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", expect=201, body={
            "red_heroes": ["Razzle"], "blue_heroes": ["Emmitt"], "seed": 29,
        })
        game = "/games/" + opened["game_id"]
        seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
        razzle, emmitt = seats["hero_razzle"], seats["hero_emmitt"]
        spectator = opened["spectator_token"]
        opening = request(server, game, token=spectator)["view"]
        board = opening["board"]
        positions = board["entity_locations"]
        piece, reserve = "hero_razzle_piece_1", "hero_razzle_piece_2"
        start = positions[piece]
        enemy = positions["hero_emmitt"]
        free = [
            tile["hex"] for tile in board["tiles"].values()
            if tile["occupant_id"] is None and not tile["is_terrain"]
        ]
        target, second_target = free[:2]
        active_zones = set(opening["battle_zones"].values())
        minion = next(
            row["id"] for row in opening["teams"]["BLUE"]["minions"]
            if any(tile["occupant_id"] == row["id"] and tile["zone_id"] in active_zones
                   for tile in board["tiles"].values())
        )
        minion_start = positions[minion]
        checkpoint = request(server, "/__lab" + game + "/checkpoint", lab=True)["checkpoint"]
        token = next(id for id in checkpoint["envelope"]["state"]["misc_entities"]
                     if id.startswith("glitch_"))

        def occupant(view, point):
            key = "_".join(str(point[axis]) for axis in ("q", "r", "s"))
            return view["board"]["tiles"][key]["occupant_id"]

        with sockets(server, game, razzle) as first, sockets(server, game, emmitt) as second:
            revision = 0
            for socket in (first, second):
                packet(socket, "STATE_UPDATE", revision)

            def vote(op, *, rejection=None, **args):
                nonlocal revision
                first.send(json.dumps({
                    "type": "PROPOSE_OVERRIDE", "family": "patch", "op": op, "args": args,
                }))
                offer = packet(first, "OVERRIDE_PROPOSED")
                assert offer["op"] == op and offer["args"] == args, offer
                assert packet(second, "OVERRIDE_PROPOSED") == offer
                second.send(json.dumps({
                    "type": "VOTE_OVERRIDE", "proposal_id": offer["proposal_id"], "approve": True,
                }))
                outcome = packet(first, "OVERRIDE_RESOLVED")
                assert packet(second, "OVERRIDE_RESOLVED") == outcome
                if rejection:
                    assert outcome["outcome"] == "rejected" and outcome["reason"]["message"] == rejection, outcome
                    assert request(server, game, token=spectator)["revision"] == revision
                    return None
                assert outcome["outcome"] == "applied", outcome
                revision += 1
                state = packet(first, "STATE_UPDATE", revision)
                packet(second, "STATE_UPDATE", revision)
                assert not state.get("events"), state["events"]
                return state["view"]

            vote("move_entity", entity_id="hero_razzle", hex=enemy, rejection="Destination is occupied")
            vote("move_entity", entity_id="hero_razzle", hex={"q": 999, "r": -999, "s": 0},
                 rejection="Destination is outside the board")
            vote("place_entity", entity_id="unregistered", hex=target, rejection="Unknown entity")
            view = vote("move_entity", entity_id="hero_razzle", hex=target)
            assert view["hero_pieces"][piece]["position"] == target
            assert occupant(view, start) is None and occupant(view, target) == piece

            view = vote("place_entity", entity_id=reserve, hex=second_target)
            assert view["hero_pieces"][reserve]["position"] == second_target
            vote("remove_entity", entity_id="hero_razzle", rejection="Specify a concrete hero piece")
            view = vote("remove_entity", entity_id=piece)
            assert piece not in view["board"]["entity_locations"]
            assert view["hero_pieces"][piece]["position"] is None and occupant(view, target) is None
            vote("move_entity", entity_id=piece, hex=start, rejection="Entity is not on the board")
            view = vote("place_entity", entity_id=piece, hex=start)
            assert occupant(view, start) == piece

            view = vote("remove_entity", entity_id=minion)
            assert occupant(view, minion_start) is None and minion not in view["board"]["entity_locations"]
            minion_tile = next(tile for tile in board["tiles"].values() if tile["hex"] == minion_start)
            lane = next(lane for lane, row in opening["battle_zone_minion_counts"].items()
                        if row["zone_id"] == minion_tile["zone_id"])
            assert (view["battle_zone_minion_counts"][lane]["counts"]["BLUE"]
                    == opening["battle_zone_minion_counts"][lane]["counts"]["BLUE"] - 1)
            view = vote("place_entity", entity_id=minion, hex=minion_start)
            assert occupant(view, minion_start) == minion
            assert view["battle_zone_minion_counts"] == opening["battle_zone_minion_counts"]

            view = vote("place_entity", entity_id=token, hex=target)
            assert occupant(view, target) == token
            assert any(row["id"] == token and row["hex"] == target for row in view["tokens"])
            view = vote("remove_entity", entity_id=token)
            assert occupant(view, target) is None
            assert all(row["id"] != token for row in view["tokens"])

            vote("add_marker", marker_type="poison", target_id="unknown", source_id="hero_emmitt", value=-2,
                 rejection="Unknown entity")
            view = vote("add_marker", marker_type="poison", target_id=piece, source_id="hero_emmitt", value=-2)
            assert view["markers"]["poison"] == {"target_id": piece, "source_id": "hero_emmitt", "value": -2}
            view = vote("add_marker", marker_type="poison", target_id="hero_emmitt", source_id="manual", value=-1)
            assert view["markers"]["poison"] == {"target_id": "hero_emmitt", "source_id": "manual", "value": -1}
            view = vote("remove_marker", marker_type="poison")
            assert view["markers"]["poison"] == {"target_id": None, "source_id": None, "value": 0}

        final = request(server, game, token=spectator)
        archive = verify_replay(server, game, operations=("patch",))
        assert len(archive["entries"]) == revision == 11

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=spectator) == final
        verify_replay(restarted, game, operations=("patch",))


def card_patch_vote_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29, "cheats_enabled": True,
        })
        game = "/games/" + opened["game_id"]
        arien, brogan = "hero_arien", "hero_brogan"
        seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
        spectator = opened["spectator_token"]

        def state(actor):
            return request(server, game, token=seats[actor])

        def cards(view, zone):
            return [card["id"] for card in hero(view, arien)[zone] if card and "id" in card]

        had_notice = False

        def vote(op, proposer=arien, *, rejection=None, private=False, **args):
            nonlocal had_notice
            voter = brogan if proposer == arien else arien
            revision = state(proposer)["revision"]
            with sockets(server, game, seats[proposer]) as first, sockets(server, game, seats[voter]) as second:
                for socket in (first, second):
                    packet(socket, "STATE_UPDATE", revision)
                    if had_notice:
                        packet(socket, "OVERRIDE_RESOLVED")
                first.send(json.dumps({"type": "PROPOSE_OVERRIDE", "family": "patch", "op": op, "args": args}))
                if private:
                    error = packet(first, "ERROR")
                    assert "Only the owner" in json.dumps(error), error
                    return
                offer = packet(first, "OVERRIDE_PROPOSED")
                assert offer["op"] == op and offer["args"] == args, offer
                assert packet(second, "OVERRIDE_PROPOSED") == offer
                second.send(json.dumps({
                    "type": "VOTE_OVERRIDE", "proposal_id": offer["proposal_id"], "approve": True,
                }))
                outcome = packet(first, "OVERRIDE_RESOLVED")
                had_notice = True
                assert packet(second, "OVERRIDE_RESOLVED") == outcome
                if rejection:
                    assert outcome["outcome"] == "rejected" and outcome["reason"]["message"] == rejection, outcome
                    assert state(proposer)["revision"] == revision
                    return
                assert outcome["outcome"] == "applied", outcome
                for socket in (first, second):
                    packet(socket, "STATE_UPDATE", revision + 1)
            return state(arien)

        card = "aspiring_duelist"
        vote("move_card", brogan, hero_id=arien, card_id=card, zone="discard", private=True)
        vote("swap_item_card", brogan, hero_id=arien, color="BLUE", tier="II", private=True)
        vote("move_card", hero_id=arien, card_id=card, zone="played", slot=1,
             rejection="No available played slot for a finished turn")
        view = vote("move_card", hero_id=arien, card_id=card, zone="discard")
        assert card not in cards(view, "hand") and card in cards(view, "discard_pile")
        view = vote("move_card", brogan, hero_id=arien, card_id=card, zone="hand")
        assert card in cards(view, "hand") and card not in cards(view, "discard_pile")
        view = vote("move_card", hero_id=arien, card_id=card, zone="deck")
        assert card in cards(view, "deck") and card not in cards(view, "hand")
        assert next(row for row in hero(view, arien)["deck"] if row["id"] == card)["state"] == "DECK"
        vote("move_card", hero_id=arien, card_id=card, zone="hand")

        request(server, game + "/cards", token=seats[arien], body={"card_id": card})
        vote("move_card", brogan, hero_id=arien, card_id=card, zone="discard", private=True)
        view = vote("move_card", hero_id=arien, card_id=card, zone="discard")
        assert hero(view, arien)["current_turn_card"] is None
        assert card in cards(view, "discard_pile")
        vote("move_card", hero_id=arien, card_id=card, zone="hand")

        request(server, game + "/cheats/gold", token=seats[arien], body={"hero_id": arien, "amount": 1})
        request(server, game + "/cards", token=seats[arien], body={"card_id": card})
        request(server, game + "/cards", token=seats[brogan], body={"card_id": "mad_dash"})

        def respond():
            for actor in (arien, brogan):
                frame = state(actor)
                prompt = frame["input_request"]
                if prompt is None:
                    continue
                choices = [row["id"] for row in prompt["options"]]
                selection = next((item for item in ("HOLD", "CONFIRM", "SKIP", "NO") if item in choices), None)
                assert selection is not None, prompt
                request(server, game + "/input", token=seats[actor], body={
                    "request_id": prompt["request_id"], "selection": selection,
                })
                return
            raise AssertionError("Expected a pending resolution prompt")

        for _ in range(12):
            if state(arien)["view"]["turn"] == 2:
                break
            respond()
        else:
            raise AssertionError("The first turn did not finish")

        view = state(arien)
        assert cards(view, "played_cards") == [card]
        other = cards(view, "hand")[0]
        vote("move_card", hero_id=arien, card_id=other, zone="played", slot=1,
             rejection="No available played slot for a finished turn")
        view = vote("move_card", hero_id=arien, card_id=card, zone="hand")
        assert hero(view, arien)["played_cards"] == [None]
        view = vote("move_card", hero_id=arien, card_id=card, zone="played")
        assert cards(view, "played_cards") == [card]

        for _ in range(48):
            view = state(arien)
            if view["view"]["phase"] == "LEVEL_UP":
                break
            if view["view"]["phase"] == "PLANNING":
                for actor in (arien, brogan):
                    available = hero(state(actor), actor)["hand"]
                    assert available
                    request(server, game + "/cards", token=seats[actor], body={"card_id": available[0]["id"]})
            else:
                respond()
        else:
            raise AssertionError("The round did not reach the upgrade prompt")

        upgrade = view["input_request"]
        assert upgrade["type"] == "UPGRADE_PHASE", upgrade
        options = upgrade["players"][arien]["options"]
        pair = options[0]
        choice, item = pair["pair"]
        request(server, game + "/input", token=seats[arien], body={
            "request_id": upgrade["request_id"], "selection": {"hero_id": arien, "card_id": choice},
        })
        before = state(arien)
        assert choice in cards(before, "hand") and item not in cards(before, "hand")
        assert next(row for row in hero(before, arien)["deck"] if row["id"] == item)["state"] == "ITEM"
        view = vote("swap_item_card", hero_id=arien, color=pair["color"], tier=pair["tier"])
        assert item in cards(view, "hand") and choice not in cards(view, "hand")
        deck = {row["id"]: row for row in hero(view, arien)["deck"]}
        assert deck[choice]["state"] == "ITEM" and deck[item]["state"] == "HAND"
        previous_items = hero(before, arien)["items"]
        items = hero(view, arien)["items"]
        old_stat, new_stat = deck[item]["item"], deck[choice]["item"]
        if old_stat == new_stat:
            assert items == previous_items
        else:
            assert items.get(old_stat, 0) == previous_items[old_stat] - 1
            assert items[new_stat] == previous_items.get(new_stat, 0) + 1

        assert view["view"]["phase"] == "PLANNING", (view["view"]["phase"], view["input_request"])
        request(server, game + "/cards", token=seats[arien], body={"card_id": item})
        view = vote("swap_item_card", hero_id=arien, color=pair["color"], tier=pair["tier"])
        committed = hero(view, arien)["current_turn_card"]
        assert committed["id"] == choice and committed["is_facedown"]
        hidden = hero(state(brogan), arien)["current_turn_card"]
        assert hidden["is_facedown"] and "id" not in hidden
        available = hero(state(brogan), brogan)["hand"]
        request(server, game + "/cards", token=seats[brogan], body={"card_id": available[0]["id"]})
        revealed = [row for row in state(brogan)["view"]["unresolved_cards"]
                    if row.get("hero_id") == arien]
        assert any(row["card"]["id"] == choice for row in revealed), revealed

        final = request(server, game, token=spectator)
        archive = verify_replay(server, game, operations=("command", "patch"))
        operations = [entry["decision"]["operation"] for entry in archive["entries"]]
        assert [op["command"]["op"] for op in operations if op["kind"] == "patch"] == ["move_card"] * 8 + ["swap_item_card"] * 2

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=spectator) == final
        verify_replay(restarted, game, operations=("patch",))


def effect_patch_vote_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", expect=201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29,
        })
        game = "/games/" + opened["game_id"]
        arien, brogan = "hero_arien", "hero_brogan"
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        spectator = opened["spectator_token"]
        card = "aspiring_duelist"
        for actor, chosen in [(arien, card), (brogan, "mad_dash")]:
            request(server, game + "/cards", token=seats[actor], body={"card_id": chosen})

        def movement(view):
            return next(option["value"] for option in view["input_request"]["options"]
                        if option["id"] == "MOVEMENT")

        def effect(id, kind, **fields):
            return {"id": id, "source_id": arien, "effect_type": kind,
                    "duration": "THIS_TURN", "created_at_turn": 1, "created_at_round": 1,
                    "scope": {"shape": "global", "affects": "self"}, **fields}

        modifier = effect("table_speed", "area_stat_modifier", source_card_id=card, is_active=True,
                          stat_type="MOVEMENT", stat_value=2)
        premature = effect("table_stop", "delayed_trigger", source_card_id=card, is_active=True,
                           finishing_steps=[{"type": "place_marker", "marker_type": "venom",
                                             "target_id": brogan, "value": -1}])
        delayed = effect("table_trigger", "delayed_trigger", is_active=True, finishing_steps=[
            {"type": "set_context_flag", "key": "table_effect_targets", "value": [brogan]},
            {"type": "for_each", "list_key": "table_effect_targets", "item_key": "table_effect_target",
             "current_index": 0, "steps_template": [
                 {"type": "place_marker", "marker_type": "poison", "target_key": "table_effect_target",
                  "value": -2},
             ]},
        ])
        with sockets(server, game, seats[arien]) as first, sockets(server, game, seats[brogan]) as second:
            revision = 2
            before = packet(first, "STATE_UPDATE", revision)
            packet(second, "STATE_UPDATE", revision)
            base_movement = movement(before)
            stale_id = before["input_request"]["request_id"]

            def propose(op, args):
                first.send(json.dumps({
                    "type": "PROPOSE_OVERRIDE", "family": "patch", "op": op, "args": args,
                }))

            def vote(op, *, rejection=None, **args):
                nonlocal revision
                propose(op, args)
                offer = packet(first, "OVERRIDE_PROPOSED")
                assert offer["op"] == op and offer["args"] == args, offer
                assert packet(second, "OVERRIDE_PROPOSED") == offer
                second.send(json.dumps({
                    "type": "VOTE_OVERRIDE", "proposal_id": offer["proposal_id"], "approve": True,
                }))
                outcome = packet(first, "OVERRIDE_RESOLVED")
                assert packet(second, "OVERRIDE_RESOLVED") == outcome
                if rejection:
                    assert outcome["outcome"] == "rejected" and outcome["reason"]["message"] == rejection, outcome
                    assert request(server, game, token=spectator)["revision"] == revision
                    return None
                assert outcome["outcome"] == "applied", outcome
                revision += 1
                view = packet(first, "STATE_UPDATE", revision)
                packet(second, "STATE_UPDATE", revision)
                return view

            propose("add_effect", {"effect": {"id": "invalid"}})
            packet(first, "ERROR")
            vote("add_effect", effect=effect("invalid", "delayed_trigger",
                                            finishing_steps=[{"type": "not_a_step"}]),
                 rejection="Unsupported effect finishing steps")
            vote("remove_effect", effect_id="unknown", rejection="No active effect matches that ID")

            view = vote("add_effect", effect=modifier)
            assert movement(view) == base_movement + 2
            assert view["input_request"]["request_id"] != stale_id
            assert hero(view, arien)["current_turn_card"]["is_active"]
            request(server, game + "/input", token=seats[arien], expect=422,
                    body={"request_id": stale_id, "selection": "HOLD"})
            vote("add_effect", effect=modifier, rejection="An effect with that ID is already active")
            vote("add_effect", effect=premature)
            view = vote("remove_effect", effect_id=modifier["id"])
            assert movement(view) == base_movement
            assert hero(view, arien)["current_turn_card"]["is_active"]
            view = vote("remove_effect", effect_id=premature["id"])
            assert not hero(view, arien)["current_turn_card"]["is_active"]
            assert not view["view"]["markers"].get("venom", {}).get("target_id")
            view = vote("add_effect", effect=delayed)
            assert [row["id"] for row in view["view"]["effects"]] == [delayed["id"]]

        # Restore the stored finishing program before it runs.
        suspended = request(server, game, token=spectator)
        verify_replay(server, game, operations=("patch", "command"))

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=spectator) == suspended
        for _ in range(12):
            public = request(restarted, game, token=spectator)
            if public["view"]["turn"] == 2:
                break
            actor, = public["awaiting_input"]
            state = request(restarted, game, token=seats[actor])
            prompt = state["input_request"]
            options = [row["id"] for row in prompt["options"]]
            choice = "HOLD" if "HOLD" in options else "CONFIRM"
            assert choice in options, prompt
            request(restarted, game + "/input", token=seats[actor],
                    body={"request_id": prompt["request_id"], "selection": choice})
        else:
            raise AssertionError("The round did not complete its first turn")
        assert not public["view"]["effects"]
        assert public["view"]["markers"]["poison"] == {"target_id": brogan, "source_id": arien, "value": -2}
        assert not public["view"]["markers"].get("venom", {}).get("target_id")
        verify_replay(restarted, game, operations=("patch", "command"))


def shutdown_smoke(binary, pack, database):
    if os.name != "posix":
        return

    def restart_close(socket, after_revision=None):
        try:
            if after_revision is not None:
                packet(socket, "STATE_UPDATE", after_revision)
            socket.recv(timeout=8)
        except ConnectionClosed as error:
            assert error.rcvd is not None and error.rcvd.code == 1012, error
        else:
            raise AssertionError("Expected a service restart close frame")

    setup = {"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29}
    with serve(binary, pack, database, process_handle=True) as (server, process):
        opened = request(server, "/games", expect=201, body=setup)
        game = "/games/" + opened["game_id"]
        arien, brogan = (seat["token"] for seat in opened["player_tokens"])
        spectator = opened["spectator_token"]
        receipt_args = {"card_id": "aspiring_duelist", "client_action_id": "before-stop",
                        "expected_revision": 0}
        receipt = request(server, game + "/cards", token=arien, body=receipt_args)
        request(server, game + "/cards", token=brogan, body={"card_id": "mad_dash"})
        pending = request(server, game, token=arien)
        assert pending["input_request"]["type"] == "CHOOSE_ACTION"

        timed = request(server, "/games", expect=201, body={
            **setup, "time_control": {
                "planning_allowance_seconds": 60, "resolution_allowance_seconds": 60,
                "response_grant_seconds": 0, "initial_time_bank_seconds": 0,
                "time_bank_increment_seconds": 0, "max_time_bank_seconds": 0,
                "upgrade_allowance_seconds": 60,
            },
        })
        timed_game = "/games/" + timed["game_id"]
        red, blue = (seat["token"] for seat in timed["player_tokens"])
        for token in (red, blue):
            request(server, timed_game + "/ready", token=token, body={"ready": True})

        with (
            sockets(server, game, arien) as player,
            sockets(server, game, spectator) as watcher,
            sockets(server, timed_game, red) as proposer,
            sockets(server, timed_game, blue) as voter,
        ):
            for socket in (player, watcher, proposer, voter):
                packet(socket, "STATE_UPDATE", 2)
            proposer.send(json.dumps({"type": "PROPOSE_OVERRIDE", "family": "pause"}))
            proposal = packet(proposer, "OVERRIDE_PROPOSED")
            assert packet(voter, "OVERRIDE_PROPOSED") == proposal
            for socket in (proposer, voter):
                assert packet(socket, "STATE_UPDATE", 3)["view"]["clock"]["active"] is None

            process.terminate()
            for socket in (player, watcher, proposer, voter):
                restart_close(socket)
            assert process.wait(timeout=12) == 0, process.stderr.read()

    with serve(binary, pack, database, process_handle=True) as (restarted, process):
        assert request(restarted, game, token=arien) == pending
        assert request(restarted, game + "/cards", token=arien, body=receipt_args) == receipt
        assert request(restarted, game, token=spectator)["revision"] == 2
        verify_replay(restarted, game, operations=("command",))
        with (
            sockets(restarted, timed_game, red) as player,
            sockets(restarted, game, arien) as deciding,
        ):
            resumed = packet(player, "STATE_UPDATE", 4)
            assert resumed["view"]["clock"]["active"]["kind"] == "PLANNING"
            notice = packet(player, "OVERRIDE_RESOLVED")
            assert notice["outcome"] == "cancelled" and notice["proposal_id"] == proposal["proposal_id"]
            verify_replay(restarted, timed_game, operations=("pause_opened", "pause_released"))

            prompt = packet(deciding, "STATE_UPDATE", 2)["input_request"]
            during_stop = {"request_id": prompt["request_id"], "selection": "HOLD",
                           "client_action_id": "during-stop", "expected_revision": 2}
            with sqlite3.connect(database) as lock:
                lock.execute("BEGIN IMMEDIATE")
                deciding.send(json.dumps({"type": "SUBMIT_INPUT", **during_stop}))
                # A reader waits on the per-game mutex while the command is
                # inside its SQLite transaction. Signal before releasing it.
                for _ in range(8):
                    try:
                        request(restarted, game, token=arien, timeout=0.25)
                    except TimeoutError:
                        break
                else:
                    raise AssertionError("The pending write did not hold the game mutex")
                process.send_signal(signal.SIGINT)
                for _ in range(20):
                    try:
                        health = request(restarted, "/health", expect=(200, 503), timeout=0.2)
                        if not health.get("ok"):
                            break
                    except OSError:
                        break
                    time.sleep(0.02)
                else:
                    raise AssertionError("The service did not stop accepting requests")
                lock.rollback()
            committed = packet(deciding, "ACTION_RESULT", 3)
            assert committed.pop("client_action_id") == during_stop["client_action_id"]
            assert committed.pop("type") == "ACTION_RESULT"
            restart_close(deciding, after_revision=3)
            restart_close(player)
            assert process.wait(timeout=12) == 0, process.stderr.read()

    with serve(binary, pack, database) as reopened:
        assert request(reopened, game + "/input", token=arien, body=during_stop) == committed
        assert request(reopened, game, token=spectator)["revision"] == 3
        verify_replay(reopened, game, operations=("command",))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("content", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="goa2-server-smoke-") as root:
        smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "games.sqlite")
        deployment_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "deployment.sqlite")
        clock_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "clocks.sqlite")
        bot_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "bots.sqlite")
        timed_bot_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "timed-bots.sqlite")
        reopened_bot_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "reopened-bot.sqlite")
        pause_vote_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "pause-votes.sqlite")
        rewind_vote_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "rewind-votes.sqlite")
        unstick_vote_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "unstick-votes.sqlite")
        patch_vote_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "patch-votes.sqlite")
        board_patch_vote_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "board-patch-votes.sqlite")
        card_patch_vote_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "card-patch-votes.sqlite")
        effect_patch_vote_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "effect-patch-votes.sqlite")
        shutdown_smoke(args.binary.resolve(), args.content.resolve(), Path(root) / "shutdown.sqlite")
    print("Rust service passed scoped HTTP/WS, failure/retry, capture, deployment, clocks, restart, timed bots, table votes, live replay and mixed-play checks")


if __name__ == "__main__":
    main()
