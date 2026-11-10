"""Disposable Rust service contract verification; no live service is contacted."""

import json
import sqlite3

from .process import hero, packet, request, serve, sockets, verify_replay


def rewind_vote_smoke(binary, pack, database):
    def send(socket, kind, **fields):
        socket.send(json.dumps({"type": kind, **fields}))

    with serve(binary, pack, database) as server:
        opened = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
            },
        )
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
                        db.execute(
                            "CREATE TRIGGER rewind_failure BEFORE UPDATE ON games "
                            "BEGIN SELECT RAISE(ABORT, 'injected rewind failure'); END"
                        )
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
            stale = request(
                server,
                game + "/input",
                token=seats[actor],
                expect=422,
                body={
                    "request_id": original,
                    "selection": "HOLD",
                },
            )
            assert "stale or unissued" in stale["detail"]

            rewind(1, 4, fail_once=True)
            state = request(server, game, token=arien)
            assert state["view"]["phase"] == "PLANNING"
            assert hero(state, "hero_arien")["current_turn_card"]["id"] == "aspiring_duelist"
            assert not hero(state, "hero_brogan")["current_turn_card"]
            history = request(server, game + "/overrides/history", token=spectator)["decisions"]
            assert [(row["type"], row["superseded"]) for row in history] == [
                ("commit", False),
                ("commit", True),
                ("ov_rewind", False),
                ("ov_rewind", False),
            ]
            assert "mad_dash" not in json.dumps(history)
            request(
                server,
                game + "/cards",
                token=brogan,
                body={
                    "card_id": "mad_dash",
                    "expected_revision": 4,
                },
            )
            packet(first, "STATE_UPDATE", 5)
            packet(second, "STATE_UPDATE", 5)
            rewind(3, 6)
            final_prompt = request(server, game, token=seats[actor])["input_request"]
            assert final_prompt["request_id"] not in {original, refreshed}

        archive = verify_replay(
            server, game, credentials=[arien, brogan, spectator], operations=("rewind",)
        )
        assert [
            row["decision"]["operation"]["to"]
            for row in archive["entries"]
            if row["decision"]["operation"]["kind"] == "rewind"
        ] == [2, 1, 3]

        timed = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
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
        result = request(
            restarted,
            game + "/input",
            token=seats[actor],
            body={
                "request_id": final_prompt["request_id"],
                "selection": final_prompt["options"][0]["id"],
                "expected_revision": 6,
            },
        )
        assert result["revision"] == 7
        verify_replay(restarted, game, operations=("rewind",))


def unstick_vote_smoke(binary, pack, database):
    cases = []
    with serve(binary, pack, database) as server:
        for op in ("skip_input", "abort_action", "end_turn", "force_actor"):
            opened = request(
                server,
                "/games",
                expect=201,
                body={
                    "red_heroes": ["Arien"],
                    "blue_heroes": ["Brogan"],
                    "seed": 29,
                },
            )
            game = "/games/" + opened["game_id"]
            seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
            arien, brogan = seats["hero_arien"], seats["hero_brogan"]
            spectator = opened["spectator_token"]
            request(server, game + "/cards", token=arien, body={"card_id": "aspiring_duelist"})
            request(server, game + "/cards", token=brogan, body={"card_id": "mad_dash"})
            before = request(server, game, token=arien)
            assert before["input_request"]["type"] == "CHOOSE_ACTION"
            if op in ("skip_input", "abort_action"):
                request(
                    server,
                    game + "/input",
                    token=arien,
                    body={
                        "request_id": before["input_request"]["request_id"],
                        "selection": "FAST_TRAVEL",
                    },
                )
                before = request(server, game, token=arien)
                assert before["input_request"]["can_skip"] is True
            old_request = before["input_request"]["request_id"]
            args = {"hero_id": "hero_brogan"} if op == "force_actor" else {}
            with sockets(server, game, arien) as first, sockets(server, game, brogan) as second:
                for socket in (first, second):
                    packet(socket, "STATE_UPDATE", before["revision"])

                def vote(command, arguments, outcome):
                    first.send(
                        json.dumps(
                            {
                                "type": "PROPOSE_OVERRIDE",
                                "family": "unstick",
                                "op": command,
                                "args": arguments,
                            }
                        )
                    )
                    proposed = packet(first, "OVERRIDE_PROPOSED")
                    assert proposed["family"] == "unstick" and proposed["op"] == command
                    assert proposed["args"] == arguments and proposed["threshold"] == 2
                    assert packet(second, "OVERRIDE_PROPOSED") == proposed
                    second.send(
                        json.dumps(
                            {
                                "type": "VOTE_OVERRIDE",
                                "proposal_id": proposed["proposal_id"],
                                "approve": True,
                            }
                        )
                    )
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
                assert (
                    state["view"]["board"]["entity_locations"]["hero_arien"]
                    == positions["hero_arien"]
                )
            if op == "end_turn":
                assert hero(state, "hero_arien")["played_cards"][0]["id"] == "aspiring_duelist"
                assert state["view"]["current_actor_id"] == "hero_brogan"
            if op == "force_actor":
                assert state["view"]["current_actor_id"] == "hero_brogan"
            rejected = request(
                server,
                game + "/input",
                token=arien,
                expect=422,
                body={
                    "request_id": old_request,
                    "selection": "HOLD",
                },
            )
            assert "stale or unissued" in rejected["detail"]
            if op == "end_turn":
                with sockets(server, game, arien) as sole:
                    packet(sole, "STATE_UPDATE", revision)
                    packet(sole, "OVERRIDE_RESOLVED")
                    sole.send(
                        json.dumps({"type": "PROPOSE_OVERRIDE", "family": "unstick", "op": op})
                    )
                    assert packet(sole, "OVERRIDE_PROPOSED")["threshold"] == 1
                    assert packet(sole, "OVERRIDE_RESOLVED")["outcome"] == "applied"
                    revision += 1
                    assert packet(sole, "STATE_UPDATE", revision)["view"]["phase"] == "PLANNING"
            archive = verify_replay(server, game, operations=("unstick",))
            assert archive["entries"][-1]["decision"]["operation"]["command"] == {"op": op, **args}
            history = request(server, game + "/overrides/history", token=spectator)
            assert history["decisions"][-1]["type"] == "ov_unstick"
            cases.append((game, spectator, revision))

        opened = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
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
            for revision, (op, expected) in enumerate(
                [("skip_input", "rejected"), ("end_turn", "applied")]
            ):
                first.send(
                    json.dumps(
                        {"type": "PROPOSE_OVERRIDE", "family": "unstick", "op": op, "args": {}}
                    )
                )
                vote = packet(first, "OVERRIDE_PROPOSED")
                packet(second, "OVERRIDE_PROPOSED")
                for socket in (first, second):
                    assert (
                        packet(socket, "STATE_UPDATE", 5 + revision * 2)["view"]["clock"]["active"]
                        is None
                    )
                second.send(
                    json.dumps(
                        {
                            "type": "VOTE_OVERRIDE",
                            "proposal_id": vote["proposal_id"],
                            "approve": True,
                        }
                    )
                )
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
        opened = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
            },
        )
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
                first.send(
                    json.dumps(
                        {
                            "type": "PROPOSE_OVERRIDE",
                            "family": "patch",
                            "op": op,
                            "args": args,
                        }
                    )
                )

            def patch(op, **args):
                nonlocal revision
                propose(op, args)
                offered = packet(first, "OVERRIDE_PROPOSED")
                assert (
                    offered["family"] == "patch" and offered["op"] == op and offered["args"] == args
                )
                assert packet(second, "OVERRIDE_PROPOSED") == offered
                second.send(
                    json.dumps(
                        {
                            "type": "VOTE_OVERRIDE",
                            "proposal_id": offered["proposal_id"],
                            "approve": True,
                        }
                    )
                )
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
            assert (
                state["view"]["teams"]["RED"]["starting_life_counters"]
                == opening["teams"]["RED"]["starting_life_counters"]
            )

            for token, card in [(arien, "aspiring_duelist"), (brogan, "mad_dash")]:
                request(
                    server,
                    game + "/cards",
                    token=token,
                    body={
                        "card_id": card,
                        "expected_revision": revision,
                    },
                )
                revision += 1
                state = packet(first, "STATE_UPDATE", revision)
                packet(second, "STATE_UPDATE", revision)
            old = state["input_request"]["request_id"]
            state = patch("set_gold", hero_id="hero_brogan", value=2)
            assert hero(state, "hero_brogan")["gold"] == 2
            assert state["input_request"]["request_id"] != old
            request(
                server,
                game + "/input",
                token=arien,
                expect=422,
                body={
                    "request_id": old,
                    "selection": "HOLD",
                },
            )

            state = patch("set_life_counters", team="RED", value=0)
            assert state["view"]["phase"] == "GAME_OVER" and state["winner"] == "BLUE"
            assert any(event["event_type"] == "GAME_OVER" for event in state["events"])
            state = patch("set_life_counters", team="RED", value=2)
            assert state["view"]["phase"] != "GAME_OVER" and state["winner"] is None
            assert state["view"]["teams"]["RED"]["life_counters"] == 2

        final = request(server, game, token=spectator)
        archive = verify_replay(server, game, operations=("patch",))
        assert archive["entries"][-1]["decision"]["operation"]["command"] == {
            "op": "set_life_counters",
            "team": "RED",
            "value": 2,
        }
        history = request(server, game + "/overrides/history", token=spectator)
        assert sum(row["type"] == "ov_patch" for row in history["decisions"]) == 10

        timed = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
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
        with sockets(server, timed_game, red) as first, sockets(server, timed_game, blue) as second:
            for socket in (first, second):
                packet(socket, "STATE_UPDATE", 2)
            for amount, revision, status in [(0, 4, "FINISHED"), (2, 5, "WAITING_FOR_PLAYERS")]:
                first.send(
                    json.dumps(
                        {
                            "type": "PROPOSE_OVERRIDE",
                            "family": "patch",
                            "op": "set_life_counters",
                            "args": {"team": "RED", "value": amount},
                        }
                    )
                )
                offer = packet(first, "OVERRIDE_PROPOSED")
                packet(second, "OVERRIDE_PROPOSED")
                if amount == 0:
                    for socket in (first, second):
                        assert packet(socket, "STATE_UPDATE", 3)["view"]["clock"]["active"] is None
                second.send(
                    json.dumps(
                        {
                            "type": "VOTE_OVERRIDE",
                            "proposal_id": offer["proposal_id"],
                            "approve": True,
                        }
                    )
                )
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
        opened = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Razzle"],
                "blue_heroes": ["Emmitt"],
                "seed": 29,
            },
        )
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
            tile["hex"]
            for tile in board["tiles"].values()
            if tile["occupant_id"] is None and not tile["is_terrain"]
        ]
        target, second_target = free[:2]
        active_zones = set(opening["battle_zones"].values())
        minion = next(
            row["id"]
            for row in opening["teams"]["BLUE"]["minions"]
            if any(
                tile["occupant_id"] == row["id"] and tile["zone_id"] in active_zones
                for tile in board["tiles"].values()
            )
        )
        minion_start = positions[minion]
        checkpoint = request(server, "/__lab" + game + "/checkpoint", lab=True)["checkpoint"]
        token = next(
            id
            for id in checkpoint["envelope"]["state"]["misc_entities"]
            if id.startswith("glitch_")
        )

        def occupant(view, point):
            key = "_".join(str(point[axis]) for axis in ("q", "r", "s"))
            return view["board"]["tiles"][key]["occupant_id"]

        with sockets(server, game, razzle) as first, sockets(server, game, emmitt) as second:
            revision = 0
            for socket in (first, second):
                packet(socket, "STATE_UPDATE", revision)

            def vote(op, *, rejection=None, **args):
                nonlocal revision
                first.send(
                    json.dumps(
                        {
                            "type": "PROPOSE_OVERRIDE",
                            "family": "patch",
                            "op": op,
                            "args": args,
                        }
                    )
                )
                offer = packet(first, "OVERRIDE_PROPOSED")
                assert offer["op"] == op and offer["args"] == args, offer
                assert packet(second, "OVERRIDE_PROPOSED") == offer
                second.send(
                    json.dumps(
                        {
                            "type": "VOTE_OVERRIDE",
                            "proposal_id": offer["proposal_id"],
                            "approve": True,
                        }
                    )
                )
                outcome = packet(first, "OVERRIDE_RESOLVED")
                assert packet(second, "OVERRIDE_RESOLVED") == outcome
                if rejection:
                    assert (
                        outcome["outcome"] == "rejected"
                        and outcome["reason"]["message"] == rejection
                    ), outcome
                    assert request(server, game, token=spectator)["revision"] == revision
                    return None
                assert outcome["outcome"] == "applied", outcome
                revision += 1
                state = packet(first, "STATE_UPDATE", revision)
                packet(second, "STATE_UPDATE", revision)
                assert not state.get("events"), state["events"]
                return state["view"]

            vote(
                "move_entity",
                entity_id="hero_razzle",
                hex=enemy,
                rejection="Destination is occupied",
            )
            vote(
                "move_entity",
                entity_id="hero_razzle",
                hex={"q": 999, "r": -999, "s": 0},
                rejection="Destination is outside the board",
            )
            vote("place_entity", entity_id="unregistered", hex=target, rejection="Unknown entity")
            view = vote("move_entity", entity_id="hero_razzle", hex=target)
            assert view["hero_pieces"][piece]["position"] == target
            assert occupant(view, start) is None and occupant(view, target) == piece

            view = vote("place_entity", entity_id=reserve, hex=second_target)
            assert view["hero_pieces"][reserve]["position"] == second_target
            vote(
                "remove_entity", entity_id="hero_razzle", rejection="Specify a concrete hero piece"
            )
            view = vote("remove_entity", entity_id=piece)
            assert piece not in view["board"]["entity_locations"]
            assert view["hero_pieces"][piece]["position"] is None and occupant(view, target) is None
            vote("move_entity", entity_id=piece, hex=start, rejection="Entity is not on the board")
            view = vote("place_entity", entity_id=piece, hex=start)
            assert occupant(view, start) == piece

            view = vote("remove_entity", entity_id=minion)
            assert (
                occupant(view, minion_start) is None
                and minion not in view["board"]["entity_locations"]
            )
            minion_tile = next(
                tile for tile in board["tiles"].values() if tile["hex"] == minion_start
            )
            lane = next(
                lane
                for lane, row in opening["battle_zone_minion_counts"].items()
                if row["zone_id"] == minion_tile["zone_id"]
            )
            assert (
                view["battle_zone_minion_counts"][lane]["counts"]["BLUE"]
                == opening["battle_zone_minion_counts"][lane]["counts"]["BLUE"] - 1
            )
            view = vote("place_entity", entity_id=minion, hex=minion_start)
            assert occupant(view, minion_start) == minion
            assert view["battle_zone_minion_counts"] == opening["battle_zone_minion_counts"]

            view = vote("place_entity", entity_id=token, hex=target)
            assert occupant(view, target) == token
            assert any(row["id"] == token and row["hex"] == target for row in view["tokens"])
            view = vote("remove_entity", entity_id=token)
            assert occupant(view, target) is None
            assert all(row["id"] != token for row in view["tokens"])

            vote(
                "add_marker",
                marker_type="poison",
                target_id="unknown",
                source_id="hero_emmitt",
                value=-2,
                rejection="Unknown entity",
            )
            view = vote(
                "add_marker",
                marker_type="poison",
                target_id=piece,
                source_id="hero_emmitt",
                value=-2,
            )
            assert view["markers"]["poison"] == {
                "target_id": piece,
                "source_id": "hero_emmitt",
                "value": -2,
            }
            view = vote(
                "add_marker",
                marker_type="poison",
                target_id="hero_emmitt",
                source_id="manual",
                value=-1,
            )
            assert view["markers"]["poison"] == {
                "target_id": "hero_emmitt",
                "source_id": "manual",
                "value": -1,
            }
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
        opened = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
                "cheats_enabled": True,
            },
        )
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
            with (
                sockets(server, game, seats[proposer]) as first,
                sockets(server, game, seats[voter]) as second,
            ):
                for socket in (first, second):
                    packet(socket, "STATE_UPDATE", revision)
                    if had_notice:
                        packet(socket, "OVERRIDE_RESOLVED")
                first.send(
                    json.dumps(
                        {"type": "PROPOSE_OVERRIDE", "family": "patch", "op": op, "args": args}
                    )
                )
                if private:
                    error = packet(first, "ERROR")
                    assert "Only the owner" in json.dumps(error), error
                    return
                offer = packet(first, "OVERRIDE_PROPOSED")
                assert offer["op"] == op and offer["args"] == args, offer
                assert packet(second, "OVERRIDE_PROPOSED") == offer
                second.send(
                    json.dumps(
                        {
                            "type": "VOTE_OVERRIDE",
                            "proposal_id": offer["proposal_id"],
                            "approve": True,
                        }
                    )
                )
                outcome = packet(first, "OVERRIDE_RESOLVED")
                had_notice = True
                assert packet(second, "OVERRIDE_RESOLVED") == outcome
                if rejection:
                    assert (
                        outcome["outcome"] == "rejected"
                        and outcome["reason"]["message"] == rejection
                    ), outcome
                    assert state(proposer)["revision"] == revision
                    return
                assert outcome["outcome"] == "applied", outcome
                for socket in (first, second):
                    packet(socket, "STATE_UPDATE", revision + 1)
            return state(arien)

        card = "aspiring_duelist"
        vote("move_card", brogan, hero_id=arien, card_id=card, zone="discard", private=True)
        vote("swap_item_card", brogan, hero_id=arien, color="BLUE", tier="II", private=True)
        vote(
            "move_card",
            hero_id=arien,
            card_id=card,
            zone="played",
            slot=1,
            rejection="No available played slot for a finished turn",
        )
        view = vote("move_card", hero_id=arien, card_id=card, zone="discard")
        assert card not in cards(view, "hand") and card in cards(view, "discard_pile")
        view = vote("move_card", brogan, hero_id=arien, card_id=card, zone="hand")
        assert card in cards(view, "hand") and card not in cards(view, "discard_pile")
        view = vote("move_card", hero_id=arien, card_id=card, zone="deck")
        assert card in cards(view, "deck") and card not in cards(view, "hand")
        assert (
            next(row for row in hero(view, arien)["deck"] if row["id"] == card)["state"] == "DECK"
        )
        vote("move_card", hero_id=arien, card_id=card, zone="hand")

        request(server, game + "/cards", token=seats[arien], body={"card_id": card})
        vote("move_card", brogan, hero_id=arien, card_id=card, zone="discard", private=True)
        view = vote("move_card", hero_id=arien, card_id=card, zone="discard")
        assert hero(view, arien)["current_turn_card"] is None
        assert card in cards(view, "discard_pile")
        vote("move_card", hero_id=arien, card_id=card, zone="hand")

        request(
            server, game + "/cheats/gold", token=seats[arien], body={"hero_id": arien, "amount": 1}
        )
        request(server, game + "/cards", token=seats[arien], body={"card_id": card})
        request(server, game + "/cards", token=seats[brogan], body={"card_id": "mad_dash"})

        def respond():
            for actor in (arien, brogan):
                frame = state(actor)
                prompt = frame["input_request"]
                if prompt is None:
                    continue
                choices = [row["id"] for row in prompt["options"]]
                selection = next(
                    (item for item in ("HOLD", "CONFIRM", "SKIP", "NO") if item in choices), None
                )
                assert selection is not None, prompt
                request(
                    server,
                    game + "/input",
                    token=seats[actor],
                    body={
                        "request_id": prompt["request_id"],
                        "selection": selection,
                    },
                )
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
        vote(
            "move_card",
            hero_id=arien,
            card_id=other,
            zone="played",
            slot=1,
            rejection="No available played slot for a finished turn",
        )
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
                    request(
                        server,
                        game + "/cards",
                        token=seats[actor],
                        body={"card_id": available[0]["id"]},
                    )
            else:
                respond()
        else:
            raise AssertionError("The round did not reach the upgrade prompt")

        upgrade = view["input_request"]
        assert upgrade["type"] == "UPGRADE_PHASE", upgrade
        options = upgrade["players"][arien]["options"]
        pair = options[0]
        choice, item = pair["pair"]
        request(
            server,
            game + "/input",
            token=seats[arien],
            body={
                "request_id": upgrade["request_id"],
                "selection": {"hero_id": arien, "card_id": choice},
            },
        )
        before = state(arien)
        assert choice in cards(before, "hand") and item not in cards(before, "hand")
        assert (
            next(row for row in hero(before, arien)["deck"] if row["id"] == item)["state"] == "ITEM"
        )
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
        revealed = [
            row for row in state(brogan)["view"]["unresolved_cards"] if row.get("hero_id") == arien
        ]
        assert any(row["card"]["id"] == choice for row in revealed), revealed

        final = request(server, game, token=spectator)
        archive = verify_replay(server, game, operations=("command", "patch"))
        operations = [entry["decision"]["operation"] for entry in archive["entries"]]
        assert [op["command"]["op"] for op in operations if op["kind"] == "patch"] == [
            "move_card"
        ] * 8 + ["swap_item_card"] * 2

    with serve(binary, pack, database) as restarted:
        assert request(restarted, game, token=spectator) == final
        verify_replay(restarted, game, operations=("patch",))


def effect_patch_vote_smoke(binary, pack, database):
    with serve(binary, pack, database) as server:
        opened = request(
            server,
            "/games",
            expect=201,
            body={
                "red_heroes": ["Arien"],
                "blue_heroes": ["Brogan"],
                "seed": 29,
            },
        )
        game = "/games/" + opened["game_id"]
        arien, brogan = "hero_arien", "hero_brogan"
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        spectator = opened["spectator_token"]
        card = "aspiring_duelist"
        for actor, chosen in [(arien, card), (brogan, "mad_dash")]:
            request(server, game + "/cards", token=seats[actor], body={"card_id": chosen})

        def movement(view):
            return next(
                option["value"]
                for option in view["input_request"]["options"]
                if option["id"] == "MOVEMENT"
            )

        def effect(id, kind, **fields):
            return {
                "id": id,
                "source_id": arien,
                "effect_type": kind,
                "duration": "THIS_TURN",
                "created_at_turn": 1,
                "created_at_round": 1,
                "scope": {"shape": "global", "affects": "self"},
                **fields,
            }

        modifier = effect(
            "table_speed",
            "area_stat_modifier",
            source_card_id=card,
            is_active=True,
            stat_type="MOVEMENT",
            stat_value=2,
        )
        premature = effect(
            "table_stop",
            "delayed_trigger",
            source_card_id=card,
            is_active=True,
            finishing_steps=[
                {"type": "place_marker", "marker_type": "venom", "target_id": brogan, "value": -1}
            ],
        )
        delayed = effect(
            "table_trigger",
            "delayed_trigger",
            is_active=True,
            finishing_steps=[
                {"type": "set_context_flag", "key": "table_effect_targets", "value": [brogan]},
                {
                    "type": "for_each",
                    "list_key": "table_effect_targets",
                    "item_key": "table_effect_target",
                    "current_index": 0,
                    "steps_template": [
                        {
                            "type": "place_marker",
                            "marker_type": "poison",
                            "target_key": "table_effect_target",
                            "value": -2,
                        },
                    ],
                },
            ],
        )
        with (
            sockets(server, game, seats[arien]) as first,
            sockets(server, game, seats[brogan]) as second,
        ):
            revision = 2
            before = packet(first, "STATE_UPDATE", revision)
            packet(second, "STATE_UPDATE", revision)
            base_movement = movement(before)
            stale_id = before["input_request"]["request_id"]

            def propose(op, args):
                first.send(
                    json.dumps(
                        {
                            "type": "PROPOSE_OVERRIDE",
                            "family": "patch",
                            "op": op,
                            "args": args,
                        }
                    )
                )

            def vote(op, *, rejection=None, **args):
                nonlocal revision
                propose(op, args)
                offer = packet(first, "OVERRIDE_PROPOSED")
                assert offer["op"] == op and offer["args"] == args, offer
                assert packet(second, "OVERRIDE_PROPOSED") == offer
                second.send(
                    json.dumps(
                        {
                            "type": "VOTE_OVERRIDE",
                            "proposal_id": offer["proposal_id"],
                            "approve": True,
                        }
                    )
                )
                outcome = packet(first, "OVERRIDE_RESOLVED")
                assert packet(second, "OVERRIDE_RESOLVED") == outcome
                if rejection:
                    assert (
                        outcome["outcome"] == "rejected"
                        and outcome["reason"]["message"] == rejection
                    ), outcome
                    assert request(server, game, token=spectator)["revision"] == revision
                    return None
                assert outcome["outcome"] == "applied", outcome
                revision += 1
                view = packet(first, "STATE_UPDATE", revision)
                packet(second, "STATE_UPDATE", revision)
                return view

            propose("add_effect", {"effect": {"id": "invalid"}})
            packet(first, "ERROR")
            vote(
                "add_effect",
                effect=effect(
                    "invalid", "delayed_trigger", finishing_steps=[{"type": "not_a_step"}]
                ),
                rejection="Unsupported effect finishing steps",
            )
            vote("remove_effect", effect_id="unknown", rejection="No active effect matches that ID")

            view = vote("add_effect", effect=modifier)
            assert movement(view) == base_movement + 2
            assert view["input_request"]["request_id"] != stale_id
            assert hero(view, arien)["current_turn_card"]["is_active"]
            request(
                server,
                game + "/input",
                token=seats[arien],
                expect=422,
                body={"request_id": stale_id, "selection": "HOLD"},
            )
            vote(
                "add_effect", effect=modifier, rejection="An effect with that ID is already active"
            )
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
            (actor,) = public["awaiting_input"]
            state = request(restarted, game, token=seats[actor])
            prompt = state["input_request"]
            options = [row["id"] for row in prompt["options"]]
            choice = "HOLD" if "HOLD" in options else "CONFIRM"
            assert choice in options, prompt
            request(
                restarted,
                game + "/input",
                token=seats[actor],
                body={"request_id": prompt["request_id"], "selection": choice},
            )
        else:
            raise AssertionError("The round did not complete its first turn")
        assert not public["view"]["effects"]
        assert public["view"]["markers"]["poison"] == {
            "target_id": brogan,
            "source_id": arien,
            "value": -2,
        }
        assert not public["view"]["markers"].get("venom", {}).get("target_id")
        verify_replay(restarted, game, operations=("patch", "command"))
