import copy

from mine_traces import normalize_choice_record


def test_mine_choice_normalization_preserves_ordered_gameplay_data():
    path = [{"q": 1, "r": 0, "s": -1}, {"q": 0, "r": 0, "s": 0}]
    record = {
        "semantic_state": {
            "execution_context": {"chosen": ["b", "a"], "trigger_queue": ["b", "a"]}
        },
        "events": [{"target_id": "b"}, {"target_id": "a"}],
        "request": {
            "prompt": "Choose which mines to move through",
            "options": [
                {
                    "id": "1",
                    "metadata": {"metadata": {"mine_hexes": path.copy(), "path": path.copy()}},
                },
                {"id": "0", "metadata": {"metadata": {"mine_hexes": [], "path": path.copy()}}},
            ],
        },
    }
    original = copy.deepcopy(record)
    normalize_choice_record(record, ["chosen"])
    assert record["semantic_state"]["execution_context"]["chosen"] == ["a", "b"]
    assert (
        record["semantic_state"]["execution_context"]["trigger_queue"]
        == original["semantic_state"]["execution_context"]["trigger_queue"]
    )
    assert record["events"] == original["events"]
    assert [o["id"] for o in record["request"]["options"]] == ["1", "0"]
    assert record["request"]["options"][0]["metadata"]["metadata"]["path"] == path
    assert record["request"]["options"][0]["metadata"]["metadata"]["mine_hexes"] == list(
        reversed(path)
    )


def test_recorded_mine_reference_defects_reproduce():
    from mine_traces import verify_errors
    from oracle import ROOT

    result = verify_errors(ROOT / "rust/tests/fixtures/mine-reference-errors.json")
    assert result["reference_errors"] == 3
    assert result["reproduced"]
