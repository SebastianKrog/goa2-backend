"""Private card equality is part of offline continuation state."""

from __future__ import annotations

import pytest
from card_equality_traces import cases
from model_traces import setup
from state_codec import export_state_with_instances, materialize


def test_private_revision_changes_equality_without_changing_visible_state():
    selected = {case["name"]: case for case in cases() if case["name"] in {"equal", "revision"}}
    assert materialize(selected["equal"]["envelope"]) == materialize(
        selected["revision"]["envelope"]
    )
    assert selected["equal"]["expected"][1] is True
    assert selected["revision"]["expected"][1] is False
    assert selected["equal"]["envelope"] != selected["revision"]["envelope"]
    case = selected["revision"]
    physical = case["queries"][1]["b"]
    assert case["envelope"]["instances"][physical]["equality"]["definition_revision"] == 1


def test_non_json_metadata_is_marked_instead_of_silently_collapsing_types():
    state = setup()
    card = next(iter(state.teams.values())).heroes[0].hand[0]
    card.metadata = {"pair": (1, 2)}
    envelope, objects = export_state_with_instances(state)
    physical = next(i for i, item in enumerate(objects) if item is card)
    assert envelope["instances"][physical]["equality"]["json_metadata"] is False
    assert card.metadata["pair"] == (1, 2)


def test_boundary_rejects_malformed_equality_metadata():
    case = next(cases())
    physical = case["queries"][0]["a"]
    envelope = case["envelope"]
    envelope["instances"][physical]["equality"]["model"] = ""
    with pytest.raises(ValueError, match="equality metadata"):
        materialize(envelope)
