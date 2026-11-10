"""Historical admission and intermediate schema shapes are explicit and bounded."""

import copy
import json
import shutil
import sqlite3
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts"))
from rust_verification.history import load, restore_database  # noqa: E402

FIXTURES = ROOT / "rust/tests/compatibility"
PACK = ROOT / "rust/content/smoke-game.json"


def test_historical_payload_admits_actual_producer_and_explicit_intermediates():
    manifest, payload = load(FIXTURES, PACK)
    assert manifest["producer_commit"] == "f0fb93e117f358266ef522e6d5f23d3452e83e97"
    assert manifest["checkpoint_version"] == manifest["replay_schema"] == 1
    assert manifest["database_versions"] == [0, 1, 2, 3]
    assert manifest["contains_real_credentials"] is False
    assert all(
        payload["database"]["tables"][name]
        for name in ["games", "accepted_actions", "replay_entries", "replay_share_groups"]
    )
    assert all("synthetic-history-" in token for token in payload["expected"]["views"])


def test_archive_compressed_tampering_is_rejected_before_restore(tmp_path):
    shutil.copytree(FIXTURES, tmp_path / "fixtures")
    manifest = json.loads((tmp_path / "fixtures/manifest.json").read_text())
    path = tmp_path / "fixtures" / manifest["payload"]["path"]
    data = bytearray(path.read_bytes())
    data[len(data) // 2] ^= 1
    path.write_bytes(data)
    with pytest.raises(AssertionError):
        load(tmp_path / "fixtures", PACK)


@pytest.mark.parametrize("version", [0, 1, 2, 3])
def test_intermediate_versions_preserve_replay_receipt_and_share_rows(tmp_path, version):
    _, payload = load(FIXTURES, PACK)
    original = copy.deepcopy(payload["database"])
    path = tmp_path / "database.sqlite"
    restore_database(original, path, version)
    with sqlite3.connect(path) as db:
        assert db.execute("PRAGMA user_version").fetchone() == (version,)
        history = {row[1] for row in db.execute("PRAGMA table_info(replay_entries)")}
        shares = {row[1] for row in db.execute("PRAGMA table_info(replay_shares)")}
        assert ("history" in history) == (version != 1)
        assert ("created_at" in shares) == (version not in (1, 2))
        for table, rows in original["tables"].items():
            assert db.execute(f"SELECT count(*) FROM {table}").fetchone() == (len(rows),)
        assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    assert original == payload["database"]
