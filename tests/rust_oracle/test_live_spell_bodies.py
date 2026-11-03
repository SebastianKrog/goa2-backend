"""Source contracts for real spell actions, dynamic values and rule consumers."""

from live_spell_body_traces import action_scene, consumer_scenes, foreign_scenes
from object_execution_traces import reference


def run(effect, variant="ordinary", mode="first"):
    state, ops, _ = action_scene(effect, mode=mode, variant=variant)
    records = reference(state, ops)
    return state, [event for row in records for event in row["events"]]


def test_all_spell_ids_have_distinct_namespaced_source_prototypes():
    from primary_builders import catalog as ordinary
    from spell_builders import EFFECTS, catalog

    data = catalog()
    assert len(EFFECTS) == 22 and set(EFFECTS) <= data["recipes"].keys()
    assert data["recipes"]["shield"][0]["fields"]["effect_type"] == "attack_immunity"
    assert ordinary()["recipes"]["shield"][0]["fields"]["effect_type"] == "minion_protection"


def test_midas_counts_other_spent_spells_with_floor_division():
    for variant, coins in (
        ("ordinary", 0),
        ("outside-two", 1),
        ("outside-three", 1),
        ("outside-many", 3),
    ):
        state, events = run("midas_touch", variant)
        gained = [e for e in events if e["event_type"] == "GOLD_GAINED"]
        assert sum(e["metadata"]["amount"] for e in gained) == coins
        assert state.get_hero("hero_gydion").gold == coins


def test_dimension_door_uses_exact_distance_and_default_zero_radius():
    from goa2.domain.hex import Hex

    for variant in ("outside-two", "radius-none", "radius-zero"):
        state, ops, _ = action_scene("dimension_door", variant=variant)
        hero = state.get_hero("hero_gydion")
        card = next(c for c in hero.spells if c.id == "dimension_door")
        expected = (card.radius_value or 0) + (2 if variant == "outside-two" else 0)
        reference(state, ops)
        final = state.get_position(hero.id)
        assert final.distance(Hex(q=0, r=0, s=0)) == expected


def test_sunburst_adds_spent_spell_count_to_attack_and_range():
    from goa2.engine.stats import compute_card_stats

    state, ops, _ = action_scene("sunburst", variant="outside-two")
    hero = state.get_hero("hero_gydion")
    card = next(c for c in hero.spells if c.id == "sunburst")
    stats = compute_card_stats(state, hero.id, card)
    assert (
        state.get_position("hero_tigerclaw").distance(state.get_position(hero.id))
        == (stats.range or 0) + 2
    )
    records = reference(state, ops)
    combats = [e for r in records for e in r["events"] if e["event_type"] == "COMBAT_RESOLVED"]
    assert combats and combats[0]["metadata"]["attack_value"] == stats.primary_value + 2


def test_fireball_rejects_friend_adjacent_targets_and_restores_origin_context():
    state, events = run("fireball")
    assert any(e["event_type"] == "COMBAT_RESOLVED" for e in events)
    state, events = run("fireball", "friendly-neighbor")
    assert not any(e["event_type"] == "COMBAT_RESOLVED" for e in events)
    assert state.execution_context["_cmf_origin_hex"] == {"retained": True}


def test_create_undead_uses_real_lane_supply_and_zone_capacity():
    for variant, placed in (
        ("ordinary", True),
        ("one-type", True),
        ("full-zone", False),
        ("no-zone", False),
        ("no-team", False),
    ):
        _, events = run("create_undead", variant)
        assert any(e["event_type"] == "UNIT_PLACED" for e in events) == placed


def test_disintegrate_removes_tokens_and_minions_without_kill_rewards():
    for variant in ("ordinary", "token"):
        _, events = run("disintegrate", variant)
        kinds = [e["event_type"] for e in events]
        assert ("TOKEN_REMOVED" in kinds) if variant == "token" else ("UNIT_REMOVED" in kinds)
        assert "GOLD_GAINED" not in kinds


def test_immunity_consumers_distinguish_basic_and_nonbasic_attacks():
    for name, state, ops in consumer_scenes():
        if "-consumer-" not in name:
            continue
        reference(state, ops)
        basic = name.endswith("-True")
        blocked = basic if "-shield-" in name else not basic
        assert state.execution_context["spell_attack_targets"] == int(not blocked)


def test_familiar_recast_moves_the_same_physical_pool_token():
    from goa2.domain.models import TokenType

    for name, state, ops in consumer_scenes():
        if not name.endswith("-recast"):
            continue
        token = state.token_pool[TokenType.FAMILIAR][0]
        old = state.get_position(token.id)
        reference(state, ops)
        assert state.token_pool[TokenType.FAMILIAR][0] is token
        assert state.get_position(token.id) != old
        assert state.execution_context["familiar_recast_done"]


def test_foreign_casters_keep_coin_effect_team_and_wish_ownership():
    for name, state, ops in foreign_scenes():
        reference(state, ops)
        assert state.get_hero("hero_gydion").wish_cast_count == 0
        if "-wish-" in name:
            assert state.get_hero("hero_arien").wish_cast_count == 1
        if "-midas_touch-" in name:
            assert state.get_hero("hero_arien").gold == 1
        if "-shield-" in name:
            assert state.active_effects[-1].source_id == "hero_arien"


def test_third_wish_wins_after_nested_resolution_or_mandatory_failure():
    for variant in ("wish-count-two", "wish-no-other", "wish-fizzle"):
        state, events = run("wish", variant)
        assert state.phase.value == "GAME_OVER"
        assert state.get_hero("hero_gydion").wish_cast_count == 3
        assert events[-1]["event_type"] == "GAME_OVER"


def test_negative_spell_attacks_remain_legal_and_unclamped():
    for effect in ("shocking_grasp", "magic_missile", "fireball"):
        _, events = run(effect, "negative-attack")
        combats = [e for e in events if e["event_type"] == "COMBAT_RESOLVED"]
        assert combats and combats[0]["metadata"]["attack_value"] < 0


def test_count_match_cleans_null_origin_only_when_evaluated_before_writing_output():
    from live_spell_primitive_traces import null_origin_scenes

    for name, state, operations in null_origin_scenes():
        reference(state, operations)
        if name.endswith(("missing", "skip")):
            assert "_cmf_origin_hex" in state.execution_context
            assert state.execution_context["_cmf_origin_hex"] is None
        elif name.endswith("output"):
            assert state.execution_context["_cmf_origin_hex"] is True
        else:
            assert "_cmf_origin_hex" not in state.execution_context
