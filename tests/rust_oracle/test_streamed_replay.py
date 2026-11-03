"""Bounded replay keeps source semantics and rejects incomplete native streams."""

from __future__ import annotations

import copy
import json
import sys
from itertools import pairwise

import pytest
from model_traces import setup
from object_execution_traces import ObjectTraceNormalizer, iter_reference, reference
from streamed_games import StreamWorker, apply_snapshot_edits, snapshot_delta

from goa2.engine.steps import SetContextFlagStep


def test_reference_iterator_executes_only_operations_consumed():
    state = setup()
    state.execution_stack = [
        SetContextFlagStep(key="later", value=2),
        SetContextFlagStep(key="first", value=1),
    ]
    ops = [{"kind": "advance_prefix", "steps": 1}] * 2
    records = iter_reference(state, ops)
    assert "first" not in state.execution_context
    first = next(records)
    assert state.execution_context["first"] == 1 and "later" not in state.execution_context
    assert first["semantic_state"]["execution_context"]["first"] == 1
    second = next(records)
    assert (
        state.execution_context["later"] == 2
        and "later" not in first["semantic_state"]["execution_context"]
    )
    assert second["error"] is None
    with pytest.raises(StopIteration):
        next(records)


def test_collected_reference_retains_error_and_snapshot_contract():
    state = setup()
    rows = reference(
        state,
        [
            {"kind": "input", "request_id": "stale", "selection": None},
            {"kind": "advance_prefix", "steps": 0},
        ],
    )
    assert len(rows) == 2 and rows[0]["error"] == "invalid_input" and rows[1]["error"] is None
    assert {
        "semantic_state",
        "object_bindings",
        "context_keys",
        "instance_bindings",
        "request",
        "events",
        "error",
    } == set(rows[0])


def apply(before, edits):
    return apply_snapshot_edits(copy.deepcopy(before), copy.deepcopy(edits))


def test_snapshot_deltas_are_lossless_for_null_arrays_removal_and_literal_keys():
    samples = [
        None,
        {"a/b~": [{"old": 1}, 2], "remove": True},
        {"a/b~": [{"new": None}, [3, 4]]},
        {"a/b~": [1, 2, 3]},
        {},
    ]
    for left, right in pairwise(samples):
        edits = list(snapshot_delta(left, right))
        assert apply(left, json.loads(json.dumps(edits))) == right


def test_delta_updates_do_not_retain_unchanged_large_subtrees():
    before = {"large": [{"value": i} for i in range(1000)]}
    after = copy.deepcopy(before)
    after["large"][500]["value"] = -1
    edits = list(snapshot_delta(before, after))
    assert edits == [{"op": "set", "path": ["large", 500, "value"], "value": -1}]


@pytest.mark.parametrize(
    "packet",
    [
        {"schema_version": 1, "id": 2, "ok": True, "result": {"complete": True, "records": 0}},
        {"schema_version": 1, "id": 1, "ok": True, "result": {"sequence": 1, "record": {}}},
        {"schema_version": 1, "id": 1, "ok": True, "result": {"complete": True, "records": 0}},
    ],
)
def test_worker_rejects_identity_sequence_and_truncated_streams(packet):
    program = (
        "import sys; sys.stdin.readline(); print("
        + repr(json.dumps(packet))
        + "); sys.stdout.flush()"
    )
    worker = StreamWorker([sys.executable, "-c", program])
    try:
        with pytest.raises(ValueError):
            list(worker.records({}, [{}]))
    finally:
        worker.abort()
    assert worker.process.stdin.closed and worker.process.stdout.closed


def test_array_insertion_preserves_unchanged_binding_rows_without_copying_them():
    before = [{"row": i, "paths": [["state", i]]} for i in range(1000)]
    after = [*before[:500], {"new": True}, *before[500:]]
    edits = list(snapshot_delta(before, after))
    assert edits == [
        {"op": "splice", "path": [], "index": 500, "delete": 0, "values": [{"new": True}]}
    ]
    assert apply(before, edits) == after


def test_fresh_stream_records_can_normalize_in_place_without_changing_default_copy_contract():
    record = {
        "semantic_state": {"execution_stack": []},
        "object_bindings": [],
        "request": {"id": "native-request-8"},
    }
    normalized = ObjectTraceNormalizer().record(record)
    assert normalized is not record and record["request"]["id"] == "native-request-8"
    assert ObjectTraceNormalizer().record(record, inplace=True) is record
    assert record["request"]["id"] == "request/1"
