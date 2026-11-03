"""Misa source contracts; raw setup preserves physical card/model identities."""

from live_misa_primary_traces import action_scene
from object_execution_traces import reference


def run(effect, entry="text", mode="first", variant="ordinary"):
    state, ops, _ = action_scene(effect, entry, mode, variant)
    records = reference(state, ops)
    return state, [event for record in records for event in record["events"]]


def test_misa_catalog_separates_pure_recipes_from_explicit_live_ultimate_shapes():
    from primary_builders import catalog
    from primary_templates import MISA_EFFECTS, MISA_STATEFUL_EFFECTS

    data = catalog()
    assert data["schema_version"] == 3
    assert len(data["recipes"]) == 111 and len(data["stateful_recipes"]) == 2
    assert set(MISA_EFFECTS) - set(data["recipes"]) == set(MISA_STATEFUL_EFFECTS)
    for effect, lengths in (("swoop_in", (4, 6)), ("watch_how_i_soar", (7, 15))):
        body = data["stateful_recipes"][effect]
        assert body["predicate"] == "misa_has_ultimate"
        assert (
            tuple(len(body["variants"][name]) for name in ("without_ultimate", "with_ultimate"))
            == lengths
        )


def test_melee_cards_grant_their_fixed_defense_after_the_attack():
    for effect, defense in (
        ("challenge_accepted", 2),
        ("matter_of_honor", 3),
        ("worthy_opponent", 5),
    ):
        state, events = run(effect)
        buff = state.active_effects[-1]
        assert buff.stat_type.value == "DEFENSE" and buff.stat_value == defense
        assert buff.duration.value == "THIS_TURN"
        assert buff.scope.origin_id == "hero_misa"
        assert any(e["event_type"] == "COMBAT_RESOLVED" for e in events)


def test_ranged_surviving_targets_move_only_at_the_required_distance():
    from goa2.domain.hex import Hex

    for effect in ("power_shot", "thunder_shot"):
        state, _ = run(effect, variant="max-range-survives")
        assert state.get_position("hero_arien") != Hex(q=2, r=0, s=-2)
        state, _ = run(effect, variant="adjacent-survives")
        assert state.get_position("hero_arien") == Hex(q=1, r=0, s=-1)


def test_blue_discard_moves_through_an_enemy_before_discarding_its_hand():
    from goa2.domain.hex import Hex

    for effect in ("dash_and_slash", "death_from_above"):
        state, _ = run(effect)
        assert state.get_position("hero_misa") == Hex(q=2, r=0, s=-2)
        assert len(state.get_hero("hero_arien").hand) == 4
        state, _ = run(effect, variant="no-crossed")
        assert state.get_position("hero_misa") == Hex(q=2, r=0, s=-2)
        assert len(state.get_hero("hero_arien").hand) == 5


def test_blue_placement_moves_the_crossed_unit_adjacent_to_the_performer():
    from goa2.domain.hex import Hex

    for effect in ("sudden_breeze", "gust_of_wind", "crushing_squall"):
        state, _ = run(effect)
        actor, target = state.get_position("hero_misa"), state.get_position("hero_arien")
        assert actor == Hex(q=2, r=0, s=-2)
        assert target != Hex(q=1, r=0, s=-1) and actor.distance(target) == 1


def test_swap_cards_preserve_their_exact_or_equal_distance_constraints():
    from goa2.domain.hex import Hex

    for effect in ("living_tornado", "storm_spirit"):
        state, _ = run(effect)
        assert state.get_position("hero_arien") == Hex(q=0, r=3, s=-3)
        assert state.get_position("hero_gydion") == Hex(q=3, r=0, s=-3)


def test_pre_action_effects_keep_fixed_distance_and_foreign_performer_binding():
    for effect, distance in (("focus", 1), ("discipline", 2), ("mastery", 3)):
        for entry, owner in (("text", "hero_misa"), ("copy", "hero_gydion")):
            state, _ = run(effect, entry)
            buff = state.active_effects[-1]
            assert buff.effect_type.value == "pre_action_movement"
            assert buff.duration.value == "NEXT_TURN" and buff.max_value == distance
            assert buff.source_id == owner and buff.scope.origin_id == owner
            assert buff.source_card_id == effect


def test_swoop_retrieves_after_placement_and_ultimate_discards_an_adjacent_hero_card():
    from goa2.domain.hex import Hex

    for variant in ("ordinary", "ultimate"):
        state, _ = run("swoop_in", variant=variant)
        assert state.get_position("hero_misa") == Hex(q=1, r=-1, s=0)
        assert not state.get_hero("hero_misa").discard_pile
        assert len(state.get_hero("hero_arien").hand) == (4 if variant == "ultimate" else 5)


def test_soar_ultimate_performs_the_two_distinct_options_in_both_orders():
    for variant, first in (("ultimate-place", "UNIT_PLACED"), ("ultimate-defeat", "UNIT_DEFEATED")):
        state, events = run("watch_how_i_soar", variant=variant)
        relevant = [
            event["event_type"]
            for event in events
            if event["event_type"] in ("UNIT_PLACED", "UNIT_DEFEATED")
        ]
        assert relevant == (
            ["UNIT_PLACED", "UNIT_DEFEATED"]
            if first == "UNIT_PLACED"
            else ["UNIT_DEFEATED", "UNIT_PLACED"]
        )
        assert len(state.get_hero("hero_arien").hand) == 4


def test_level_seven_or_missing_ultimate_cannot_grant_the_extra_option():
    for variant in ("level-seven", "no-ultimate", "ultimate-decline"):
        _, events = run("watch_how_i_soar", variant=variant)
        assert sum(e["event_type"] == "UNIT_PLACED" for e in events) == 1
        assert not any(e["event_type"] == "UNIT_DEFEATED" for e in events)


def test_foreign_ultimate_gate_uses_the_performer_and_accepts_a_different_ultimate():
    from goa2.domain.hex import Hex

    state, _ = run("swoop_in", "copy", variant="foreign-ultimate")
    assert state.get_hero("hero_misa").ultimate_card is None
    assert state.get_hero("hero_gydion").ultimate_card.id == "the_archwizard"
    assert state.get_position("hero_misa") == Hex(q=0, r=0, s=0)
    assert state.get_position("hero_gydion") == Hex(q=1, r=-1, s=0)
    assert len(state.get_hero("hero_arien").hand) == 4


def test_negative_attack_values_remain_legal_for_all_misa_attack_bodies():
    for effect in (
        "challenge_accepted",
        "matter_of_honor",
        "worthy_opponent",
        "power_shot",
        "thunder_shot",
    ):
        _, events = run(effect, variant="negative-attack")
        attacks = [e for e in events if e["event_type"] == "COMBAT_RESOLVED"]
        assert attacks and attacks[0]["metadata"]["attack_value"] < 0
