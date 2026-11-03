"""Process export must preserve oracle values, ordering and alias identities."""

import copy
from unittest.mock import patch

import object_execution_traces as traces
from live_gydion_primary_traces import action_scene
from stack_traces import read_case


def test_parallel_source_export_matches_serial_snapshots_and_aliases(tmp_path):
    samples = []
    for effect, variant in (("cantrip", "ordinary"), ("prepare_spells", "archwizard-third-fizzle")):
        state, ops, _ = action_scene(effect, variant=variant)
        samples.append((effect + "-" + variant, state, ops))
    with patch.object(traces, "scenes", lambda: iter(copy.deepcopy(samples))):
        serial_path, parallel_path = tmp_path / "serial.json", tmp_path / "parallel.json"
        serial = traces.export(serial_path)
        parallel = traces.export(parallel_path, workers=2)
    assert serial["semantic_sha256"] == parallel["semantic_sha256"]
    assert [(e["name"], e["case_sha256"]) for e in serial["cases"]] == [
        (e["name"], e["case_sha256"]) for e in parallel["cases"]
    ]
    for left, right in zip(serial["cases"], parallel["cases"], strict=True):
        assert read_case(serial_path, left) == read_case(parallel_path, right)


def test_export_rejects_invalid_worker_budget():
    import pytest

    with pytest.raises(ValueError, match="must be positive"):
        next(traces._export_cases([], 0))
