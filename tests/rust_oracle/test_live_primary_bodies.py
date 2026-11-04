"""Frozen source contracts for primary body dispatch and physical recipes."""

import copy
import json

import pytest
from live_primary_body_traces import scenes
from object_execution_traces import reference
from oracle import ROOT
from primary_builders import catalog


@pytest.fixture(scope="module")
def primary_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(primary_scenes, name):
    state, ops = copy.deepcopy(primary_scenes["primary-text-" + name])
    records = reference(state, ops)
    return state, records


def test_primary_catalog_reproduces_source_and_keeps_spell_registry_separate():
    expected = json.loads((ROOT / "rust/tests/fixtures/primary-builders.json").read_text())
    assert catalog() == expected
    assert len(expected["recipes"]) == 447
    assert len(expected["stateful_recipes"]) == 101
    assert len(expected["methods"]) == 596
    assert len(expected["spell_effects"]) == 22
    assert (
        len([value for value in expected["methods"].values() if value == "CardEffect.build_steps"])
        == 48
    )


def test_context_uses_current_physical_card_even_when_facedown(primary_scenes):
    for variant in ("ordinary", "facedown"):
        state, _ = run(primary_scenes, "liquid_leap-" + variant)
        hero = state.get_hero("hero_arien")
        assert state.execution_context["current_card_id"] == hero.current_turn_card.id
        assert state.execution_context["current_alias"] is hero.current_turn_card
        assert state.execution_context["stored_primary_step"].card_id == "ignored-literal"
        assert [step.type.value for step in state.execution_stack] == (
            ["set_context_flag"]
            if variant == "facedown"
            else ["set_context_flag", "place_unit", "select_step"]
        )


def test_null_and_empty_effects_take_different_source_paths(primary_scenes):
    state, _ = run(primary_scenes, "fallback-no-effect")
    assert [step.type.value for step in state.execution_stack] == ["set_context_flag"]
    state, _ = run(primary_scenes, "fallback-empty-effect")
    assert [step.type.value for step in state.execution_stack] == [
        "set_context_flag",
        "log_message",
    ]


def test_skips_and_missing_current_do_not_replace_existing_context(primary_scenes):
    for name in (
        "liquid_leap-skip",
        "fallback-missing-current",
        "fallback-missing-hero",
        "fallback-inactive",
    ):
        state, _ = run(primary_scenes, name)
        assert state.execution_context["current_card_id"] == "old-card"
        assert [step.type.value for step in state.execution_stack] == ["set_context_flag"]


def test_empty_registered_bodies_do_not_fall_back_to_primary_primitives(primary_scenes):
    names = [name for name in primary_scenes if name.startswith("primary-text-empty-")]
    assert len(names) == 45
    for name in names:
        state, _ = run(primary_scenes, name.removeprefix("primary-text-"))
        assert [step.type.value for step in state.execution_stack] == ["set_context_flag"]
        assert state.execution_context["current_card_id"] != "old-card"


def test_negative_attack_fallback_and_defense_skill_noop(primary_scenes):
    state, _ = run(primary_scenes, "fallback-attack")
    attack = state.execution_stack[-1]
    assert attack.damage == -3
    assert attack.range_val == 1
    assert not attack.is_ranged
    state, _ = run(primary_scenes, "fallback-defense-skill")
    assert [step.type.value for step in state.execution_stack] == ["set_context_flag"]


def test_complete_placement_swap_and_marker_flows_are_observable(primary_scenes):
    for effect, event_type in (
        ("liquid_leap", "UNIT_PLACED"),
        ("back_to_back", "UNITS_SWAPPED"),
        ("poisoned_dart", "MARKER_PLACED"),
    ):
        state, records = run(primary_scenes, "positive-" + effect)
        assert state.execution_context["tail"]
        assert not state.execution_stack
        assert any(event["event_type"] == event_type for row in records for event in row["events"])


def test_mandatory_target_failure_aborts_the_remaining_primary_text(primary_scenes):
    state, records = run(primary_scenes, "flow-arcane_whirlpool-first")
    assert not state.execution_stack
    assert "tail" not in state.execution_context
    assert not any(row["events"] for row in records)


def test_positive_primary_retrieval_and_forced_discard_change_real_cards(primary_scenes):
    for effect in ("devoted_followers", "fresh_converts", "steady_advance", "unwavering_resolve"):
        state, records = run(primary_scenes, "positive-" + effect)
        assert state.execution_context["tail"]
        assert any(
            event["event_type"] == "CARD_RETRIEVED" for row in records for event in row["events"]
        )
        assert state.execution_context["retrieved_card"]
    for effect in ("close_support", "covering_fire"):
        state, records = run(primary_scenes, "positive-" + effect)
        assert state.execution_context["tail"]
        owner = state.get_hero(state.execution_context["discarded_card_owner_id"])
        assert state.execution_context["discarded_card_id"] in {
            card.id for card in owner.discard_pile
        }
        assert state.execution_context["discarded_card_id"] not in {card.id for card in owner.hand}
        assert any(row["request"] for row in records)
