"""Disposable Rust service contract verification; no live service is contacted."""

import json
import sqlite3
import subprocess
from concurrent.futures import ThreadPoolExecutor

from .process import hero, packet, request, serve, sockets, verify_lab_gold_cheat, verify_replay


def smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        identity = json.loads(subprocess.check_output([binary, "--check-content", pack], text=True))
        assert identity["engine"] == "rust"
        assert request(server, "/health") == {"ok": True, **identity}
        request(server, "/__lab/health", expect=401)
        assert "Arien" in request(server, "/heroes")
        verify_lab_gold_cheat(server)
        setup = {
            "red_heroes": ["Arien", "Brogan"],
            "blue_heroes": ["Tigerclaw", "Sabina"],
            "seed": 29,
            "player_names": {"Arien": "  Lab Arien  "},
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
        request(
            server,
            game + "/cards",
            token=spectator,
            body={"card_id": "aspiring_duelist"},
            expect=403,
        )
        request(server, game + "/advance", token=spectator, method="POST", expect=403)
        request(server, game + "/advance", token=arien, method="POST", expect=400)

        first = {
            "card_id": "aspiring_duelist",
            "expected_revision": 0,
            "client_action_id": "arien-commit",
        }
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
        request(
            server,
            game + "/cards",
            token=arien,
            body={**first, "card_id": "spell_break"},
            expect=409,
        )
        request(server, game + "/uncommit", token=arien, body={"expected_revision": 0}, expect=409)

        with sqlite3.connect(database) as db:
            db.execute(
                "CREATE TRIGGER reject_update BEFORE INSERT ON replay_entries "
                "BEGIN SELECT RAISE(ABORT, 'injected storage error'); END"
            )
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

        for revision, (actor, card) in enumerate(
            [
                ("hero_arien", "aspiring_duelist"),
                ("hero_brogan", "mad_dash"),
                ("hero_tigerclaw", "hit_and_run"),
                ("hero_sabina", "listen_up"),
            ],
            start=2,
        ):
            last = request(
                server,
                game + "/cards",
                token=seats[actor],
                body={"card_id": card, "expected_revision": revision},
            )
        assert last["revision"] == 6
        revealed = request(server, history, token=spectator)["decisions"]
        assert len(revealed) == 6 and [row["index"] for row in revealed] == list(range(6))
        assert revealed[2]["label"] == "Lab Arien committed Aspiring Duelist"
        (responder,) = request(server, lab_game + "/responders", lab=True)["players"]
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
        assert {
            hero["id"] for team in snapshot["state"]["teams"].values() for hero in team["heroes"]
        } == seats.keys()
        assert request(
            server,
            "/__lab/validate-snapshot",
            lab=True,
            body={"checkpoint_json": snapshot["native_checkpoint_json"]},
        ) == {"ok": True}
        request(
            server, "/__lab/validate-snapshot", lab=True, expect=422, body={"checkpoint_json": "{}"}
        )
        cloned = request(
            server,
            "/__lab/restore",
            lab=True,
            expect=201,
            body={
                "checkpoint_json": snapshot["native_checkpoint_json"],
                "hero_names": snapshot["hero_names"],
            },
        )
        clone_seats = {row["hero_id"]: row["token"] for row in cloned["player_tokens"]}
        assert clone_seats[responder] != seats[responder]
        clone_game = f"/games/{cloned['game_id']}"
        request(server, clone_game, token=seats[responder], expect=401)
        assert request(server, clone_game, token=clone_seats[responder]) == pending
        unanchored = {
            key: value for key, value in captured["checkpoint"].items() if key != "rollback"
        }
        early = request(
            server, "/__lab/restore", lab=True, expect=201, body={"checkpoint": unanchored}
        )
        early_game = f"/games/{early['game_id']}"
        early_seats = {row["hero_id"]: row["token"] for row in early["player_tokens"]}
        other = next(token for actor, token in early_seats.items() if actor != responder)
        assert not request(server, early_game, token=early_seats[responder])["input_request"].get(
            "can_rollback"
        )
        request(server, early_game + "/starting-position", token=other, body={"op": "cancel"})
        assert not request(server, early_game, token=early_seats[responder])["input_request"].get(
            "can_rollback"
        )

        def concurrent_input(action_id):
            return request(
                server,
                clone_game + "/input",
                token=clone_seats[responder],
                expect=(200, 409),
                body={
                    "request_id": pending["input_request"]["request_id"],
                    "selection": "FAST_TRAVEL",
                    "expected_revision": 6,
                    "client_action_id": action_id,
                },
            )

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
        request(
            restarted,
            clone_game + "/rollback",
            token=other,
            body={"expected_revision": 7},
            expect=422,
        )
        with sockets(restarted, clone_game, clone_seats[responder]) as player:
            assert packet(player, "STATE_UPDATE", 7)["input_request"] == move_prompt
            player.send(
                json.dumps(
                    {
                        "type": "ROLLBACK",
                        "expected_revision": 7,
                        "client_action_id": "undo-after-restart",
                    }
                )
            )
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
        request(
            restarted,
            clone_game + "/input",
            token=clone_seats[responder],
            expect=422,
            body={"request_id": pending["input_request"]["request_id"], "selection": "FAST_TRAVEL"},
        )
        assert request(
            restarted,
            clone_game + "/rollback",
            token=clone_seats[responder],
            body={"expected_revision": 7, "client_action_id": "undo-after-restart"},
        ) == {k: v for k, v in result.items() if k not in ("type", "client_action_id")}
        assert request(restarted, game, token=seats[responder]) == pending
        assert request(restarted, game + "/cards", token=arien, body=first) == original
        with (
            sockets(restarted, game, spectator) as public,
            sockets(restarted, game, seats[responder]) as player,
        ):
            assert packet(public, "STATE_UPDATE", 6)["awaiting_input"] == [responder]
            assert packet(player, "STATE_UPDATE", 6)["input_request"] == pending["input_request"]
            result = request(
                restarted,
                lab_game + "/command",
                lab=True,
                body={
                    "actor": responder,
                    "expected_revision": 6,
                    "message": {
                        "type": "SUBMIT_INPUT",
                        "request_id": pending["input_request"]["request_id"],
                        "selection": "FAST_TRAVEL",
                    },
                },
            )
            assert result["revision"] == 7
            assert packet(public, "STATE_UPDATE", 7)["input_request"] is None
            move = packet(player, "STATE_UPDATE", 7)["input_request"]
            assert move["type"] == "SELECT_HEX"
            destination = move["valid_hexes"][0]
            player.send(
                json.dumps(
                    {
                        "type": "SUBMIT_INPUT",
                        "request_id": move["request_id"],
                        "selection": destination,
                        "expected_revision": 7,
                        "client_action_id": "move-after-restart",
                    }
                )
            )
            assert packet(player, "ACTION_RESULT", 8)["client_action_id"] == "move-after-restart"
            own = packet(player, "STATE_UPDATE", 8)
            observed = packet(public, "STATE_UPDATE", 8)
            assert own["client_action_id"] == "move-after-restart"
            assert "client_action_id" not in observed
            assert own["view"]["board"]["entity_locations"][responder] == destination
            assert observed["view"]["board"]["entity_locations"][responder] == destination
            assert any(event["to_hex"] == destination for event in observed["events"])
        request(
            restarted,
            lab_game + "/command",
            lab=True,
            expect=409,
            body={
                "actor": responder,
                "expected_revision": 6,
                "message": {
                    "type": "SUBMIT_INPUT",
                    "request_id": pending["input_request"]["request_id"],
                    "selection": "FAST_TRAVEL",
                },
            },
        )
        assert request(restarted, game, token=spectator)["revision"] == 8
        undo = request(
            restarted, game + "/rollback", token=seats[responder], body={"expected_revision": 8}
        )
        assert undo["revision"] == 9
        assert request(restarted, game, token=seats[responder])["view"] == pending["view"]
        replay = verify_replay(
            restarted,
            game,
            credentials=(spectator, *seats.values()),
            operations=("command",),
            tamper=True,
        )
        assert replay["start_revision"] == 0 and replay["revision"] == 9
        replay = verify_replay(restarted, clone_game, operations=("command",))
        assert replay["start_revision"] == 6 and replay["revision"] == 8
        choices = request(restarted, history, token=spectator)["decisions"]
        assert len(choices) == 9 and choices[-1]["type"] == "rollback"
        assert choices[6]["type"] == "input" and "FAST_TRAVEL" not in choices[6]["label"]
        assert pending["input_request"]["request_id"] not in json.dumps(choices)

    with sqlite3.connect(database) as db:
        db.execute("ALTER TABLE replay_entries DROP COLUMN history")
        db.execute("PRAGMA user_version = 0")  # Reconstruct an actual unversioned legacy database.
    with serve(binary, pack, database) as migrated:
        old = request(migrated, history, token=spectator)["decisions"]
        assert [row["type"] for row in old] == [row["type"] for row in choices]
        assert all(row["round"] is None and row["turn"] is None for row in old)

    # Simulate a database written by the server before live journals existed.
    with sqlite3.connect(database) as db:
        db.execute("DROP TABLE replay_entries")
        db.execute("DROP TABLE replay_starts")
        db.execute("PRAGMA user_version = 0")
    with serve(binary, pack, database) as migrated:
        replay = verify_replay(migrated, game)
        assert replay["start_revision"] == replay["revision"] == 9
        prompt = request(migrated, game, token=seats[responder])["input_request"]
        request(
            migrated,
            game + "/input",
            token=seats[responder],
            body={
                "request_id": prompt["request_id"],
                "selection": "FAST_TRAVEL",
                "expected_revision": 9,
            },
        )
        replay = verify_replay(migrated, game, operations=("command",))
        assert replay["start_revision"] == 9 and replay["revision"] == 10


def deployment_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        opened = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien", "Hanu"],
                "blue_heroes": ["Brogan", "Tigerclaw"],
                "seed": 20261017,
            },
        )
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
        request(
            server,
            route,
            token=arien,
            expect=422,
            body={"op": "move", "destination": locations["hero_brogan"]},
        )
        request(server, route, token=spectator, expect=403, body={"op": "cancel"})
        destination = position["destinations"][0]
        moved = request(
            server,
            route,
            token=arien,
            body={
                "op": "move",
                "destination": destination,
                "expected_revision": 0,
            },
        )
        assert moved["revision"] == 1
        snapshot = request(server, "/__lab" + game + "/checkpoint", lab=True)
        assert (
            snapshot["checkpoint"]["envelope"]["state"]["last_turn_positions"]["hero_arien"]
            == destination
        )
        with sockets(server, game, arien) as player:
            assert (
                packet(player, "STATE_UPDATE", 1)["view"]["board"]["entity_locations"]["hero_arien"]
                == destination
            )
            player.send(
                json.dumps(
                    {
                        "type": "STARTING_POSITION",
                        "op": "request_swap",
                        "target": "hero_hanu",
                        "expected_revision": 1,
                    }
                )
            )
            packet(player, "STARTING_POSITION_UPDATED", 2)
            (row,) = packet(player, "STATE_UPDATE", 2)["view"]["starting_position"]["requests"]
        assert row["from"] == "hero_arien" and row["to"] == "hero_hanu"
        assert not request(server, game, token=brogan)["view"]["starting_position"]["requests"]
        accept = {"op": "respond_swap", "request_id": row["id"], "accept": True}
        request(server, route, token=brogan, expect=422, body=accept)

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=hanu)["view"]["starting_position"]["requests"] == [
            row
        ]
        result = request(restarted, route, token=hanu, body={**accept, "expected_revision": 2})
        assert result["revision"] == 3
        swapped = request(restarted, game, token=arien)["view"]
        assert swapped["board"]["entity_locations"]["hero_hanu"] == destination
        assert swapped["board"]["entity_locations"]["hero_arien"] == locations["hero_hanu"]
        assert not swapped["starting_position"]["requests"]
        request(restarted, route, token=hanu, expect=422, body=accept)
        for respond in ({"op": "respond_swap", "accept": False}, {"op": "cancel"}):
            request(
                restarted, route, token=arien, body={"op": "request_swap", "target": "hero_hanu"}
            )
            (item,) = request(restarted, game, token=hanu)["view"]["starting_position"]["requests"]
            assert item["id"] != row["id"]
            request(restarted, route, token=hanu, body={**respond, "request_id": item["id"]})
            assert not request(restarted, game, token=arien)["view"]["starting_position"][
                "requests"
            ]
        request(restarted, route, token=arien, body={"op": "request_swap", "target": "hero_hanu"})
        (item,) = request(restarted, game, token=hanu)["view"]["starting_position"]["requests"]
        request(restarted, game + "/cards", token=arien, body={"card_id": "aspiring_duelist"})
        assert request(restarted, game, token=arien)["view"]["starting_position"] is None
        assert not request(restarted, game, token=hanu)["view"]["starting_position"]["requests"]
        request(restarted, route, token=hanu, expect=422, body={**accept, "request_id": item["id"]})
