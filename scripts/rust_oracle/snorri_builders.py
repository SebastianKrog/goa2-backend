"""Freeze source recipes for Snorri’s rune-dependent primary actions."""

from unittest.mock import patch

from primary_builders import CARD, OWNER, freeze_steps
from primary_templates import PRIMARY, RANGE, RADIUS, SNORRI_STATEFUL_EFFECTS


def stateful_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import RuneType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    result = {}
    for effect, name in SNORRI_STATEFUL_EFFECTS.items():
        hero = HeroRegistry.get(name).model_copy(update={"id": OWNER})
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        implementation = CardEffectRegistry.get(effect)
        build = implementation.build_steps if raw else implementation.get_steps_with_stats
        variants = {}
        runes = (RuneType.BIRD, RuneType.ANVIL, RuneType.HORN)
        if not effect.endswith("passage"):
            runes += (RuneType.AXE,)
        for mask in range(1 << len(runes)):
            active = {rune for index, rune in enumerate(runes)
                      if mask & (1 << index)}
            with patch("goa2.scripts.snorri_effects.active_runes", return_value=active):
                steps = build(state, hero, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS))
            rows = freeze_steps(steps)
            if effect == "rune_sigils" and RuneType.AXE in active:
                def bind_damage(value):
                    if isinstance(value, dict):
                        for key, child in value.items():
                            if key == "damage" and child == PRIMARY + 3:
                                value[key] = 1234568104
                            else:
                                bind_damage(child)
                    elif isinstance(value, list):
                        for child in value:
                            bind_damage(child)

                bind_damage(rows)
            if effect in ("runetrap", "runebomb", "ancestral_boon", "ancestral_grace"):
                for row in rows:
                    for filter_ in row["fields"].get("filters", []):
                        if filter_.get("max_range") == RADIUS:
                            # Source uses radius or zero for this required filter.
                            filter_["max_range"] = 1234568103
            if effect == "deep_passage" and RuneType.HORN in active:
                assert rows[0]["fields"]["range_val"] == PRIMARY + 2
                rows[0]["fields"]["range_val"] = 1234568102  # Native PRIMARY_PLUS_TWO operand.
            variants[str(mask)] = rows
        result[effect] = {
            "predicate": "snorri_active_runes",
            "variants": variants,
        }
    return result
