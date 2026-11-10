"""Disposable Rust service contract verification; no live service is contacted."""

import json
import sqlite3
import time

from .process import hero, packet, request, serve, sockets, verify_replay, wait_view


def clock_smoke(binary, pack, database):
    advisory = {
        "planning_allowance_seconds": 5,
        "resolution_allowance_seconds": 1,
        "initiative_bonus_seconds": 2,
        "response_grant_seconds": 1,
        "initial_time_bank_seconds": 0,
        "time_bank_increment_seconds": 0,
        "max_time_bank_seconds": 10,
        "upgrade_allowance_seconds": 1,
        "enforce_timeouts": False,
    }
    setup = {"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29}
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", body={**setup, "time_control": advisory}, expect=201)
        game = f"/games/{opened['game_id']}"
        lab_game = "/__lab" + game
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        arien, brogan = seats["hero_arien"], seats["hero_brogan"]
        assert (
            request(server, game, token=arien)["view"]["clock"]["status"] == "WAITING_FOR_PLAYERS"
        )
        request(
            server, game + "/cards", token=arien, expect=422, body={"card_id": "aspiring_duelist"}
        )
        with sockets(server, game, arien) as player:
            packet(player, "STATE_UPDATE", 0)
            player.send(json.dumps({"type": "SET_READY", "ready": True}))
            assert packet(player, "READY_UPDATED", 1)["hero_id"] == "hero_arien"
            assert packet(player, "STATE_UPDATE", 1)["view"]["clock"]["ready_hero_ids"] == [
                "hero_arien"
            ]
        running = request(server, game + "/ready", token=brogan, body={"ready": True})["view"][
            "clock"
        ]
        assert running["status"] == "RUNNING"
        assert running["active"]["running_hero_ids"] == ["hero_arien", "hero_brogan"]
        before = request(server, lab_game + "/checkpoint", lab=True)
        time.sleep(0.08)
        after = request(server, game, token=arien)["view"]["clock"]
        assert after["server_now_ms"] > running["server_now_ms"]
        assert (
            after["players"]["hero_arien"]["planning_allowance_ms"]
            < running["players"]["hero_arien"]["planning_allowance_ms"]
        )
        assert request(server, lab_game + "/checkpoint", lab=True) == before
        snapshot = request(server, lab_game + "/snapshot", lab=True)["checkpoint"]
        saved_clock = json.loads(snapshot["native_checkpoint_json"])["envelope"]["state"]["clock"]
        assert saved_clock["last_settled_at_ms"] >= after["server_now_ms"]
        assert (
            saved_clock["players"]["hero_arien"]["planning_allowance_ms"]
            <= after["players"]["hero_arien"]["planning_allowance_ms"]
        )
        frozen = request(
            server,
            "/__lab/restore",
            lab=True,
            expect=201,
            body={
                "checkpoint_json": snapshot["native_checkpoint_json"],
                "freeze_clock": True,
            },
        )

        def frozen_view():
            return request(server, "/games/" + frozen["game_id"], token=frozen["spectator_token"])[
                "view"
            ]["clock"]

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
        (actor,) = resolution["active"]["hero_ids"]
        assert resolution["players"][actor]["initiative_bonus_ms"] > 0

        enforcing = {
            **advisory,
            "planning_allowance_seconds": 0,
            "resolution_allowance_seconds": 0,
            "initiative_bonus_seconds": 0,
            "response_grant_seconds": 0,
            "upgrade_allowance_seconds": 0,
            "enforce_timeouts": True,
            "automatic_turn_limit": 1,
        }
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
            assert {
                event["actor_id"]
                for event in timeouts
                if event["metadata"]["clock_kind"] == "PLANNING"
            } == {"hero_arien", "hero_brogan"}
            assert all("card_id" not in event["metadata"] for event in timeouts)
        suspended = request(server, timed_game, token=spectator)
        assert suspended["view"]["clock"]["ready_hero_ids"] == []
        restarting = request(
            server,
            "/games",
            body={
                **setup,
                "time_control": {
                    **enforcing,
                    "planning_allowance_seconds": 2,
                },
            },
            expect=201,
        )
        restarting_game = f"/games/{restarting['game_id']}"
        for row in restarting["player_tokens"]:
            request(server, restarting_game + "/ready", token=row["token"], body={"ready": True})
        assert (
            request(server, restarting_game, token=restarting["spectator_token"])["revision"] == 2
        )

    with serve(binary, pack, database) as restarted:
        automatic = wait_view(
            restarted,
            restarting_game,
            restarting["spectator_token"],
            lambda value: value["view"]["clock"]["status"] == "SUSPENDED_FOR_INACTIVITY",
            timeout=10,
        )
        assert automatic["view"]["turn"] == 2
        verify_replay(restarted, restarting_game, operations=("command", "timeout"))
        assert (
            request(restarted, timed_game, token=spectator)["view"]["clock"]["status"]
            == "SUSPENDED_FOR_INACTIVITY"
        )
        resumed = request(restarted, game, token=arien)["view"]["clock"]
        assert resumed["status"] == "RUNNING"
        assert resumed["active"]["request_id"] == resolution["active"]["request_id"]


def pause_vote_smoke(binary, pack, database):
    setup = {
        "red_heroes": ["Arien", "Hanu"],
        "blue_heroes": ["Brogan", "Tigerclaw"],
        "seed": 29,
        "time_control": {
            "planning_allowance_seconds": 60,
            "resolution_allowance_seconds": 60,
            "response_grant_seconds": 20,
            "initial_time_bank_seconds": 0,
            "time_bank_increment_seconds": 0,
            "max_time_bank_seconds": 0,
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
            if value["type"] == kind and all(
                value.get(key) == expected for key, expected in fields.items()
            ):
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
            "type": "PLAYER_LINK_REVEALED",
            "hero_id": target,
            "token": token,
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
                assert (
                    packet(socket, "STATE_UPDATE", 6)["view"]["clock"]["active"]["kind"]
                    == "PLANNING"
                )
        archive = verify_replay(
            server,
            running_game,
            credentials=[*running_seats.values(), running_spectator],
            operations=("pause_opened", "pause_released"),
        )
        assert [entry["decision"]["operation"]["kind"] for entry in archive["entries"]][-2:] == [
            "pause_opened",
            "pause_released",
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
            assert (
                request(server, game, token=spectator)["view"]["clock"]["players"]
                == held["players"]
            )
            request(
                server,
                game + "/cards",
                token=seats["hero_arien"],
                expect=422,
                body={"card_id": "aspiring_duelist"},
            )
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
                assert (
                    0
                    <= held["players"][name]["planning_allowance_ms"]
                    - resumed["players"][name]["planning_allowance_ms"]
                    < 140
                )

            vote = propose(first)
            send(first, "CANCEL_OVERRIDE", proposal_id=vote)
            resolved(first, vote, "cancelled")
            vote = propose(first)
            resolved(first, vote, "expired")
            assert (
                request(server, game, token=spectator)["view"]["clock"]["active"]["kind"]
                == "PLANNING"
            )

            for name, token in seats.items():
                card = hero(request(server, game, token=token), name)["hand"][0]["id"]
                request(server, game + "/cards", token=token, body={"card_id": card})
            before = request(server, game, token=spectator)
            assert before["view"]["phase"] == "RESOLUTION"
            active = before["view"]["clock"]["active"]
            vote = propose(first)
            with sqlite3.connect(database) as db:
                db.execute(
                    "CREATE TRIGGER pause_failure BEFORE UPDATE ON games "
                    "BEGIN SELECT RAISE(ABORT, 'failed pause transaction'); END"
                )
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            receive(second, "ERROR")
            assert request(server, game, token=spectator)["view"]["clock"]["active"] is None
            with sqlite3.connect(database) as db:
                db.execute("DROP TRIGGER pause_failure")
            send(second, "VOTE_OVERRIDE", proposal_id=vote, approve=True)
            resolved(first, vote, "applied")
            paused = receive(first, "STATE_UPDATE", revision=before["revision"] + 2)["view"][
                "clock"
            ]
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

        opened = request(
            restarted,
            "/games",
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
            },
            expect=201,
        )
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
