"""Explicit Perfect Self variants; retired-card IDs are bound at execution time."""

from primary_builders import CARD, OWNER, freeze_steps
from primary_templates import PRIMARY, RADIUS, RANGE

RETIRED_CARD = "template-retired-tier-two"


def stateful_recipes(state):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import CardState, CardTier
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    variants = {}
    for enabled, name in ((False, "without_retired_tier_two"), (True, "with_retired_tier_two")):
        hero = HeroRegistry.get("Min").model_copy(deep=True, update={"id": OWNER})
        for card in hero.deck:
            if card.tier == CardTier.II:
                card.state = CardState.DECK
        retired = next(c for c in hero.deck if c.tier == CardTier.II)
        retired.id = RETIRED_CARD
        if enabled:
            retired.state = CardState.RETIRED
        card = next(c for c in hero.deck if c.effect_id == "perfect_self").model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get("perfect_self")
        stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
        rows = freeze_steps(impl.get_steps_with_stats(state, hero, card, stats))
        assert len(rows) == (6 if enabled else 1)
        if enabled:
            assert rows[4]["fields"]["allowed_card_ids"] == [RETIRED_CARD]
        for radius in (None, 0):
            assert (
                freeze_steps(
                    impl.get_steps_with_stats(
                        state,
                        hero,
                        card,
                        CardStats(primary_value=PRIMARY, range=RANGE, radius=radius),
                    )
                )
                == rows
            )
        variants[name] = rows
    return {
        "perfect_self": {
            "predicate": "min_has_retired_tier_two",
            "retired_card_operand": RETIRED_CARD,
            "variants": variants,
        }
    }
