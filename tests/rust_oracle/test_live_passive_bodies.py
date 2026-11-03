"""Frozen behavior contracts for accepted passive construction and rune choices."""

from live_passive_body_traces import scenes as body_scenes
from live_rune_traces import scenes as rune_scenes
from object_execution_traces import reference


def test_body_trigger_exceptions_rune_order_and_empty_usage():
    wanted = {
        "passive-body-bullet_time-after_basic_action-wrong-trigger",
        "passive-body-rune_mastery-before_action-many-runes",
        "passive-body-rune_mastery-before_action-duplicate-runes",
        "passive-body-rune_mastery-before_defense-many-runes",
        "passive-body-rune_mastery-before_action-no-runes",
        "passive-body-flurry_of_blows-after_attack-accepted",
        "passive-body-march_of_nature-after_resolve_card-radius-item",
    }
    checked = set()
    for name, state, operations in body_scenes():
        if name not in wanted:
            continue
        checked.add(name)
        before_radius = state.teams[next(iter(state.teams))].heroes[0].ultimate_card.radius_value
        record = reference(state, operations)[0]["semantic_state"]
        stack = record["execution_stack"]
        if name.endswith(("no-runes", "flurry_of_blows-after_attack-accepted")):
            assert stack == []
        elif "rune_mastery" in name:
            step = stack[-1]
            assert step["options"] == (
                ["bird", "horn"] if "duplicate-runes" in name else ["bird", "anvil", "horn"]
            )
            assert step["output_key"] == (
                "snorri_ult_rune_defense" if "before_defense" in name else "snorri_ult_rune_action"
            )
        elif "bullet_time" in name:
            assert stack[-1]["type"] == "perform_primary_action"
        else:
            assert stack[-1]["filters"][0]["max_range"] == before_radius
    assert checked == wanted


def test_supported_passives_complete_and_mark_only_after_their_body():
    checked = 0
    for name, state, operations in body_scenes():
        if not name.endswith(("-complete", "-complete-runes")):
            continue
        checked += 1
        result = reference(state, operations)[-1]["semantic_state"]
        assert result["execution_stack"] == []
        hero = result["teams"]["RED"]["heroes"][0]
        assert hero["ultimate_card"]["passive_uses_this_turn"] == 1
        if name.endswith("complete-runes"):
            output = (
                "snorri_ult_rune_defense" if "before_defense" in name else "snorri_ult_rune_action"
            )
            assert result["execution_context"][output] == "bird"
        else:
            assert sum(len(cards) for cards in result["turn_discard_log"].values()) == 1
    assert checked == 3


def test_rune_retries_preserve_aliases_and_unmapped_stale_values():
    wanted = {
        "rune-pending-invalid",
        "rune-pending-null",
        "rune-unmapped",
        "rune-single-invalid",
        "rune-no-actor",
        "rune-empty",
    }
    checked = set()
    for name, state, operations in rune_scenes():
        if name not in wanted:
            continue
        checked.add(name)
        records = reference(state, operations)
        if name in {"rune-pending-invalid", "rune-pending-null"}:
            first = records[0]["semantic_state"]["execution_context"]
            assert first["step_alias"]["pending_input"] is None
            assert first["pending_alias"] is not None
        final = records[-1]["semantic_state"]["execution_context"]
        if name == "rune-unmapped":
            assert final["rune"] == "bird" and final["matched"] is None
            assert final["mapped"] == "old"
        if name == "rune-single-invalid":
            assert final["rune"] == "axe" and final["matched"] is True
            assert final["mapped"] == "A"
            assert final["step_alias"]["pending_request_id"] is not None
        if name == "rune-no-actor":
            assert records[0]["request"]["player_id"] == ""
        if name == "rune-empty":
            assert records[0]["request"]["options"] == []
            assert final["rune"] == "old"
    assert checked == wanted
