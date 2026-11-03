"""Rock-ultimate gates, directional topology, piece routing and copy identity."""

import copy

import pytest
from live_rock_ultimate_traces import scenes
from object_execution_traces import ObjectTraceNormalizer, reference


@pytest.fixture(scope="module")
def rock_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def test_early_gates_preserve_old_affected_list_and_never_mark_used(rock_scenes):
    for variant in (
        "level7",
        "no-ultimate",
        "no-actor",
        "unknown-actor",
        "token-actor",
        "far",
        "offboard",
        "friendly-only",
        "skip",
        "active",
    ):
        state, ops = rock_scenes[f"rock-ultimate-{variant}-first"]
        old = state.execution_context["old_affected"]
        records = reference(state, ops)
        assert state.execution_context["_rock_ult_affected"] is old
        assert "rock_ultimate_used" not in state.execution_context
        assert all(record["request"] is None for record in records)
        assert state.execution_context["tail"] is True


def test_acceptance_marks_actual_flag_even_when_the_victim_has_no_cards(rock_scenes):
    for variant in (
        "plain",
        "duplicate-rocks",
        "custom-used",
        "empty-used",
        "empty-hand",
        "facedown",
        "used-false",
    ):
        state, ops = rock_scenes[f"rock-ultimate-{variant}-first"]
        victim = state.get_hero("hero_brogan")
        before = len(victim.hand)
        records = reference(state, ops)
        ctx = state.execution_context
        assert ctx["_rock_ult_affected"] == ["hero_brogan"]
        assert ctx["_rock_ult_affected"] is not ctx["old_affected"]
        flag = (
            "custom_used"
            if variant == "custom-used"
            else "" if variant == "empty-used" else "rock_ultimate_used"
        )
        assert ctx[flag] is True
        assert ctx["old_affected"] == ["old"]
        assert len(victim.hand) == before - (0 if variant == "empty-hand" else 1)
        # Frozen ForceDiscard changes card collections without emitting events.
        assert not [event for row in records for event in row["events"]]


def test_declining_does_not_consume_the_once_per_turn_offer(rock_scenes):
    for mode, expected in (("first", 1), ("last", 2)):
        state, ops = rock_scenes[f"rock-ultimate-repeat-{mode}"]
        records = reference(state, ops)
        number_requests = [
            r["request"]
            for r in records
            if r["request"] is not None and r["request"]["request_type"] == "SELECT_NUMBER"
        ]
        assert len(number_requests) == expected
        if mode == "last":
            assert "rock_ultimate_used" not in state.execution_context
            assert state.execution_context["rock_ult_yes"] is None
            assert not [event for row in records for event in row["events"]]
        else:
            assert state.execution_context["rock_ultimate_used"] is True


def test_piece_targets_keep_roster_order_and_share_the_hero_hand(rock_scenes):
    state, ops = rock_scenes["rock-ultimate-pieces-False-first"]
    hero = state.get_hero("hero_razzle")
    before = len(hero.hand)
    records = reference(state, ops)
    assert state.execution_context["_rock_ult_affected"] == [
        "hero_razzle_piece_1",
        "hero_razzle_piece_2",
        "hero_brogan",
    ]
    assert len(hero.hand) == before - 2
    assert not [event for row in records for event in row["events"]]
    state, ops = rock_scenes["rock-ultimate-pieces-True-first"]
    reference(state, ops)
    assert state.current_actor_id == "hero_razzle_piece_1"
    assert state.execution_context["rock_ultimate_used"] is True


def test_actor_immunity_and_directional_isolation_use_source_rules(rock_scenes):
    for variant, offered in (
        ("attack-shield", True),
        ("action-shield", False),
        ("inactive-shield", True),
        ("split", True),
        ("isolation-rock", False),
        ("isolation-enemy", True),
    ):
        state, ops = rock_scenes[f"rock-ultimate-{variant}-first"]
        reference(state, ops)
        assert state.execution_context.get("rock_ultimate_used", False) == offered


def test_full_source_stone_grip_body_completes_with_or_without_unlocked_ultimate(rock_scenes):
    count = 0
    for name, (state, ops) in rock_scenes.items():
        if not name.startswith("rock-ultimate-stone-grip-"):
            continue
        victim = state.get_hero("hero_brogan")
        before = len(victim.hand)
        records = reference(state, ops)
        events = [e["event_type"] for r in records for e in r["events"]]
        assert events.count("TOKEN_PLACED") == 3
        accepted = name.startswith("rock-ultimate-stone-grip-True-") and not name.endswith("-last")
        assert len(victim.hand) == before - int(accepted)
        assert state.execution_stack == []
        assert state.execution_context["tail"] is True
        count += 1
    assert count == 12


def test_generated_template_deepcopy_retains_textual_id_but_has_new_identity(rock_scenes):
    from goa2.engine.steps import ForEachStep

    state, ops = rock_scenes["rock-ultimate-two-enemies-invalid"]
    reference(state, [ops[0]])
    loop = next(step for step in state.execution_stack if isinstance(step, ForEachStep))
    template = loop.steps_template[0]
    from unittest.mock import patch

    from goa2.engine.steps import ForceDiscardStep

    copies = []
    original_resolve = ForceDiscardStep.resolve

    def capture(step, world, context):
        copies.append(step)
        return original_resolve(step, world, context)

    with patch.object(ForceDiscardStep, "resolve", capture):
        reference(state, ops[1:])
    unique = {id(step): step for step in copies}
    assert len(unique) == 2
    assert all(
        step is not template and step.step_id == template.step_id for step in unique.values()
    )
    assert template.pending_input is None


def test_trace_identity_distinguishes_copies_and_preserves_literal_prefixes():
    step = {"step_id": "native-step-3", "pending_request_id": None}
    record = {
        "semantic_state": {
            "execution_stack": [
                copy.deepcopy(step),
                copy.deepcopy(step),
                {"step_id": "native-step-user", "pending_request_id": None},
            ],
            "execution_context": {"note": "native-step-3", "alias": copy.deepcopy(step)},
        },
        "request": None,
        "object_bindings": [
            {"kind": "step", "paths": [["execution_stack", 0], ["execution_context", "alias"]]},
            {"kind": "step", "paths": [["execution_stack", 1]]},
            {"kind": "step", "paths": [["execution_stack", 2]]},
        ],
        "generated_step_objects": [[0, 100], [1, 101]],
    }
    normalizer = ObjectTraceNormalizer()
    actual = normalizer.record(record)
    assert [s["step_id"] for s in actual["semantic_state"]["execution_stack"]] == [
        "dynamic-step/1",
        "dynamic-step/2",
        "native-step-user",
    ]
    assert actual["semantic_state"]["execution_context"]["alias"]["step_id"] == "dynamic-step/1"
    assert actual["semantic_state"]["execution_context"]["note"] == "native-step-3"
    assert record["semantic_state"]["execution_stack"][0]["step_id"] == "native-step-3"
    assert "generated_step_objects" not in actual
