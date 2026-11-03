"""Compressed cache preserves exact fixtures, corruption checks and atomicity."""

import gzip
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))

import fixture_cache
import pytest
from oracle import digest
from stack_traces import merge_corpora, read_case, write_corpus_stream


@pytest.fixture
def cache(tmp_path, monkeypatch):
    monkeypatch.setattr(fixture_cache, "FIXTURES", tmp_path)
    return tmp_path


def test_compressed_export_and_merge_preserve_semantics_and_old_readers(cache):
    target = cache / "stack-traces.json"
    extra = cache / "object-arena.json"
    metadata = {"schema_version": 1, "source_sha256": "test"}
    first = {"name": "first", "ordered": {"z": 1, "a": "æ → 😀"}, "expected": [1]}
    second = {"name": "second", "expected": [2]}
    old = write_corpus_stream(target, metadata, iter([first]))
    path = cache / old["cases"][0]["path"]
    assert not path.exists() and fixture_cache.stored_path(path).exists()
    assert read_case(target, old["cases"][0]) == first
    write_corpus_stream(extra, metadata, iter([second]))
    extra_index, _ = __import__("stack_traces").read_index(extra)
    added_file = fixture_cache.stored_path(cache / extra_index["cases"][0]["path"])
    merged = merge_corpora(target, extra)
    merged_file = fixture_cache.stored_path(cache / merged["cases"][1]["path"])
    assert added_file.stat().st_ino == merged_file.stat().st_ino
    assert merged["semantic_sha256"] == digest({**metadata, "cases": [first, second]})
    assert [read_case(target, entry) for entry in merged["cases"]] == [first, second]
    assert read_case(target, old["cases"][0]) == first
    assert not list((cache / "stack-traces").glob("*.json"))


def test_corrupted_compressed_case_is_rejected(cache):
    target = cache / "stack-traces.json"
    index = write_corpus_stream(
        target, {"schema_version": 1}, iter([{"name": "a", "expected": []}])
    )
    entry = index["cases"][0]
    fixture_cache.stored_path(cache / entry["path"]).write_bytes(
        gzip.compress(b'{"name":"a","expected":[1]}')
    )
    with pytest.raises(ValueError, match="content identity mismatch"):
        read_case(target, entry)


def test_failed_compressed_publication_keeps_existing_cache(cache, monkeypatch):
    path = cache / "payload.json"
    fixture_cache.write_bytes(path, b"old")

    def fail(*args):
        raise OSError("interrupted")

    monkeypatch.setattr(fixture_cache.os, "replace", fail)
    with pytest.raises(OSError, match="interrupted"):
        fixture_cache.write_bytes(path, b"new")
    assert fixture_cache.read_bytes(path) == b"old"
    assert not list(cache.glob("*.tmp"))


def test_compression_is_deterministic_and_missing_cache_has_actionable_error(cache):
    first, second = cache / "one.json", cache / "two.json"
    for path in (first, second):
        fixture_cache.write_text(path, '{"x":1}')
    assert (
        fixture_cache.stored_path(first).read_bytes()
        == fixture_cache.stored_path(second).read_bytes()
    )
    with pytest.raises(FileNotFoundError, match="Regenerate"):
        fixture_cache.read_bytes(cache / "missing.json")


def test_offline_migration_verifies_and_preserves_hard_links(cache, monkeypatch):
    import os

    monkeypatch.setattr(fixture_cache, "ROOT", cache.parent)
    original = cache / "payload.json"
    original.write_bytes(b'{"ordered":{"z":1,"a":2}}\n')
    other = cache / "cases" / ("a" * 64 + ".json")
    other.parent.mkdir()
    os.link(original, other)
    result = fixture_cache.compress_existing(workers=2)
    assert result["files"] == 2
    assert not original.exists() and not other.exists()
    assert fixture_cache.read_bytes(original) == fixture_cache.read_bytes(other)
    assert (
        fixture_cache.stored_path(original).stat().st_ino
        == fixture_cache.stored_path(other).stat().st_ino
    )
    manifest = json.loads((cache / "cache-identities.json").read_text())
    assert manifest["payload.json"]["raw_bytes"] == len(fixture_cache.read_bytes(original))
