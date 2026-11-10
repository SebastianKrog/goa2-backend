"""Disposable Rust service contract verification; no live service is contacted."""

import json
import os
import subprocess
import time
from contextlib import contextmanager
from urllib.error import HTTPError
from urllib.request import Request, urlopen

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
    server,
    path,
    *,
    token=None,
    lab=False,
    body=None,
    expect=200,
    method=None,
    provision_token=None,
    timeout=10,
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
        hero
        for team in response["view"]["teams"].values()
        for hero in team["heroes"]
        if hero["id"] == hero_id
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
    opening = request(
        server,
        "/__lab/replay",
        lab=True,
        body={
            "replay_json": archive_text,
            "revision": start,
        },
    )
    assert opening["revision"] == start
    assert json.loads(opening["checkpoint"]["native_checkpoint_json"]) == archive["start"]
    if tamper:
        request(server, lab_game + "/replay", token=credentials[0], expect=401)
        request(server, "/__lab/replay", body={"replay_json": archive_text}, expect=401)
        request(
            server,
            "/__lab/replay",
            lab=True,
            body={
                "replay_json": archive_text,
                "revision": end + 1,
            },
            expect=422,
        )
        assert "client_action_id" not in archive_text
        archive["entries"][-1]["checkpoint_sha256"] = "0" * 64
        invalid = request(
            server,
            "/__lab/replay",
            lab=True,
            body={
                "replay_json": json.dumps(archive),
            },
            expect=422,
        )
        assert (
            str(end) in invalid["detail"]
            and "reconstructed checkpoint differs" in invalid["detail"]
        ), invalid
        archive["entries"].pop()
        invalid = request(
            server,
            "/__lab/replay",
            lab=True,
            body={
                "replay_json": json.dumps(archive),
            },
            expect=422,
        )
        assert "not contiguous" in invalid["detail"]
    return archive


def verify_lab_gold_cheat(server):
    opened = request(
        server,
        "/games",
        body={
            "red_heroes": ["Arien"],
            "blue_heroes": ["Brogan"],
            "cheats_enabled": True,
            "seed": 29,
        },
        expect=201,
    )
    game = "/games/" + opened["game_id"]
    lab_game = "/__lab" + game
    seats = {seat["hero_id"]: seat["token"] for seat in opened["player_tokens"]}
    actor, target = "hero_arien", "hero_brogan"
    current = request(server, game, token=seats[actor])
    gold = hero(current, target)["gold"]
    command = {
        "actor": actor,
        "expected_revision": 0,
        "message": {"type": "CHEATS_GOLD", "hero_id": target, "amount": 7},
    }
    preview = request(server, lab_game + "/comparison", lab=True, body=command)
    assert preview["revision"] == 0
    assert hero(preview["views"]["spectator"], target)["gold"] == gold + 7
    assert hero(request(server, game, token=seats[actor]), target)["gold"] == gold
    assert request(server, lab_game + "/command", lab=True, body=command)["revision"] == 1
    for revision, (seat, card) in enumerate([(actor, "aspiring_duelist"), (target, "mad_dash")], 1):
        request(
            server,
            game + "/cards",
            token=seats[seat],
            body={"card_id": card, "expected_revision": revision},
        )
    current = request(server, game, token=seats[actor])
    assert current["view"]["phase"] == "RESOLUTION"
    request(
        server,
        game + "/cheats/gold",
        token=seats[actor],
        body={
            "hero_id": target,
            "amount": 7,
            "expected_revision": 3,
        },
        expect=422,
    )
    assert request(server, game, token=seats[actor])["revision"] == 3


def wait_view(server, game, token, predicate, timeout=25):
    deadline = time.monotonic() + timeout
    while True:
        value = request(server, game, token=token)
        if predicate(value):
            return value
        if time.monotonic() >= deadline:
            raise AssertionError(
                (value["revision"], value["view"]["phase"], value["awaiting_input"])
            )
        time.sleep(0.08)
