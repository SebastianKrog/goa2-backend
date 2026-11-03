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


def test_public_planning_pass_and_finish_preserve_source_aliases_and_errors():
    from model_traces import setup
    from object_execution_traces import planning_command_scenes, reference

    checked = set()
    for name, state, operations in planning_command_scenes(lambda red=None: setup(red=red)):
        pending = state.pending_inputs
        done = state.planning_done
        requests = state.starting_position_requests
        records = reference(state, operations)
        if name.endswith(
            (
                "pass-nonempty",
                "pass-unknown",
                "pass-duplicate",
                "pass-wrong-phase",
                "finish-missing",
            )
        ):
            assert records[0]["error"] == "invalid_planning_action"
            assert state.pending_inputs is pending
            assert state.planning_done is done
            assert state.starting_position_requests is requests
        elif name.endswith("pass-requests"):
            assert state.starting_position_requests is not requests
            assert list(state.starting_position_requests) == ["keep"]
            assert state.starting_position_requests["keep"] is requests["keep"]
            assert len(requests) == 3
            assert state.pending_inputs is pending
        elif name.endswith("finish-wrong-phase"):
            assert records[0]["error"] is None
            assert state.planning_done is done and done == []
        elif name.endswith(("finish-last", "finish-dual-last", "pass-last")):
            assert state.phase.value == "RESOLUTION"
            assert state.pending_inputs is not pending
            assert state.planning_done is not done
        elif name.endswith(("finish", "finish-unknown-pending")):
            assert state.planning_done is done
            assert done == [operations[0]["hero"]]
        checked.add(name)
    assert len(checked) == 13


def test_card_planning_commands_preserve_lifo_aliases_and_rule_errors():
    from model_traces import setup
    from object_execution_traces import card_planning_scenes, reference

    checked = set()
    for name, state, operations in card_planning_scenes(lambda red=None: setup(red=red)):
        hero = next(iter(state.teams.values())).heroes[0]
        hand, done, pending = hero.hand, state.planning_done, state.pending_inputs
        first = state.execution_context["first"]
        requests = state.starting_position_requests
        records = reference(state, operations)
        assert state.execution_context["hand"] is hand
        if name.endswith("dual-cycle"):
            assert [row["error"] for row in records] == [
                None,
                None,
                "invalid_planning_action",
                None,
                None,
            ]
            assert state.pending_inputs == {} and state.pending_second_cards == {}
            assert hand[-2] is state.execution_context["second_card"] and hand[-1] is first
            assert hero.current_turn_card is None
        elif name.endswith("duplicate-done"):
            assert state.planning_done is done and done == [str(hero.id)]
        elif name.endswith("override-copy"):
            assert first.initiative_override is None
            assert state.execution_context["copy"].initiative_override == 19
            assert (
                records[0]["semantic_state"]["execution_context"]["first"]["initiative_override"]
                == 19
            )
        elif name.endswith("shared-deck"):
            assert hero.deck is hero.hand is hand and hand[-1] is first
        elif name.endswith("shared-hero-hand"):
            assert hero.hand is state.teams[hero.team].heroes[1].hand
            assert first not in hand
        elif name.endswith("requests"):
            if name.endswith("dual-requests"):
                assert state.starting_position_requests is requests
                assert len(requests) == 3
            else:
                assert state.starting_position_requests is not requests
                assert list(state.starting_position_requests) == ["keep"]
                assert state.starting_position_requests["keep"] is requests["keep"]
        elif name.endswith("dual-second-last"):
            assert records[-1]["request"] is not None
            assert state.phase.value == "RESOLUTION"
        elif name.endswith(("dual-first-last", "dual-last-card", "last")):
            assert state.phase.value == "RESOLUTION"
        elif name.endswith("spell-foreign"):
            assert records[0]["error"] is None and state.pending_inputs == {}
        elif name.endswith("uncommit-twice"):
            assert records[-1]["error"] == "invalid_planning_action"
            assert pending == {}
        elif name.endswith("wrong-phase"):
            assert records[0]["error"] is None
            assert records[1]["error"] == "invalid_planning_action"
        checked.add(name)
    assert len(checked) == 23


def test_live_confirmation_preserves_pending_aliases_flags_control_and_abort_boundaries():
    from model_traces import setup
    from object_execution_traces import confirmation_scenes, reference

    checked = set()
    for name, state, operations in confirmation_scenes(lambda red=None: setup(red=red)):
        step = state.execution_context["confirmation"]
        pending = step.pending_input
        records = reference(state, operations)
        if "rollback_" in name:
            flag = "rollback_frozen" if "rollback_frozen" in name else "rollback_reanchor_pending"
            if state.execution_context[flag]:
                assert records[0]["request"] is None
                assert step.pending_input is pending
                assert step.pending_request_id == "fixture/confirmation/frozen-request"
            else:
                assert records[0]["request"] is not None
                assert step.pending_input is None
        elif name.endswith("retained-valid"):
            assert step.pending_input is None
            assert state.execution_context["old_pending"] == {"selection": "CONFIRM"}
            assert step.pending_request_id == "fixture/confirmation/retained-request"
        elif name.endswith("controlled"):
            assert records[0]["request"]["player_id"] == "hero_hanu"
            assert records[0]["request"]["context"]["controlled_hero_id"] == "hero_arien"
        elif name.endswith("foreign-prompt"):
            assert records[0]["request"]["player_id"] == "hero_brogan"
        elif name.endswith("piece-owner"):
            assert records[0]["request"]["player_id"] == "hero_razzle_piece_1"
        elif name.endswith("stale"):
            assert records[1]["error"] == "invalid_input"
        elif "abort-" in name:
            assert "should_not_run" not in state.execution_context
            if name.endswith("finalize"):
                assert state.execution_stack[-1] is state.execution_context["boundary"]
            elif name.endswith("scope-confirm"):
                assert state.execution_context["current_card_id"] == "outer"
                assert not state.execution_context.get("action_context_stack")
        elif name.endswith(("loop", "repeated-step")):
            assert records[0]["request"]["id"] != records[2]["request"]["id"]
            assert state.execution_stack == []
        elif name.endswith("retry"):
            ids = [record["request"]["id"] for record in records if record["request"]]
            assert len(ids) == len(set(ids)) == 5
        checked.add(name)
    assert len(checked) == 32


def test_frozen_confirmation_preserves_even_truthy_nonmapping_pending_input():
    from model_traces import setup
    from object_execution_traces import confirmation_scenes, reference

    _, state, _ = next(confirmation_scenes(lambda red=None: setup(red=red)))
    step = state.execution_context["confirmation"]
    step.pending_input = 1
    state.execution_context["rollback_frozen"] = True
    records = reference(state, [{"kind": "advance"}])
    assert records[0]["request"] is None
    assert step.pending_input == 1


def test_live_finalization_preserves_slots_activation_counters_and_context_identity():
    from model_traces import setup
    from object_execution_traces import finalization_scenes, reference

    checked = set()
    for name, state, operations in finalization_scenes(lambda red=None: setup(red=red)):
        hero = next(iter(state.teams.values())).heroes[0]
        card = state.execution_context["card"]
        played, context = hero.played_cards, state.execution_context
        guess, reveal = state.card_guess, state.card_reveal
        records = reference(state, operations)
        if name.endswith("skipped"):
            assert state.execution_context is context and context
            assert records[0]["events"] == []
            continue
        assert state.execution_context is context and context == {}
        assert state.current_actor_id is None and state.resolution_owner_id is None
        assert state.acting_piece_id is None
        assert hero.played_cards is played
        if not name.endswith("unknown-hero"):
            assert hero.current_turn_card is None
            assert hero.resolved_turn_count >= state.turn
        if name == "finalization-positive-uses":
            assert all(item is None or item.passive_uses_this_turn == 0 for item in played)
            assert hero.ultimate_card.passive_uses_this_turn == 0
            assert hero.hand[0].passive_uses_this_turn == 5
        elif name.endswith("nonpositive-uses"):
            assert card.passive_uses_this_turn == -2
            assert hero.ultimate_card.passive_uses_this_turn == -1
        elif name.endswith(("effects", "duplicate-effect-ids", "shared-effect")):
            assert [item.is_active for item in state.active_effects[:4]] == [
                True,
                True,
                False,
                False,
            ]
            assert [item.passive_uses_this_turn for item in state.active_effects[:4]] == [
                0,
                3,
                0,
                5,
            ]
            assert card.is_active is False
        elif name.endswith(("own-own", "own-foreign", "foreign-own", "foreign-foreign")):
            own_guess, own_reveal = name.removeprefix("finalization-").split("-")
            assert (state.card_guess is guess) == (own_guess == "own")
            assert (state.card_reveal is reveal) == (own_reveal == "own")
            retained = state.active_effects[0].finishing_steps[0].value
            assert retained[0] is guess and retained[1] is reveal
        elif name.endswith("empty-reveals"):
            assert state.card_guess is guess and state.card_reveal is reveal
        elif name.endswith("missing-owner"):
            assert state.card_guess is None and state.card_reveal is None
        elif name.endswith("retained-context"):
            assert state.active_effects[0].finishing_steps[0].value is context
        elif name.endswith("shared-played"):
            assert state.teams[hero.team].heroes[1].played_cards is played
        elif name.endswith("override"):
            assert card.initiative_override is None
        elif name.endswith("facedown"):
            assert card.is_facedown is True
        elif name.endswith("extra-card"):
            assert hero.extra_turn_card is not None
            assert hero.extra_turn_card.passive_uses_this_turn == 5
        assert [step.type.value for step in state.execution_stack[-3:]] == [
            "find_next_actor",
            "check_lane_push",
            "return_minion_to_zone",
        ]
        assert records[0]["events"][0]["event_type"] == "TURN_ENDED"
        checked.add(name)
    assert len(checked) == 27  # Skipped finalization deliberately has no body effects.


def test_piece_planning_resolves_the_hero_without_rewriting_buffer_keys():
    from model_traces import setup
    from object_execution_traces import piece_planning_scenes, reference

    checked = set()
    for name, state, operations in piece_planning_scenes(lambda red=None: setup(red=red)):
        piece = "hero_razzle_piece_1"
        hero = state.get_hero(piece)
        assert hero.id == "hero_razzle"
        records = reference(state, operations)
        if name.endswith("cycle"):
            assert list(state.pending_inputs) == [str(hero.id)]
            assert records[-1]["error"] == "invalid_planning_action"
        elif name.endswith("pass"):
            assert state.pending_inputs == {piece: None}
            assert state.planning_done == [piece]
            assert records[-1]["error"] == "invalid_planning_action"
        elif name.endswith("buffers"):
            assert [item["error"] for item in records] == [
                None,
                "invalid_planning_action",
                None,
                "invalid_planning_action",
                None,
            ]
            assert state.pending_inputs == {} and state.planning_done == []
        else:
            assert state.phase.value == "RESOLUTION"
        checked.add(name)
    assert len(checked) == 4


def test_live_minion_return_preserves_input_aliases_and_fresh_continuations():
    from model_traces import setup
    from object_execution_traces import minion_return_scenes, reference

    from goa2.domain.models import GamePhase

    def base():
        state = setup()
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        return state

    checked = set()
    for name, state, operations in minion_return_scenes(base):
        records = reference(state, operations)
        record = records[-1]
        context = record["semantic_state"]["execution_context"]
        step = context["return_step"]
        assert context["processed"] == state.execution_context["processed"]
        assert step["processed_minion_ids"] == context["processed"]
        if name.endswith("pending-valid"):
            assert step["pending_input"] == context["pending_alias"]
            assert step["pending_input"]["retained"]
        if name.endswith("pending-invalid"):
            assert step["pending_input"] is None
            assert context["pending_alias"]["selection"] == "SKIP"
            assert record["request"]["player_id"] == "team:RED"
        if name.endswith("pending-false"):
            assert step["pending_input"] is False
        if name.endswith("pending-empty"):
            assert step["pending_input"] == {}
        if name.endswith("invalid-retry"):
            assert len({records[i]["request"]["id"] for i in (0, 2)}) == 2
            assert step["pending_input"]["selection"] == operations[-2]["selection"]
        if name.endswith("shared-list"):
            stack = record["semantic_state"]["execution_stack"]
            assert [frame["type"] for frame in stack] == ["return_minion_to_zone", "move_unit"]
            assert len(stack[0]["processed_minion_ids"]) == len(context["processed"]) + 1
            assert context["retained_step"]["value"] == context["processed"]
        if name in (
            "minion-return-cap-zero",
            "minion-return-cap-one",
            "minion-return-cap-two",
            "minion-return-cap-actions-only",
        ):
            assert record["request"] is None
            expected_type = (
                "place_unit"
                if name in ("minion-return-cap-zero", "minion-return-cap-one")
                else "move_unit"
            )
            assert record["semantic_state"]["execution_stack"][-1]["type"] == expected_type
        checked.add(name)
    assert len(checked) == 31


def test_live_placement_retains_hex_tile_snapshot_and_ignored_input_contracts():
    from model_traces import setup
    from object_execution_traces import placement_scenes, reference

    from goa2.domain.models import GamePhase

    def base():
        state = setup()
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        return state

    def aliases(record, path):
        return next(
            binding["paths"] for binding in record["object_bindings"] if path in binding["paths"]
        )

    checked = set()
    for name, state, operations in placement_scenes(base):
        if not name.startswith("placement-alias-"):
            continue
        records = reference(state, operations)
        record = records[-1]
        semantic = record["semantic_state"]
        context = semantic["execution_context"]
        assert ["execution_context", "locations"] in aliases(record, ["entity_locations"])
        current = ["entity_locations", "hero_brogan"]
        if name.endswith("immutable-context"):
            assert ["execution_context", "target_hex"] in aliases(record, current)
        if name.endswith("dict-target"):
            assert ["execution_context", "target_hex"] not in aliases(record, current)
            assert semantic["entity_locations"]["hero_brogan"] == context["target_hex"]
        if name.endswith("board-hex"):
            assert ["board", "tiles", "(2, -1, -1)", "hex"] in aliases(record, current)
        if name.endswith("snapshot-alias"):
            assert ["last_turn_positions"] in aliases(record, ["entity_locations"])
        if name.endswith("old-snapshot"):
            assert semantic["last_turn_positions"]["retained"] == context["old_hex"]
            assert (
                semantic["last_turn_positions"]["retained"]
                != semantic["entity_locations"]["hero_brogan"]
            )
        if name.endswith("occupied-mandatory") or name.endswith("foreign-abort"):
            assert context["after"]
        if name.endswith("self-mandatory") or name.endswith("same-abort"):
            assert "after" not in context
        if name.endswith("off-map-collision"):
            assert (
                semantic["entity_locations"]["hero_arien"]
                == semantic["entity_locations"]["hero_brogan"]
            )
            assert ["entity_locations", "hero_arien"] in aliases(record, current)
        if name.endswith("dirty-pending"):
            assert ["execution_context", "pending"] in aliases(
                record, ["execution_context", "placement_step", "pending_input"]
            )
            assert context["placement_step"]["pending_input"]["selection"] == "ignored"
            assert context["placement_step"]["pending_request_id"] is not None
        if name.endswith("pending-false"):
            assert context["placement_step"]["pending_input"] is False
        if name.endswith("repeated"):
            assert len(records[0]["events"]) == 1 and not records[1]["events"]
        checked.add(name)
    assert len(checked) == 29


def test_live_walking_preserves_both_skip_boundaries_and_spatial_aliases():
    from model_traces import setup
    from object_execution_traces import movement_scenes, reference

    from goa2.domain.models import GamePhase

    def base():
        state = setup()
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        return state

    checked = 0
    for name, state, operations in movement_scenes(base):
        if not name.startswith("walking-alias-"):
            continue
        checked += 1
        record = reference(state, operations)[-1]
        context = record["semantic_state"]["execution_context"]
        if name.endswith(("success-active", "negative-range", "same-place-negative")):
            assert "moved" not in context
            assert context["crossed"] == ["old"]
            assert not record["events"]
        elif name.endswith("success-skip"):
            assert context["moved"] is True
            assert context["crossed"] == ["old"]
            assert not record["events"]
        else:
            assert context["moved"] is True
            assert context["crossed"] == []
            assert record["events"][0]["event_type"] == "UNIT_MOVED"
        if name.endswith("snapshot-alias"):
            paths = next(
                binding["paths"]
                for binding in record["object_bindings"]
                if ["entity_locations"] in binding["paths"]
            )
            assert ["last_turn_positions"] in paths
            assert ["execution_context", "locations"] in paths
    assert checked == 11


def test_live_mines_preserve_pending_aliases_and_ordered_removal():
    from object_execution_traces import live_mine_scenes, reference

    wanted = {
        "mines-choice-alias-pending-valid",
        "mines-choice-alias-pending-invalid",
        "mines-trigger-ordered",
        "mines-trigger-duplicates",
        "mines-trigger-empty-literal",
        "mines-trigger-linked-effects",
        "mines-trigger-mutate-old-effects",
    }
    checked = set()
    for name, state, operations in live_mine_scenes(None):
        if name not in wanted:
            continue
        checked.add(name)
        records = reference(state, operations)
        record = records[-1]
        targets = [event["target_id"] for record in records for event in record["events"]]
        semantic = record["semantic_state"]
        context = semantic["execution_context"]
        if name == "mines-choice-alias-pending-valid":
            assert context["choice_step"]["pending_input"] == context["pending_alias"]
            assert context["pending_alias"]["selection"] == "1"
        if name == "mines-choice-alias-pending-invalid":
            assert context["pending_alias"]["selection"] == 0
            assert context["choice_step"]["pending_input"]["selection"] == "0"
        if name == "mines-trigger-ordered":
            assert targets == ["mine_b", "mine_a", "mine_c"]
        if name == "mines-trigger-duplicates":
            assert targets == ["mine_a", "mine_b"]
            # Inspect the trigger boundary before the extended driver completes
            # the discard and its passive scan.
            assert (
                sum(
                    step["type"] == "force_discard"
                    for step in records[0]["semantic_state"]["execution_stack"]
                )
                == 2
            )
            assert all(step["type"] != "force_discard" for step in semantic["execution_stack"])
        if name == "mines-trigger-empty-literal":
            assert context["triggered_mine_ids"] == ["mine_a"]
            assert "rollback_reanchor_pending" not in context
        if name in ("mines-trigger-linked-effects", "mines-trigger-mutate-old-effects"):
            assert [effect["id"] for effect in semantic["active_effects"]] == ["duplicate"]
            assert len(context["effects"]) == (5 if name.endswith("mutate-old-effects") else 4)
            assert context["rollback_reanchor_pending"] is True
    assert checked == wanted


def test_live_forced_discard_snapshots_victim_filters_shields_and_ignored_input():
    from object_execution_traces import forced_discard_scenes, reference

    checked = 0
    for name, state, operations in forced_discard_scenes(None):
        record = reference(state, operations)[0]
        checked += 1
        semantic = record["semantic_state"]
        context = semantic["execution_context"]
        stack = semantic["execution_stack"]
        if name.endswith(
            ("empty-hand", "no-basic", "immune", "no-victim", "unknown-victim", "skipped")
        ):
            assert all(step["type"] != "select_step" for step in stack)
        if stack and stack[-1]["type"] == "select_step":
            select, discard = stack[-1], stack[-2]
            assert select["target_type"] == "CARD"
            assert (
                select["context_hero_id"]
                == select["override_player_id"]
                == discard["hero_id"]
                == "hero_mrak"
            )
            assert select["decision_purpose"] == "discard"
            if name == "forced-discard-basic":
                assert "stone_carapace" in select["allowed_card_ids"]
                assert select["restrict_played_to_shields"] is True
        if name.endswith("dirty-pending"):
            assert context["discard_step"]["pending_input"]["selection"] == "ignored"
            assert context["discard_step"]["pending_request_id"].startswith("request/")
    assert checked == 28


def test_live_card_selection_retains_pending_aliases_and_clears_only_selected_outputs():
    from object_execution_traces import card_selection_scenes, reference

    wanted = {
        "card-selector-pending",
        "card-selector-optional-empty",
        "card-selector-optional-skip",
        "card-selector-invalid-dict",
        "card-selector-include-hidden",
        "card-selector-piece-owner",
        "card-selector-alias",
    }
    checked = set()
    for name, state, operations in card_selection_scenes(None):
        if name not in wanted:
            continue
        checked.add(name)
        records = reference(state, operations)
        context = records[-1]["semantic_state"]["execution_context"]
        if name in ("card-selector-optional-empty", "card-selector-optional-skip"):
            assert "chosen" not in context
            assert context["chosen_color"] == "old-color"
            assert context["chosen_tier"] == "old-tier"
        else:
            assert context["selector"]["pending_input"] is None
            assert context["selector"]["pending_request_id"] is None
            assert isinstance(context["chosen"], str)
        if name == "card-selector-pending":
            assert context["pending_alias"]["selection"] == context["chosen"]
        if name == "card-selector-alias":
            assert records[0]["request"]["options"][0] == records[0]["request"]["options"][1]
        if name == "card-selector-piece-owner":
            assert records[0]["request"]["player_id"] == "hero_razzle"
    assert checked == wanted


def test_live_discard_equal_models_have_distinct_mutations_and_source_precedence():
    from model_traces import setup
    from object_execution_traces import discard_scenes, reference

    from goa2.domain.models import GamePhase

    def base():
        state = setup()
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        return state

    checked = 0
    for name, state, operations in discard_scenes(base):
        record = reference(state, operations)[0]
        checked += 1
        semantic = record["semantic_state"]
        hero = semantic["teams"]["RED"]["heroes"][0]
        context = semantic["execution_context"]
        if name in {"discard-equal-current", "discard-metadata-order-current"}:
            assert hero["current_turn_card"] is None
            assert hero["played_cards"][0]["state"] == "DISCARD"
            assert context["current_alias"]["state"] == "RESOLVED"
        if name in {"discard-revision-current", "discard-metadata-current"}:
            assert hero["current_turn_card"]["state"] == "RESOLVED"
            assert hero["played_cards"][0] is None
        if name == "discard-effects":
            assert [effect["id"] for effect in semantic["active_effects"]] == ["same-id"]
            assert len(context["old_effects"]) == 3
            assert "finishing_ran" not in context
        if name == "discard-effects-global-first":
            assert hero["current_turn_card"]["is_active_base"] is False
            assert hero["discard_pile"][-1]["is_active_base"] is True
        if name in {"discard-log-existing", "discard-log-shared"}:
            assert len(context["log_alias"]) == 2
            assert len(semantic["public_revealed_cards"]) == 1
    assert checked == 25
