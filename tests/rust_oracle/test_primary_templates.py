"""Frozen primary recipes must remain exact public CardEffect API output."""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))
from primary_templates import build_catalog


def test_primary_catalog_is_exact_frozen_effect_api_output():
    root = Path(__file__).resolve().parents[2]
    frozen = json.loads((root / "rust/tests/fixtures/primary-effects.json").read_text())
    assert build_catalog() == frozen


def test_loaded_map_placement_raises_before_changing_the_board():
    import pytest
    from model_traces import setup

    from goa2.domain.hex import Hex
    from goa2.engine.steps import PlaceUnitStep

    state = setup()
    before = dict(state.entity_locations)
    with pytest.raises(ValueError, match="not on the board"):
        PlaceUnitStep(unit_id="hero_arien", target_hex_arg=Hex(q=1000, r=-1000, s=0)).resolve(
            state, state.execution_context
        )
    assert state.entity_locations == before


def test_compute_hex_preserves_python_flooring_failure_for_off_axis_vectors():
    import pytest
    from model_traces import setup

    from goa2.domain.board import Board
    from goa2.domain.hex import Hex
    from goa2.domain.tile import Tile
    from goa2.engine.steps import ComputeHexStep

    state = setup()
    for unit in list(state.entity_locations):
        state.remove_entity(unit)
    origin = Hex(q=0, r=0, s=0)
    target = Hex(q=2, r=-1, s=-1)
    state.board = Board(tiles={h: Tile(hex=h) for h in [origin, target]})
    state.place_entity("hero_arien", origin)
    state.place_entity("hero_brogan", target)
    state.current_actor_id = "hero_arien"
    state.execution_context = {"victim": "hero_brogan"}
    with pytest.raises(ValueError):
        ComputeHexStep(target_key="victim").resolve(state, state.execution_context)


def test_aura_capabilities_match_frozen_registry_method_implementations():
    from goa2.bootstrap import register_all_effects
    from goa2.engine.effects import CardEffect, CardEffectRegistry

    register_all_effects()
    root = Path(__file__).resolve().parents[2]
    frozen = json.loads((root / "rust/tests/fixtures/instruction-templates.json").read_text())
    hooks = ("get_stat_auras", "get_movement_aura", "get_action_prevention_reason")
    actual = sorted(
        key
        for key, effect in CardEffectRegistry._effects.items()
        if any(getattr(type(effect), hook) is not getattr(CardEffect, hook) for hook in hooks)
    )
    assert frozen["aura_effects"] == actual
    assert "noble_blade" not in actual
    assert "witching_hour" in actual


def test_ultimate_unlock_catalog_matches_frozen_registered_hooks():
    from goa2.bootstrap import register_all_effects
    from goa2.engine.effects import CardEffectRegistry

    register_all_effects()
    root = Path(__file__).resolve().parents[2]
    frozen = json.loads((root / "rust/tests/fixtures/instruction-templates.json").read_text())
    actual = sorted(
        key
        for key, effect in CardEffectRegistry._effects.items()
        if hasattr(effect, "on_ultimate_unlocked")
    )
    assert frozen["ultimate_unlock_effects"] == actual
    assert actual == ["ready_for_war", "unbound_fury"]


def test_native_flat_and_count_aura_specs_match_registered_public_hooks():
    from goa2.bootstrap import register_all_effects
    from goa2.domain.models import StatType
    from goa2.engine.effects import CardEffectRegistry

    register_all_effects()
    flat = CardEffectRegistry.get("unlimited_firepower").get_stat_auras()
    assert [(a.stat_type, a.flat_bonus, a.basic_only, a.action_type_only) for a in flat] == [
        (StatType.RANGE, 1, False, None),
        (StatType.RADIUS, 1, False, None),
    ]
    counted = CardEffectRegistry.get("metamorphosis").get_stat_auras()
    assert [a.stat_type for a in counted] == [StatType.MOVEMENT, StatType.INITIATIVE]
    for aura in counted:
        assert aura.flat_bonus is None and aura.multiplier == 1
        assert not aura.basic_only and aura.action_type_only is None
        assert [
            (f.type.value, getattr(f, "relation", None), getattr(f, "max_range", None))
            for f in aura.count_filters
        ] == [("team_filter", "ENEMY", None), ("range_filter", None, 1)]
    assert CardEffectRegistry.get("metamorphosis").get_movement_aura().pass_through_obstacles


def test_conditional_attack_aura_matches_the_registered_public_hook():
    from goa2.bootstrap import register_all_effects
    from goa2.domain.models import ActionType, StatType
    from goa2.engine.effects import CardEffectRegistry

    register_all_effects()
    effect = CardEffectRegistry.get("big_sodding_gun")
    assert [
        (a.stat_type, a.flat_bonus, a.basic_only, a.action_type_only)
        for a in effect.get_stat_auras()
    ] == [
        (StatType.ATTACK, 2, True, ActionType.ATTACK),
        (StatType.RANGE, 2, True, ActionType.ATTACK),
    ]
    config = effect.get_passive_config()
    assert config.trigger.value == "after_push" and config.uses_per_turn == 0
    assert not config.is_optional


def test_passive_config_catalog_matches_frozen_public_api():
    from goa2.bootstrap import register_all_effects
    from goa2.engine.effects import CardEffect, CardEffectRegistry

    register_all_effects()
    root = Path(__file__).resolve().parents[2]
    frozen = json.loads((root / "rust/tests/fixtures/instruction-templates.json").read_text())
    actual = {
        key: [
            {
                "trigger": c.trigger.value,
                "uses": c.uses_per_turn,
                "optional": c.is_optional,
                "prompt": c.prompt,
            }
            for c in effect.get_passive_configs()
        ]
        for key, effect in CardEffectRegistry._effects.items()
    }
    assert frozen["passive_configs"] == actual
    effect = CardEffectRegistry.get("big_sodding_gun")
    assert type(effect).should_offer_passive is CardEffect.should_offer_passive


def test_optional_passive_rechecks_removed_and_facedown_cards():
    from model_traces import setup

    from goa2.engine.steps import OfferPassiveStep

    for remove in [False, True]:
        state = setup(["Sabina", "Arien"], ["Razzle", "Brogan"])
        state.current_actor_id = "hero_sabina"
        hero = state.get_hero("hero_sabina")
        card = hero.ultimate_card
        step = OfferPassiveStep(card_id=str(card.id), trigger="after_push", is_optional=True)
        initial = step.resolve(state, state.execution_context)
        assert initial.requires_input
        if remove:
            hero.ultimate_card = None
        else:
            card.is_facedown = True
        step.pending_input = {"selection": "YES"}
        result = step.resolve(state, state.execution_context)
        assert result.is_finished and not result.new_steps
        assert card.passive_uses_this_turn == 0


def test_effect_defaults_are_exact_frozen_model_output():
    from effect_step_traces import defaults

    root = Path(__file__).resolve().parents[2]
    assert defaults() == json.loads((root / "rust/tests/fixtures/effect-defaults.json").read_text())
