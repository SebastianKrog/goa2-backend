"""Trace normalization must preserve all semantic identities and collection order."""

import copy
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))
from stack_traces import TraceNormalizer


def test_only_typed_request_ids_are_normalized_without_mutating_source():
    record = {
        "request": {"id": "ephemeral"},
        "semantic_state": {
            "execution_stack": [
                {
                    "step_id": "step/meaningful",
                    "pending_request_id": "ephemeral",
                    "metadata": {
                        "id": "card_instance_7",
                        "pending_request_id": "semantic_metadata",
                    },
                    "steps_template": [{"step_id": "child", "pending_request_id": "other"}],
                }
            ],
            "execution_context": {"target_id": "hero_hanu", "id": "ephemeral"},
        },
        "events": [{"metadata": {"id": "token_1"}}],
        "error": None,
    }
    before = copy.deepcopy(record)
    result = TraceNormalizer().record(record)
    assert record == before
    assert result["request"]["id"] == "request/1"
    step = result["semantic_state"]["execution_stack"][0]
    assert step["pending_request_id"] == "request/1"
    assert step["steps_template"][0]["pending_request_id"] == "request/2"
    assert step["metadata"] == before["semantic_state"]["execution_stack"][0]["metadata"]
    assert step["step_id"] == "step/meaningful"
    assert (
        result["semantic_state"]["execution_context"]
        == before["semantic_state"]["execution_context"]
    )
    assert result["events"] == before["events"]


def test_normalization_does_not_add_missing_fields_and_reuses_request_identity():
    normalizer = TraceNormalizer()
    record = {
        "request": {"id": "first"},
        "semantic_state": {"execution_stack": [{"step_id": "only_step"}]},
        "events": [],
        "error": None,
    }
    first = normalizer.record(record)
    assert first["semantic_state"]["execution_stack"] == [{"step_id": "only_step"}]
    assert normalizer.record(record) == first


def test_worker_transport_preserves_semantic_dictionary_order(monkeypatch):
    import io
    import json

    import oracle

    class Process:
        stdin = io.StringIO()
        stdout = io.StringIO('{"schema_version":1,"id":1,"ok":true,"result":{}}\n')

    process = Process()
    monkeypatch.setattr(oracle.subprocess, "Popen", lambda *args, **kwargs: process)
    worker = oracle.Worker(["unused"])
    worker.request("stack_trace", envelope={"board": {"tiles": {"z": {}, "a": {}}}})
    sent = json.loads(process.stdin.getvalue())
    assert list(sent["envelope"]["board"]["tiles"]) == ["z", "a"]


def test_oracle_identity_includes_semantic_dictionary_order():
    from oracle import digest

    assert digest({"z": 1, "a": 2}) != digest({"a": 2, "z": 1})


def test_generated_python_frames_keep_identity_even_when_raw_step_labels_collide():
    class Step:
        step_id = "reused-python-address"
        steps_template = ()

    one, two = Step(), Step()
    normalizer = TraceNormalizer()
    normalizer.bind_steps([one, two])
    record = {
        "request": None,
        "events": [],
        "error": None,
        "semantic_state": {"execution_stack": [{"step_id": one.step_id}, {"step_id": two.step_id}]},
    }
    first = normalizer.record(record, [one, two])
    assert [step["step_id"] for step in first["semantic_state"]["execution_stack"]] == [
        "dynamic-step/1",
        "dynamic-step/2",
    ]
    record["semantic_state"]["execution_stack"] = [{"step_id": two.step_id}]
    again = normalizer.record(record, [two])
    assert again["semantic_state"]["execution_stack"][0]["step_id"] == "dynamic-step/2"


def test_changed_oracle_runtime_is_rejected(monkeypatch):
    import oracle
    import pytest

    monkeypatch.setattr(oracle, "reference_runtime", lambda: {"python": "different runtime"})
    with pytest.raises(ValueError, match="runtime drift"):
        oracle.verify_sources()


def test_indexed_trace_storage_preserves_complete_records_and_dictionary_order(tmp_path):
    import json

    from oracle import digest
    from stack_traces import read_case, read_index, write_corpus

    case = {
        "name": "sample",
        "envelope": {"ordered": {"z": 1, "a": 2}},
        "operations": [{"kind": "advance"}],
        "expected": [
            {"semantic_state": {"id": "keep-me"}, "events": [{"metadata": {"id": "also-keep"}}]}
        ],
    }
    source = {
        "schema_version": 1,
        "oracle_revision": "frozen",
        "source_sha256": "pinned",
        "cases": [case],
    }
    source["corpus_sha256"] = digest(source)
    before = json.dumps(source)
    path = tmp_path / "traces.json"
    index = write_corpus(path, source)
    loaded, identity = read_index(path)
    assert identity == index["corpus_sha256"]
    assert loaded["semantic_sha256"] == source["corpus_sha256"]
    assert json.dumps(source) == before
    assert json.dumps(read_case(path, loaded["cases"][0])) == json.dumps(case)
    assert list(read_case(path, loaded["cases"][0])["envelope"]["ordered"]) == ["z", "a"]


def test_indexed_trace_storage_rejects_changed_snapshot_and_escaping_path(tmp_path):
    import json

    import pytest
    from oracle import digest
    from stack_traces import read_case, write_corpus

    case = {"name": "sample", "envelope": {}, "operations": [], "expected": []}
    source = {"schema_version": 1, "source_sha256": "pinned", "cases": [case]}
    source["corpus_sha256"] = digest(source)
    path = tmp_path / "traces.json"
    entry = write_corpus(path, source)["cases"][0]
    changed = {**entry, "path": "../outside.json"}
    with pytest.raises(ValueError, match="path mismatch"):
        read_case(path, changed)
    (path.parent / entry["path"]).write_text(json.dumps({**case, "expected": ["changed"]}))
    with pytest.raises(ValueError, match="content identity mismatch"):
        read_case(path, entry)


def test_indexed_trace_storage_rejects_modified_index_and_duplicate_names(tmp_path):
    import json

    import pytest
    from oracle import digest
    from stack_traces import read_index, write_corpus

    source = {
        "schema_version": 1,
        "source_sha256": "pinned",
        "cases": [{"name": "sample", "envelope": {}, "operations": [], "expected": []}],
    }
    source["corpus_sha256"] = digest(source)
    path = tmp_path / "traces.json"
    index = write_corpus(path, source)
    path.write_text(json.dumps({**index, "source_sha256": "changed"}))
    with pytest.raises(ValueError, match="index identity mismatch"):
        read_index(path)
    index.pop("corpus_sha256")
    index["cases"].append(index["cases"][0])
    path.write_text(json.dumps({**index, "corpus_sha256": digest(index)}))
    with pytest.raises(ValueError, match="duplicate trace"):
        read_index(path)


def test_atomic_reference_boundaries_preserve_events_and_generated_hooks():
    from stack_traces import reference_trace

    from goa2.domain.board import Board
    from goa2.domain.models import GamePhase, Team, TeamColor
    from goa2.domain.state import GameState
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import PlaceMarkerStep, RemoveMarkerStep

    state = GameState(
        board=Board(),
        teams={c: Team(color=c, heroes=[]) for c in TeamColor},
        phase=GamePhase.RESOLUTION,
        rules_version=9,
    )
    first = PlaceMarkerStep(
        marker_type="venom", target_id="target", value=-2, step_id="fixture/place"
    )
    last = RemoveMarkerStep(marker_type="venom", step_id="fixture/remove")
    push_steps(state, [first, last])
    records = reference_trace(state, [{"kind": "advance_prefix", "steps": 1} for _ in range(3)])
    assert [e["event_type"] for e in records[0]["events"]] == ["MARKER_PLACED"]
    assert records[0]["semantic_state"]["execution_stack"][-1]["type"] == "check_passive_abilities"
    assert records[1]["events"] == []
    assert [e["event_type"] for e in records[2]["events"]] == ["MARKER_REMOVED"]
    assert records[2]["semantic_state"]["markers"]["venom"]["target_id"] is None


def test_trace_export_prunes_only_superseded_generated_hash_files(tmp_path):
    from oracle import digest
    from stack_traces import read_case, write_corpus

    path = tmp_path / "traces.json"

    def source(value):
        data = {
            "schema_version": 1,
            "source_sha256": "pinned",
            "cases": [{"name": "sample", "envelope": {}, "operations": [], "expected": [value]}],
        }
        return {**data, "corpus_sha256": digest(data)}

    first = write_corpus(path, source("before"))
    old = tmp_path / first["cases"][0]["path"]
    notes = path.parent / path.stem / "notes.txt"
    notes.write_text("keep")
    second = write_corpus(path, source("after"))
    assert not old.exists()
    assert notes.read_text() == "keep"
    assert read_case(path, second["cases"][0])["expected"] == ["after"]


@pytest.mark.parametrize("can_link", [True, False])
def test_streaming_merge_preserves_full_semantic_digest_and_order(tmp_path, monkeypatch, can_link):
    import stack_traces

    if not can_link:

        def unsupported_link(*args):
            raise OSError("cross-device link")

        monkeypatch.setattr(stack_traces.os, "link", unsupported_link)
    from oracle import digest
    from stack_traces import merge_corpora, read_case, write_corpus

    def case(name, value):
        return {"name": name, "envelope": {}, "operations": [], "expected": [value]}

    def source(cases):
        data = {"schema_version": 1, "source_sha256": "pinned", "cases": cases}
        return {**data, "corpus_sha256": digest(data)}

    path = tmp_path / "traces.json"
    additions = tmp_path / "more.json"
    first = case("first", {"b": 2, "a": 1})
    old = case("replace", "before")
    new = case("replace", "after")
    last = case("unicode", "\u00e5")
    original = write_corpus(path, source([first, old]))
    unchanged = path.parent / original["cases"][0]["path"]
    original_bytes = unchanged.read_bytes()
    write_corpus(additions, source([new, last]))
    index = merge_corpora(path, additions)
    assert index["semantic_sha256"] == source([first, new, last])["corpus_sha256"]
    assert [read_case(path, entry) for entry in index["cases"]] == [first, new, last]
    assert unchanged.read_bytes() == original_bytes
    assert not (path.parent / original["cases"][1]["path"]).exists()


def test_streaming_merge_rejects_metadata_and_case_corruption_before_index_swap(tmp_path):
    import pytest
    from oracle import digest
    from stack_traces import merge_corpora, write_corpus

    def source(pinned):
        data = {
            "schema_version": 1,
            "source_sha256": pinned,
            "cases": [{"name": "sample", "envelope": {}, "operations": [], "expected": []}],
        }
        return {**data, "corpus_sha256": digest(data)}

    path = tmp_path / "traces.json"
    additions = tmp_path / "more.json"
    write_corpus(path, source("pinned"))
    before = path.read_bytes()
    write_corpus(additions, source("changed"))
    with pytest.raises(ValueError, match="incompatible"):
        merge_corpora(path, additions)
    assert path.read_bytes() == before
    index = write_corpus(additions, source("pinned"))
    (additions.parent / index["cases"][0]["path"]).write_text("{}")
    with pytest.raises(ValueError, match="content identity"):
        merge_corpora(path, additions)
    assert path.read_bytes() == before


def test_interrupted_case_export_preserves_installed_snapshot(tmp_path, monkeypatch):
    from oracle import digest
    from stack_traces import read_case, write_corpus

    case = {"name": "sample", "envelope": {}, "operations": [], "expected": ["keep"]}
    data = {"schema_version": 1, "source_sha256": "pinned", "cases": [case]}
    source = {**data, "corpus_sha256": digest(data)}
    path = tmp_path / "traces.json"
    index = write_corpus(path, source)
    before = path.read_bytes()
    original = Path.write_text

    def interrupted(self, text, *args, **kwargs):
        if self.suffix == ".tmp":
            original(self, "partial")
            raise OSError("disk full")
        return original(self, text, *args, **kwargs)

    monkeypatch.setattr(Path, "write_text", interrupted)
    with pytest.raises(OSError, match="disk full"):
        write_corpus(path, source)
    assert path.read_bytes() == before
    assert read_case(path, index["cases"][0]) == case


def test_retry_replaces_orphaned_link_without_overwriting_its_source(tmp_path):
    from oracle import digest
    from stack_traces import merge_corpora, read_case, write_corpus

    case = {"name": "sample", "envelope": {}, "operations": [], "expected": ["keep"]}
    data = {"schema_version": 1, "source_sha256": "pinned", "cases": [case]}
    source = {**data, "corpus_sha256": digest(data)}
    path, additions = tmp_path / "traces.json", tmp_path / "more.json"
    index = write_corpus(path, source)
    incoming = write_corpus(additions, source)
    original = additions.parent / incoming["cases"][0]["path"]
    target = path.parent / index["cases"][0]["path"]
    temporary = target.with_suffix(".json.tmp")
    try:
        temporary.hardlink_to(original)
    except OSError:
        pytest.skip("filesystem does not support hard links")
    before = original.read_bytes()
    merged = merge_corpora(path, additions)
    assert original.read_bytes() == before
    assert read_case(path, merged["cases"][0]) == case
    assert not temporary.exists()
