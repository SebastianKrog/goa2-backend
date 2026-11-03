"""Frozen cleanup contracts preserve physical card/list/effect identity."""

import copy

import pytest
from live_cleanup_traces import scenes
from object_execution_traces import reference

from goa2.domain.models.effect import DurationType

NAMES = {
    "cleanup-live-post-round-commit",
    "cleanup-live-physical-flags",
    "cleanup-live-hand-flags",
    "cleanup-live-markers",
    "cleanup-live-persistent-token",
    "cleanup-live-temporary-token",
    "cleanup-live-round-finishing-ignored",
    "cleanup-live-level-gold-3",
    "cleanup-live-Takahide-ultimate",
    "cleanup-live-Ursafar-ultimate",
    f"cleanup-live-{DurationType.PASSIVE.value}-True-discard",
}


@pytest.fixture(scope="module")
def inputs():
    rows = {name: (state, ops) for name, state, ops in scenes(NAMES)}
    assert rows.keys() == NAMES
    return rows


def run(inputs, name):
    state, ops = copy.deepcopy(inputs[name])
    return state, reference(state, ops)


def test_retrieval_resets_only_returned_cards_and_preserves_list_objects(inputs):
    state, _ = run(inputs, "cleanup-live-physical-flags")
    hero = state.get_hero("hero_arien")
    hand, played, discard, deck = state.execution_context["hero_arien-lists"]
    assert (
        hero.hand is hand
        and hero.played_cards is played
        and hero.discard_pile is discard
        and hero.deck is deck
    )
    assert not played and not discard and hero.current_turn_card is None
    assert all(
        not c.is_facedown and not c.played_this_round and c.initiative_override is None
        for c in hand
    )
    other, _ = run(inputs, "cleanup-live-hand-flags")
    card = other.get_hero("hero_arien").hand[0]
    assert card.is_facedown and card.played_this_round and card.initiative_override == 17


def test_discard_linked_passive_survives_retrieval_with_its_active_card_flag(inputs):
    state, _ = run(inputs, f"cleanup-live-{DurationType.PASSIVE.value}-True-discard")
    effect = state.active_effects[0]
    card = state.get_card_by_id(effect.source_card_id)
    assert card.state.value == "HAND" and card.is_active
    assert effect is state.execution_context["effects_alias"][0]


def test_cleanup_drops_round_finishing_payloads_without_executing_them(inputs):
    state, _ = run(inputs, "cleanup-live-round-finishing-ignored")
    assert "should_not_finish" not in state.execution_context
    assert not state.active_effects
    assert state.execution_context["effects_alias"][0].finishing_steps


def test_only_transient_pool_tokens_and_their_effects_are_removed(inputs):
    transient, records = run(inputs, "cleanup-live-temporary-token")
    persistent, _ = run(inputs, "cleanup-live-persistent-token")
    assert transient.get_position("cleanup-token") is None and not transient.active_effects
    assert persistent.get_position("cleanup-token") is not None and persistent.active_effects
    assert any(e["event_type"] == "TOKEN_REMOVED" for row in records for e in row["events"])


def test_markers_return_in_place_and_planning_snapshot_is_a_new_shallow_copy(inputs):
    state, _ = run(inputs, "cleanup-live-markers")
    assert state.markers is state.execution_context["markers_alias"]
    assert all(
        m.target_id is None and m.source_id is None and m.value == 0 for m in state.markers.values()
    )
    assert state.last_turn_positions is not state.execution_context["snapshot_alias"]
    assert state.last_turn_positions is not state.entity_locations
    assert all(state.last_turn_positions[k] is value for k, value in state.entity_locations.items())


def test_level_purchases_keep_pending_maps_but_copy_sparse_item_reveal(inputs):
    state, _ = run(inputs, "cleanup-live-level-gold-3")
    hero = state.get_hero("hero_arien")
    assert (
        state.phase.value == "LEVEL_UP"
        and state.pending_upgrades[hero.id] == 2
        and hero.level == 3
        and hero.gold == 0
    )
    assert state.pending_upgrades is state.execution_context["pending_upgrades_alias"]
    assert state.upgrade_reveal_snapshot is state.execution_context["reveals_alias"]
    reveal = state.upgrade_reveal_snapshot[hero.id]
    assert reveal == hero.items and reveal is not hero.items and len(reveal) == 2
    assert state.active_effects and state.execution_stack[-1].type.value == "resolve_upgrades"


def test_ready_for_war_swaps_silver_and_retrieves_both_golds_in_order(inputs):
    state, records = run(inputs, "cleanup-live-Takahide-ultimate")
    hero = state.teams["RED"].heroes[0]
    events = [e for row in records for e in row["events"]]
    assert [e["event_type"] for e in events] == [
        "DECK_CARD_SWAPPED",
        "CARD_RETRIEVED",
        "CARD_RETRIEVED",
    ]
    assert hero.level == 8 and hero.gold == 0 and state.phase.value == "PLANNING"
    assert len(hero.hand) == 6


def test_unbound_fury_marks_every_physical_card_without_pity_gold(inputs):
    state, _ = run(inputs, "cleanup-live-Ursafar-ultimate")
    hero = state.teams["RED"].heroes[0]
    assert hero.level == 8 and hero.gold == 0 and state.phase.value == "PLANNING"
    assert all(c.enraged_active_override for c in [*hero.deck, *hero.hand, hero.ultimate_card])


def test_next_round_can_commit_and_uncommit_after_retrieving_current_card(inputs):
    state, records = run(inputs, "cleanup-live-post-round-commit")
    assert all(row["error"] is None for row in records)
    assert state.phase.value == "PLANNING"
    assert state.get_hero("hero_arien").current_turn_card is None
    assert len(state.get_hero("hero_arien").hand) == 5
