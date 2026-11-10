"""Disposable Rust service contract verification; no live service is contacted."""

import json
import sqlite3
import time

from .process import hero, packet, request, serve, sockets, verify_replay, wait_view


def bot_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        setup = {"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29}
        request(
            server, "/games", body={**setup, "bots": {"hero_hanu": {"kind": "rust"}}}, expect=422
        )
        with sqlite3.connect(database) as db:
            db.execute(
                "CREATE TRIGGER bot_restart BEFORE UPDATE ON games "
                "BEGIN SELECT RAISE(ABORT, 'bot pending across restart'); END"
            )
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
        request(
            server,
            game + "/cards",
            token=arien,
            body={"card_id": "liquid_leap", "expected_revision": 1},
        )
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
            request(
                server,
                game + "/input",
                token=arien,
                expect=(200, 409),
                body={
                    "request_id": prompt["request_id"],
                    "selection": choice,
                    "expected_revision": state["revision"],
                },
            )
        else:
            raise AssertionError("Mixed game did not complete its first turn")


def timed_bot_smoke(binary, pack, database):
    setup = {
        "red_heroes": ["Arien"],
        "blue_heroes": ["Brogan"],
        "seed": 29,
        "bots": {"hero_brogan": {"kind": "rust"}},
        "time_control": {
            "planning_allowance_seconds": 12,
            "resolution_allowance_seconds": 0,
            "response_grant_seconds": 0,
            "initial_time_bank_seconds": 0,
            "time_bank_increment_seconds": 0,
            "max_time_bank_seconds": 0,
            "upgrade_allowance_seconds": 0,
            "automatic_turn_limit": 1,
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
        request(
            server, game + "/ready", token=seats["hero_brogan"], body={"ready": True}, expect=403
        )
        snapshot = request(server, lab_game + "/snapshot", lab=True)["checkpoint"]
        request(
            server,
            "/__lab/restore",
            lab=True,
            expect=422,
            body={
                "checkpoint_json": snapshot["native_checkpoint_json"],
                "freeze_clock": True,
                "bots": setup["bots"],
            },
        )
        time.sleep(0.15)
        assert request(server, game, token=spectator)["revision"] == waiting["revision"]

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=spectator)["view"]["clock"]["ready_hero_ids"] == [
            "hero_brogan"
        ]
        with sockets(restarted, game, spectator) as public:
            packet(public, "STATE_UPDATE", waiting["revision"])
            ready = request(
                restarted, game + "/ready", token=seats["hero_arien"], body={"ready": True}
            )
            assert ready["view"]["clock"]["status"] == "RUNNING"
            saw_bot_plan = False
            for _ in range(36):
                update = json.loads(public.recv(timeout=17))
                if update.get("type") != "STATE_UPDATE":
                    continue
                clock = update["view"]["clock"]
                brogan = clock["players"]["hero_brogan"]
                if update["view"]["phase"] == "PLANNING" and update["view"]["turn"] == 1:
                    saw_bot_plan |= (
                        brogan["planning_complete"] and not brogan["planning_locked_by_timeout"]
                    )
                if clock["status"] == "SUSPENDED_FOR_INACTIVITY":
                    assert update["view"]["turn"] == 2
                    break
            else:
                raise AssertionError("Timed bot match did not suspend after the unattended turn")
            assert (
                saw_bot_plan
            ), "Brogan's own choice should finish planning before the clock fallback"
        assert request(restarted, lab_game + "/responders", lab=True)["players"] == ["hero_arien"]
        replay = verify_replay(restarted, game, operations=("command", "timeout"))
        assert replay["bot_seats"] == ["hero_brogan"]
        resumed = request(
            restarted, game + "/ready", token=seats["hero_arien"], body={"ready": True}
        )
        clock = resumed["view"]["clock"]
        assert clock["status"] == "RUNNING"
        assert clock["ready_hero_ids"] == ["hero_arien", "hero_brogan"]

        resolution = request(
            restarted,
            "/games",
            lab=True,
            expect=201,
            body={
                **setup,
                "time_control": {
                    **setup["time_control"],
                    "resolution_allowance_seconds": 30,
                },
            },
        )
        game = "/games/" + resolution["game_id"]
        human = next(
            row["token"] for row in resolution["player_tokens"] if row["hero_id"] == "hero_arien"
        )
        with sockets(restarted, game, resolution["spectator_token"]) as public:
            packet(public, "STATE_UPDATE", 0)
            request(restarted, game + "/ready", token=human, body={"ready": True})
            request(restarted, game + "/cards", token=human, body={"card_id": "aspiring_duelist"})
            bot_prompt = None
            for _ in range(12):
                update = json.loads(public.recv(timeout=17))
                if update.get("type") != "STATE_UPDATE":
                    continue
                assert all(
                    event["event_type"] != "TIMER_EXPIRED" for event in update.get("events", [])
                )
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
                    request(
                        restarted,
                        game + "/input",
                        token=human,
                        body={
                            "request_id": offered["request_id"],
                            "selection": selection,
                        },
                    )
            else:
                raise AssertionError("The bot did not answer its timed resolution prompt")


def reopened_bot_smoke(binary, pack, database):
    setup = {
        "red_heroes": ["Arien"],
        "blue_heroes": ["Brogan"],
        "seed": 29,
        "bots": {"hero_brogan": {"kind": "rust"}},
        "time_control": {
            "planning_allowance_seconds": 30,
            "resolution_allowance_seconds": 30,
            "response_grant_seconds": 0,
            "initial_time_bank_seconds": 0,
            "time_bank_increment_seconds": 0,
            "max_time_bank_seconds": 0,
            "upgrade_allowance_seconds": 30,
        },
    }

    def patch_lives(player, amount, revision):
        player.send(
            json.dumps(
                {
                    "type": "PROPOSE_OVERRIDE",
                    "family": "patch",
                    "op": "set_life_counters",
                    "args": {"team": "RED", "value": amount},
                }
            )
        )
        packet(player, "OVERRIDE_PROPOSED")
        assert packet(player, "OVERRIDE_RESOLVED")["outcome"] == "applied"
        return packet(player, "STATE_UPDATE", revision)

    with serve(binary, pack, database) as server:
        opened = request(server, "/games", lab=True, body=setup, expect=201)
        game = "/games/" + opened["game_id"]
        human = next(
            seat["token"] for seat in opened["player_tokens"] if seat["hero_id"] == "hero_arien"
        )
        with sockets(server, game, human) as player:
            assert (
                packet(player, "STATE_UPDATE", 0)["view"]["clock"]["status"]
                == "WAITING_FOR_PLAYERS"
            )
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
