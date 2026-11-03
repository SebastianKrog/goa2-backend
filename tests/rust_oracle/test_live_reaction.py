"""Frozen defense and live-actor contracts with retained arena alias witnesses.

Raw scenarios deliberately exercise handler boundaries and model identity rather
than normal effect-author setup helpers. No oracle source code is changed.
"""

import copy

import pytest
from live_reaction_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def reaction_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(reaction_scenes, name):
    state, ops = copy.deepcopy(reaction_scenes[name])
    return state, reference(state, ops)


def requests(records):
    return [row["request"] for row in records if row["request"] is not None]


def test_all_heroes_offer_defense_and_recover_from_stale_choices(reaction_scenes):
    from goa2.data.heroes.registry import HeroRegistry

    for hero in HeroRegistry._heroes:
        state, records = run(reaction_scenes, f"live-reaction-{hero}-prompt-invalid-pass")
        prompts = requests(records)
        assert len(prompts) == 2
        assert prompts[0]["id"] != prompts[1]["id"]
        assert prompts[0]["options"][-1]["id"] == "PASS"
        assert state.execution_context["defense_value"] is None
        assert state.execution_context["is_primary_defense"] is False


def test_reset_order_missing_targets_and_handler_gates(reaction_scenes):
    keys = (
        "auto_block",
        "defense_invalid",
        "defense_bonus",
        "ignore_minion_defense",
        "snorri_ult_rune_defense",
    )
    for variant in ("no-target", "false-target", "zero-target", "empty-target", "reset-target"):
        state, records = run(reaction_scenes, "live-reaction-context-" + variant)
        assert not requests(records)
        assert not any(key in state.execution_context for key in keys)
    for variant in ("skip", "active"):
        state, records = run(reaction_scenes, "live-reaction-context-" + variant)
        assert not requests(records)
        assert all(key in state.execution_context for key in keys)
    state, _ = run(reaction_scenes, "live-reaction-context-numeric-target")
    assert state.execution_context["defender_id"] == "17"


def test_attack_request_preserves_bool_float_and_negative_values(reaction_scenes):
    for variant, raw in (
        ("attack-none", None),
        ("attack-false", False),
        ("attack-true", True),
        ("attack-float", 2.5),
        ("attack-negative", -16),
    ):
        _state, records = run(reaction_scenes, "live-reaction-context-" + variant)
        prompt = requests(records)[0]
        if raw is None or isinstance(raw, bool):
            assert prompt["context"]["attack_value"] is raw
        else:
            assert prompt["context"]["attack_value"] == raw
        if raw is not None:
            assert f"Attack: {raw}," in prompt["prompt"]
            assert prompt["context"]["defense_needed"] == raw - prompt["context"]["minion_modifier"]


def test_selected_card_appends_to_original_action_stack(reaction_scenes):
    state, _ = run(reaction_scenes, "live-reaction-context-custom-target")
    ctx = state.execution_context
    assert ctx["action_type_stack"] is ctx["stack_alias"]
    assert ctx["action_type_stack"] == ["previous", "ATTACK"]
    assert ctx["current_action_type"] == "DEFENSE"
    assert ctx["defender_id"] == "hero_brogan"
    assert ctx["defense_card_id"] not in [card.id for card in ctx["defense_hand_alias"]]
    state, _ = run(reaction_scenes, "live-reaction-context-stack-aggregate-current")
    assert (
        state.execution_context["action_type_stack"][-1] is state.execution_context["current_alias"]
    )


def test_false_current_action_does_not_require_an_appendable_stack(reaction_scenes):
    for variant in (
        "stack-empty-current",
        "stack-false-current",
        "stack-null-current",
        "stack-tuple-no-current",
        "stack-null-no-current",
    ):
        state, _ = run(reaction_scenes, "live-reaction-context-" + variant)
        assert state.execution_context["current_action_type"] == "DEFENSE"
        if variant == "stack-null-no-current":
            assert state.execution_context["action_type_stack"] is None
        elif variant == "stack-tuple-no-current":
            assert state.execution_context["action_type_stack"] == ("previous",)
        else:
            assert state.execution_context["action_type_stack"] == ["previous"]


def test_stale_pending_payload_detaches_without_mutating_its_alias(reaction_scenes):
    state, records = run(reaction_scenes, "live-reaction-context-pending-invalid")
    assert state.execution_context["pending_alias"] == {"selection": "bogus"}
    assert (
        state.execution_context["reaction_alias"].pending_input
        is not state.execution_context["pending_alias"]
    )
    assert requests(records)
    state, records = run(reaction_scenes, "live-reaction-context-pending-pass")
    assert not requests(records)
    assert (
        state.execution_context["reaction_alias"].pending_input
        is state.execution_context["pending_alias"]
    )


def test_shield_deduplication_uses_model_equality_not_physical_identity(reaction_scenes):
    for variant, count in (
        ("alias-hand", 1),
        ("equal-hand", 1),
        ("unequal-hand", 2),
        ("duplicate-printed", 1),
        ("inactive", 0),
        ("facedown", 0),
    ):
        state, records = run(reaction_scenes, "live-reaction-shield-" + variant)
        shield = state.execution_context["shield_alias"]
        assert (
            sum(option["id"] == str(shield.id) for option in requests(records)[0]["options"])
            == count
        )
    state, _ = run(reaction_scenes, "live-reaction-shield-equal-hand")
    assert state.execution_context["clone_alias"] is not state.execution_context["shield_alias"]


def test_piece_defender_keeps_physical_id_and_routes_owner_prompt(reaction_scenes):
    for variant in ("owner", "piece-0", "piece-1", "piece-2", "piece-3", "initiative", "pass"):
        state, records = run(reaction_scenes, "live-reaction-piece-" + variant)
        prompt = requests(records)[0]
        assert prompt["player_id"] == prompt["decision_context"]["actor_id"] == "hero_razzle"
        assert state.execution_context["defender_id"] == state.execution_context["target_id"]
        expected = "INITIATIVE" if variant == "initiative" else "DEFENSE"
        assert all(
            fact["defense_stat"] == expected
            for fact in prompt["decision_context"]["choice_facts"].values()
        )


def test_reused_step_preserves_pending_pass_and_reopens_discarded_card_choice(reaction_scenes):
    for mode, count in (("pass", 1), ("card", 2)):
        state, records = run(reaction_scenes, "live-reaction-reused-" + mode)
        assert len(requests(records)) == count
        assert "auto_block" not in state.execution_context
        assert (
            state.execution_context["reaction_alias"]
            is state.execution_context["defeat_steps"][0]
            is state.execution_context["defeat_steps"][2]
        )


def test_attack_actor_switch_and_live_actor_save_precedence(reaction_scenes):
    for variant in ("minion", "hero-pass", "hero-card"):
        state, _ = run(reaction_scenes, "live-reaction-attack-" + variant)
        assert str(state.current_actor_id) == state.execution_context["defender_id"]
        assert state.execution_context["_pre_pam_actor"] == "hero_swift"
        assert "saved_actor_id" not in state.execution_context
    for variant, actor in (
        ("literal", "hero_hanu"),
        ("key-wins", "hero_brogan"),
        ("save-then-read", "hero_swift"),
        ("missing-key", "hero_swift"),
        ("numeric", "17"),
        ("boolean", "True"),
    ):
        state, _ = run(reaction_scenes, "live-reaction-actor-" + variant)
        assert str(state.current_actor_id) == actor
        assert state.execution_context["saved_custom"] == "hero_swift"
