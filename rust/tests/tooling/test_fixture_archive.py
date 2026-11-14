"""Reference acquisition must be complete, isolated and atomic."""

import gzip
import hashlib
import importlib.util
import io
import json
import os
import struct
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


@pytest.mark.parametrize(
    "encoding,valid", [("canonical", True), ("pretty", True), ("changed", False)]
)
def test_indexed_trace_preflight_preserves_frozen_identity(reference, encoding, valid):
    archive, _ = reference
    fixtures = archive.fixtures
    base = fixtures.FIXTURES
    index = json.loads((base / "frozen.json").read_bytes())
    row = index["cases"][0]
    payload = json.loads((base / row["path"]).read_bytes())
    row["path"] = f"stack-traces/{row['case_sha256']}.json"
    del index["corpus_sha256"]
    index["corpus_sha256"] = fixtures.digest(index)
    encoded = fixtures.canonical(index)
    (base / "stack-traces.json").write_bytes(encoded)
    registry = json.loads((base / "registry.json").read_text())
    registry["families"][0].update(
        family="stack-traces", path="stack-traces.json", sha256=hashlib.sha256(encoded).hexdigest()
    )
    (base / "registry.json").write_text(json.dumps(registry))
    target = base / row["path"]
    target.parent.mkdir()
    if encoding == "changed":
        payload["expected"] = [2]
    target.write_text(
        json.dumps(payload, indent=2)
        if encoding == "pretty"
        else fixtures.canonical(payload).decode() + chr(10)
    )
    (result,) = fixtures.full(["stack-traces"])
    assert (result["executed"], result["failed"], result["missing"]) == (
        int(valid),
        int(not valid),
        0,
    )


@pytest.mark.skipif(sys.platform != "linux", reason="seekable pack uses Linux libzstd")
@pytest.mark.parametrize("variant", ["valid", "bad-seek-table", "bad-range", "bad-frame"])
def test_explicit_seekable_pack_rejects_invalid_payloads(tmp_path, reference, monkeypatch, variant):
    archive, _ = reference
    fixtures = archive.fixtures
    pack = tmp_path / "seekable"
    pack.mkdir()
    # A checksummed zstd frame containing the two bytes b"{}".
    frame = bytes.fromhex("28b52ffd24021100007b7dd194f27a")
    if variant == "bad-frame":
        frame = frame[:-1] + bytes([frame[-1] ^ 1])
    seek = struct.pack("<IIIIIBI", 0x184D2A5E, 17, len(frame), 2, 1, 0, 0x8F92EAB1)
    if variant == "bad-seek-table":
        seek = seek[:-1]
    (pack / "fixtures-4MiB.tar.zst").write_bytes(frame + seek)
    index = {
        "schema": 1,
        "fixtures": {
            "case.json": [1 if variant == "bad-range" else 0, 2, hashlib.sha256(b"{}").hexdigest()]
        },
    }
    (pack / "fixtures.index.json").write_text(json.dumps(index))
    monkeypatch.setenv("GOA2_FIXTURE_PACK", str(pack))
    if variant == "valid":
        assert fixtures.read_fixture("case.json", None) == b"{}"
        # An explicit pack must not use an unrelated checkout payload.
        assert fixtures.read_fixture("external.json", None) is None
    else:
        with pytest.raises(ValueError, match="fixture pack"):
            fixtures.read_fixture("case.json", None)


@pytest.mark.skipif(sys.platform != "linux", reason="seekable pack uses Linux libzstd")
def test_seekable_alias_reconstructs_the_original_named_bytes(tmp_path, reference, monkeypatch):
    archive, _ = reference
    frame = bytes.fromhex(
        "28b52ffd241ff900007b226e616d65223a226669727374222c226578706563746564223a5b315d7d36e83162"
    )
    first = b'{"name":"first","expected":[1]}'
    second = b'{"name":"second","expected":[1]}'
    seek = struct.pack("<IIIIIBI", 0x184D2A5E, 17, len(frame), len(first), 1, 0, 0x8F92EAB1)
    (tmp_path / "fixtures-4MiB.tar.zst").write_bytes(frame + seek)
    index = {
        "schema": 2,
        "fixtures": {
            name: [0, len(first), hashlib.sha256(data).hexdigest()]
            for name, data in [("first.json", first), ("second.json", second), ("bad.json", first)]
        },
        "aliases": {
            name: ["first.json", '{"name":"first",', '{"name":"second",']
            for name in ("second.json", "bad.json")
        },
    }
    (tmp_path / "fixtures.index.json").write_text(json.dumps(index))
    monkeypatch.setenv("GOA2_FIXTURE_PACK", str(tmp_path))
    assert archive.fixtures.read_fixture("first.json", None) == first
    assert archive.fixtures.read_fixture("second.json", None) == second
    with pytest.raises(ValueError, match="alias checksum mismatch"):
        archive.fixtures.read_fixture("bad.json", None)


def test_pinned_seekable_manifest_detects_changed_archive_or_index(tmp_path, reference):
    archive, _ = reference
    fixtures = archive.fixtures
    oracle = json.loads((fixtures.WORKSPACE / "oracle-manifest.json").read_bytes())
    manifest = {"schema": 1, **oracle}
    for name in ("fixtures-4MiB.tar.zst", "fixtures.index.json"):
        (tmp_path / name).write_bytes(name.encode())
        manifest[name] = hashlib.sha256(name.encode()).hexdigest()
    encoded = fixtures.canonical(manifest)
    (tmp_path / "manifest.json").write_bytes(encoded)
    expected = hashlib.sha256(encoded).hexdigest()
    fixtures.verify_pack_identity(tmp_path, expected)
    with pytest.raises(ValueError, match="manifest checksum mismatch"):
        fixtures.verify_pack_identity(tmp_path, "0" * 64)
    for name in ("fixtures-4MiB.tar.zst", "fixtures.index.json"):
        (tmp_path / name).write_bytes(b"changed")
        with pytest.raises(ValueError, match="pack checksum mismatch"):
            fixtures.verify_pack_identity(tmp_path, expected)
        (tmp_path / name).write_bytes(name.encode())


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


def test_local_archive_uses_no_duplicate_data_copy(tmp_path, reference):
    archive, members = reference
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)
    result = archive.install(str(source), identity, tmp_path / "cache")
    assert result["source_hardlinked"] is True
    assert source.stat().st_nlink == 1
    assert hashlib.sha256(source.read_bytes()).hexdigest() == identity


def test_archive_falls_back_to_copy_when_hardlinks_are_unavailable(
    tmp_path, reference, monkeypatch
):
    archive, members = reference
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)

    def cannot_link(*args, **kwargs):
        raise OSError("Cross-filesystem link unavailable")

    monkeypatch.setattr(archive.os, "link", cannot_link)
    result = archive.install(str(source), identity, tmp_path / "cache")
    assert result["source_hardlinked"] is False
    assert sum(row["executed"] for row in result["reports"]) == 2


def test_changed_local_archive_is_rejected_before_cache_publication(
    tmp_path, reference, monkeypatch
):
    archive, members = reference
    source = tmp_path / "source.tar"
    identity = write_archive(source, members)
    original = archive.fixtures.full

    def verify_then_change(*args, **kwargs):
        result = original(*args, **kwargs)
        with source.open("ab") as output:
            output.write(b"changed after extraction")
        return result

    monkeypatch.setattr(archive.fixtures, "full", verify_then_change)
    cache = tmp_path / "cache"
    with pytest.raises(ValueError, match="changed during installation"):
        archive.install(str(source), identity, cache)
    assert not cache.exists()
    assert not list(tmp_path.glob(".reference-*"))



@pytest.mark.parametrize(
    ("mutation", "error"),
    [("omit", "1 missing"), ("hash", "payload differs from fixture index")],
)
def test_paired_coverage_rejects_the_same_missing_or_wrong_case_in_both_runs(
    tmp_path, mutation, error
):
    from compare_trace_coverage import compare

    expected = {("gate", "first"): "a" * 64, ("gate", "second"): "b" * 64}
    rows = [
        {
            "index": gate,
            "case": name,
            "fixture_sha256": digest,
            "operations": 1,
            "replay_key": None,
            "replayed_as": name,
        }
        for (gate, name), digest in expected.items()
    ]
    if mutation == "omit":
        rows.pop()
    else:
        rows[-1]["fixture_sha256"] = "c" * 64
    logs = [tmp_path / mode for mode in ("baseline", "shared")]
    for log in logs:
        records = "".join("trace-coverage " + json.dumps(row) + chr(10) for row in rows)
        log.write_text("test native::gate ... " + records + "test result: ok. 1 passed; 0 failed;")
    with pytest.raises(ValueError, match=error):
        compare(*logs, expected)
