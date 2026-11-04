"""Freeze Whisper's primary builders for each supported state variant."""

from unittest.mock import patch

from primary_builders import CARD, OWNER, freeze_steps
from primary_templates import PRIMARY, RANGE, RADIUS, WHISPER_TRAIL_EFFECTS, WHISPER_ULTIMATE_EFFECTS


def stateful_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    result = {}
    hero = HeroRegistry.get("Whisper").model_copy(update={"id": OWNER})
    for effect, maximum in WHISPER_TRAIL_EFFECTS.items():
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        implementation = CardEffectRegistry.get(effect)
        build = implementation.build_steps if raw else implementation.get_steps_with_stats
        variants = {}
        for distance in range(maximum + 1):
            with patch.object(type(implementation), "_compute_move_distance", return_value=distance):
                variants[str(distance)] = freeze_steps(
                    build(state, hero, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS))
                )
        result[effect] = {"predicate": "whisper_trail_spawn_count", "variants": variants}
    for effect in WHISPER_ULTIMATE_EFFECTS:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        implementation = CardEffectRegistry.get(effect)
        build = implementation.build_steps if raw else implementation.get_steps_with_stats
        variants = {}
        for name, level in (("without_ultimate", 7), ("with_ultimate", 8)):
            performer = hero.model_copy(update={"level": level})
            assert performer.ultimate_card is not None
            variants[name] = freeze_steps(
                build(state, performer, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS))
            )
        result[effect] = {"predicate": "has_level_eight_ultimate", "variants": variants}
    return result
