"""Streaming export preserves the existing content and identity contract."""

from oracle import digest
from stack_traces import read_case, read_index, write_corpus, write_corpus_stream


def test_streaming_corpus_is_identical_to_complete_snapshot_export(tmp_path):
    metadata = {"schema_version": 1, "oracle_revision": "test", "source_sha256": "test"}
    cases = [
        {"name": "a", "expected": [{"order": {"z": 1, "a": 2}}]},
        {"name": "b", "expected": []},
    ]
    complete = {**metadata, "cases": cases}
    complete["corpus_sha256"] = digest(complete)
    old_path, streamed_path = tmp_path / "old/corpus.json", tmp_path / "stream/corpus.json"
    old = write_corpus(old_path, complete)
    streamed = write_corpus_stream(streamed_path, metadata, (case for case in cases))
    assert old == streamed
    index, identity = read_index(streamed_path)
    assert identity == streamed["corpus_sha256"]
    assert [read_case(streamed_path, entry) for entry in index["cases"]] == cases


def test_merge_reuses_verified_encoding_with_identical_unicode_identity(tmp_path):
    from stack_traces import merge_corpora

    metadata = {"schema_version": 1, "oracle_revision": "test", "source_sha256": "test"}
    before = [{"name": "a", "expected": [1]}, {"name": "b", "expected": [2]}]
    additions = [
        {"name": "a", "expected": ["æ → 😀"]},
        {"name": "c", "expected": [{"z": 1, "a": 2}]},
    ]
    target = tmp_path / "target/corpus.json"
    extra = tmp_path / "extra/additions.json"
    expected = tmp_path / "expected/corpus.json"
    write_corpus_stream(target, metadata, iter(before))
    write_corpus_stream(extra, metadata, iter(additions))
    full = {**metadata, "cases": [before[1], *additions]}
    full["corpus_sha256"] = digest(full)
    assert merge_corpora(target, extra) == write_corpus(expected, full)
    index, _ = read_index(target)
    assert [read_case(target, e) for e in index["cases"]] == full["cases"]


def test_merge_detects_corrupted_snapshot_without_replacing_index(tmp_path):
    import pytest
    from stack_traces import merge_corpora

    metadata = {"schema_version": 1, "oracle_revision": "test", "source_sha256": "test"}
    target = tmp_path / "target/corpus.json"
    extra = tmp_path / "extra/additions.json"
    write_corpus_stream(target, metadata, iter([{"name": "old", "expected": []}]))
    additions = write_corpus_stream(extra, metadata, iter([{"name": "new", "expected": []}]))
    snapshot = target.read_bytes()
    path = extra.parent / additions["cases"][0]["path"]
    path.write_text('{"name":"new","expected":[1]}')
    with pytest.raises(ValueError, match="content identity mismatch"):
        merge_corpora(target, extra)
    assert target.read_bytes() == snapshot


def test_merge_accepts_equivalent_pretty_snapshot_with_same_semantic_identity(tmp_path):
    import json

    from stack_traces import merge_corpora

    metadata = {"schema_version": 1, "source_sha256": "test"}
    target, extra = tmp_path / "target.json", tmp_path / "extra.json"
    original = {"name": "old", "expected": ["æ → 😀"]}
    added = {"name": "new", "expected": [3]}
    write_corpus_stream(target, metadata, iter([original]))
    incoming = write_corpus_stream(extra, metadata, iter([added]))
    (extra.parent / incoming["cases"][0]["path"]).write_text(json.dumps(added, indent=2))
    result = merge_corpora(target, extra)
    full = {**metadata, "cases": [original, added]}
    assert result["semantic_sha256"] == digest(full)
    index, _ = read_index(target)
    assert [read_case(target, entry) for entry in index["cases"]] == full["cases"]


def test_merge_rejects_valid_content_hash_under_a_false_entry_name(tmp_path):
    import json

    import pytest
    from stack_traces import merge_corpora

    metadata = {"schema_version": 1, "source_sha256": "test"}
    target, extra = tmp_path / "target.json", tmp_path / "extra.json"
    write_corpus_stream(target, metadata, iter([{"name": "keep", "expected": []}]))
    write_corpus_stream(extra, metadata, iter([{"name": "real", "expected": []}]))
    index, _ = read_index(extra)
    index["cases"][0]["name"] = "false"
    extra.write_text(json.dumps({**index, "corpus_sha256": digest(index)}))
    before = target.read_bytes()
    with pytest.raises(ValueError, match="content identity mismatch"):
        merge_corpora(target, extra)
    assert target.read_bytes() == before
