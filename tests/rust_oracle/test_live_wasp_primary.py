"""Frozen-source Wasp behavior; raw traces retain physical objects and events."""

from live_wasp_primary_traces import ATTACKS, ORBITS, TELEPORTS, action_scene, query_scenes
from object_execution_traces import reference


def run(effect, entry="text", mode="first", variant="ordinary"):
    state, ops, _ = action_scene(effect, entry, mode, variant)
    records = reference(state, ops)
    return state, [event for row in records for event in row["events"]]


def test_all_wasp_primary_bodies_are_pure_source_recipes():
    from primary_builders import catalog
    from primary_templates import WASP_EFFECTS

    data = catalog()
    assert len(data["recipes"]) == 111 and len(data["stateful_recipes"]) == 2
    assert set(WASP_EFFECTS) <= set(data["recipes"])
    assert len(WASP_EFFECTS) == 14


def test_repeat_constructors_isolate_original_steps_from_templates():
    from live_wasp_primary_traces import base
    from primary_builders import freeze_steps

    from goa2.engine.effects import CardEffectRegistry

    for effect in (*ORBITS, "mass_telekinesis"):
        state, hero, card = base(effect)
        steps = CardEffectRegistry.get(effect).get_steps(state, hero, card)
        template = steps[-1].steps_template
        assert all(child is not root for child in template for root in steps)
        assert freeze_steps(steps)


def test_orbits_move_two_or_three_times_and_preserve_distance():
    from goa2.domain.hex import Hex

    for effect, moves in (("lift_up", 2), ("control_gravity", 2), ("center_of_mass", 3)):
        state, events = run(effect)
        assert sum(e["event_type"] == "UNIT_MOVED" for e in events) == moves
        assert not any(e["event_type"] == "UNIT_PLACED" for e in events)
        assert state.get_position("hero_brogan").distance(Hex(q=0, r=0, s=0)) == 1
        _, events = run(effect, variant="decline")
        assert sum(e["event_type"] == "UNIT_MOVED" for e in events) == 1


def test_orbits_move_the_same_physical_token():
    from goa2.domain.models import TokenType

    for effect in ORBITS:
        state, events = run(effect, variant="token")
        token = state.execution_context["wasp_token_alias"]
        assert token is state.token_pool[TokenType.TREE][0]
        assert state.get_position(token.id).distance(state.get_position("hero_wasp")) == 1
        assert any(e["event_type"] == "UNIT_MOVED" for e in events)


def test_telekinesis_places_targets_adjacent_and_mass_repeats_once():
    for effect, count in zip(TELEPORTS, (1, 2), strict=True):
        state, events = run(effect)
        assert sum(e["event_type"] == "UNIT_PLACED" for e in events) == count
        assert state.get_position("hero_brogan").distance(state.get_position("hero_wasp")) == 1
        state, events = run(effect, variant="token")
        assert (
            state.get_position(state.execution_context["wasp_token_id"]).distance(
                state.get_position("hero_wasp")
            )
            == 1
        )
        assert any(e["event_type"] == "TOKEN_MOVED" for e in events)


def test_kinetic_collision_discards_only_for_heroes():
    for effect in ("kinetic_repulse", "kinetic_blast"):
        state, events = run(effect)
        assert len(state.get_hero("hero_brogan").hand) == 5
        assert sum(e["event_type"] == "UNIT_PUSHED" for e in events) == 2
        state, _ = run(effect, variant="collision")
        assert len(state.get_hero("hero_brogan").hand) == 4
        assert state.execution_context["push_collision"] is True
        assert state.execution_context["is_hero"] is False
        assert state.execution_context["should_discard"] is False


def test_kinetic_blast_uses_chosen_distance_and_detects_board_edge_collision():
    state, _ = run("kinetic_blast", variant="distance-four")
    assert state.execution_context["push_distance"] == 4
    assert state.get_position("hero_brogan").q == 4
    assert len(state.get_hero("hero_brogan").hand) == 4


def test_only_electroblast_defeats_the_empty_hand_post_attack_victim():
    for effect, count in (("shock", 1), ("electrocute", 1), ("electroblast", 2)):
        _, events = run(effect, variant="empty-hand")
        assert sum(e["event_type"] == "UNIT_DEFEATED" for e in events) == count


def test_thunder_boomerang_repeats_on_a_different_target_and_respects_skip():
    for variant in ("ordinary", "survives"):
        _, events = run("thunder_boomerang", variant=variant)
        attacks = [e for e in events if e["event_type"] == "COMBAT_RESOLVED"]
        assert len(attacks) == 2
        assert attacks[0]["target_id"] != attacks[1]["target_id"]
    _, events = run("thunder_boomerang", mode="skip")
    assert sum(e["event_type"] == "COMBAT_RESOLVED" for e in events) == 1


def test_created_effects_bind_foreign_performer_card_and_radius_items():
    for effect in ("magnetic_dagger", "static_barrier"):
        for entry, owner in (("text", "hero_wasp"), ("copy", "hero_wuk")):
            state, _ = run(effect, entry, variant="items")
            buff = state.active_effects[-1]
            assert buff.source_id == owner and buff.source_card_id == effect
            if effect == "static_barrier":
                assert buff.barrier_origin_id == owner
            else:
                assert buff.scope.origin_id == owner


def test_negative_attacks_are_legal_and_unclamped():
    for effect in ATTACKS:
        _, events = run(effect, variant="negative-attack")
        attacks = [e for e in events if e["event_type"] == "COMBAT_RESOLVED"]
        assert attacks and attacks[0]["metadata"]["attack_value"] < 0


def test_unit_type_queries_preserve_piece_normalization_unknown_minion_and_false():
    names = {
        "wasp-query-type-HERO-piece": True,
        "wasp-query-type-MINION-unknown": True,
        "wasp-query-type-MINION-token": True,
        "wasp-query-type-HERO-unknown": False,
        "wasp-query-type-HERO-missing": False,
        "wasp-query-type-HERO-zero": False,
        "wasp-query-type-HERO-numeric": True,
        "wasp-query-type-UNKNOWN-hero": False,
        "wasp-query-type-HERO-literal-not-a-unit": False,
        "wasp-query-type-HERO-literal-empty": True,
    }
    for name, state, ops in query_scenes():
        if name in names:
            reference(state, ops)
            assert state.execution_context["query_result"] is names.pop(name)
    assert not names


def test_boolean_queries_use_container_truthiness_and_unknown_means_and():
    names = {
        "wasp-query-boolean-OR-5": True,
        "wasp-query-boolean-AND-5": False,
        "wasp-query-boolean-AND-8": True,
        "wasp-query-boolean-unknown-8": True,
        "wasp-query-boolean-or-1": False,
        "wasp-query-boolean-OR-missing": False,
    }
    for name, state, ops in query_scenes():
        if name in names:
            reference(state, ops)
            assert state.execution_context["query_result"] is names.pop(name)
    assert not names
