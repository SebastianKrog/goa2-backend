"""Freeze Takahide's primary builders with and without gold replacements."""

from primary_builders import CARD, OWNER, freeze_steps
from primary_templates import PRIMARY, RANGE, RADIUS, TAKAHIDE_GOLD_EFFECTS


def stateful_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import CardColor, CardState
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    result = {}
    for effect in TAKAHIDE_GOLD_EFFECTS:
        variants = {}
        for has_replacement in (False, True):
            hero = HeroRegistry.get("Takahide").model_copy(deep=True, update={"id": OWNER})
            golds = [card for card in hero.deck if card.color == CardColor.GOLD]
            for gold in golds:
                gold.state = CardState.DECK if has_replacement else CardState.HAND
            card = next(card for card in hero.deck if card.effect_id == effect)
            if effect == "bushido":
                golds[0].state = CardState.HAND
                golds[0].id = CARD
            card.state = CardState.UNRESOLVED
            card = card.model_copy(update={"id": CARD, "is_facedown": False})
            implementation = CardEffectRegistry.get(effect)
            build = implementation.build_steps if raw else implementation.get_steps_with_stats
            variants["with_replacement" if has_replacement else "without_replacement"] = freeze_steps(
                build(state, hero, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS))
            )
        result[effect] = {"predicate": "takahide_deck_gold", "variants": variants}
    return result
