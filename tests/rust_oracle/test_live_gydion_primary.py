"""Frozen-source Gydion contracts; raw setup retains physical object aliases."""

from live_gydion_primary_traces import EFFECTS, action_scene
from object_execution_traces import reference


def run(effect, entry="text", mode="first", variant="ordinary"):
    state, ops, _ = action_scene(effect, entry, mode, variant)
    records = reference(state, ops)
    return state, records


def events(records):
    return [event for record in records for event in record["events"]]


def test_reviewed_school_recipes_offer_exact_source_school_spell_ids():
    from primary_builders import catalog

    from goa2.scripts.gydion_effects import SPELL_ACCESS_MAP

    recipes = catalog()["recipes"]
    assert set(EFFECTS) == {"prepare_spells", *SPELL_ACCESS_MAP}
    for effect, allowed in SPELL_ACCESS_MAP.items():
        assert len(recipes[effect]) == 1
        fields = recipes[effect][0]["fields"]
        assert fields["type"] == "cast_spell"
        assert fields["allowed_spell_ids"] == list(allowed)
        assert fields["caster_id"] == "template-passive-owner"


def test_each_school_casts_only_an_allowed_spell_from_the_same_physical_book():
    from goa2.domain.models import CardState
    from goa2.scripts.gydion_effects import SPELL_ACCESS_MAP

    for effect, allowed in SPELL_ACCESS_MAP.items():
        for mode in ("first", "last"):
            state, records = run(effect, mode=mode)
            hero = state.get_hero("hero_gydion")
            casts = [event for event in events(records) if event["event_type"] == "SPELL_CAST"]
            assert len(casts) == 1
            assert casts[0]["metadata"]["spell_id"] in allowed
            assert casts[0]["actor_id"] == hero.id
            spell = next(s for s in hero.spells if s.id == casts[0]["metadata"]["spell_id"])
            assert spell.state == CardState.OUTSIDE_SPELLBOOK
            assert state.execution_context["spells_alias"] is hero.spells


def test_spent_or_absent_book_cannot_cast_from_an_access_card():
    for variant in ("spent", "no-owner"):
        state, records = run("cantrip", variant=variant)
        assert not any(event["event_type"] == "SPELL_CAST" for event in events(records))
        assert state.execution_context["gydion_tail"] is True


def test_preparation_returns_and_hides_spells_in_the_existing_physical_collection():
    from goa2.domain.models import CardState

    state, records = run("prepare_spells", variant="spent")
    hero = state.get_hero("hero_gydion")
    assert len(hero.spells) == 22
    assert all(s.state == CardState.SPELLBOOK and s.is_facedown for s in hero.spells)
    assert state.execution_context["spells_alias"] is hero.spells
    assert not any(e["event_type"] == "SPELL_CAST" for e in events(records))


def test_archwizard_acceptance_replaces_preparation_but_decline_prepares_normally():
    from goa2.domain.models import CardState

    for variant in ("archwizard-yes", "archwizard-all-prepared", "archwizard-no"):
        state, records = run("prepare_spells", variant=variant)
        hero = state.get_hero("hero_gydion")
        assert hero.wish_cast_count == (0 if variant == "archwizard-no" else 1)
        if variant == "archwizard-no":
            assert all(s.state == CardState.SPELLBOOK for s in hero.spells)
        else:
            assert (
                next(s for s in hero.spells if s.id == "wish").state == CardState.OUTSIDE_SPELLBOOK
            )
            casts = [e for e in events(records) if e["event_type"] == "SPELL_CAST"]
            assert casts[0]["metadata"]["spell_id"] == "wish"


def test_archwizard_requires_current_actor_and_prepared_wish_then_prepares_when_absent():
    from goa2.domain.models import CardState

    for variant in ("archwizard-spent-wish", "archwizard-wrong-actor"):
        state, records = run("prepare_spells", variant=variant)
        hero = state.get_hero("hero_gydion")
        assert hero.wish_cast_count == 0
        assert all(s.state == CardState.SPELLBOOK for s in hero.spells)
        assert not any(e["event_type"] == "SPELL_CAST" for e in events(records))


def test_third_wish_through_preparation_wins_after_nested_action_or_fizzle():
    for variant in ("archwizard-third", "archwizard-third-fizzle", "archwizard-third-no-other"):
        state, records = run("prepare_spells", variant=variant)
        assert state.phase.value == "GAME_OVER"
        assert state.get_hero("hero_gydion").wish_cast_count == 3
        assert events(records)[-1]["event_type"] == "GAME_OVER"


def test_foreign_copies_cast_as_the_performer_and_do_not_use_the_owners_archwizard():
    state, records = run("cantrip", entry="copy")
    casts = [e for e in events(records) if e["event_type"] == "SPELL_CAST"]
    assert len(casts) == 1
    assert casts[0]["actor_id"] == "hero_arien"
    assert casts[0]["metadata"]["owner_id"] == "hero_gydion"
    state, records = run("prepare_spells", entry="copy", variant="archwizard-yes")
    assert state.get_hero("hero_gydion").wish_cast_count == 0
    assert not any(e["event_type"] == "SPELL_CAST" for e in events(records))


def test_dense_and_authoritative_recipe_sets_follow_reviewed_live_dependencies():
    from primary_builders import catalog
    from primary_templates import (
        ALL_EFFECTS,
        DENSE_EFFECTS,
        GYDION_EFFECTS,
        MIN_EFFECTS,
        MISA_EFFECTS,
        WASP_EFFECTS,
    )

    recipes = catalog()["recipes"]
    live = {
        effect
        for effect, rows in recipes.items()
        if any(row["fields"]["type"] in ("cast_spell", "prepare_spellbook") for row in rows)
    }
    assert live == set(GYDION_EFFECTS)
    assert set(ALL_EFFECTS) == set(DENSE_EFFECTS) | live | set(MISA_EFFECTS) | set(
        WASP_EFFECTS
    ) | set(MIN_EFFECTS)
    assert len(DENSE_EFFECTS) == 64
