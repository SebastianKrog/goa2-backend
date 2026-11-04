"""Two-turn workload boundaries and the benchmark's narrow set comparison."""

import copy

import pytest
from oracle import ROOT
from simulation_bench import canonical_set_fields, ordinal, workloads
from stack_traces import read_case, read_index


def test_six_windows_are_verified_complete_two_turn_simulations():
    path = ROOT / "rust/tests/fixtures/complete-games-hanu.json"
    index, _ = read_index(path)
    names = []
    for entry in index["cases"]:
        for workload in workloads(read_case(path, entry)):
            assert ordinal(*workload["end"]) - ordinal(*workload["start"]) == 2
            assert workload["counts"]["requests"] > 0 and workload["counts"]["events"] > 0
            assert workload["counts"]["operations"] == len(workload["operations"])
            assert not any(op["kind"] == "advance_prefix" for op in workload["operations"])
            names.append(workload["name"])
    assert len(names) == len(set(names)) == 6


def test_only_zone_hex_sets_are_canonicalized_and_identity_paths_are_guarded():
    a, b = {"q": 0, "r": 0, "s": 0}, {"q": 1, "r": 0, "s": -1}
    record = {
        "object_bindings": [],
        "semantic_state": {
            "board": {"zones": {"Mid": {"hexes": [b, a]}}},
            "hand": ["b", "a"],
            "ordered_hexes": [b, a],
        },
    }
    result = canonical_set_fields(copy.deepcopy(record))
    assert result["semantic_state"]["board"]["zones"]["Mid"]["hexes"] == [a, b]
    assert result["semantic_state"]["hand"] == ["b", "a"]
    assert result["semantic_state"]["ordered_hexes"] == [b, a]
    record["object_bindings"] = [{"paths": [["board", "zones", "Mid", "hexes", 0]]}]
    with pytest.raises(ValueError, match="identity paths"):
        canonical_set_fields(record)
