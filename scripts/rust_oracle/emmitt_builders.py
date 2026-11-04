"""Freeze one discard-retrieval group for Emmitt's live primary builders."""

from primary_builders import CARD, OWNER, freeze_steps
from primary_templates import EMMITT_RETRIEVAL_EFFECTS, PRIMARY, RANGE, RADIUS

DISCARD = "template-time-capsule-discard"


def stateful_recipes(*, raw=False):
    from model_traces import setup
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
    hero = state.get_hero("hero_emmitt")
    hero.id = OWNER
    result = {}
    for effect in EMMITT_RETRIEVAL_EFFECTS:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        implementation = CardEffectRegistry.get(effect)
        build = implementation.build_steps if raw else implementation.get_steps_with_stats
        variants = {}
        for name, log in (("empty", {}), ("one", {OWNER: [DISCARD]})):
            state.turn_discard_log = log
            variants[name] = freeze_steps(
                build(state, hero, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS))
            )
        base = variants["empty"]
        one = variants["one"]
        start = 3 if effect == "future_proof" else 0
        assert one[:start] + one[start + 5:] == base
        result[effect] = {
            "predicate": "emmitt_turn_discards", "insert_at": start, "variants": variants,
        }
    return result
