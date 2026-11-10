"""Rehearse online WAL backup and restore on disposable synthetic games only."""

import hashlib
import json
import os
import sqlite3
import subprocess
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


def database_rows(path):
    with sqlite3.connect(path) as db:
        assert db.execute("PRAGMA quick_check").fetchone() == ("ok",)
        assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        rows = {
            table: db.execute(f"SELECT * FROM {table} ORDER BY 1, 2").fetchall() for table in TABLES
        }
        rows["_schema"] = db.execute(
            "SELECT type, name, sql FROM sqlite_master ORDER BY type, name"
        ).fetchall()
        rows["_version"] = db.execute("PRAGMA user_version").fetchone()
        return rows


def backup_restore_smoke(binary, pack, database):
    restored = database.with_name("online-backup.sqlite")
    setup = {
        "red_heroes": ["Arien"],
        "blue_heroes": ["Brogan"],
        "seed": 2**64 - 1,
        "future_optional": True,
    }
    with serve(binary, pack, database) as server:
        opened = request(server, "/games", body=setup, expect=201)
        game = "/games/" + opened["game_id"]
        seats = {row["hero_id"]: row["token"] for row in opened["player_tokens"]}
        spectator = opened["spectator_token"]
        action = {
            "card_id": "aspiring_duelist",
            "client_action_id": "before-backup",
            "expected_revision": 0,
        }
        # Keep a read snapshot open so newer commits genuinely reside in the WAL.
        with sqlite3.connect(database) as reader:
            assert reader.execute("PRAGMA journal_mode").fetchone() == ("wal",)
            reader.execute("BEGIN")
            reader.execute("SELECT count(*) FROM games").fetchone()
            receipt = request(server, game + "/cards", token=seats["hero_arien"], body=action)
            assert receipt["revision"] == 1
            request(server, game + "/share", token=seats["hero_arien"], method="POST", expect=409)
            # A trusted, explicitly synthetic finished checkpoint exercises share
            # storage without pretending to replay an unavailable complete corpus.
            saved = request(server, "/__lab" + game + "/checkpoint", lab=True)["checkpoint"]
            terminal = json.loads(json.dumps(saved))
            state = terminal["envelope"]["state"]
            state.update(
                phase="GAME_OVER",
                winner="RED",
                victory_condition="SYNTHETIC_BACKUP",
                execution_stack=[],
            )
            finished = request(
                server, "/__lab/restore", lab=True, expect=201, body={"checkpoint": terminal}
            )
            finished_game = "/games/" + finished["game_id"]
            owner = finished["player_tokens"][0]["token"]
            share = request(
                server, finished_game + "/share", token=owner, method="POST", expect=201, timeout=60
            )
            link = share["url"]
            metadata = request(server, link)
            frames = request(server, link + "/state")
            public = json.dumps([metadata, frames])
            for secret in [
                spectator,
                *seats.values(),
                finished["spectator_token"],
                *[row["token"] for row in finished["player_tokens"]],
            ]:
                assert secret not in public
            assert "native_checkpoint_json" not in public and "native_resume" not in public
            views = {
                token: request(server, game, token=token) for token in [*seats.values(), spectator]
            }
            archive = verify_replay(
                server, game, credentials=[*seats.values(), spectator], operations=("command",)
            )
            for key, value in [("schema", 2), ("future", True)]:
                invalid = {**archive, key: value}
                request(
                    server,
                    "/__lab/replay",
                    lab=True,
                    body={"replay_json": json.dumps(invalid)},
                    expect=422,
                )
            checkpoint = request(server, "/__lab" + game + "/checkpoint", lab=True)["checkpoint"]
            wal = Path(str(database) + "-wal")
            assert wal.exists() and wal.stat().st_size > 32
            before = database_rows(database)
            assert all(before[table] for table in TABLES if table != "reported_replays")
            backup_database(binary, database, restored)
            assert database_rows(restored) == before
            digest = hashlib.sha256(restored.read_bytes()).hexdigest()
            rejected = subprocess.run(
                [binary, "--backup", restored],
                env={**os.environ, "GOA2_GAME_DATABASE": str(database)},
                capture_output=True,
            )
            assert rejected.returncode != 0
            assert hashlib.sha256(restored.read_bytes()).hexdigest() == digest
            if os.name == "posix":
                assert restored.stat().st_mode & 0o077 == 0
            # Changes after the snapshot cannot leak into the restored backup.
            request(
                server, game + "/uncommit", token=seats["hero_arien"], body={"expected_revision": 1}
            )
            assert request(server, game, token=spectator)["revision"] == 2
            reader.rollback()

    with serve(binary, pack, restored) as server:
        assert database_rows(restored) == before
        for token, view in views.items():
            assert request(server, game, token=token) == view
        assert request(server, game + "/cards", token=seats["hero_arien"], body=action) == receipt
        request(
            server,
            game + "/cards",
            token=seats["hero_arien"],
            body={**action, "card_id": "spell_break"},
            expect=409,
        )
        assert (
            request(server, "/__lab" + game + "/checkpoint", lab=True)["checkpoint"] == checkpoint
        )
        verify_replay(
            server, game, credentials=[*seats.values(), spectator], operations=("command",)
        )
        assert request(server, link) == metadata
        assert request(server, link + "/state") == frames
        with sockets(server, game, spectator) as socket:
            assert packet(socket, "STATE_UPDATE", 1)["input_request"] is None
        assert (
            request(
                server, game + "/uncommit", token=seats["hero_arien"], body={"expected_revision": 1}
            )["revision"]
            == 2
        )
