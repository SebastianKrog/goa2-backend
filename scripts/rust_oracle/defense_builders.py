"""Freeze registered defense/on-block recipes and source method inventory."""

from __future__ import annotations

import json
from unittest.mock import patch

from oracle import ROOT, digest, verify_sources

RANGE = 61003
RADIUS = 61007
BONUS = 61013
OWNER = "template-passive-owner"
CARD = "template-defense-card"


def catalog():
    from aura_traces import _board
    from model_traces import setup
    from shared_objects import capture

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import GamePhase, MarkerType, RuneType
    from goa2.engine.effects import CardEffect, CardEffectRegistry
    from goa2.engine.stats import CardStats
    from goa2.scripts import dodger_effects

    manifest = verify_sources()
    setup()
    methods = {
        name: {
            "defense": type(effect).build_defense_steps.__qualname__,
            "on_block": type(effect).build_on_block_steps.__qualname__,
            "primary": type(effect).build_steps.__qualname__,
        }
        for name, effect in sorted(CardEffectRegistry._effects.items())
    }
    recipes = {}
    runes = {}
    stats = CardStats(primary_value=4, range=RANGE, radius=RADIUS)

    def freeze(name, steps, owner, card):
        rows = []
        for step in steps:
            raw = step.model_dump(mode="json")
            bindings, _ = capture(step, raw)

            def bind(value):
                if isinstance(value, dict):
                    return {k: "template" if k == "step_id" else bind(v) for k, v in value.items()}
                if isinstance(value, list):
                    return [bind(v) for v in value]
                if value == owner:
                    return OWNER
                if value == card:
                    return CARD
                return value

            fields = bind(raw)
            if fields.get("type") == "add_context_value" and fields.get("key") == "defense_bonus":
                fields["amount"] = BONUS
            rows.append({"fields": fields, "objects": bindings})
        recipes[name] = rows

    for name, definition in sorted(HeroRegistry._heroes.items()):
        for original in [
            *definition.deck,
            *([definition.ultimate_card] if definition.ultimate_card else []),
        ]:
            effect = CardEffectRegistry.get(original.effect_id)
            if effect is None:
                continue
            custom = type(effect).build_defense_steps is not CardEffect.build_defense_steps
            onblock = type(effect).build_on_block_steps is not CardEffect.build_on_block_steps
            if not custom and not onblock and original.effect_id != "awaken":
                continue
            state = setup([name, "Swift"], ["Brogan", "Hanu"])
            h = _board(state)
            hero = state.get_hero("hero_" + name.lower())
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = "hero_brogan"
            state.place_entity(hero.id, h(0, 0))
            for index, piece in enumerate(state.get_piece_ids(str(hero.id))):
                if piece != hero.id:
                    state.place_entity(piece, h(index + 1, 0))
            if hero.id == "hero_razzle":
                for index in range(1, hero.piece_supply + 1):
                    state.place_entity(f"hero_razzle_piece_{index}", h(index, 0))
            card = next(
                card
                for card in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
                if card.id == original.id
            )
            context = {
                "defender_id": str(hero.id),
                "attack_is_ranged": True,
                "attack_is_basic": True,
            }
            if str(hero.id) == "hero_razzle":
                context["defender_id"] = state.get_piece_ids(str(hero.id))[0]
            state.execution_context = context

            def defense(effect=effect, state=state, hero=hero, card=card, context=context):
                with (patch("goa2.engine.stats.compute_card_stats", return_value=stats),):
                    return effect.get_defense_steps(state, hero, card, context)

            if original.effect_id.startswith("oath_of_"):
                runes[original.effect_id] = {
                    "blocks": {rune.value: value for rune, value in effect.rune_blocks.items()},
                    "choose_one": effect.choose_one,
                }
                hero.rune_slots = {}
                freeze(original.effect_id + "/defense/none", defense(), str(hero.id), str(card.id))
                hero.rune_slots = {state.turn: RuneType.AXE}
                context["attack_is_ranged"] = False
                freeze(original.effect_id + "/defense/match", defense(), str(hero.id), str(card.id))
                if effect.choose_one:
                    context["snorri_ult_rune_defense"] = RuneType.HORN
                    freeze(
                        original.effect_id + "/defense/choose",
                        defense(),
                        str(hero.id),
                        str(card.id),
                    )
            elif custom:
                method = type(effect).build_defense_steps.__qualname__
                if (
                    "DuelistEffect" in method
                    or method.startswith("UnstableTimeline")
                    or method.startswith(
                        ("PoofEffect", "VanishEffect", "RuseEffect", "CrowdControlEffect")
                    )
                ):
                    freeze(
                        original.effect_id + "/defense/default",
                        defense(),
                        str(hero.id),
                        str(card.id),
                    )
                    if method.startswith(("PoofEffect", "VanishEffect", "RuseEffect")):
                        state.remove_entity(hero.id)
                        freeze(
                            original.effect_id + "/defense/no-origin",
                            defense(),
                            str(hero.id),
                            str(card.id),
                        )
                elif method.startswith(
                    ("PerfectGetawayEffect", "CloseCallEffect", "NarrowEscapeEffect")
                ):
                    for enabled in (False, True):
                        marker = state.get_marker(MarkerType.BOUNTY)
                        marker.target_id = str(hero.id) if enabled else None
                        freeze(
                            original.effect_id + "/defense/" + ("yes" if enabled else "no"),
                            defense(),
                            str(hero.id),
                            str(card.id),
                        )
                elif method.startswith("_SpawnPointDefenseEffect"):
                    for enabled in (False, True):
                        with patch.object(
                            dodger_effects,
                            "_count_empty_spawn_points",
                            return_value=2 if enabled else 0,
                        ):
                            freeze(
                                original.effect_id + "/defense/" + ("yes" if enabled else "no"),
                                defense(),
                                str(hero.id),
                                str(card.id),
                            )
                else:
                    for ranged in (False, True):
                        context["attack_is_ranged"] = ranged
                        freeze(
                            original.effect_id + "/defense/" + ("ranged" if ranged else "melee"),
                            defense(),
                            str(hero.id),
                            str(card.id),
                        )
            if onblock:
                with patch("goa2.engine.stats.compute_card_stats", return_value=stats):
                    freeze(
                        original.effect_id + "/on_block/default",
                        effect.get_on_block_steps(state, hero, card, context),
                        str(hero.id),
                        str(card.id),
                    )
            if original.effect_id == "awaken":
                with patch("goa2.engine.stats.compute_card_stats", return_value=stats):
                    freeze(
                        "awaken/primary/default",
                        effect.get_steps(state, hero, card),
                        str(hero.id),
                        str(card.id),
                    )
    data = {
        "schema_version": 1,
        "source_sha256": manifest["source_sha256"],
        "methods": methods,
        "runes": runes,
        "recipes": recipes,
        "bindings": {
            "owner": OWNER,
            "card": CARD,
            "range": RANGE,
            "radius": RADIUS,
            "bonus": BONUS,
        },
    }
    data["catalog_sha256"] = digest(data)
    return data


if __name__ == "__main__":
    data = catalog()
    path = ROOT / "rust/tests/fixtures/defense-builders.json"
    path.write_text(json.dumps(data, indent=2) + "\n")
    print(
        {
            "effects": len(data["methods"]),
            "recipes": len(data["recipes"]),
            "catalog_sha256": data["catalog_sha256"],
        }
    )
