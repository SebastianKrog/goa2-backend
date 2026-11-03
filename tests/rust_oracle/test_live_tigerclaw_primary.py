"""Frozen source contracts for Tigerclaw bodies and shared radius operands."""

from live_tigerclaw_primary_traces import action_scene, base, steal_scenes, terrain_scenes
from object_execution_traces import reference


def run(effect, mode="first", variant="ordinary"):
    state, ops = action_scene(effect, "text", mode, variant)
    return state, reference(state, ops)


def test_flanking_sets_bonus_and_ballista_blocks_primary_defense():
    for effect in ("backstab", "backstab_with_a_ballista"):
        state, records = run(effect)
        assert any(
            row["semantic_state"]["execution_context"].get("atk_bonus") == 2 for row in records
        )
        assert any(
            event["event_type"] == "COMBAT_RESOLVED" for row in records for event in row["events"]
        )
        if effect == "backstab_with_a_ballista":
            assert any(
                row["semantic_state"]["execution_context"].get("block_primary_defense")
                for row in records
            )
        state, _records = run(effect, variant="no-flank")
        assert not state.execution_context.get("has_flanking")


def test_combat_reflexes_moves_before_or_after_attack_but_not_both():
    for variant, before in (("pre-hit", True), ("pre-skip", False)):
        _state, records = run("combat_reflexes", variant=variant)
        events = [event["event_type"] for row in records for event in row["events"]]
        assert events.count("UNIT_MOVED") == 1
        assert (events.index("UNIT_MOVED") < events.index("COMBAT_RESOLVED")) == before


def test_theft_transfers_real_gold_and_gates_post_movement():
    for effect in ("light_fingered", "pick_pocket", "master_thief"):
        state, records = run(effect, mode="last")
        events = [event for row in records for event in row["events"]]
        gold = next(
            event
            for event in events
            if event["event_type"] == "GOLD_GAINED" and event["metadata"].get("reason") == "steal"
        )
        amount = 2 if effect == "master_thief" else 1
        assert gold["metadata"]["amount"] == amount
        assert state.get_hero("hero_tigerclaw").gold == 1 + amount
        assert state.get_hero("hero_brogan").gold == 3 - amount
        assert any(event["event_type"] == "UNIT_MOVED" for event in events)
        state, records = run(effect, variant="no-coins")
        assert not state.execution_context.get("stole_coins")
        assert not any(
            event["event_type"] == "UNIT_MOVED" for row in records for event in row["events"]
        )


def test_blend_zero_and_absent_radius_use_two_and_create_next_turn_immunity():
    from goa2.domain.models.effect import DurationType, EffectType

    choices = []
    for variant in ("ordinary", "radius-none", "radius-zero"):
        state, records = run("blend_into_shadows", variant=variant)
        request = next(row["request"] for row in records if row["request"])
        choices.append(request["options"])
        created = state.active_effects[-1]
        assert created.effect_type == EffectType.ATTACK_IMMUNITY
        assert created.duration == DurationType.NEXT_TURN
        assert created.source_card_id == "blend_into_shadows"
    assert choices[0] == choices[1] == choices[2]
    state, _ = run("blend_into_shadows", variant="no-terrain")
    assert not state.active_effects
    state, _ = run("blend_into_shadows", variant="petrify")
    assert state.execution_context["can_blend"]


def test_terrain_filter_distinguishes_strings_and_hexes_including_virtual_edge():
    from goa2.domain.hex import Hex
    from goa2.engine.filters import TerrainFilter

    state, _hero, _card, h = base("blend_into_shadows")
    for wanted in (True, False):
        predicate = TerrainFilter(is_terrain=wanted)
        assert predicate.apply("hero_brogan", state, {}) is wanted
        assert predicate.apply(Hex(q=99, r=0, s=-99), state, {}) is wanted
        assert predicate.apply(h(0, -1), state, {}) is wanted
        assert predicate.apply(h(-1, 0), state, {}) is not wanted
    for name, state, ops in terrain_scenes():
        if "-UNIT-" in name or "-UNIT_OR_TOKEN-" in name:
            record = reference(state, ops)[0]
            assert (record["request"] is not None) == ("-True-" in name)


def test_coin_transfer_self_piece_and_stale_output_match_source():
    wanted = {"self", "piece-self", "dynamic-true", "stale-output", "override-actor", "repeated"}
    for name, state, ops in steal_scenes():
        variant = name.removeprefix("tiger-primary-steal-")
        if variant not in wanted:
            continue
        actor = state.execution_context["steal_actor_alias"]
        old = actor.gold
        records = reference(state, ops)
        events = [event for row in records for event in row["events"]]
        if variant in ("self", "piece-self"):
            assert actor.gold == old and len(events) == 1
        if variant == "piece-self":
            assert events[0]["actor_id"] == events[0]["target_id"] == "hero_razzle_piece_1"
        if variant == "dynamic-true":
            assert events[0]["metadata"]["amount"] is True
        if variant == "stale-output":
            assert not events and state.execution_context["stolen"] is True
        if variant == "override-actor":
            assert events[0]["actor_id"] == "hero_hanu"
        if variant == "repeated":
            assert len(events) == 2


def test_all_referenced_active_tigerclaw_builders_have_reviewed_recipes():
    from model_traces import setup
    from primary_templates import EFFECTS

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffect, CardEffectRegistry

    setup()
    hero = HeroRegistry.get("Tigerclaw")
    active = {
        card.effect_id
        for card in hero.deck
        if CardEffectRegistry.get(card.effect_id) is not None
        and type(CardEffectRegistry.get(card.effect_id)).build_steps is not CardEffect.build_steps
    }
    assert len(active) == 12
    assert active <= EFFECTS.keys()


def test_negative_attack_values_remain_legal_and_are_not_clamped():
    for effect in ("backstab", "backstab_with_a_ballista"):
        _state, records = run(effect, variant="negative-attack")
        combat = [
            event
            for row in records
            for event in row["events"]
            if event["event_type"] == "COMBAT_RESOLVED"
        ]
        assert len(combat) == 1
        assert combat[0]["metadata"]["attack_value"] == -2
