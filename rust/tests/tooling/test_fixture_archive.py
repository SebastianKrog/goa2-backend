"""Reference acquisition must be complete, isolated and atomic."""

import gzip
import hashlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tarfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "rust/tools"))


@pytest.fixture
def reference(tmp_path, monkeypatch):
    spec = importlib.util.spec_from_file_location(
        "fixture_archive", ROOT / "rust/tools/fixture_archive.py"
    )
    archive = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(archive)
    fixtures = archive.fixtures
    workspace = tmp_path / "workspace"
    base = workspace / "tests/fixtures"
    base.mkdir(parents=True)
    oracle = {"oracle_revision": "frozen", "source_sha256": "source", "rules_version": 9}
    (workspace / "oracle-manifest.json").write_text(json.dumps(oracle))
    case = {"name": "kept", "expected": [1]}
    identity = fixtures.digest(case)
    relative = f"frozen/{identity}.json"
    index = {
        "schema_version": 2,
        **oracle,
        "cases": [{"name": "kept", "path": relative, "case_sha256": identity}],
    }
    index["corpus_sha256"] = fixtures.digest(index)
    encoded_index = json.dumps(index).encode()
    (base / "frozen.json").write_bytes(encoded_index)
    external = b'{"independent":"reference"}'
    (base / "registry.json").write_text(
        json.dumps(
            {
                "families": [
                    {
                        "family": "frozen",
                        "path": "frozen.json",
                        "category": "parity-index",
                        "case_count": 1,
                        "sha256": hashlib.sha256(encoded_index).hexdigest(),
                    },
                    {
                        "family": "external",
                        "path": "external.json",
                        "category": "external-parity",
                        "raw_sha256": hashlib.sha256(external).hexdigest(),
                    },
                ]
            }
        )
    )
    encoded_case = fixtures.canonical(case) + b"\n"
    local = base / relative
    local.parent.mkdir()
    local.write_bytes(encoded_case)
    (base / "external.json").write_bytes(external)
    monkeypatch.setattr(fixtures, "WORKSPACE", workspace)
    monkeypatch.setattr(fixtures, "FIXTURES", base)
    members = [
        (relative + ".gz", gzip.compress(encoded_case, mtime=0)),
        ("external.json", external),
    ]
    return archive, members


def write_archive(path, members):
    with tarfile.open(path, "w") as output:
        for name, value in members:
            info = tarfile.TarInfo(name)
            if value is None:
                info.type = tarfile.SYMTYPE
                info.linkname = "../escape"
                output.addfile(info)
            else:
                info.size = len(value)
                output.addfile(info, io.BytesIO(value))
    return hashlib.sha256(path.read_bytes()).hexdigest()


def test_reference_archive_installs_verified_plain_and_gzip_files(tmp_path, reference):
    archive, members = reference
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)
    cache = tmp_path / "cache"
    result = archive.install(str(source), identity, cache)
    assert result["files"] == 2
    assert sum(row["executed"] for row in result["reports"]) == 2
    assert not any(row["missing"] or row["failed"] for row in result["reports"])
    assert (cache / members[0][0]).read_bytes() == members[0][1]
    assert not list(tmp_path.glob(".reference-*"))


@pytest.mark.parametrize("failure", ["missing", "checksum", "changed"])
def test_reference_failure_never_uses_checkout_payloads_or_publishes(tmp_path, reference, failure):
    archive, members = reference
    if failure == "missing":
        members = members[1:]
    elif failure == "changed":
        members[0] = (members[0][0], gzip.compress(b'{"name":"changed"}', mtime=0))
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)
    cache = tmp_path / "cache"
    with pytest.raises(ValueError):
        archive.install(str(source), "0" * 64 if failure == "checksum" else identity, cache)
    assert not cache.exists()
    assert not list(tmp_path.glob(".reference-*"))


@pytest.mark.parametrize("kind", ["traversal", "link", "duplicate", "extra-chunk"])
def test_reference_archive_rejects_unadmitted_members(tmp_path, reference, kind):
    archive, members = reference
    extra = {
        "traversal": ("../escape", b"bad"),
        "link": ("frozen/link.json", None),
        "duplicate": members[0],
        "extra-chunk": ("complete-games/chunks-v3/" + "0" * 64 + ".json", b"[]"),
    }[kind]
    source = tmp_path / "source.tar"
    identity = write_archive(source, [*members, extra])
    cache = tmp_path / "cache"
    with pytest.raises(ValueError):
        archive.install(str(source), identity, cache)
    assert not cache.exists()
    assert not (tmp_path / "escape").exists()
    assert not list(tmp_path.glob(".reference-*"))


def test_reference_archive_preserves_existing_cache(tmp_path, reference):
    archive, members = reference
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)
    cache = tmp_path / "cache"
    cache.mkdir()
    (cache / "keep").write_text("original")
    with pytest.raises(FileExistsError):
        archive.install(str(source), identity, cache)
    assert (cache / "keep").read_text() == "original"


def test_reference_pack_is_deterministic_excludes_unreferenced_files_and_roundtrips(
    tmp_path, reference
):
    archive, _ = reference
    source = archive.fixtures.FIXTURES
    (source / "private.sqlite").write_bytes(b"not a fixture")
    first = tmp_path / "first.tar"
    second = tmp_path / "second.tar"
    result = archive.pack(source, first)
    assert archive.pack(source, second)["sha256"] == result["sha256"]
    assert first.read_bytes() == second.read_bytes()
    with tarfile.open(first) as packed:
        members = packed.getmembers()
        assert len(members) == 2
        assert all(row.uid == row.gid == row.mtime == 0 for row in members)
        assert all(not row.uname and not row.gname for row in members)
        assert "private.sqlite" not in packed.getnames()
    cache = tmp_path / "cache"
    assert archive.install(str(first), result["sha256"], cache)["files"] == 2
    with pytest.raises(FileExistsError):
        archive.pack(source, first)


def test_reference_installer_lock_releases_without_a_stale_lock_blocking_retries(
    tmp_path, reference
):
    archive, members = reference
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)
    cache = tmp_path / "cache"
    with (tmp_path / "cache.install.lock").open("a") as lock:
        archive.fcntl.flock(lock, archive.fcntl.LOCK_EX | archive.fcntl.LOCK_NB)
        with pytest.raises(BlockingIOError):
            archive.install(str(source), identity, cache)
    assert archive.install(str(source), identity, cache)["files"] == 2


def test_complete_game_archive_includes_and_verifies_every_snapshot_chunk(tmp_path, reference):
    archive, _ = reference
    fixtures = archive.fixtures
    base = fixtures.FIXTURES
    oracle = json.loads((fixtures.WORKSPACE / "oracle-manifest.json").read_text())
    chunk = b"[[1]]"
    chunk_id = hashlib.sha256(chunk).hexdigest()
    chunk_path = f"complete-games/chunks-v3/{chunk_id}.json"
    case = {
        "name": "complete",
        "operations": [{"kind": "advance"}],
        "expected_delta_chunks": [{"path": chunk_path, "sha256": chunk_id, "records": 1}],
    }
    identity = fixtures.digest(case)
    relative = f"complete-games/{identity}.json"
    index = {
        "schema_version": 2,
        **oracle,
        "cases": [{"name": "complete", "path": relative, "case_sha256": identity}],
    }
    index["corpus_sha256"] = fixtures.digest(index)
    encoded_index = json.dumps(index).encode()
    (base / "complete-games.json").write_bytes(encoded_index)
    (base / "registry.json").write_text(
        json.dumps(
            {
                "families": [
                    {
                        "family": "complete-games",
                        "category": "parity-index",
                        "path": "complete-games.json",
                        "sha256": hashlib.sha256(encoded_index).hexdigest(),
                        "case_count": 1,
                    }
                ]
            }
        )
    )
    (base / relative).parent.mkdir(parents=True)
    (base / relative).write_text(json.dumps(case))
    encoded_chunk = base / (chunk_path + ".gz")
    encoded_chunk.parent.mkdir()
    encoded_chunk.write_bytes(gzip.compress(chunk, mtime=0))
    source = tmp_path / "complete.tar"
    identity = archive.pack(base, source)["sha256"]
    cache = tmp_path / "cache"
    result = archive.install(str(source), identity, cache)
    chunks = next(row for row in result["reports"] if row.get("unit") == "snapshot-chunk")
    assert chunks["expected"] == chunks["executed"] == 1
    assert (cache / (chunk_path + ".gz")).read_bytes() == encoded_chunk.read_bytes()


@pytest.mark.parametrize("configured", [True, False])
def test_ci_acquisition_steps_quote_paths_and_require_artifact_identity(tmp_path, configured):
    workflow = (ROOT / ".github/workflows/rust.yml").read_text().splitlines()
    scripts = []
    for index, line in enumerate(workflow):
        if line.strip() == "- name: Acquire and verify the complete admitted reference archive":
            body = []
            for command in workflow[index + 2 :]:
                if not command.startswith("          "):
                    break
                body.append(command[10:])
            scripts.append("\n".join(body))
    assert len(scripts) == 2
    python = tmp_path / "python3"
    python.write_text(f"#!{sys.executable}\nimport json, sys\nprint(json.dumps(sys.argv[1:]))\n")
    python.chmod(0o700)
    environment = {
        **os.environ,
        "PATH": str(tmp_path) + os.pathsep + os.environ["PATH"],
        "GOA2_FIXTURE_ARCHIVE_URL": "https://artifacts.invalid/reference.tar",
        "GOA2_FIXTURE_ARCHIVE_SHA256": "1" * 64,
        "GOA2_FIXTURE_CACHE": str(tmp_path / "cache with spaces"),
    }
    if not configured:
        environment.pop("GOA2_FIXTURE_ARCHIVE_SHA256")
    for script in scripts:
        result = subprocess.run(
            ["bash", "-c", script], env=environment, capture_output=True, text=True
        )
        if configured:
            assert result.returncode == 0, result.stderr
            assert json.loads(result.stdout) == [
                "rust/tools/fixture_archive.py",
                "install",
                environment["GOA2_FIXTURE_ARCHIVE_URL"],
                "--sha256",
                "1" * 64,
                "--output",
                environment["GOA2_FIXTURE_CACHE"],
            ]
        else:
            assert result.returncode != 0
            assert "independently trusted SHA256" in result.stderr


def test_compressed_reference_expansion_is_bounded_before_parsing(tmp_path, reference, monkeypatch):
    archive, members = reference
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)
    monkeypatch.setattr(archive.fixtures, "MAX_REFERENCE_BYTES", 8)
    cache = tmp_path / "cache"
    with pytest.raises(ValueError, match="expanded size limit"):
        archive.install(str(source), identity, cache)
    assert not cache.exists()
    assert not list(tmp_path.glob(".reference-*"))
