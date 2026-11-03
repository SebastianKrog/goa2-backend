"""Freeze reviewed, data-only primary effect recipes using the real effect API.

Only the named pure recipe builders are supported. The native interpreter binds
reserved hero/card/stat operands; arbitrary Python effects are never inferred.
"""

from __future__ import annotations

import json

from oracle import ROOT, digest, verify_sources

HERO = "native-primary-hero"
CARD = "native-primary-card"
PRIMARY = 1234567891
RANGE = 1234567892
RADIUS = 1234567893
EFFECTS = {
    "bullrush": "Brogan",
    "furious_charge": "Brogan",
    "throwing_axe": "Brogan",
    "throwing_spear": "Brogan",
    "shield": "Brogan",
    "bolster": "Brogan",
    "fortify": "Brogan",
    "war_drummer": "Brogan",
    "master_skald": "Brogan",
    "bulwark": "Brogan",
    "one_man_army": "Brogan",
    "backstab": "Tigerclaw",
    "backstab_with_a_ballista": "Tigerclaw",
    "combat_reflexes": "Tigerclaw",
    "light_fingered": "Tigerclaw",
    "pick_pocket": "Tigerclaw",
    "master_thief": "Tigerclaw",
    "blend_into_shadows": "Tigerclaw",
    "mind_grip": "NebKher",
    "spell_break": "Arien",
    "ebb_and_flow": "Arien",
    "dangerous_current": "Arien",
    "raging_stream": "Arien",
    "violent_torrent": "Arien",
    "slippery_ground": "Arien",
    "deluge": "Arien",
    "noble_blade": "Arien",
    "arcane_whirlpool": "Arien",
    "liquid_leap": "Arien",
    "magical_current": "Arien",
    "stranger_tide": "Arien",
    "onslaught": "Brogan",
    "hit_and_run": "Tigerclaw",
    "leaping_strike": "Tigerclaw",
    "poisoned_dagger": "Tigerclaw",
    "poisoned_dart": "Tigerclaw",
    "rogue_wave": "Arien",
    "tidal_blast": "Arien",
    "blink_strike": "Tigerclaw",
    "mad_dash": "Brogan",
    "brutal_jab": "Brogan",
    "mighty_punch": "Brogan",
    "savage_kick": "Brogan",
    "point_blank_shot": "Sabina",
    "back_to_back": "Sabina",
    "listen_up": "Sabina",
    "roger_roger": "Sabina",
    "ready_and_waiting": "Sabina",
    "troop_movement": "Sabina",
    "marching_orders": "Sabina",
    "path_to_victory": "Sabina",
    "close_support": "Sabina",
    "covering_fire": "Sabina",
    "shootout": "Sabina",
    "bullet_hell": "Sabina",
    "quickdraw": "Sabina",
    "gunslinger": "Sabina",
    "dead_shot": "Sabina",
    "shield_bash": "Brogan",
    "counterattack": "Brogan",
    "steady_advance": "Sabina",
    "unwavering_resolve": "Sabina",
    "prepare_spells": "Gydion",
    "cantrip": "Gydion",
    "elementary_evocation": "Gydion",
    "lesser_evocation": "Gydion",
    "greater_evocation": "Gydion",
    "elementary_abjuration": "Gydion",
    "lesser_abjuration": "Gydion",
    "greater_abjuration": "Gydion",
    "elementary_enchantment": "Gydion",
    "lesser_enchantment": "Gydion",
    "greater_enchantment": "Gydion",
    "lesser_necromancy": "Gydion",
    "greater_necromancy": "Gydion",
    "lesser_conjuration": "Gydion",
    "greater_conjuration": "Gydion",
    "lesser_transmutation": "Gydion",
    "greater_transmutation": "Gydion",
    "challenge_accepted": "Misa",
    "matter_of_honor": "Misa",
    "worthy_opponent": "Misa",
    "power_shot": "Misa",
    "thunder_shot": "Misa",
    "dash_and_slash": "Misa",
    "death_from_above": "Misa",
    "sudden_breeze": "Misa",
    "gust_of_wind": "Misa",
    "crushing_squall": "Misa",
    "living_tornado": "Misa",
    "storm_spirit": "Misa",
    "focus": "Misa",
    "discipline": "Misa",
    "mastery": "Misa",
    "power_overwhelming": "Misa",
    "magnetic_dagger": "Wasp",
    "charged_boomerang": "Wasp",
    "shock": "Wasp",
    "electrocute": "Wasp",
    "telekinesis": "Wasp",
    "mass_telekinesis": "Wasp",
    "electroblast": "Wasp",
    "thunder_boomerang": "Wasp",
    "lift_up": "Wasp",
    "control_gravity": "Wasp",
    "center_of_mass": "Wasp",
    "kinetic_repulse": "Wasp",
    "kinetic_blast": "Wasp",
    "static_barrier": "Wasp",
    "devoted_followers": "Xargatha",
    "fresh_converts": "Xargatha",
}


# These reviewed builders require the authoritative spellbook bindings. The
# canonical dense-path corpus keeps its own reviewed closure until that bridge
# is implemented; the object corpus covers every recipe in EFFECTS.
GYDION_EFFECTS = (
    "prepare_spells",
    "cantrip",
    "elementary_evocation",
    "lesser_evocation",
    "greater_evocation",
    "elementary_abjuration",
    "lesser_abjuration",
    "greater_abjuration",
    "elementary_enchantment",
    "lesser_enchantment",
    "greater_enchantment",
    "lesser_necromancy",
    "greater_necromancy",
    "lesser_conjuration",
    "greater_conjuration",
    "lesser_transmutation",
    "greater_transmutation",
)
MISA_STATEFUL_EFFECTS = {"swoop_in": "Misa", "watch_how_i_soar": "Misa"}
MISA_EFFECTS = (
    "challenge_accepted",
    "matter_of_honor",
    "worthy_opponent",
    "power_shot",
    "thunder_shot",
    "dash_and_slash",
    "death_from_above",
    "sudden_breeze",
    "gust_of_wind",
    "crushing_squall",
    "living_tornado",
    "storm_spirit",
    "focus",
    "discipline",
    "mastery",
    "power_overwhelming",
    "swoop_in",
    "watch_how_i_soar",
)
WASP_EFFECTS = (
    "magnetic_dagger",
    "charged_boomerang",
    "shock",
    "electrocute",
    "telekinesis",
    "mass_telekinesis",
    "electroblast",
    "thunder_boomerang",
    "lift_up",
    "control_gravity",
    "center_of_mass",
    "kinetic_repulse",
    "kinetic_blast",
    "static_barrier",
)
ALL_EFFECTS = {**EFFECTS, **MISA_STATEFUL_EFFECTS}
DENSE_EFFECT_IDS = (
    "bullrush",
    "furious_charge",
    "throwing_axe",
    "throwing_spear",
    "shield",
    "bolster",
    "fortify",
    "war_drummer",
    "master_skald",
    "bulwark",
    "one_man_army",
    "backstab",
    "backstab_with_a_ballista",
    "combat_reflexes",
    "light_fingered",
    "pick_pocket",
    "master_thief",
    "blend_into_shadows",
    "mind_grip",
    "spell_break",
    "ebb_and_flow",
    "dangerous_current",
    "raging_stream",
    "violent_torrent",
    "slippery_ground",
    "deluge",
    "noble_blade",
    "arcane_whirlpool",
    "liquid_leap",
    "magical_current",
    "stranger_tide",
    "onslaught",
    "hit_and_run",
    "leaping_strike",
    "poisoned_dagger",
    "poisoned_dart",
    "rogue_wave",
    "tidal_blast",
    "blink_strike",
    "mad_dash",
    "brutal_jab",
    "mighty_punch",
    "savage_kick",
    "point_blank_shot",
    "back_to_back",
    "listen_up",
    "roger_roger",
    "ready_and_waiting",
    "troop_movement",
    "marching_orders",
    "path_to_victory",
    "close_support",
    "covering_fire",
    "shootout",
    "bullet_hell",
    "quickdraw",
    "gunslinger",
    "dead_shot",
    "shield_bash",
    "counterattack",
    "steady_advance",
    "unwavering_resolve",
    "devoted_followers",
    "fresh_converts",
)
DENSE_EFFECTS = {effect: EFFECTS[effect] for effect in DENSE_EFFECT_IDS}


def build_catalog():
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    state = setup()
    recipes = {}
    ranged_steps = {}
    radius_fallbacks = {}
    from primary_radius import bind_radius_fallbacks

    for effect_id, name in EFFECTS.items():
        hero = HeroRegistry.get(name).model_copy(update={"id": HERO})
        card = next(
            c
            for c in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
            if c.effect_id == effect_id
        ).model_copy(update={"id": CARD, "is_facedown": False})
        stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
        steps = CardEffectRegistry.get(effect_id).get_steps_with_stats(state, hero, card, stats)
        wires = [step.model_dump(mode="json") for step in steps]

        def labels(wire):
            wire["step_id"] = "native-step-primary-template"
            for child in wire.get("steps_template", []):
                labels(child)

        for wire in wires:
            labels(wire)
        variants = []
        for ranged in (False, True):
            changed = card.model_copy(update={"is_ranged": ranged})
            changed_steps = CardEffectRegistry.get(effect_id).get_steps_with_stats(
                state, hero, changed, stats
            )
            variant = [step.model_dump(mode="json") for step in changed_steps]
            for wire in variant:
                labels(wire)
            variants.append(variant)
        assert len(variants[0]) == len(
            variants[1]
        ), "stateful ranged recipe shape needs an explicit native port"
        bindings = []
        for i, (a, b) in enumerate(zip(*variants, strict=True)):
            if a != b:
                assert (
                    a["type"] == b["type"] == "attack_sequence" and a["is_ranged"] != b["is_ranged"]
                )
                assert {**a, "is_ranged": False} == {
                    **b,
                    "is_ranged": False,
                }, "unsupported ranged-dependent recipe"
                bindings.append(i)
        ranged_steps[effect_id] = bindings
        variants = []
        for radius in (None, 0):
            other = CardEffectRegistry.get(effect_id).get_steps_with_stats(
                state, hero, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=radius)
            )
            variant = [step.model_dump(mode="json") for step in other]
            for wire in variant:
                labels(wire)
            variants.append(variant)
        recipes[effect_id] = bind_radius_fallbacks(
            wires, variants[0], variants[1], RADIUS, radius_fallbacks
        )
    source = {
        "schema_version": 2,
        "source_sha256": verify_sources()["source_sha256"],
        "operands": {
            "hero": HERO,
            "card": CARD,
            "primary": PRIMARY,
            "range": RANGE,
            "radius": RADIUS,
        },
        "recipes": recipes,
        "ranged_steps": ranged_steps,
        "radius_fallbacks": radius_fallbacks,
    }
    payload = {**source, "catalog_sha256": digest(source)}
    return payload


def export():
    payload = build_catalog()
    recipes = payload["recipes"]
    (ROOT / "rust/tests/fixtures/primary-effects.json").write_text(
        json.dumps(payload, indent=2) + "\n"
    )
    print(
        {
            "effects": len(recipes),
            "steps": sum(map(len, recipes.values())),
            "catalog_sha256": payload["catalog_sha256"],
        }
    )


if __name__ == "__main__":
    export()
