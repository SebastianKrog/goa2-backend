"""Frozen dynamic passive builders retain raw/bound source API differences."""

from __future__ import annotations


def scenes(names=None, *, effects=None):
    from live_passive_traces import build, retain
    from primary_templates import ALL_EFFECTS

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import CardState
    from goa2.engine.steps import OfferPassiveStep

    effects = effects or (
        "crane_stance",
        "dragon_stance",
        "war_drummer",
        "inner_strength",
        "death_grenade",
        "magnetic_dagger",
        "thunder_boomerang",
        "electrocute",
        "slippery_ground",
        "master_thief",
        "swoop_in",
        "watch_how_i_soar",
        "perfect_self",
        "cantrip",
        "trip_mine",
        "fast_as_lightning",
    )
    for passive, owner, trigger in (
        ("flurry_of_blows", "Min", "after_attack"),
        ("cloak_and_daggers", "Tigerclaw", "after_basic_action"),
    ):
        for effect in effects:
            variants = ["ordinary", "facedown", "registry-override", "no-target"]
            if effect in {"crane_stance", "inner_strength", "slippery_ground", "electrocute"}:
                variants += ["complete", "complete-skip", "skill", "negative-attack"]
                if passive == "flurry_of_blows":
                    variants += ["played", "discard", "victim-fallback", "already-repeat"]
            for variant in variants:
                name = f"dynamic-passive-{passive}-{effect}-{variant}"
                if names is not None and name not in names:
                    continue
                state, hero = build(owner)
                state.battle_zones = {}  # The fixture uses an ad-hoc board without lanes.
                hero.level = 8
                ability = hero.ultimate_card
                ability.state, ability.is_facedown = CardState.RESOLVED, False
                definition = HeroRegistry.get(ALL_EFFECTS[effect])
                source = next(
                    c
                    for c in [*definition.deck, definition.ultimate_card]
                    if c is not None and c.effect_id == effect
                ).model_copy(deep=True)
                source.id = "repeat-source"
                source.state = CardState.RESOLVED
                source.is_facedown = variant == "facedown"
                if variant == "registry-override":
                    source.effect_id = "unknown-physical-effect"
                if variant == "negative-attack":
                    from goa2.domain.models import StatType

                    hero.items[StatType.ATTACK] = -20
                hero.current_turn_card = source
                if variant in {"played", "discard"}:
                    hero.current_turn_card = None
                    if variant == "played":
                        hero.played_cards = [None, source]
                    else:
                        hero.discard_pile = [source]
                context = state.execution_context
                enemy = next(t.heroes[0] for t in state.teams.values() if t.color != hero.team)
                context.update(current_card_id="outer-action", sentinel=[source, ability])
                if passive == "flurry_of_blows":
                    context.update(attack_card_id=source.id, attack_effect_id=effect)
                    if variant != "no-target":
                        context["defender_id"] = str(enemy.id)
                    if variant == "victim-fallback":
                        context["defender_id"] = ""
                        context["victim_id"] = str(enemy.id)
                    if variant == "already-repeat":
                        context["is_flurry_repeat"] = True
                else:
                    context.update(
                        basic_action_type="ATTACK",
                        basic_action_value=3,
                        basic_action_effect_id=effect,
                        basic_action_range=2,
                    )
                    context["last_combat_target"] = str(enemy.id)
                    if variant == "skill":
                        context["basic_action_type"] = "SKILL"
                step = OfferPassiveStep(
                    step_id="fixture/dynamic-passive/offer",
                    card_id=ability.id,
                    trigger=trigger,
                    pending_input={"selection": "YES"},
                )
                operations = retain(state, step)
                if variant in {"complete", "complete-skip"}:
                    from object_execution_traces import live_choice_operations

                    from goa2.domain.hex import Hex

                    for index, team in enumerate(state.teams.values()):
                        for offset, unit in enumerate(team.heroes):
                            q, r = (offset, 1 - offset) if index else (-offset, 0)
                            state.place_entity(unit.id, Hex(q=q, r=r, s=-q - r))
                    operations = live_choice_operations(
                        state, stop_types=(), mode="skip" if variant == "complete-skip" else "first"
                    )
                    operations.append({"kind": "advance_prefix", "steps": 0})
                yield name, state, operations
