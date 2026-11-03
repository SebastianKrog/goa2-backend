"""Frozen-source behavioral contracts; raw setup preserves live model aliases."""

from live_brogan_primary_traces import action_scene, query_scenes
from object_execution_traces import reference


def run(effect, mode="first", variant="ordinary"):
    state, ops, _ = action_scene(effect, "text", mode, variant)
    records = reference(state, ops)
    return state, records


def test_rewards_use_root_defeat_record_including_zero_marks():
    for effect, low, high in (("war_drummer", 1, 3), ("master_skald", 2, 4)):
        for variant in ("ordinary", "defeated", "empty-defeat-mark"):
            state, records = run(effect, variant=variant)
            events = [e for r in records for e in r["events"] if e["event_type"] == "GOLD_GAINED"]
            assert len(events) == 1
            assert events[0]["metadata"]["amount"] == (low if variant == "ordinary" else high)
            assert state.execution_context["hero_died"] is (None if variant == "ordinary" else True)


def test_spear_can_use_existing_discard_while_axe_requires_new_discard():
    for effect in ("throwing_axe", "throwing_spear"):
        _, records = run(effect, "ranged", "existing-discard")
        combat = [e for r in records for e in r["events"] if e["event_type"] == "COMBAT_RESOLVED"]
        assert bool(combat) == (effect == "throwing_spear")
        _, records = run(effect, "skip")
        assert not any(e["event_type"] == "COMBAT_RESOLVED" for r in records for e in r["events"])
        _, records = run(effect, "ranged")
        assert any(e["event_type"] == "COMBAT_RESOLVED" for r in records for e in r["events"])


def test_failed_charge_never_attacks_and_success_moves_before_attack():
    for effect in ("bullrush", "furious_charge"):
        _, records = run(effect)
        events = [e["event_type"] for r in records for e in r["events"]]
        assert events.index("UNIT_MOVED") < events.index("COMBAT_RESOLVED")
        for variant in ("no-enemy", "blocked"):
            _, records = run(effect, variant=variant)
            assert not any(
                e["event_type"] == "COMBAT_RESOLVED" for r in records for e in r["events"]
            )


def test_protection_defaults_and_discard_colors():
    for effect, fallback in (("shield", 1), ("bolster", 1), ("fortify", 2)):
        for variant in ("radius-none", "radius-zero"):
            state, _ = run(effect, variant=variant)
            created = state.active_effects[-1]
            assert created.scope.range == fallback
            assert [c.value for c in created.allowed_discard_colors] == (
                ["GOLD", "SILVER"] if effect == "fortify" else ["SILVER"]
            )


def test_bulwark_retrieves_and_blocks_only_enemy_displacement():
    state, _ = run("bulwark", variant="retrieve")
    assert not state.get_hero("hero_brogan").discard_pile
    assert len(state.active_effects) == 2
    for effect in state.active_effects:
        assert effect.blocks_enemy_actors
        assert not effect.blocks_friendly_actors and not effect.blocks_self
        assert {v.value for v in effect.displacement_blocks} == {
            "MOVE",
            "PUSH",
            "SWAP",
            "PLACE",
        }


def test_queries_count_holes_and_preserve_skipped_outputs():
    for name, state, ops in query_scenes():
        reference(state, ops)
        value = state.execution_context["query_result"]
        if name.endswith(("-skip", "-inactive")):
            assert value == "retained"
        elif "-defeat-" in name:
            assert value is (None if name.endswith("-empty") else True)
        elif name.endswith(("-unknown", "-no-actor")):
            assert value == 0
        elif "-PLAYED-actor" in name or "-PLAYED-empty-key" in name or "-PLAYED-literal" in name:
            assert value == 2


def test_all_brogan_builder_overrides_have_recipes():
    from model_traces import setup
    from primary_templates import EFFECTS

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffect, CardEffectRegistry

    setup()
    hero = HeroRegistry.get("Brogan")
    active = {
        c.effect_id
        for c in [*hero.deck, hero.ultimate_card]
        if c and type(CardEffectRegistry.get(c.effect_id)).build_steps is not CardEffect.build_steps
    }
    assert len(active) == 18
    assert active <= EFFECTS.keys()


def test_created_protections_are_consumed_by_real_defeat_and_displacement():
    from live_brogan_primary_traces import protection_scenes

    for name, state, ops in protection_scenes():
        records = reference(state, ops)
        events = [e for r in records for e in r["events"]]
        if "-bulwark-" in name:
            moved = any(e["event_type"] == "UNIT_PUSHED" for e in events)
            assert moved == ("-dormant-" in name or "-hero_arien-" not in name)
        else:
            protected = any(e["event_type"] == "MINION_PROTECTED" for e in events)
            assert protected == (name.endswith("-protect") or name.endswith("-fortify-gold-only"))
            assert any(e["event_type"] == "GOLD_GAINED" for e in events)
