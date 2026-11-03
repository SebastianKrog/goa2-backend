"""Source contracts for the native action constructors and retained live models."""

import json

from action_builders import catalog
from live_card_action_traces import edge_scenes
from object_execution_traces import reference
from oracle import ROOT


def test_action_constructor_catalog_matches_frozen_source():
    assert catalog() == json.loads((ROOT / "rust/tests/fixtures/action-builders.json").read_text())


def test_action_edges_keep_negative_values_primary_semantics_and_piece_inputs():
    checked = 0
    for name, state, operations in edge_scenes():
        records = reference(state, operations)
        checked += 1
        final = records[-1]["semantic_state"]
        context = final["execution_context"]
        if "piece-input" in name:
            assert not final["execution_stack"]
            assert (
                final["acting_piece_id"]
                == state.get_piece_ids("hero_razzle")[-1 if name.endswith("last") else 0]
            )
            assert context["choice_alias"]["pending_input"]["selection"] == final["acting_piece_id"]
            if name.endswith("invalid"):
                requests = [r["request"] for r in records if r["request"]]
                assert len(requests) == 2
                # Submission consumes the old token before the invalid value re-prompts.
                assert requests[0]["id"] != requests[1]["id"]
        elif name.endswith(("offboard", "no-card", "missing-hero")):
            assert context["current_card_id"] == "old-card"
            assert context["current_action_type"] == "old-action"
            assert not final["execution_stack"]
        elif name.endswith(("raw-defense-skill", "negative-attack")):
            steps = list(reversed(final["execution_stack"]))
            assert steps[0]["type"] == "choose_acting_piece"
            assert any(step["type"] == "resolve_card_text" for step in steps)
            assert any(step.get("trigger") == "after_primary_action" for step in steps)
            if name.endswith("negative-attack"):
                assert any(
                    step.get("key") == "basic_action_value" and step.get("value") == -3
                    for step in steps
                )
            else:
                assert context["current_action_type"] == "DEFENSE_SKILL"
        elif name.endswith("facedown"):
            assert context["current_action_type"] == "HOLD"
            assert not any(step["type"] == "resolve_card_text" for step in final["execution_stack"])
        elif name.endswith("piece-operand"):
            assert records[0]["request"]["player_id"] == "hero_razzle_piece_1"
    assert checked == 10
