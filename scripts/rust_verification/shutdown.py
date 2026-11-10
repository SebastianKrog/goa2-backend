"""Disposable Rust service contract verification; no live service is contacted."""

import json
import os
import signal
import sqlite3
import time

from websockets.exceptions import ConnectionClosed

from .process import packet, request, serve, sockets, verify_replay


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
        receipt_args = {
            "card_id": "aspiring_duelist",
            "client_action_id": "before-stop",
            "expected_revision": 0,
        }
        receipt = request(server, game + "/cards", token=arien, body=receipt_args)
        request(server, game + "/cards", token=brogan, body={"card_id": "mad_dash"})
        pending = request(server, game, token=arien)
        assert pending["input_request"]["type"] == "CHOOSE_ACTION"

        timed = request(
            server,
            "/games",
            expect=201,
            body={
                **setup,
                "time_control": {
                    "planning_allowance_seconds": 60,
                    "resolution_allowance_seconds": 60,
                    "response_grant_seconds": 0,
                    "initial_time_bank_seconds": 0,
                    "time_bank_increment_seconds": 0,
                    "max_time_bank_seconds": 0,
                    "upgrade_allowance_seconds": 60,
                },
            },
        )
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
            assert (
                notice["outcome"] == "cancelled"
                and notice["proposal_id"] == proposal["proposal_id"]
            )
            verify_replay(restarted, timed_game, operations=("pause_opened", "pause_released"))

            prompt = packet(deciding, "STATE_UPDATE", 2)["input_request"]
            during_stop = {
                "request_id": prompt["request_id"],
                "selection": "HOLD",
                "client_action_id": "during-stop",
                "expected_revision": 2,
            }
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
