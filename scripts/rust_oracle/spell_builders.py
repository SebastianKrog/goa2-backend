"""Freeze reviewed spell constructors in their own effect-ID namespace.

Five state-dependent builders use explicitly named prototypes. Rust selects and
patches those prototypes from live owner/caster/state operands; this export does
not infer a Python builder's purity from a successful call.
"""

from __future__ import annotations

from fixture_cache import write_text

import json

from oracle import ROOT, digest, verify_sources
from primary_builders import CARD, OWNER
from primary_templates import PRIMARY, RADIUS, RANGE

EFFECTS = (
    "shocking_grasp",
    "magic_missile",
    "expeditious_retreat",
    "burning_hands",
    "suggestion",
    "shield",
    "vampiric_touch",
    "fireball",
    "create_undead",
    "midas_touch",
    "disintegrate",
    "dominate_person",
    "find_familiar",
    "dimension_door",
    "banishment",
    "sunburst",
    "energy_drain",
    "cloud_kill",
    "invulnerability",
    "power_word_kill",
    "polymorph",
    "wish",
)


def catalog():
    from model_traces import setup
    from shared_objects import capture

    from goa2.domain.models import CardState
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    state = setup(["Gydion", "Hanu"], ["Arien", "Tigerclaw"])
    original = state.get_hero("hero_gydion")
    hero = original.model_copy(update={"id": OWNER})
    for spell in original.spells:
        spell.state = CardState.SPELLBOOK
    assert set(EFFECTS) == set(CardEffectRegistry._spell_effects)
    recipes = {}

    def freeze(steps):
        wires = [step.model_dump(mode="json") for step in steps]
        bindings, _ = capture(steps, wires)
        assert all(len({p[0] for p in b["paths"] if p}) <= 1 for b in bindings)

        def labels(v):
            if isinstance(v, dict):
                return {k: "template" if k == "step_id" else labels(x) for k, x in v.items()}
            if isinstance(v, list):
                return [labels(x) for x in v]
            return v

        rows = []
        for step, wire in zip(steps, wires, strict=True):
            bindings, _ = capture(step, wire)
            rows.append({"fields": labels(wire), "objects": bindings})
        return rows

    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    for effect in EFFECTS:
        card = next(s for s in original.spells if s.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        if effect == "midas_touch":
            for spell in original.spells[:2]:
                spell.state = CardState.OUTSIDE_SPELLBOOK
        impl = CardEffectRegistry.get_for_card(card)
        recipes[effect] = freeze(impl.get_steps_with_stats(state, hero, card, stats))
        for spell in original.spells:
            spell.state = CardState.SPELLBOOK
        if effect == "wish":
            state.execution_context["wish_cast_counted_caster_id"] = OWNER
            recipes["wish_counted"] = freeze(impl.get_steps_with_stats(state, hero, card, stats))
            state.execution_context.clear()
    source = dict(
        schema_version=1,
        source_sha256=verify_sources()["source_sha256"],
        effects=list(EFFECTS),
        recipes=recipes,
        operands=dict(owner=OWNER, card=CARD, primary=PRIMARY, range=RANGE, radius=RADIUS),
    )
    return {**source, "catalog_sha256": digest(source)}


def export():
    data = catalog()
    write_text(ROOT / "rust/tests/fixtures/spell-builders.json", json.dumps(data, indent=2) + "\n")
    print(
        dict(
            effects=len(data["effects"]),
            prototypes=len(data["recipes"]),
            catalog_sha256=data["catalog_sha256"],
        ),
        flush=True,
    )


if __name__ == "__main__":
    export()
