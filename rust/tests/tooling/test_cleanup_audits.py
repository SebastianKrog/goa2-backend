"""Extraction admission, ordered identity and baseline privacy contracts."""

import copy
import gzip
import hashlib
import importlib.util
import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "rust/tools"))
sys.path.insert(0, str(ROOT / "scripts"))


def tool(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "rust/tools" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_extraction_excludes_secret_database_and_cache_files_under_broad_globs(tmp_path):
    audit = tool("dependency_audit")
    for name in [
        "docs/guide.md",
        "docs/.env.local",
        "docs/match.sqlite",
        "rust/tests/fixtures/smoke/a.json.gz",
        "rust/tests/fixtures/cache/a.json.gz",
    ]:
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("synthetic")
    manifest = {
        "groups": [{"category": "development", "paths": ["docs/**/*", "rust/tests/fixtures/**/*"]}],
        "excluded": [{"paths": [".env*", "*.sqlite*", "rust/tests/fixtures/*/*.json.gz"]}],
    }
    selected = audit.selected_paths(tmp_path, manifest)
    assert set(selected) == {"docs/guide.md", "rust/tests/fixtures/smoke/a.json.gz"}


def test_extraction_rejects_symbolic_aliases_instead_of_copying_their_contents(tmp_path):
    audit = tool("dependency_audit")
    (tmp_path / "source.md").write_text("source")
    (tmp_path / "alias.md").symlink_to(tmp_path / "source.md")
    with pytest.raises(ValueError, match="Symlink"):
        audit.selected_paths(tmp_path, {"groups": [{"category": "development", "paths": ["*.md"]}]})


def test_extractor_refuses_recursive_destination_before_creating_it(tmp_path, monkeypatch):
    extract = tool("extract")
    (tmp_path / "manifest.json").write_text("{}")
    monkeypatch.setattr(extract, "ROOT", tmp_path)
    monkeypatch.setattr(extract, "MANIFEST", tmp_path / "manifest.json")
    destination = tmp_path / "nested"
    with pytest.raises(ValueError, match="outside"):
        extract.extract(destination)
    assert not destination.exists()


def test_full_index_cannot_resign_changed_reference_metadata(tmp_path, monkeypatch):
    fixtures = tool("fixtures")
    base = tmp_path / "tests/fixtures"
    base.mkdir(parents=True)
    oracle = {"oracle_revision": "frozen", "source_sha256": "source", "rules_version": 9}
    (tmp_path / "oracle-manifest.json").write_text(json.dumps(oracle))
    case = {"name": "one", "ordered": {"z": None, "a": [1, 2]}, "seed": 2**64 - 1}
    identity = fixtures.digest(case)
    index = {
        "schema_version": 2,
        **oracle,
        "cases": [{"name": "one", "path": f"full/{identity}.json", "case_sha256": identity}],
    }
    index["corpus_sha256"] = fixtures.digest(index)
    admitted = json.dumps(index).encode()
    (base / "registry.json").write_text(
        json.dumps(
            {
                "families": [
                    {
                        "family": "full",
                        "sha256": hashlib.sha256(admitted).hexdigest(),
                        "case_count": 1,
                    }
                ]
            }
        )
    )
    (base / "full.json").write_bytes(admitted)
    payload = base / f"full/{identity}.json.gz"
    payload.parent.mkdir()
    payload.write_bytes(gzip.compress(fixtures.canonical(case), mtime=0))
    monkeypatch.setattr(fixtures, "WORKSPACE", tmp_path)
    monkeypatch.setattr(fixtures, "FIXTURES", base)
    assert fixtures.full(["full"])[0]["executed"] == 1
    corrupt = copy.deepcopy(index)
    corrupt["oracle_revision"] = "other"
    del corrupt["corpus_sha256"]
    corrupt["corpus_sha256"] = fixtures.digest(corrupt)
    (base / "full.json").write_text(json.dumps(corrupt))
    with pytest.raises(ValueError, match="Registry index byte"):
        fixtures.full(["full"])
    # Even without a registry byte pin, a self-consistent wrong revision is rejected.
    (base / "registry.json").write_text('{"families":[{"family":"full"}]}')
    with pytest.raises(ValueError, match="oracle_revision"):
        fixtures.full(["full"])


def test_cache_roundtrip_keeps_order_null_instances_and_reference_identity(tmp_path, monkeypatch):
    sys.path.insert(0, str(ROOT / "scripts/rust_oracle"))
    import fixture_cache

    monkeypatch.setattr(fixture_cache, "FIXTURES", tmp_path)
    case = {
        "ordered": {"z": None, "a": False},
        "instances": [{"printed_id": "same"}, {"printed_id": "same"}],
        "hand": [{"$goa2_instance": 0}, {"$goa2_instance": 1}],
        "pending": {"$goa2_instance": 0},
        "seed": 2**64 - 1,
    }
    encoded = json.dumps(case, separators=(",", ":")).encode()
    path = tmp_path / "case.json"
    fixture_cache.write_bytes(path, encoded)
    assert fixture_cache.read_bytes(path) == encoded
    restored = json.loads(fixture_cache.read_bytes(path))
    assert list(restored["ordered"]) == ["z", "a"]
    assert restored["ordered"]["z"] is None and restored["ordered"]["a"] is False
    assert len(restored["instances"]) == 2 and restored["pending"] == restored["hand"][0]
    assert restored["hand"][0] != restored["hand"][1] and restored["seed"] == 2**64 - 1
    reordered = {**case, "ordered": {"a": False, "z": None}}
    fixtures = tool("fixtures")
    assert fixtures.digest(case) != fixtures.digest(reordered)


def test_baseline_projection_preserves_order_and_never_records_private_payloads():
    from rust_verification.baselines import Recorder, command_projection, projection

    secret = "PRIVATE-CREDENTIAL-SENTINEL"
    value = {
        "type": "STATE_UPDATE",
        "revision": 4,
        "token": secret,
        "checkpoint": secret,
        "replay_json": secret,
        "events": [
            {"event_type": "FIRST", "metadata": {"secret": secret}},
            {"event_type": "SECOND"},
        ],
        "input_request": {
            "type": "SELECT_CARD",
            "request_id": secret,
            "context": {"secret": secret},
        },
        "view": {
            "phase": "RESOLUTION",
            "teams": {},
            "clock": {"status": "RUNNING", "token": secret},
        },
    }
    result = projection(value)
    assert secret not in json.dumps(result)
    assert result["ordered_event_types"] == ["FIRST", "SECOND"] and result["revision"] == 4
    assert secret not in json.dumps(
        command_projection(
            {
                "request_id": secret,
                "token": secret,
                "selection": {"hero_id": "hero_arien", "token": secret},
            }
        )
    )
    recorder = Recorder()
    raw = "rust-" + "a" * 32
    assert recorder.path("/games/" + raw + "/input?token=" + secret) == "/games/game-1/input"
    assert recorder.path("/__lab/games/" + raw + "/replay") == "/__lab/games/game-1/replay"


def test_named_gate_rejects_missing_inputs_and_zero_test_selection(monkeypatch):
    gates = tool("gates")
    with pytest.raises(ValueError, match="Missing explicit gate inputs"):
        gates.render({"required_variables": ["ABSENT"], "commands": []}, {})
    gate = {
        "id": "zero",
        "scope": "test",
        "required_variables": [],
        "commands": [{"argv": ["fake-command"], "minimum_tests": 1}],
    }

    class Result:
        returncode = 0
        stdout = "test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n"

    monkeypatch.setattr(gates.subprocess, "run", lambda *args, **kwargs: Result())
    with pytest.raises(ValueError, match="selected 0 tests"):
        gates.execute(gate, {})


def test_gate_uses_explicit_service_source_environment(monkeypatch):
    gates = tool("gates")
    observed = []
    gate = {
        "id": "local",
        "scope": "test",
        "required_variables": ["SERVICE_SOURCE"],
        "environment": {"PYTHONPATH": "${SERVICE_SOURCE}"},
        "commands": [{"argv": ["fake-command"]}],
    }

    class Result:
        returncode = 0
        stdout = "passed\n"

    def run(*args, **kwargs):
        observed.append(kwargs["env"]["PYTHONPATH"])
        return Result()

    monkeypatch.setattr(gates.subprocess, "run", run)
    assert gates.execute(gate, {"SERVICE_SOURCE": "/explicit/source"})["status"] == "passed"
    assert observed == ["/explicit/source"]


def test_pinned_wrapper_copies_only_admitted_metadata_not_full_payloads(tmp_path):
    sys.path.insert(0, str(ROOT / "scripts/rust_oracle"))
    import run_pinned

    source = tmp_path / "source"
    destination = tmp_path / "destination"
    fixtures = source / "rust/tests/fixtures"
    fixtures.mkdir(parents=True)
    (fixtures / "stack-traces.json").write_text('{"cases":[]}')
    (fixtures / "models.json").write_text("ignored monolithic full corpus")
    (fixtures / "private-game.json").write_text("synthetic private sentinel")
    (fixtures / "stack-traces").mkdir()
    (fixtures / "stack-traces/case.json").write_text("full case")
    copied = run_pinned.copy_fixture_metadata(source, destination)
    assert copied == ["stack-traces.json"]
    assert sorted(p.name for p in (destination / "rust/tests/fixtures").iterdir()) == copied


def test_crates_io_remains_disabled_for_every_workspace_crate():
    import tomllib

    workspace = tomllib.loads((ROOT / "rust/Cargo.toml").read_text())
    assert workspace["workspace"]["package"]["publish"] is False
    crates = list((ROOT / "rust/crates").glob("*/Cargo.toml"))
    assert len(crates) == 3
    for path in crates:
        package = tomllib.loads(path.read_text())["package"]
        assert package["publish"] == {"workspace": True}, path
