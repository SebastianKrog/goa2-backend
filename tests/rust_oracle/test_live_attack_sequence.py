"""Source contracts for attack macro expansion and retained model identity.

These raw boundary fixtures intentionally stop before reaction/defense bodies;
normal card-effect author helpers do not expose the required alias graph.
"""

import copy

import pytest
from live_attack_sequence_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def attack_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(attack_scenes, variant):
    state, ops = copy.deepcopy(attack_scenes["attack-macro-" + variant])
    return state, reference(state, ops)


def test_child_order_and_custom_target_keys(attack_scenes):
    state, records = run(attack_scenes, "default")
    children = list(reversed(state.execution_stack))[:-1]
    assert [step.type.value for step in children] == [
        "select_step",
        "reaction_window",
        "set_actor",
        "check_passive_abilities",
        "resolve_pre_action_movement",
        "resolve_pre_action_discard",
        "set_actor",
        "resolve_defense_text",
        "resolve_combat",
        "resolve_on_block_effect",
        "restore_action_type",
    ]
    assert (
        children[0].output_key
        == children[1].target_player_key
        == children[8].target_key
        == "victim_id"
    )
    assert children[2].actor_key == "defender_id" and children[2].save_key == "_pre_pam_actor"
    assert children[5].abort_on_defeat is False
    assert not records[0]["events"]
    state, _ = run(attack_scenes, "existing-output")
    children = list(reversed(state.execution_stack))[:-1]
    assert children[0].type.value == "reaction_window" and children[0].target_player_key == "chosen"


def test_history_marks_mutate_the_original_dictionary(attack_scenes):
    for variant, key in (
        ("default", "victim_id"),
        ("marks-key-alias", "defeat_marks"),
        ("existing", "chosen"),
    ):
        state, _ = run(attack_scenes, variant)
        ctx = state.execution_context
        assert ctx["defeat_marks"] is ctx["marks_alias"]
        assert ctx["defeat_marks"][key] == 1 and ctx["defeat_marks"]["previous"] == 0
        assert ctx["defeat_log"] is ctx["log_alias"]


def test_optional_attack_copy_shares_filters_but_keeps_original_operands(attack_scenes):
    state, ops = copy.deepcopy(attack_scenes["attack-macro-optional-shared"])
    reference(state, ops[:1])
    original = state.execution_context["attack_alias"]
    continuation, selector = state.execution_stack[-2:]
    assert continuation is not original and continuation.step_id == original.step_id
    assert (
        continuation.target_filters
        is original.target_filters
        is state.execution_context["filters_alias"]
    )
    assert selector.filters[2] is selector.filters[4] is state.execution_context["filter_alias"]
    assert continuation.target_id_key == continuation.active_if_key == "victim_id"
    assert original.target_id_key is None and original.active_if_key is None
    assert continuation.target_output_key is None


def test_optional_skip_never_expands_defense_or_switches_actor(attack_scenes):
    state, _ = run(attack_scenes, "optional-skip")
    assert state.current_actor_id == "hero_swift"
    assert [step.type.value for step in state.execution_stack] == ["set_context_flag"]
    assert "victim_id" not in state.execution_context
    state, _ = run(attack_scenes, "optional-first")
    assert state.execution_stack[-1].type.value == "reaction_window"
    assert state.execution_context["victim_id"]


def test_bonus_coercion_and_negative_attacks(attack_scenes):
    for variant, damage, maximum in (
        ("negative", -16, 2),
        ("zero", 0, 2),
        ("negative-range", 4, -3),
        ("bonus-int", -3, 4),
        ("bonus-bool", 5, 4),
        ("bonus-float", 2, 4),
        ("bonus-string", 14, 4),
        ("bonus-missing", 4, 2),
    ):
        state, _ = run(attack_scenes, variant)
        assert state.execution_context["attack_damage"] == damage
        assert state.execution_stack[-1].filters[0].max_range == maximum


def test_basic_card_context_and_per_attack_flags(attack_scenes):
    for variant, basic in (
        ("basic-current", True),
        ("colored-current", False),
        ("performing-card", True),
    ):
        state, _ = run(attack_scenes, variant)
        assert state.execution_context["attack_is_basic"] is basic
    for variant in ("default", "ranged", "initiative"):
        state, _ = run(attack_scenes, variant)
        assert state.execution_context["attack_is_ranged"] is (variant == "ranged")
        assert state.execution_context["defense_uses_initiative"] is (variant == "initiative")


def test_missing_targets_and_handler_gates(attack_scenes):
    for variant in (
        "missing-mandatory",
        "missing-optional",
        "target-zero",
        "target-false",
        "target-empty",
        "skip",
        "active",
    ):
        state, _ = run(attack_scenes, variant)
        assert state.execution_context["attack_damage"] == "old"
        assert state.execution_context["defeat_marks"] == {"previous": 0}
    state, _ = run(attack_scenes, "fallback-output")
    assert state.execution_stack[-1].output_key == "new_target"


def test_history_length_and_new_marks_follow_source_sized_values(attack_scenes):
    for variant, length in (
        ("tuple-log", 1),
        ("dict-log", 2),
        ("string-log", 3),
        ("shared-tracking", 1),
        ("marks-absent", 1),
    ):
        state, _ = run(attack_scenes, variant)
        assert state.execution_context["defeat_marks"]["victim_id"] == length
    state, _ = run(attack_scenes, "shared-tracking")
    assert state.execution_context["defeat_marks"] is state.execution_context["defeat_log"]
    state, _ = run(attack_scenes, "marks-absent")
    assert state.execution_context["defeat_marks"] is not state.execution_context["marks_alias"]


def test_piece_actors_keep_owner_items_and_physical_actor_identity(attack_scenes):
    for variant in ("owner", "bound-0", "bound-1", "piece"):
        state, _ = run(attack_scenes, "piece-" + variant)
        assert state.execution_context["attack_damage"] == 0
        assert state.execution_stack[-1].filters[0].max_range == 2
        assert state.execution_context["attacker_id"] == str(state.current_actor_id)


def test_conditional_aura_reads_the_performing_card_for_this_attack(attack_scenes):
    results = {}
    for variant in (
        "basic",
        "nonbasic",
        "ctx-basic",
        "ctx-nonbasic",
        "card-facedown",
        "ctx-owner-preferred",
    ):
        state, _ = run(attack_scenes, "aura-conditional-" + variant)
        results[variant] = state.execution_context["attack_is_basic"]
    assert results == {
        "basic": True,
        "nonbasic": False,
        "ctx-basic": True,
        "ctx-nonbasic": False,
        "card-facedown": True,
        "ctx-owner-preferred": True,
    }
