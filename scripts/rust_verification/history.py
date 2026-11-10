"""Admit historical synthetic producer data and verify current readers/rollback."""

import base64
import copy
import gzip
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from .process import backup_database, packet, request, serve, sockets, verify_replay

TABLES = (
    "games",
    "accepted_actions",
    "replay_starts",
    "replay_entries",
    "reported_replays",
    "replay_shares",
    "replay_share_groups",
)
PRODUCER_COMMIT = "f0fb93e117f358266ef522e6d5f23d3452e83e97"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def encode(value):
    return {"blob": base64.b64encode(value).decode()} if isinstance(value, bytes) else value


def decode(value):
    return base64.b64decode(value["blob"], validate=True) if isinstance(value, dict) else value


def dump_database(database):
    with sqlite3.connect(database) as db:
        schema = [
            row[0]
            for row in db.execute(
                "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END,name"
            )
        ]
        return {
            "schema": schema,
            "tables": {
                table: [
                    [encode(v) for v in row]
                    for row in db.execute(f"SELECT * FROM {table} ORDER BY 1,2")
                ]
                for table in TABLES
            },
            "user_version": db.execute("PRAGMA user_version").fetchone()[0],
        }


def restore_database(snapshot, path, version):
    if path.exists():
        raise FileExistsError(path)
    with sqlite3.connect(path) as db:
        for statement in snapshot["schema"]:
            db.execute(statement)
        for table, rows in snapshot["tables"].items():
            for row in rows:
                db.execute(
                    f"INSERT INTO {table} VALUES ({','.join('?' for _ in row)})",
                    [decode(v) for v in row],
                )
        # v1/v2 are retained numbered migration stages, not separately released
        # historical binaries. Preserve all version-appropriate rows.
        if version == 1:
            db.execute("ALTER TABLE replay_entries DROP COLUMN history")
        if version in (1, 2):
            db.execute("ALTER TABLE replay_shares DROP COLUMN created_at")
        db.execute(f"PRAGMA user_version = {version}")
        assert db.execute("PRAGMA quick_check").fetchone() == ("ok",)
    path.chmod(0o600)


def generate(producer, pack, output):
    if output.exists():
        raise FileExistsError("Refuse to replace admitted historical fixtures")
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="goa2-history-producer-") as name:
        root = Path(name)
        database = root / "producer.sqlite"
        with serve(producer, pack, database) as server:
            opened = request(
                server,
                "/games",
                expect=201,
                body={"red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "seed": 29},
            )
            game = "/games/" + opened["game_id"]
            seats = {r["hero_id"]: r["token"] for r in opened["player_tokens"]}
            opening_checkpoint = request(server, "/__lab" + game + "/checkpoint", lab=True)[
                "checkpoint"
            ]
            first = {
                "card_id": "aspiring_duelist",
                "expected_revision": 0,
                "client_action_id": "historical-first",
            }
            receipt = request(server, game + "/cards", token=seats["hero_arien"], body=first)
            request(
                server,
                game + "/cards",
                token=seats["hero_brogan"],
                body={
                    "card_id": "mad_dash",
                    "expected_revision": 1,
                    "client_action_id": "historical-second",
                },
            )
            terminal = copy.deepcopy(opening_checkpoint)
            terminal["envelope"]["state"].update(
                phase="GAME_OVER",
                winner="RED",
                victory_condition="SYNTHETIC_HISTORY",
                execution_stack=[],
            )
            finished = request(
                server, "/__lab/restore", lab=True, expect=201, body={"checkpoint": terminal}
            )
            share = request(
                server,
                "/games/" + finished["game_id"] + "/share",
                token=finished["player_tokens"][0]["token"],
                method="POST",
                expect=201,
            )
        # Replace disposable generated bearer values before admitting any bytes.
        # The historical producer reopens and authenticates these test-only labels.
        with sqlite3.connect(database) as db:
            for game_id, raw in db.execute("SELECT id,credentials FROM games").fetchall():
                credentials = json.loads(raw)
                for row in credentials["player_tokens"]:
                    row["token"] = "synthetic-history-" + game_id + "-" + row["hero_id"]
                credentials["spectator_token"] = "synthetic-history-" + game_id + "-spectator"
                db.execute(
                    "UPDATE games SET credentials=? WHERE id=?",
                    (json.dumps(credentials, separators=(",", ":")).encode(), game_id),
                )
        with serve(producer, pack, database) as server:
            with sqlite3.connect(database) as db:
                credentials = json.loads(
                    db.execute(
                        "SELECT credentials FROM games WHERE id=?", (opened["game_id"],)
                    ).fetchone()[0]
                )
            seats = {r["hero_id"]: r["token"] for r in credentials["player_tokens"]}
            spectator = credentials["spectator_token"]
            actor = request(server, "/__lab" + game + "/responders", lab=True)["players"][0]
            view = request(server, game, token=seats[actor])
            prompt = view["input_request"]
            assert prompt["type"] == "CHOOSE_ACTION"
            assert any(row["id"] == "FAST_TRAVEL" for row in prompt["options"])
            archive = verify_replay(
                server, game, credentials=[*seats.values(), spectator], operations=("command",)
            )
            original = {
                "game": game,
                "seats": seats,
                "spectator": spectator,
                "actor": actor,
                "views": {
                    token: request(server, game, token=token)
                    for token in [*seats.values(), spectator]
                },
                "checkpoint": request(server, "/__lab" + game + "/checkpoint", lab=True)[
                    "checkpoint"
                ],
                "archive": archive,
                "first": first,
                "receipt": receipt,
                "share": share,
                "share_metadata": request(server, share["url"]),
                "share_state": request(server, share["url"] + "/state"),
            }
            backup = root / "original-backup.sqlite"
            backup_database(producer, database, backup)
        with sqlite3.connect(backup) as db:
            assert db.execute("PRAGMA user_version").fetchone() == (0,)
        snapshot = dump_database(backup)
        reference = root / "reference.sqlite"
        shutil.copyfile(backup, reference)
        with serve(producer, pack, reference) as server:
            next_body = {
                "request_id": prompt["request_id"],
                "selection": "FAST_TRAVEL",
                "expected_revision": 2,
                "client_action_id": "historical-next",
            }
            original["next_body"] = next_body
            original["next_reply"] = request(
                server, game + "/input", token=seats[actor], body=next_body
            )
            original["rollback_reply"] = request(
                server,
                game + "/rollback",
                token=seats[actor],
                body={"expected_revision": 3, "client_action_id": "historical-rollback"},
            )
        payload = {
            "schema_version": 1,
            "provenance": "Actual retained historical producer on disposable synthetic games. Only credentials are replaced with inert test-only labels, then accepted by the same producer. Terminal share explicitly synthetic; no real match or cloud credential.",
            "database": snapshot,
            "expected": original,
        }
        raw = json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode()
        file = output / "legacy-v0.json.gz"
        file.write_bytes(gzip.compress(raw, mtime=0))
        manifest = {
            "schema_version": 1,
            "producer_commit": PRODUCER_COMMIT,
            "producer_binary_sha256": sha(producer),
            "content_sha256": sha(pack),
            "payload": {
                "path": file.name,
                "sha256": hashlib.sha256(raw).hexdigest(),
                "gzip_sha256": sha(file),
                "bytes": len(raw),
            },
            "checkpoint_version": 1,
            "replay_schema": 1,
            "database_versions": [0, 1, 2, 3],
            "stage_provenance": {
                "0": "Actual pre-versioning producer database; implicit optional columns retained.",
                "1": "Synthetic numbered migration intermediate: omit history/created_at columns.",
                "2": "Synthetic numbered migration intermediate: retain history, omit created_at.",
                "3": "Same producer rows at current schema version; no gameplay regeneration.",
            },
            "redistribution": "Synthetic test archive; separate public rights/payload decision remains C01/C07.",
            "contains_real_credentials": False,
            "contains_real_matches": False,
        }
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        return {
            "payload_bytes": len(raw),
            "gzip_bytes": file.stat().st_size,
            "producer_commit": PRODUCER_COMMIT,
            "database_versions": 4,
        }


def load(fixtures, pack):
    manifest = json.loads((fixtures / "manifest.json").read_text())
    path = fixtures / manifest["payload"]["path"]
    assert sha(path) == manifest["payload"]["gzip_sha256"]
    assert path.resolve().is_relative_to(
        fixtures.resolve()
    ), "Historical payload escapes fixture root"
    with gzip.open(path, "rb") as stream:
        raw = stream.read(16 * 1024 * 1024 + 1)
    assert len(raw) <= 16 * 1024 * 1024, "Historical test archive exceeds bounded size"
    assert len(raw) == manifest["payload"]["bytes"]
    assert hashlib.sha256(raw).hexdigest() == manifest["payload"]["sha256"]
    assert sha(pack) == manifest["content_sha256"]
    payload = json.loads(raw)
    assert payload["database"]["user_version"] == 0
    return manifest, payload


def check_open(server, expected):
    game = expected["game"]
    for token, view in expected["views"].items():
        assert request(server, game, token=token) == view
    assert (
        request(
            server, game + "/cards", token=expected["seats"]["hero_arien"], body=expected["first"]
        )
        == expected["receipt"]
    )
    assert (
        request(server, "/__lab" + game + "/checkpoint", lab=True)["checkpoint"]
        == expected["checkpoint"]
    )
    assert verify_replay(server, game, operations=("command",)) == expected["archive"]
    assert request(server, expected["share"]["url"]) == expected["share_metadata"]
    assert request(server, expected["share"]["url"] + "/state") == expected["share_state"]


def verify(binary, pack, fixtures, producer=None):
    manifest, payload = load(fixtures, pack)
    expected = payload["expected"]
    reports = []
    with tempfile.TemporaryDirectory(prefix="goa2-history-consumer-") as name:
        root = Path(name)
        for version in manifest["database_versions"]:
            database = root / f"v{version}.sqlite"
            restore_database(payload["database"], database, version)
            with serve(binary, pack, database) as server:
                check_open(server, expected)
                game = expected["game"]
                actor = expected["actor"]
                token = expected["seats"][actor]
                with sockets(server, game, token) as socket:
                    packet(socket, "STATE_UPDATE", 2)
                    reply = request(
                        server, game + "/input", token=token, body=expected["next_body"]
                    )
                    assert reply == expected["next_reply"]
                    packet(socket, "STATE_UPDATE", 3)
                verify_replay(server, game, operations=("command",))
            with serve(binary, pack, database) as server:
                assert (
                    request(server, game + "/input", token=token, body=expected["next_body"])
                    == expected["next_reply"]
                )
                rollback = request(
                    server,
                    game + "/rollback",
                    token=token,
                    body={"expected_revision": 3, "client_action_id": "historical-rollback"},
                )
                assert rollback == expected["rollback_reply"]
                assert (
                    rollback["input_request"]["request_id"] != expected["next_body"]["request_id"]
                )
                verify_replay(server, game, operations=("command",))
            with sqlite3.connect(database) as db:
                assert db.execute("PRAGMA user_version").fetchone() == (3,)
                assert db.execute("PRAGMA quick_check").fetchone() == ("ok",)
            reports.append(
                {
                    "source_database_version": version,
                    "consumer_database_version": 3,
                    "expected": 1,
                    "executed": 1,
                    "failed": 0,
                    "scope": "load/scopes/links/receipt/replay/share/command/event-order/restart/retry/rollback",
                }
            )
        future = root / "future.sqlite"
        restore_database(payload["database"], future, 4)
        before = sha(future)
        env = {
            **os.environ,
            "GOA2_GAME_CONTENT": str(pack),
            "GOA2_GAME_DATABASE": str(future),
            "GOA2_GAME_BIND": "127.0.0.1:0",
        }
        rejected = subprocess.run([str(binary)], env=env, capture_output=True, timeout=20)
        assert rejected.returncode and sha(future) == before
        rollback_report = None
        if producer:
            assert sha(producer) == manifest["producer_binary_sha256"], "Unadmitted rollback binary"
            database = root / "rollback-live.sqlite"
            restore_database(payload["database"], database, 0)
            backup = root / "pre-upgrade.sqlite"
            with serve(producer, pack, database) as server:
                check_open(server, expected)
                backup_database(producer, database, backup)
            with serve(binary, pack, database) as server:
                check_open(server, expected)
                request(
                    server,
                    expected["game"] + "/input",
                    token=expected["seats"][expected["actor"]],
                    body=expected["next_body"],
                )
            with sqlite3.connect(database) as db:
                assert db.execute("PRAGMA user_version").fetchone() == (3,)
            # Never attempt the unguarded old binary on a newer live schema.
            with serve(producer, pack, backup) as server:
                check_open(server, expected)
                assert (
                    request(
                        server,
                        expected["game"] + "/input",
                        token=expected["seats"][expected["actor"]],
                        body=expected["next_body"],
                    )
                    == expected["next_reply"]
                )
            rollback_report = {
                "expected": 1,
                "executed": 1,
                "failed": 0,
                "from_database_version": 3,
                "restored_matching_backup_version": 0,
                "old_binary_on_new_live_database_attempted": False,
            }
    return {
        "producer_commit": manifest["producer_commit"],
        "matrix": reports,
        "future_database_rejected_without_mutation": True,
        "rollback": rollback_report,
        "full_frozen_parity": False,
    }
