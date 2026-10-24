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
    "devoted_followers": "Xargatha",
    "fresh_converts": "Xargatha",
}


def build_catalog():
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    state = setup()
    recipes = {}
    ranged_steps = {}
    for effect_id, name in EFFECTS.items():
        hero = HeroRegistry.get(name).model_copy(update={"id": HERO})
        card = next(c for c in hero.deck if c.effect_id == effect_id).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
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
        recipes[effect_id] = wires
    source = {
        "schema_version": 1,
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
