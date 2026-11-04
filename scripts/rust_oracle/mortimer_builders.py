"""Freeze Mortimer's primary builders at each Master of Puppets choice limit."""

from primary_builders import CARD, OWNER, freeze_steps
from primary_templates import MORTIMER_ULTIMATE_EFFECTS, PRIMARY, RANGE, RADIUS


def stateful_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("Mortimer").model_copy(update={"id": OWNER})
    result = {}
    for effect in MORTIMER_ULTIMATE_EFFECTS:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        implementation = CardEffectRegistry.get(effect)
        build = implementation.build_steps if raw else implementation.get_steps_with_stats
        variants = {}
        for name, level in (("without_ultimate", 7), ("with_ultimate", 8)):
            performer = hero.model_copy(update={"level": level})
            variants[name] = freeze_steps(
                build(state, performer, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS))
            )
        result[effect] = {
            "predicate": "has_level_eight_ultimate", "required_ultimate": "master_of_puppets",
            "variants": variants,
        }
    return result
