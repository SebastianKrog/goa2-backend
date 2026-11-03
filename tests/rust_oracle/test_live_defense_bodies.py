"""Frozen source contracts for complete primary-defense construction."""

import copy
import json
from unittest.mock import patch

import pytest
from defense_builders import catalog
from live_defense_body_traces import scenes
from object_execution_traces import reference
from oracle import ROOT


@pytest.fixture(scope="module")
def defense_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(defense_scenes, name):
    state, ops = copy.deepcopy(defense_scenes[name])
    return state, reference(state, ops)


def events(records, kind):
    return [event for row in records for event in row["events"] if event["event_type"] == kind]


def test_frozen_catalog_matches_source_methods_and_recipes():
    expected = json.loads((ROOT / "rust/tests/fixtures/defense-builders.json").read_text())
    assert catalog() == expected
    assert (
        len(
            {
                row["defense"]
                for row in expected["methods"].values()
                if row["defense"] != "CardEffect.build_defense_steps"
            }
        )
        == 21
    )
    assert (
        len(
            {
                row["on_block"]
                for row in expected["methods"].values()
                if row["on_block"] != "CardEffect.build_on_block_steps"
            }
        )
        == 7
    )


def test_duelist_effects_bind_the_defense_card_and_restore_attacker(defense_scenes):
    for effect in ("expert_duelist", "master_duelist"):
        state, records = run(defense_scenes, f"defense-body-{effect}-text-repeated")
        assert state.current_actor_id == "hero_brogan"
        assert state.execution_context["ignore_minion_defense"]
        assert len(state.active_effects) == 1
        assert state.active_effects[0].source_id == "hero_arien"
        assert state.active_effects[0].source_card_id == state.execution_context["defense_card_id"]
        assert state.active_effects[0].except_attacker_ids == ["hero_brogan"]
        assert len(events(records, "EFFECT_CREATED")) == 1


def test_defense_board_guard_is_not_added_to_on_block(defense_scenes):
    state, _ = run(defense_scenes, "defense-body-oath_of_endurance-text-offboard")
    assert not state.active_effects
    state, records = run(defense_scenes, "defense-body-oath_of_endurance-on-block-offboard")
    assert len(state.active_effects) == 1
    assert len(events(records, "EFFECT_CREATED")) == 1
    assert state.current_actor_id == "hero_brogan"
    assert "_pre_onblock_actor" in state.execution_context
    assert "_pre_defense_actor" not in state.execution_context


def test_spawn_counts_use_defender_terrain_immunity_and_topology(defense_scenes):
    for variant, positive in (
        ("two-spawns", True),
        ("one-spawn", False),
        ("outside-zone", False),
        ("occupied", False),
        ("barrier-friendly", True),
        ("barrier-enemy", False),
        ("tide-override", True),
        ("topology", False),
    ):
        state, _ = run(defense_scenes, "defense-spawn-shield_of_decay-" + variant)
        assert (state.execution_context["defense_bonus"] > 0) is positive, variant


def test_smoke_swap_body_finishes_and_restores_actor(defense_scenes):
    state, records = run(defense_scenes, "defense-body-poof-text-first")
    original = defense_scenes["defense-body-poof-text-first"][0]
    assert state.execution_context["auto_block"]
    assert state.current_actor_id == "hero_brogan"
    assert len(events(records, "UNITS_SWAPPED")) == 1
    assert state.get_position("hero_min") != original.get_position("hero_min")
    state, _ = run(defense_scenes, "defense-body-poof-text-no-smoke")
    assert state.execution_context["defense_invalid"]
    assert not state.execution_context.get("auto_block")


def test_glitch_defense_uses_three_tokens_and_card_bound_cleanup(defense_scenes):
    state, records = run(defense_scenes, "defense-body-unstable_timeline-text-first")
    assert state.current_actor_id == "hero_brogan"
    assert len(events(records, "TOKEN_PLACED")) == 3
    assert len(events(records, "UNITS_SWAPPED")) == 1
    cleanup = state.active_effects[-1]
    assert cleanup.source_card_id == state.execution_context["defense_card_id"]
    assert len(cleanup.finishing_steps) == 2
    assert cleanup.finishing_steps[1].steps_template[0].type.value == "remove_token"


def test_rune_choices_reset_old_flags_and_use_the_card_owner(defense_scenes):
    state, _ = run(defense_scenes, "defense-body-oath_of_perseverance-text-no-runes")
    assert state.execution_context["auto_block"] is False
    assert state.execution_context["defense_invalid"] is True
    state, records = run(defense_scenes, "defense-body-oath_of_perseverance-text-many-runes")
    assert any(
        row["request"] and row["request"]["request_type"] == "SELECT_OPTION" for row in records
    )
    assert state.execution_context["snorri_oath_rune"] == "axe"
    assert state.execution_context["auto_block"] is False
    assert state.execution_context["defense_invalid"] is True
    state, _ = run(defense_scenes, "defense-body-oath_of_endurance-text-other-owner")
    assert state.execution_context["auto_block"] is True


def test_awaken_fallback_builds_and_executes_four_optional_placements(defense_scenes):
    state, records = run(defense_scenes, "defense-body-awaken-text-first")
    assert len(events(records, "TOKEN_PLACED")) == 4
    assert state.current_actor_id == "hero_brogan"
    state, records = run(defense_scenes, "defense-body-awaken-text-skip")
    assert not events(records, "TOKEN_PLACED")
    assert state.current_actor_id == "hero_brogan"


def test_empty_defense_hook_does_not_call_primary_fallback(defense_scenes):
    from goa2.engine.effects import CardEffectRegistry

    state, _ = copy.deepcopy(defense_scenes["defense-body-shield_of_decay-text-first"])
    effect = CardEffectRegistry.get("shield_of_decay")
    with patch.object(
        effect, "get_steps", side_effect=AssertionError("unexpected primary fallback")
    ):
        result = state.execution_context["defense_step_alias"].resolve(
            state, state.execution_context
        )
    assert result.is_finished and not result.new_steps


def test_hex_exclusions_keep_model_list_and_tuple_equality_distinct(defense_scenes):
    for variant, excluded in (
        ("hex", 1),
        ("dict", 0),
        ("hex-list", 1),
        ("dict-list", 0),
        ("tuple", 0),
        ("mixed-list", 1),
        ("none", 0),
        ("skip-text", 0),
        ("nested-list", 0),
        ("nested-exclude", 1),
    ):
        name = "defense-hex-predicate-" + variant
        state, records = run(defense_scenes, name)
        request = next(row["request"] for row in records if row["request"])
        assert len(request["options"]) == len(state.board.tiles) - excluded, variant


def test_empty_and_short_circuit_composites(defense_scenes):
    for variant, has_request in (
        ("and-empty", True),
        ("or-empty", False),
        ("or-short", True),
        ("and-short", False),
    ):
        state, records = run(defense_scenes, "defense-hex-predicate-" + variant)
        assert any(row["request"] is not None for row in records) is has_request
        assert state.execution_context["tail"]


def test_copied_smoke_card_uses_owner_origin_not_attacked_piece(defense_scenes):
    for effect_id in ("poof", "vanish", "ruse"):
        state, _ = run(defense_scenes, "defense-copied-smoke-" + effect_id + "-unbound-owner")
        assert state.execution_context["defense_invalid"] is True
        assert state.current_actor_id == "hero_brogan"
