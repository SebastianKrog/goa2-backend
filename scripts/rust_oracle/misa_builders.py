"""Explicit source prototypes for Misa's two level/ultimate-dependent builders."""

from primary_builders import CARD, OWNER, freeze_steps
from primary_templates import MISA_STATEFUL_EFFECTS, PRIMARY, RADIUS, RANGE


def stateful_recipes(state):
    from primary_radius import bind_radius_fallbacks

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    result = {}
    expected = {"swoop_in": (4, 6), "watch_how_i_soar": (7, 15)}
    for effect in MISA_STATEFUL_EFFECTS:
        variants = {}
        for index, name in enumerate(("without_ultimate", "with_ultimate")):
            hero = HeroRegistry.get("Misa").model_copy(
                update={"id": OWNER, "level": 8 if index else 1}
            )
            card = next(c for c in hero.deck if c.effect_id == effect).model_copy(
                update={"id": CARD, "is_facedown": False}
            )
            impl = CardEffectRegistry.get(effect)

            def build(radius, impl=impl, hero=hero, card=card):
                return freeze_steps(
                    impl.get_steps_with_stats(
                        state,
                        hero,
                        card,
                        CardStats(primary_value=PRIMARY, range=RANGE, radius=radius),
                    )
                )

            rows = build(RADIUS)
            assert len(rows) == expected[effect][index]
            absent, zero = build(None), build(0)
            assert (
                [r["objects"] for r in rows]
                == [r["objects"] for r in absent]
                == [r["objects"] for r in zero]
            )
            fallbacks = {}
            fields = bind_radius_fallbacks(
                [r["fields"] for r in rows],
                [r["fields"] for r in absent],
                [r["fields"] for r in zero],
                RADIUS,
                fallbacks,
            )
            assert not fallbacks, "Misa builders use nullable radius directly"
            for row, bound_fields in zip(rows, fields, strict=True):
                row["fields"] = bound_fields
            variants[name] = rows
        result[effect] = {"predicate": "misa_has_ultimate", "variants": variants}
    return result
