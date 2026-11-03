"""Freeze reviewed primary builders with physical nested-object bindings.

The list is deliberately reviewed rather than inferred from a successful call.
Pure recipes are separate from named stateful variants, whose selection requires
an explicit native runtime port.
"""

from __future__ import annotations

from fixture_cache import write_text

import json

from oracle import ROOT, digest, verify_sources
from primary_templates import EFFECTS, PRIMARY, RADIUS, RANGE

OWNER = "template-passive-owner"
CARD = "template-primary-card"


def freeze_steps(steps):
    from shared_objects import capture

    wires = [step.model_dump(mode="json") for step in steps]
    bindings, _ = capture(steps, wires)
    # Per-root recipe allocation must never erase cross-root identity.
    for binding in bindings:
        roots = {path[0] for path in binding["paths"] if path}
        assert len(roots) <= 1, "cross-root primary aliases require grouped allocation"

    def labels(value):
        if isinstance(value, dict):
            return {
                key: "template" if key == "step_id" else labels(child)
                for key, child in value.items()
            }
        if isinstance(value, list):
            return [labels(child) for child in value]
        return value

    rows = []
    for step, wire in zip(steps, wires, strict=True):
        bindings, _ = capture(step, wire)
        rows.append({"fields": labels(wire), "objects": bindings})
    return rows


def raw_binding_paths(bound, raw):
    """Record only differences observed between both real source APIs.

    Runtime can undo attribution without duplicating instructions or guessing
    which nested fields the source's generic binder visits. Physical model
    topology must be identical, including aliases, for this patch to be valid.
    """
    assert len(bound) == len(raw), "raw primary root shape changed"
    result = []
    for ordinary, unbound in zip(bound, raw, strict=True):
        assert ordinary["objects"] == unbound["objects"], "raw primary aliases changed"
        paths = []

        def compare(left, right, path, paths=paths):
            if left == right:
                return
            if isinstance(left, dict) and isinstance(right, dict):
                assert left.keys() == right.keys(), "raw primary fields changed"
                for key in left:
                    compare(left[key], right[key], [*path, key])
            elif isinstance(left, list) and isinstance(right, list):
                assert len(left) == len(right), "raw primary list shape changed"
                for index, (a, b) in enumerate(zip(left, right, strict=True)):
                    compare(a, b, [*path, index])
            else:
                assert path[-1] == "source_card_id" and left == CARD and right is None, (
                    "raw primary difference needs an explicit native port",
                    path,
                    left,
                    right,
                )
                paths.append(path)

        compare(ordinary["fields"], unbound["fields"], [])
        result.append(paths)
    return result


def catalog():
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffect, CardEffectRegistry
    from goa2.engine.stats import CardStats

    state = setup()
    recipes, ranged_steps, radius_fallbacks, raw_bindings = {}, {}, {}, {}
    from primary_radius import bind_radius_fallbacks

    methods = {
        name: type(effect).build_steps.__qualname__
        for name, effect in sorted(CardEffectRegistry._effects.items())
    }

    for effect, name in EFFECTS.items():
        hero = HeroRegistry.get(name).model_copy(update={"id": OWNER})
        card = next(
            card
            for card in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
            if card.effect_id == effect
        ).model_copy(update={"id": CARD, "is_facedown": False})
        stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
        impl = CardEffectRegistry.get(effect)
        recipes[effect] = freeze_steps(impl.get_steps_with_stats(state, hero, card, stats))
        raw_bindings[effect] = raw_binding_paths(
            recipes[effect], freeze_steps(impl.build_steps(state, hero, card, stats))
        )
        variants = []
        for radius in (None, 0):
            variant = freeze_steps(
                impl.get_steps_with_stats(
                    state, hero, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=radius)
                )
            )
            assert len(variant) == len(recipes[effect])
            for present, changed in zip(recipes[effect], variant, strict=True):
                assert present["objects"] == changed["objects"]
            variants.append([row["fields"] for row in variant])
        fields = bind_radius_fallbacks(
            [row["fields"] for row in recipes[effect]],
            variants[0],
            variants[1],
            RADIUS,
            radius_fallbacks,
        )
        for row, bound_fields in zip(recipes[effect], fields, strict=True):
            row["fields"] = bound_fields
        variants = []
        for ranged in (False, True):
            changed = card.model_copy(update={"is_ranged": ranged})
            variants.append(freeze_steps(impl.get_steps_with_stats(state, hero, changed, stats)))
        assert len(variants[0]) == len(variants[1])
        changed = []
        for index, (a, b) in enumerate(zip(*variants, strict=True)):
            if a != b:
                assert a["objects"] == b["objects"]
                assert a["fields"]["type"] == b["fields"]["type"] == "attack_sequence"
                assert {**a["fields"], "is_ranged": False} == {**b["fields"], "is_ranged": False}
                changed.append(index)
        ranged_steps[effect] = changed
    # Primitive fallback construction is frozen separately from registered empty bodies.
    from goa2.engine.steps import AttackSequenceStep, LogMessageStep, MoveSequenceStep

    fallback = {
        "movement": freeze_steps([MoveSequenceStep(unit_id=OWNER, range_val=PRIMARY)]),
        "attack": freeze_steps([AttackSequenceStep(damage=PRIMARY, range_val=RANGE)]),
        "defense": freeze_steps([LogMessageStep(message="template-primary-defense-log")]),
        "skill": freeze_steps([LogMessageStep(message="template-primary-skill-log")]),
    }
    from min_builders import stateful_recipes as min_stateful_recipes
    from misa_builders import stateful_recipes

    stateful = {**stateful_recipes(state), **min_stateful_recipes(state)}
    raw_stateful = {
        **stateful_recipes(state, raw=True),
        **min_stateful_recipes(state, raw=True),
    }
    for effect, body in stateful.items():
        body["raw_card_bindings"] = {
            name: raw_binding_paths(rows, raw_stateful[effect]["variants"][name])
            for name, rows in body["variants"].items()
        }
    source = {
        "schema_version": 3,
        "source_sha256": verify_sources()["source_sha256"],
        "methods": methods,
        "spell_effects": sorted(CardEffectRegistry._spell_effects),
        "recipes": recipes,
        "stateful_recipes": stateful,
        "raw_card_bindings": raw_bindings,
        "ranged_steps": ranged_steps,
        "radius_fallbacks": radius_fallbacks,
        "fallback": fallback,
        "operands": {
            "owner": OWNER,
            "card": CARD,
            "primary": PRIMARY,
            "range": RANGE,
            "radius": RADIUS,
        },
        "default_method": CardEffect.build_steps.__qualname__,
    }
    return {**source, "catalog_sha256": digest(source)}


def export():
    payload = catalog()
    write_text(ROOT / "rust/tests/fixtures/primary-builders.json", json.dumps(payload, indent=2) + "\n")
    print(
        {
            "effects": len(payload["recipes"]),
            "methods": len(payload["methods"]),
            "catalog_sha256": payload["catalog_sha256"],
        }
    )


if __name__ == "__main__":
    export()
