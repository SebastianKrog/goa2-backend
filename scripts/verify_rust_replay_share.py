"""Verify that a complete native match publishes an independent replay share."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import gzip
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import time
from unittest.mock import patch
from urllib.request import Request, urlopen

import jsonpatch
from fastapi.testclient import TestClient

from goa2.server.app import create_app

from verify_rust_game_server import backup_database, hero, packet, request, serve, sockets

PROVISION_TOKEN = "test-replay-share-provision"
ADMIN = {"Authorization": "Bearer test-replay-share-admin"}


def web_environment(server, root):
    return patch.dict(os.environ, {
        "GOA2_RUST_DRAFT_ORIGIN": f"http://{server}",
        "GOA2_GAME_PROVISION_TOKEN": PROVISION_TOKEN,
        "GOA2_ADMIN_TOKEN": ADMIN["Authorization"].removeprefix("Bearer "),
        "GOA2_REPLAY_API": "0",
        "GOA2_SAVE_DIR": str(root / "python-games"),
        "GOA2_SHARE_DIR": str(root / "python-shares"),
        "GOA2_REPLAY_DIR": str(root / "python-replays"),
        "GOA2_PREWARM_WORKERS": "0",
    })


def finish_game(server, fixture):
    initial = json.loads(fixture["checkpoint"]["native_checkpoint_json"])["envelope"]["state"]
    game_id = "rust-" + "1" * 32
    game = "/games/" + game_id
    opened = request(
        server, "/__hosted" + game, method="PUT", provision_token=PROVISION_TOKEN, expect=201,
        body={
            "map_name": fixture["map"], "seed": initial["rng_seed"],
            "tie_breaker_team": initial["tie_breaker_team"], "game_type": initial["game_type"],
            **{
                color.lower() + "_heroes": [row["name"] for row in team["heroes"]]
                for color, team in initial["teams"].items()
            },
        },
    )
    seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
    spectator = opened["spectator_token"]
    request(server, game + "/share", method="POST", expect=401)
    request(server, game + "/share", token=spectator, method="POST", expect=403)
    request(server, game + "/share", token=next(iter(seats.values())), method="POST", expect=409)
    request(server, "/shared/rust-" + "0" * 32, expect=404)
    first = fixture["trace"][0]["commitments"][0]
    token = seats[first["hero"]]
    request(server, game + "/cards", token=token, body={"card_id": first["first"], "expected_revision": 0})
    with sockets(server, game, token) as player:
        packet(player, "STATE_UPDATE", 1)
        player.send(json.dumps({"type": "PROPOSE_OVERRIDE", "family": "rewind", "to": 0}))
        vote = packet(player, "OVERRIDE_PROPOSED")
        assert vote["threshold"] == 1 and vote["to"] == 0
        assert packet(player, "OVERRIDE_RESOLVED")["outcome"] == "applied"
        state = packet(player, "STATE_UPDATE", 2)
        assert not hero(state, first["hero"])["current_turn_card"]
    revision = 2
    for decision in fixture["trace"]:
        if decision["kind"] == "planning":
            for commitment in decision["commitments"]:
                for card in [commitment["first"], *([commitment["second"]] if commitment["second"] else [])]:
                    result = request(
                        server, game + ("/cards" if card else "/pass"),
                        token=seats[commitment["hero"]],
                        body={"expected_revision": revision, **({"card_id": card} if card else {})},
                    )
                    revision += 1
                    assert result["revision"] == revision
        else:
            assert decision["kind"] == "resolution"
            seat = seats[decision["hero"]]
            offer = request(server, game, token=seat)["input_request"]
            selection = decision["selection"]
            if selection == "CONFIRM" and not any(
                option["id"] == "CONFIRM" for option in (offer or {}).get("options", [])
            ):
                continue
            assert offer, (revision, decision)
            result = request(server, game + "/input", token=seat, body={
                "request_id": offer["request_id"], "selection": selection,
                "expected_revision": revision,
            })
            revision += 1
            assert result["revision"] == revision
    end = request(server, game, token=spectator)
    endpoint = fixture["endpoint"]
    assert end["view"]["phase"] == endpoint["phase"] == "GAME_OVER"
    assert (end["view"]["round"], end["view"]["turn"], end["winner"]) == (
        endpoint["round"], endpoint["turn"], endpoint["winner_team"],
    )
    assert end["revision"] == revision
    return game_id, seats, spectator, revision


def verify_gameplay(binary, content, fixture, database):
    with serve(binary, content, database, hosted_token=PROVISION_TOKEN) as server:
        game_id, seats, spectator, revision = finish_game(server, fixture)
        game = "/games/" + game_id
        tokens = [*seats.values(), spectator]
        views = [request(server, game, token=token) for token in tokens]
    with serve(binary, content, database, hosted_token=PROVISION_TOKEN) as server:
        assert [request(server, game, token=token) for token in tokens] == views
    print(f"Verified hosted {fixture['map']} through {revision} revisions, including restart")

def play_during_share(server, number):
    game = "/games/rust-" + f"{number:032x}"
    opened = request(
        server, "/__hosted" + game, method="PUT", provision_token=PROVISION_TOKEN, expect=201,
        body={"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": number},
    )
    actor = next(row for row in opened["player_tokens"] if row["hero_id"] == "hero_arien")
    seat, spectator = actor["token"], opened["spectator_token"]
    card = hero(request(server, game, token=seat), "hero_arien")["hand"][0]["id"]
    with sockets(server, game, seat) as player, sockets(server, game, spectator) as observer:
        packet(player, "STATE_UPDATE", 0)
        packet(observer, "STATE_UPDATE", 0)
        for revision in range(1, 7):
            if revision % 2:
                body = {"card_id": card, "client_action_id": f"commit-{revision}",
                        "expected_revision": revision - 1}
                action = request(server, game + "/cards", token=seat, body=body)
                assert action["revision"] == revision
                assert request(server, game + "/cards", token=seat, body=body) == action
            else:
                player.send(json.dumps({"type": "UNCOMMIT_CARD", "expected_revision": revision - 1}))
                packet(player, "ACTION_RESULT", revision)
            own = hero(packet(player, "STATE_UPDATE", revision), "hero_arien")
            other = hero(packet(observer, "STATE_UPDATE", revision), "hero_arien")
            assert not other["hand"]
            if revision % 2:
                assert own["current_turn_card"]["id"] == card
                assert (other["current_turn_card"] or {}).get("id") != card
    assert request(server, game, token=seat)["revision"] == 6
    return game, seat


def verify_share(binary, content, fixture, database):
    with serve(binary, content, database, hosted_token=PROVISION_TOKEN) as server:
        game_id, seats, spectator, revision = finish_game(server, fixture)
        game = "/games/" + game_id
        share = game + "/share"
        request(server, share, token=spectator, method="POST", expect=403)
        with sqlite3.connect(database) as db:
            db.execute("CREATE TRIGGER reject_share BEFORE INSERT ON replay_share_groups "
                       "BEGIN SELECT RAISE(ABORT, 'injected share storage error'); END")
        first_seat = next(iter(seats.values()))
        request(server, share, token=first_seat, method="POST", expect=500, timeout=120)
        with sqlite3.connect(database) as db:
            assert db.execute("SELECT count(*) FROM replay_shares").fetchone() == (0,)
            assert db.execute("SELECT count(*) FROM replay_share_groups").fetchone() == (0,)
            db.execute("DROP TRIGGER reject_share")

        request(server, "/__hosted/replays", token=first_seat, expect=401)
        request(server, "/__hosted/shares", expect=401)
        request(server, "/__hosted" + share, method="POST", token=first_seat, expect=401)
        legacy = database.parent / "python-replays" / "legacy.jsonl"
        legacy.parent.mkdir(exist_ok=True)
        legacy.write_text(json.dumps({"type": "setup", "game_id": "legacy", "created_at": 1}) + "\n")

        started = time.monotonic()
        with web_environment(server, database.parent), TestClient(create_app()) as client:
            assert client.get("/replays").status_code == 401
            assert client.get("/shares").status_code == 401
            staff_share = "/replays/" + game_id + "/share"
            assert client.post(staff_share, headers={"Authorization": f"Bearer {first_seat}"}).status_code == 401
            catalog = client.get("/replays", headers=ADMIN)
            assert catalog.status_code == 200, catalog.text
            native, old = catalog.json()
            assert old["game_id"] == "legacy"
            assert native["game_id"] == game_id and native["engine"] == "rust"
            assert native["num_decisions"] == revision and native["map"] == fixture["map"]
            assert all(token not in catalog.text for token in [*seats.values(), spectator, PROVISION_TOKEN])
            with ThreadPoolExecutor(max_workers=6) as pool:
                player = pool.submit(request, server, share, token=first_seat,
                                     method="POST", expect=201, timeout=120)
                staff = pool.submit(client.post, staff_share, headers=ADMIN)
                live_started = time.monotonic()
                running = [pool.submit(play_during_share, server, number) for number in range(2, 6)]
                live = [game.result() for game in running]
                assert not player.done() and not staff.done(), "Gameplay did not overlap the replay bake"
                print(f"Verified 24 live revisions during replay bake in {time.monotonic() - live_started:.2f}s",
                      flush=True)
                staff_reply = staff.result()
                assert staff_reply.status_code == 201, staff_reply.text
                first, second = player.result(), staff_reply.json()
            catalog = client.get("/shares", headers=ADMIN)
            assert catalog.status_code == 200, catalog.text
            published, = catalog.json()
            assert published["token"] == first["token"] and published["game_id"] == game_id
            assert published["engine"] == "rust" and published["total_decisions"] == revision
            assert published["size_bytes"] > 0 and abs(published["created_at"] - time.time()) < 120
        print(f"Published replay in {time.monotonic() - started:.2f}s", flush=True)
        assert first == second and first["url"] == "/shared/" + first["token"]
        link = first["url"]
        meta = request(server, link)
        assert meta["setup"]["game_id"] == game_id
        assert meta["setup"]["map"] == fixture["map"]
        assert len(meta["decisions"]) == revision
        assert [row["index"] for row in meta["decisions"]] == list(range(revision))
        first_commit = fixture["trace"][0]["commitments"][0]
        assert meta["decisions"][0]["card"] == first_commit["first"]
        assert meta["decisions"][1]["type"] == "ov_rewind"
        assert meta["decisions"][1]["sel"] == {"to": 0}

        def frames_for(group):
            position = group["keyframe"]
            yield position
            for patch in group["patches"]:
                position = jsonpatch.apply_patch(position, patch)
                yield position
        frames = []
        for start in range(0, revision + 1, 32):
            group = request(server, link + f"/state?decision={start}")
            assert group["start"] == start
            frames.extend(frames_for(group))
        assert [frame["position"]["decision_index"] for frame in frames] == list(range(revision + 1))
        opening, final = frames[0], frames[-1]
        assert opening["view"]["phase"] == "PLANNING"
        assert final["view"]["phase"] == "GAME_OVER"
        assert final["winner"] == fixture["endpoint"]["winner_team"]
        assert all(hero(opening, seat)["hand"] for seat in seats)
        assert hero(frames[1], first_commit["hero"])["current_turn_card"]["id"] == first_commit["first"]
        assert not hero(frames[2], first_commit["hero"])["current_turn_card"]
        serialized = json.dumps([meta, *frames[::32]])
        assert all(token not in serialized for token in [*seats.values(), spectator, PROVISION_TOKEN])
        assert "request_id" not in serialized
        assert request(server, link + "/state?decision=-4")["start"] == 0
        assert request(server, link + "/state?decision=100000")["start"] == revision // 32 * 32
        wanted = next(row for row in meta["decisions"] if row["r"] == 2)
        assert request(server, link + "/state?round=2")["start"] == wanted["index"] // 32 * 32
        with urlopen(Request(f"http://{server}{link}/state", headers={"Accept-Encoding": "gzip"})) as zipped:
            assert zipped.headers["Content-Encoding"] == "gzip"
            assert zipped.headers["Cache-Control"] == "no-store"
            assert json.loads(gzip.decompress(zipped.read())) == request(server, link + "/state")

        backup = database.with_name("restored.sqlite")
        backup_database(binary, database, backup)

    # Restore the online backup separately, then remove only its live game.
    # The durable share remains available until staff revokes it.
    with sqlite3.connect(backup) as db:
        db.execute("PRAGMA foreign_keys = ON")
        db.execute("DELETE FROM games WHERE id=?", (game_id,))
    with serve(binary, content, backup, hosted_token=PROVISION_TOKEN) as restarted:
        for live_game, seat in live:
            assert request(restarted, live_game, token=seat)["revision"] == 6
        request(restarted, game, token=first_seat, expect=404)
        assert request(restarted, link) == meta
        restored = list(frames_for(request(restarted, link + "/state")))[-1]
        assert restored == final
        revoke = "/shares/" + first["token"]
        request(restarted, "/__hosted" + revoke, method="DELETE", expect=401)
        with web_environment(restarted, database.parent), TestClient(create_app()) as client:
            assert client.post("/replays/" + game_id + "/share", headers=ADMIN).json() == first
            assert client.delete(revoke).status_code == 401
            assert client.delete(revoke, headers=ADMIN).status_code == 204
            assert client.delete(revoke, headers=ADMIN).status_code == 404
            assert client.get("/shares", headers=ADMIN).json() == []
        request(restarted, link, expect=404)
        request(restarted, link + "/state", expect=404)
        with sqlite3.connect(backup) as db:
            assert db.execute("SELECT count(*) FROM replay_share_groups").fetchone() == (0,)
    print(f"Verified hosted replay share across {revision} commands and {len(frames)} frames")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("content", type=Path)
    parser.add_argument("fixture", type=Path, help="Verified arena match fixture from lab_match_fixture")
    parser.add_argument("--gameplay-only", action="store_true",
                        help="Verify hosted gameplay and restart without publishing a replay")
    args = parser.parse_args()
    fixture = json.loads(args.fixture.read_text(encoding="utf-8"))
    verify = verify_gameplay if args.gameplay_only else verify_share
    with tempfile.TemporaryDirectory(prefix="goa2-hosted-replay-") as root:
        verify(args.binary.resolve(), args.content.resolve(), fixture, Path(root) / "game.sqlite")


if __name__ == "__main__":
    main()
