"""Acquisition failures must preserve existing data and reject unadmitted inputs."""

import gzip
import hashlib
import importlib.util
import io
import json
import sys
import tarfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts/rust_oracle"))


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "rust/tools" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def archive(rows):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w") as tar:
        for name, data in rows:
            member = tarfile.TarInfo(name)
            member.size = len(data)
            tar.addfile(member, io.BytesIO(data))
    return stream.getvalue()


def test_source_archive_rejects_extra_missing_corrupt_and_duplicate_members():
    from pinned_source import verify_archive

    value = b"source\n"
    manifest = {"files": {"src/goa2/example.py": hashlib.sha256(value).hexdigest()}}
    admitted = [("src/goa2/example.py", value)]
    verify_archive(archive(admitted), manifest)
    for rows in [
        [],
        admitted + admitted,
        [*admitted, ("../escape", b"x")],
        [("src/goa2/example.py", b"bad")],
    ]:
        with pytest.raises(ValueError):
            verify_archive(archive(rows), manifest)


def test_content_hash_or_version_failure_does_not_publish(tmp_path):
    content = load("content")
    oracle = json.loads((ROOT / "rust/oracle-manifest.json").read_text())
    source = tmp_path / "input.json"
    destination = tmp_path / "output.json"
    data = {
        "version": 1,
        "oracle_revision": oracle["oracle_revision"],
        "source_sha256": oracle["source_sha256"],
    }
    source.write_text(json.dumps(data))
    checksum = hashlib.sha256(source.read_bytes()).hexdigest()
    with pytest.raises(ValueError, match="checksum"):
        content.install(str(source), "0" * 64, destination)
    assert not destination.exists()
    content.install(str(source), checksum, destination)
    assert destination.read_bytes() == source.read_bytes()
    with pytest.raises(FileExistsError):
        content.install(str(source), checksum, destination)
    source.write_text(json.dumps({**data, "version": 2}))
    with pytest.raises(ValueError, match="Unsupported"):
        content.install(
            str(source), hashlib.sha256(source.read_bytes()).hexdigest(), tmp_path / "future.json"
        )
    assert not (tmp_path / "future.json").exists()
    assert not list(tmp_path.glob(".content-*"))


def test_full_parity_never_substitutes_a_bundled_smoke_case(tmp_path, monkeypatch):
    fixtures = load("fixtures")
    base = tmp_path / "tests/fixtures"
    base.mkdir(parents=True)
    (tmp_path / "oracle-manifest.json").write_text(
        '{"source_sha256":"oracle","oracle_revision":"reference"}'
    )
    (base / "registry.json").write_text('{"families":[{"family":"full"}]}')
    case = {"name": "kept", "expected": [1]}
    identity = fixtures.digest(case)
    relative = f"full/{identity}.json"
    index = {
        "schema_version": 2,
        "oracle_revision": "reference",
        "source_sha256": "oracle",
        "cases": [{"name": "kept", "path": relative, "case_sha256": identity}],
    }
    index["corpus_sha256"] = fixtures.digest(index)
    (base / "full.json").write_text(json.dumps(index))
    smoke = base / "smoke" / (relative + ".gz")
    smoke.parent.mkdir(parents=True)
    smoke.write_bytes(gzip.compress(fixtures.canonical(case), mtime=0))
    monkeypatch.setattr(fixtures, "WORKSPACE", tmp_path)
    monkeypatch.setattr(fixtures, "FIXTURES", base)
    report = fixtures.full(["full"])[0]
    assert report["expected"] == report["missing"] == 1
    assert report["executed"] == 0
    cache = tmp_path / "cache"
    payload = cache / (relative + ".gz")
    payload.parent.mkdir(parents=True)
    payload.write_bytes(smoke.read_bytes())
    assert fixtures.full(["full"], cache)[0]["executed"] == 1
    payload.write_bytes(gzip.compress(b'{"name":"kept","expected":[2]}', mtime=0))
    assert fixtures.full(["full"], cache)[0]["failed"] == 1


@pytest.mark.parametrize("payload", [None, b"[]\n", b"[[1]]\n"])
def test_full_preflight_checks_complete_game_snapshot_chunks(tmp_path, monkeypatch, payload):
    fixtures = load("fixtures")
    base = tmp_path / "tests/fixtures"
    base.mkdir(parents=True)
    oracle = {"source_sha256": "oracle", "oracle_revision": "reference"}
    (tmp_path / "oracle-manifest.json").write_text(json.dumps(oracle))
    (base / "registry.json").write_text('{"families":[{"family":"game"}]}')
    encoded = b"[[1]]"
    chunk_identity = hashlib.sha256(encoded).hexdigest()
    relative = f"game/chunks-v3/{chunk_identity}.json"
    case = {
        "name": "whole-game",
        "operations": [{"op": "advance"}],
        "expected_delta_chunks": [{"path": relative, "sha256": chunk_identity, "records": 1}],
    }
    identity = fixtures.digest(case)
    case_path = f"game/{identity}.json"
    index = {
        "schema_version": 2,
        **oracle,
        "cases": [{"name": case["name"], "path": case_path, "case_sha256": identity}],
    }
    index["corpus_sha256"] = fixtures.digest(index)
    (base / "game.json").write_text(json.dumps(index))
    cached_case = tmp_path / "cache" / case_path
    cached_case.parent.mkdir(parents=True)
    cached_case.write_text(json.dumps(case))
    if payload is not None:
        cached_chunk = tmp_path / "cache" / relative
        cached_chunk.parent.mkdir(parents=True)
        Path(str(cached_chunk) + ".gz").write_bytes(gzip.compress(payload, mtime=0))
    monkeypatch.setattr(fixtures, "WORKSPACE", tmp_path)
    monkeypatch.setattr(fixtures, "FIXTURES", base)
    reports = fixtures.full(["game"], tmp_path / "cache")
    chunks = next(row for row in reports if row.get("unit") == "snapshot-chunk")
    assert chunks["expected"] == 1
    assert chunks["executed"] == int(payload == encoded + b"\n")
    assert chunks["missing"] == int(payload is None)
    assert chunks["failed"] == int(payload is not None and payload != encoded + b"\n")
    assert next(row for row in reports if row.get("unit") != "snapshot-chunk")["executed"] == 1


def test_fixture_registry_allowlist_and_ignore_rules_agree():
    import fixture_cache

    registry = json.loads((ROOT / "rust/tests/fixtures/registry.json").read_text())
    oracle = json.loads((ROOT / "rust/oracle-manifest.json").read_text())
    for family in registry["families"]:
        assert family["rules_version"] == oracle["rules_version"]
        if "generator_argv" in family:
            generator = ROOT / family["generator"]
            assert hashlib.sha256(generator.read_bytes()).hexdigest() == family["generator_sha256"]
            assert family["generator_argv"][:2] == ["python", "scripts/rust_oracle/run_pinned.py"]
            for consumer in family["consumers"]:
                assert (ROOT / consumer).is_file(), consumer
    expected = {
        row["path"] for row in registry["families"] if row["category"] != "external-parity"
    } | {"registry.json"}
    assert expected == fixture_cache.COMMITTED
    exceptions = {
        line.removeprefix("!/tests/fixtures/")
        for line in (ROOT / "rust/.gitignore").read_text().splitlines()
        if line.startswith("!/tests/fixtures/")
        and line.endswith(".json")
        and "/" not in line.removeprefix("!/tests/fixtures/")
    }
    assert exceptions == expected
