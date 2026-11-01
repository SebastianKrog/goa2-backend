"""Frozen-source execution contracts for live shared arena bindings."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))


def test_shared_step_reprompts_and_all_stored_references_observe_pending_changes():
    from object_execution_traces import reference, scenes

    _, state, operations = next(scene for scene in scenes() if scene[0] == "shared-pending-step")
    records = reference(state, operations)
    assert records[1]["error"] == "invalid_input"
    assert records[3]["request"]["id"] == records[4]["request"]["id"]
    assert len({records[i]["request"]["id"] for i in (0, 3, 6)}) == 3
    step = records[6]["semantic_state"]["execution_context"]["left"]
    assert step["pending_input"] is None
    assert step["pending_request_id"] == records[6]["request"]["id"]
    assert state.execution_context["left"] is state.execution_context["right"]
    assert state.active_effects[0].finishing_steps[0] is state.active_effects[1].finishing_steps[0]
    assert state.execution_context["left"].pending_request_id is None
    assert state.execution_context["left"].pending_input is None


def test_record_target_preserves_live_future_set_payload_and_new_list_aliases():
    from object_execution_traces import reference, scenes

    checked = set()
    for name, state, operations in scenes():
        if name not in ("live-set-payload", "new-shared-list"):
            continue
        reference(state, operations)
        context = state.execution_context
        if name == "live-set-payload":
            assert context["result"] is context["history"]
            assert context["result"] == [1, 2]
        else:
            assert context["history"][0] is context["target"]
            assert context["history"][1] is context["target"]
        checked.add(name)
    assert checked == {"live-set-payload", "new-shared-list"}


def test_request_normalization_only_changes_typed_step_request_fields():
    from object_execution_traces import ObjectTraceNormalizer

    state = {
        "step": {"pending_request_id": "raw-token", "step_id": "same-id"},
        "comment": "raw-token",
    }
    record = {
        "semantic_state": state,
        "request": {"id": "raw-token"},
        "object_bindings": [{"kind": "step", "paths": [["step"]]}],
    }
    result = ObjectTraceNormalizer().record(record)
    assert result["semantic_state"]["step"]["pending_request_id"] == "request/1"
    assert result["semantic_state"]["step"]["step_id"] == "same-id"
    assert result["semantic_state"]["comment"] == "raw-token"
    assert state["step"]["pending_request_id"] == "raw-token"


def test_capture_rejects_mutable_aliases_hidden_inside_physical_card_payloads():
    import pytest
    from model_traces import setup
    from shared_objects import export_state_with_shared_objects

    state = setup()
    card = state.get_hero("hero_arien").hand[0]
    card.metadata = {"history": []}
    state.execution_context = {"history": card.metadata["history"]}
    with pytest.raises(ValueError, match="physical instance child aliases"):
        export_state_with_shared_objects(state)
    state.execution_context = card.metadata
    with pytest.raises(ValueError, match="physical instance child aliases"):
        export_state_with_shared_objects(state)


def test_physical_card_reference_itself_remains_a_valid_shared_atomic_value():
    from model_traces import setup
    from shared_objects import export_state_with_shared_objects

    state = setup()
    card = state.get_hero("hero_arien").hand[0]
    state.execution_context = {"card": card}
    envelope, _ = export_state_with_shared_objects(state)
    assert "$goa2_instance" in envelope["state"]["execution_context"]["card"]


def test_for_each_records_original_aggregate_and_physical_item_references():
    from object_execution_traces import reference, scenes

    _, state, operations = next(scene for scene in scenes() if scene[0] == "loop-reference-items")
    original = list(state.execution_context["items"])
    reference(state, operations)
    assert all(
        actual is expected
        for actual, expected in zip(state.execution_context["seen"], original, strict=True)
    )
    assert state.execution_context["stored"].current_index == len(original)


def test_template_deepcopy_has_one_memo_per_root_and_detaches_physical_payloads():
    from object_execution_traces import reference, scenes

    checked = set()
    for name, state, operations in scenes():
        if not name.startswith("loop-deep-"):
            continue
        original = state.execution_context["original"]
        reference(state, operations)
        copies = state.execution_context["seen"]
        assert len(copies) == 4
        assert len({id(value) for value in copies}) == 4
        if name == "loop-deep-world-list":
            for value in copies:
                assert value is not original
                assert value[-1] is state.execution_context["extra"]
                assert all(
                    copied is not source
                    for copied, source in zip(value[:-1], original, strict=True)
                )
        else:
            for value in copies:
                assert value["left"] is value["right"]
                if name == "loop-deep-frozen":
                    assert value["left"] is original["left"]
                else:
                    assert value["left"] is not original["left"]
        checked.add(name)
    assert len(checked) == 9


def test_loop_rechecks_mutated_lists_but_does_not_requeue_after_last_item():
    from object_execution_traces import reference, scenes

    expected = {"loop-append": [1, 2, 3], "loop-append-last": [1], "loop-replace": [1]}
    for name, state, operations in scenes():
        if name in expected:
            reference(state, operations)
            assert state.execution_context["seen"] == expected.pop(name)
    assert not expected


def test_shared_loop_objects_share_their_counter_but_equal_independent_loops_do_not():
    from object_execution_traces import reference, scenes

    checked = set()
    for name, state, operations in scenes():
        if name not in ("loop-shared", "loop-independent"):
            continue
        reference(state, operations)
        assert state.execution_context["stored"].current_index == 2
        assert len(state.execution_context["seen"]) == (2 if name == "loop-shared" else 4)
        checked.add(name)
    assert len(checked) == 2


def test_capture_distinguishes_exact_hex_copy_sharing_from_frozen_subclass_copies():
    from object_execution_traces import scenes
    from shared_objects import export_state_with_shared_objects

    checked = set()
    for name, state, _ in scenes():
        if name not in ("loop-deep-frozen", "loop-deep-frozen-subclass"):
            continue
        envelope, _ = export_state_with_shared_objects(state)
        binding = next(
            binding
            for binding in envelope["object_bindings"]
            if ["execution_context", "original", "left"] in binding["paths"]
        )
        assert binding["kind"] == "frozen_model"
        assert binding.get("deepcopy_shared", False) == (name == "loop-deep-frozen")
        checked.add(name)
    assert len(checked) == 2


def test_loop_number_copies_have_independent_requests_and_leave_templates_pristine():
    from object_execution_traces import reference, scenes

    _, state, operations = next(scene for scene in scenes() if scene[0] == "loop-input")
    records = reference(state, operations)
    assert records[0]["request"]["id"] != records[2]["request"]["id"]
    assert state.execution_context["seen"] == [2, 1]
    assert state.execution_context["number_template"].pending_input is None
    assert state.execution_context["number_template"].pending_request_id is None


def test_action_scope_preserves_present_null_and_absence_separately():
    from object_execution_traces import reference, scenes

    for name, state, operations in scenes():
        if name not in ("scope-presence-missing", "scope-presence-present-null"):
            continue
        reference(state, operations)
        context = state.execution_context
        assert "action_context_stack" not in context
        if name.endswith("present-null"):
            assert "current_card_id" in context and context["current_card_id"] is None
        else:
            assert "current_card_id" not in context
        assert context["unrelated"] == "before"


def test_action_scope_restore_keeps_live_aggregate_references_and_unrelated_changes():
    from object_execution_traces import reference, scenes

    _, state, operations = next(scene for scene in scenes() if scene[0] == "scope-restore-2")
    reference(state, operations)
    context = state.execution_context
    assert context["token_type_override"] is context["outer_values"]
    assert context["snapshot"]["token_type_override"]["value"] is context["outer_values"]
    assert context["outer_values"] == [1, 7]
    assert context["unrelated"] == "after"
    assert context["current_card_id"] == "outer/card"
    assert context["stack_alias"] == []
    assert "action_context_stack" not in context


def test_nested_abort_unwinds_own_actions_and_preserves_other_hero_boundaries():
    from object_execution_traces import reference, scenes

    checked = set()
    for name, state, operations in scenes():
        if name not in ("scope-abort-own", "scope-abort-other", "scope-abort-extra-scopes"):
            continue
        reference(state, operations)
        context = state.execution_context
        assert context["current_card_id"] == "outer/card"
        assert "action_context_stack" not in context
        assert context["stack_alias"] == []
        assert "skipped" not in context
        assert ("after_outer" in context) == (name == "scope-abort-other")
        checked.add(name)
    assert len(checked) == 3


def test_copied_loop_abort_boundaries_use_the_same_source_scope_unwinding():
    from object_execution_traces import reference, scenes

    for name, state, operations in scenes():
        if not name.startswith("scope-loop-abort-"):
            continue
        reference(state, operations)
        other = name.endswith("True")
        assert state.execution_context["stored_loop"].current_index == (2 if other else 1)
        assert ("after_boundary" in state.execution_context) == other
        assert "action_context_stack" not in state.execution_context


def test_action_type_restore_pops_shared_list_and_assigns_the_original_model_reference():
    from object_execution_traces import reference, scenes

    for name, state, operations in scenes():
        if name not in ("scope-action-type-shared", "scope-action-type-model"):
            continue
        reference(state, operations)
        context = state.execution_context
        assert context["action_type_stack"] == []
        if name.endswith("shared"):
            assert context["action_type_stack"] is context["mirror"]
        else:
            assert context["current_action_type"] is context["original"]


def test_actor_save_precedes_key_lookup_and_swaps_preserve_controller_remapping():
    from object_execution_traces import reference, scenes

    for name, state, operations in scenes():
        if name == "scope-actor-same-save-key":
            records = reference(state, operations)
            assert records[0]["request"]["player_id"] == "hero_arien"
            assert state.execution_context["candidate"] == "hero_arien"
        elif name == "scope-actor-swap-True":
            records = reference(state, operations)
            assert records[0]["request"]["player_id"] == "hero_hanu"
            assert "controlled_hero_id" not in records[0]["request"]["context"]
            assert records[2]["request"]["context"]["controlled_hero_id"] == "hero_arien"
            assert state.current_actor_id == "hero_arien"


def test_effect_expiry_replaces_the_root_list_without_mutating_old_aliases():
    from object_execution_traces import reference, scenes

    _, state, operations = next(scene for scene in scenes() if scene[0] == "effects-all-durations")
    old = state.active_effects
    reference(state, operations)
    assert state.active_effects == []
    assert state.execution_context["old_effects"] is old and len(old) == 4
    assert state.execution_context["card"].is_active_base is False


def test_effect_flag_changes_do_not_change_previously_deepcopied_card_payloads():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "effects-physical-copy-snapshot"
    )
    reference(state, operations)
    context = state.execution_context
    assert context["card"].is_active_base is False
    assert [card.is_active_base for card in context["copies"]] == [False, True]
    assert all(card is not context["card"] for card in context["copies"])


def test_expiry_returns_shared_original_step_lists_without_running_them():
    from object_execution_traces import scenes

    from goa2.domain.models.effect import DurationType
    from goa2.engine.effect_manager import EffectManager

    _, state, _ = next(scene for scene in scenes() if scene[0] == "effects-expired-finishing-True")
    effect = state.active_effects[0]
    result = EffectManager.expire_effects(state, DurationType.THIS_ROUND)
    assert len(result) == 2
    assert result[0][1] is result[1][1] is effect.finishing_steps
    assert "after_expiry" not in state.execution_context


def test_removed_effects_still_share_pending_state_with_their_queued_finishing_steps():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "effects-live-finishing-RESOLUTION"
    )
    reference(state, operations)
    assert state.active_effects == []
    stored = state.execution_context["saved_effect"].finishing_steps[0]
    assert stored.pending_input is None and stored.pending_request_id is None
    assert state.execution_context["chosen"] == 2


def test_old_effect_lists_are_detached_mutable_data_after_root_replacement():
    from object_execution_traces import reference, scenes

    _, state, operations = next(scene for scene in scenes() if scene[0] == "effects-detached-list")
    reference(state, operations)
    assert state.active_effects == []
    old = state.execution_context["old_effects"]
    assert len(old) == 2 and old[0] is old[1]


def test_finishing_marker_stops_abort_cascades_and_restores_action_scope_first():
    from object_execution_traces import reference, scenes

    checked = set()
    for name, state, operations in scenes():
        if name not in (
            "effects-finishing-marker-plain",
            "effects-finishing-marker-scope",
            "effects-finishing-marker-copied-loop",
        ):
            continue
        reference(state, operations)
        assert state.execution_context["after_marker"] is True
        assert "skip_this" not in state.execution_context
        assert state.execution_context["current_card_id"] == "outer"
        assert "action_context_stack" not in state.execution_context
        checked.add(name)
    assert len(checked) == 3


def test_after_reveal_queues_original_payloads_with_distinct_generated_wrapper_identities():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "scheduled-after-cards-same-object"
    )
    original = state.active_effects[0].finishing_steps[0]
    records = reference(state, operations[:1])
    assert state.execution_stack.count(original) == 2
    assert state.active_effects == []
    wrappers = [
        step
        for step in state.execution_stack
        if step is not original and not step.step_id.startswith("fixture/")
    ]
    assert len(wrappers) == 4
    assert len({id(step) for step in wrappers}) == 4
    normalized = records[0]["semantic_state"]["execution_stack"]
    ids = [step["step_id"] for step in normalized if step["step_id"].startswith("dynamic-step/")]
    assert len(set(ids)) == 4


def test_after_reveal_removal_uses_effect_type_rather_than_duplicate_id():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "scheduled-after-cards-duplicate-id"
    )
    old = state.active_effects
    survivor = old[1]
    reference(state, operations)
    assert state.active_effects == [survivor]
    assert state.active_effects[0] is survivor
    assert state.execution_context["old_effects"] is old
    assert len(old) == 2
    assert state.execution_context["card"].is_active
    assert state.execution_context["count"] == 1


def test_stale_cross_round_future_and_empty_after_reveal_triggers_do_not_fire():
    from object_execution_traces import reference, scenes

    wanted = {
        "scheduled-after-cards-" + variant
        for variant in ("stale", "future", "cross-round", "empty")
    }
    checked = set()
    for name, state, operations in scenes():
        if name not in wanted:
            continue
        reference(state, operations)
        assert not state.active_effects
        assert "count" not in state.execution_context
        assert not state.execution_context["card"].is_active
        checked.add(name)
    assert checked == wanted


def test_turn_expiry_collection_does_not_remove_effects_or_copy_finishing_lists():
    from object_execution_traces import scenes

    from goa2.engine.effect_manager import EffectManager

    _, state, _ = next(
        scene for scene in scenes() if scene[0] == "scheduled-turn-expiry-same-object"
    )
    old = state.active_effects
    finishing, ids = EffectManager.collect_turn_expiry(state)
    assert state.active_effects is old
    assert len(finishing) == len(ids) == 2
    assert finishing[0][1] is finishing[1][1] is old[0].finishing_steps
    assert state.execution_context["card"].is_active
    EffectManager.remove_effects(state, ids)
    assert not state.active_effects
    assert not state.execution_context["card"].is_active


def test_scheduled_mandatory_abort_stops_at_marker_and_preserves_outer_tail():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "scheduled-after-cards-abort"
    )
    reference(state, operations)
    assert "count" not in state.execution_context
    assert state.execution_context["after"] is True
    assert not state.execution_stack


def test_detached_finishing_selection_observes_options_mutated_before_requeue():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "scheduled-detached-mutable-options"
    )
    records = reference(state, operations)
    options = records[3]["request"]["options"]
    assert [option["id"] for option in options] == ["1", "2"]
    assert state.execution_context["chosen"] == 2
    assert (
        state.execution_context["saved"].finishing_steps[0].number_options
        is state.execution_context["history"]
    )


def test_rule_v9_turn_finishing_defers_effect_removal_until_advance():
    from object_execution_traces import reference, scenes

    wanted = {f"turn-boundary-version-{version}-turn-1" for version in (9,)}
    checked = set()
    for name, state, operations in scenes():
        if name not in wanted:
            continue
        records = reference(state, operations)
        live = name.endswith("9-turn-1")
        assert bool(records[0]["semantic_state"]["active_effects"]) is live
        assert records[0]["semantic_state"]["execution_context"]["card"]["is_active_base"] is live
        assert records[-2]["semantic_state"]["turn"] == 1
        assert records[-1]["semantic_state"]["turn"] == 2
        assert not state.active_effects
        checked.add(name)
    assert checked == wanted


def test_turn_snapshot_is_a_new_dictionary_with_original_hex_model_references():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "turn-boundary-no-effects-1"
    )
    previous = state.last_turn_positions
    reference(state, operations)
    assert state.last_turn_positions is not previous
    assert state.last_turn_positions is not state.entity_locations
    assert all(
        state.last_turn_positions[key] is value for key, value in state.entity_locations.items()
    )
    assert state.execution_context["old_snapshot"] is previous
    assert state.execution_context["old_log"]
    assert state.turn_discard_log == {}


def test_auto_pass_mutates_pending_dict_in_place_until_complete_revelation():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "turn-boundary-auto-pass-one"
    )
    old = state.pending_inputs
    reference(state, operations)
    assert state.pending_inputs is old is state.execution_context["old_pending"]
    assert state.pending_inputs == {"hero_arien": None}
    assert state.phase.value == "PLANNING"


def test_all_pass_revelation_updates_the_same_played_card_list_and_physical_card():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "turn-boundary-auto-pass-all-current"
    )
    played = state.get_hero("hero_arien").played_cards
    card = state.get_hero("hero_arien").current_turn_card
    reference(state, operations[:1])
    assert state.phase.value == "CLEANUP"
    assert state.turn == 4
    assert state.get_hero("hero_arien").played_cards is played is state.execution_context["played"]
    assert played == [None, card, None, None]
    assert card.state.value == "RESOLVED"
    assert state.get_hero("hero_arien").current_turn_card is None
    assert state.pending_inputs == {}
    assert state.pending_inputs is not state.execution_context["old_pending"]


def test_after_reveal_trigger_runs_before_repeated_auto_pass_advances_again():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "turn-boundary-auto-pass-all-trigger"
    )
    records = reference(state, operations)
    assert records[0]["semantic_state"]["phase"] == "RESOLUTION"
    assert records[0]["semantic_state"]["turn"] == 2
    assert records[3]["semantic_state"]["execution_context"]["after_auto_pass"] is True
    assert records[3]["semantic_state"]["turn"] == 2
    assert state.turn == 4
    assert state.phase.value == "CLEANUP"


def test_round_expiry_removes_effects_before_scheduling_their_original_finishing_steps():
    from object_execution_traces import reference, scenes

    _, state, operations = next(
        scene for scene in scenes() if scene[0] == "turn-boundary-end-phase-flag"
    )
    effect = state.active_effects[0]
    step = effect.finishing_steps[0]
    reference(state, operations[:1])
    assert not state.active_effects
    assert step in state.execution_stack
    assert state.execution_context["effect"] is effect
    assert not state.execution_context["card"].is_active


def _revelation_cases():
    from model_traces import setup
    from object_execution_traces import revelation_scenes

    return revelation_scenes(lambda red=None: setup(red=red))


def test_revelation_detaches_buffers_but_preserves_known_frozen_record_identity():
    from object_execution_traces import reference

    _, state, operations = next(
        scene for scene in _revelation_cases() if scene[0] == "revelation-known-card"
    )
    pending = state.pending_inputs
    known = state.public_revealed_cards[0]
    reference(state, operations)
    assert state.pending_inputs is not pending
    assert state.execution_context["old_pending"] is pending
    assert isinstance(state.public_revealed_cards, tuple)
    assert state.public_revealed_cards[0] is known is state.execution_context["known"]
    assert len(state.public_revealed_cards) == 4
    assert all(not card.is_facedown for card in pending.values())


def test_two_card_retrieval_precedes_scheduled_trigger_and_retains_the_same_hand():
    from object_execution_traces import reference

    _, state, operations = next(
        scene for scene in _revelation_cases() if scene[0] == "revelation-dual-trigger-after"
    )
    first = state.execution_context["first"]
    hand = state.execution_context["hand"]
    records = reference(state, operations)
    assert records[1]["request"]["request_type"] == "SELECT_CARD"
    assert first in hand
    assert state.execution_context["returned_hand"] is hand
    assert state.get_hero("hero_emmitt").extra_turn_card is None
    assert first.initiative_override is None


def test_reverse_initiative_uses_only_its_global_activation_and_due_turn_predicate():
    from object_execution_traces import reference

    expected = {
        "reversed": "hero_tigerclaw",
        "token-passive": "hero_tigerclaw",
        "inactive": "hero_arien",
        "stale": "hero_arien",
        "cross-round": "hero_arien",
    }
    checked = set()
    for name, state, operations in _revelation_cases():
        variant = name.removeprefix("revelation-rank-")
        if variant not in expected:
            continue
        reference(state, operations)
        assert state.current_actor_id == expected[variant]
        checked.add(variant)
    assert checked == expected.keys()


def test_cross_team_tie_only_queues_the_coin_flip_after_confirmation():
    from object_execution_traces import reference

    from goa2.domain.models import TeamColor
    from goa2.engine.steps.utility import FlipTieBreakerCoinStep

    _, state, operations = next(
        scene for scene in _revelation_cases() if scene[0] == "revelation-tie-cross"
    )
    original = state.tie_breaker_team
    reference(state, operations)
    steps = list(reversed(state.execution_stack))
    assert state.tie_breaker_team is original is TeamColor.RED
    assert any(isinstance(step, FlipTieBreakerCoinStep) for step in steps)
    assert steps[1].type.value == "confirm_resolution"
    assert isinstance(steps[2], FlipTieBreakerCoinStep)
    assert steps[3].type.value == "finalize_hero_turn"


def test_tie_consumer_reads_the_live_candidate_list_and_preserves_pool_identity():
    from object_execution_traces import reference

    _, state, operations = next(
        scene for scene in _revelation_cases() if scene[0] == "revelation-tie-append-candidate"
    )
    pool = state.unresolved_hero_ids
    ties = state.execution_context["ties"]
    reference(state, operations)
    assert state.current_actor_id == "hero_tigerclaw"
    assert state.unresolved_hero_ids is pool
    assert state.execution_context["tie"].tied_hero_ids is ties
    assert "hero_tigerclaw" in ties


def test_retrieval_keeps_finished_step_input_and_omits_cleared_override_without_changing_copy():
    from object_execution_traces import reference

    _, state, operations = next(
        scene for scene in _revelation_cases() if scene[0] == "revelation-retrieve-override"
    )
    step = state.execution_context["step"]
    first = state.execution_context["first"]
    reference(state, operations)
    assert step.pending_input == {"selection": first.id}
    assert step.pending_request_id == "fixture/dirty-retrieval"
    assert first.state.value == "HAND"
    assert "initiative_override" not in first.model_dump(mode="json")
    assert state.execution_context["card_copy"].initiative_override == 30


def test_auto_pass_observes_the_two_card_open_gate_and_explicit_done_signal():
    from object_execution_traces import reference

    checked = set()
    for name, state, operations in _revelation_cases():
        variant = name.removeprefix("revelation-auto-pass-")
        if variant not in ("open", "done", "two"):
            continue
        records = reference(state, operations)
        if variant == "open":
            assert state.phase.value == "PLANNING"
            assert state.execution_context["first"].is_facedown
        else:
            assert state.phase.value == "RESOLUTION"
            assert not state.execution_context["first"].is_facedown
            if variant == "two":
                assert records[-1]["request"]["request_type"] == "SELECT_CARD"
        checked.add(variant)
    assert checked == {"open", "done", "two"}
