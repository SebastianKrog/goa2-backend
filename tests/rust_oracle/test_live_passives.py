"""Source contracts for live passive eligibility, pending aliases and usage."""

from live_passive_traces import scenes
from object_execution_traces import reference


def test_all_custom_live_predicates_offer_and_negative_branches_filter():
    wanted = {
        "passive-scan-jinx-after_card_discard-enabled",
        "passive-scan-legend_of_the_skies-after_primary_action-enabled",
        "passive-scan-battle_fury-after_card_discard-enabled",
        "passive-scan-the_archwizard-before_prepare_spellbook-enabled",
        "passive-scan-rune_mastery-before_action-enabled",
        "passive-scan-bullet_time-after_basic_action-enabled",
        "passive-scan-reign_of_winter-after_attack-enabled",
        "passive-scan-dragon_knight-after_movement-enabled",
        "passive-predicate-jinx-capped",
        "passive-predicate-jinx-surrounded",
        "passive-predicate-battle_fury-blocked",
        "passive-predicate-dragon_knight-blocked",
        "passive-predicate-the_archwizard-spent-wish",
        "passive-predicate-reign_of_winter-old-defeat",
    }
    checked = set()
    for name, state, operations in scenes():
        if name not in wanted:
            continue
        checked.add(name)
        snapshot = reference(state, operations)[0]["semantic_state"]
        offers = [step for step in snapshot["execution_stack"] if step["type"] == "offer_passive"]
        assert bool(offers) == name.endswith("enabled"), name
    assert checked == wanted


def test_provider_marking_mutates_the_first_matching_physical_model():
    checked = set()
    for name, state, operations in scenes():
        if not name.startswith("passive-mark-provider-alias-"):
            continue
        checked.add(name)
        before = state.active_effects[0].passive_uses_this_turn
        result = reference(state, operations)[0]["semantic_state"]
        uses = [effect["passive_uses_this_turn"] for effect in result["active_effects"]]
        assert uses == ([before + 1, before + 1] if name.endswith("True") else [before + 1, before])
    assert len(checked) == 2


def test_passive_decline_keeps_original_pending_mapping_and_request_token():
    for name, state, operations in scenes():
        if name == "passive-offer-living_tsunami-before_attack-NO":
            step = state.execution_stack[-1]
            step.pending_request_id = "old-passive-token"
            snapshot = reference(state, operations)[0]["semantic_state"]
            alias = snapshot["execution_context"]["step_alias"]
            assert alias["pending_input"] == {"selection": "NO"}
            assert alias["pending_request_id"] is not None
            assert snapshot["execution_stack"] == []
            return
    raise AssertionError("missing passive decline fixture")
