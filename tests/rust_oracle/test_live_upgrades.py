"""Frozen-source contracts for native simultaneous upgrades and round reset."""

from __future__ import annotations

import copy

import pytest
from model_traces import setup
from state_codec import export_state

from goa2.domain.models import CardState, GamePhase, StatType
from goa2.engine.steps import ResolveUpgradesStep, RoundResetStep
from goa2.engine.steps.cards import apply_hero_upgrade


def upgrade_state(count=3):
    state = setup()
    state.phase = GamePhase.LEVEL_UP
    state.pending_upgrades = {"hero_arien": count}
    return state


def test_reset_retains_defeated_list_and_shallow_copies_position_mapping():
    state = upgrade_state()
    defeated = state.heroes_defeated_this_round
    defeated.append("hero_arien")
    old = state.last_turn_positions
    locations = state.entity_locations
    RoundResetStep().resolve(state, state.execution_context)
    assert state.heroes_defeated_this_round is defeated and not defeated
    assert state.last_turn_positions is not old and state.last_turn_positions is not locations
    assert all(state.last_turn_positions[k] is v for k, v in locations.items())
    assert state.phase == GamePhase.PLANNING and state.turn == 1 and state.round == 2


def test_empty_upgrade_loop_clears_reveal_mapping_in_place_and_queues_reset():
    state = upgrade_state(0)
    state.pending_upgrades.clear()
    state.upgrade_reveal_snapshot = {"hero_arien": {StatType.ATTACK: 2}}
    reveal = state.upgrade_reveal_snapshot
    old = reveal["hero_arien"]
    out = ResolveUpgradesStep().resolve(state, state.execution_context)
    assert state.upgrade_reveal_snapshot is reveal and not reveal
    assert old == {StatType.ATTACK: 2}
    assert [s.type.value for s in out.new_steps] == ["round_reset"]


def test_upgrade_reuses_hand_deck_items_and_clears_lifecycle_initiative_only():
    state = upgrade_state(1)
    hero = state.get_hero("hero_arien")
    hand, deck, items = hero.hand, hero.deck, hero.items
    step = ResolveUpgradesStep()
    pair_ids = step._get_upgrade_options(state, str(hero.id))[0]["pair"]
    chosen, paired = [next(c for c in deck if c.id == cid) for cid in pair_ids]
    old = next(c for c in hand if c.color == chosen.color)
    for card in (old, chosen, paired):
        card.initiative_override = 17
        card.played_this_round = True
        card.is_active = True
        card.is_facedown = True
    apply_hero_upgrade(state, str(hero.id), chosen.id)
    assert hero.hand is hand and hero.deck is deck and hero.items is items
    assert hand[-1] is chosen and old not in hand
    assert (old.state, chosen.state, paired.state) == (
        CardState.RETIRED,
        CardState.HAND,
        CardState.ITEM,
    )
    assert all(c.initiative_override is None for c in (old, chosen, paired))
    assert all(c.played_this_round and c.is_active for c in (old, chosen, paired))
    assert not chosen.is_facedown and old.is_facedown and paired.is_facedown


def test_invalid_ordered_batch_does_not_spend_any_upgrade_or_change_physical_cards():
    state = upgrade_state(3)
    step = ResolveUpgradesStep()
    pair = step._get_upgrade_options(state, "hero_arien")[0]["pair"]
    before = export_state(state)
    with pytest.raises(ValueError):
        step.validate_input(state, {"hero_id": "hero_arien", "card_ids": [*pair, "missing"]})
    assert export_state(state) == before


def test_all_six_upgrades_batch_is_validated_against_each_intermediate_tier():
    state = upgrade_state(6)
    preview = copy.deepcopy(state)
    step = ResolveUpgradesStep()
    cards = []
    for _ in range(6):
        chosen = step._get_upgrade_options(preview, "hero_arien")[0]["pair"][0]
        cards.append(chosen)
        apply_hero_upgrade(preview, "hero_arien", chosen)
    before = export_state(state)
    step.validate_input(state, {"hero_id": "hero_arien", "card_ids": cards})
    assert export_state(state) == before
    step.pending_input = {"selection": {"hero_id": "hero_arien", "card_ids": cards}}
    out = step.resolve(state, state.execution_context)
    assert state.get_hero("hero_arien").model_dump() == preview.get_hero("hero_arien").model_dump()
    assert not state.pending_upgrades and out.new_steps[0].type.value == "round_reset"


def test_sparse_negative_item_accumulates_in_place():
    state = upgrade_state(1)
    hero = state.get_hero("hero_arien")
    pair = ResolveUpgradesStep()._get_upgrade_options(state, "hero_arien")[0]["pair"]
    paired = next(c for c in hero.deck if c.id == pair[1])
    assert paired.item
    hero.items = {paired.item: -2}
    items = hero.items
    apply_hero_upgrade(state, "hero_arien", pair[0])
    assert hero.items is items and items == {paired.item: -1}


def test_imported_invalid_pending_input_is_cleared_and_loop_reissues_prompt():
    state = upgrade_state(1)
    step = ResolveUpgradesStep(
        pending_input={"selection": {"hero_id": "hero_arien", "card_id": "missing"}}
    )
    before = export_state(state)
    out = step.resolve(state, state.execution_context)
    assert step.pending_input is None and export_state(state) == before
    assert out.input_request.request_type.value == "UPGRADE_PHASE"


def test_simultaneous_request_preserves_pending_order_and_selection_counts():
    state = upgrade_state()
    state.pending_upgrades = {"hero_hanu": 1, "hero_arien": 3}
    req = ResolveUpgradesStep().resolve(state, state.execution_context).input_request
    assert list(req.context["players"]) == ["hero_hanu", "hero_arien"]
    assert req.context["players"]["hero_arien"]["selection_counts"] == [1, 3]
    assert req.context["players"]["hero_hanu"]["selection_counts"] == [1]
    assert req.context["supports_batch"] and req.context["supports_preview"]


def test_falsey_pending_input_mapping_is_retained_when_prompting():
    state = upgrade_state(1)
    step = ResolveUpgradesStep(pending_input={})
    pending = step.pending_input
    out = step.resolve(state, state.execution_context)
    assert step.pending_input is pending and not pending
    assert out.input_request.request_type.value == "UPGRADE_PHASE"
