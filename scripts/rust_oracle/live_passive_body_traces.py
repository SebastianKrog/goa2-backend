"""Accepted passive construction uses actual source bodies and registered cards."""

from __future__ import annotations


def scenes():
    from combat_effect_traces import effect
    from live_passive_traces import build, enable, retain
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import ActionType, CardState, RuneType, StatType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.steps import OfferPassiveStep

    setup()
    matrix = []
    for name, hero in sorted(HeroRegistry._heroes.items()):
        for card in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]:
            body = CardEffectRegistry.get(card.effect_id)
            if body:
                for config in body.get_passive_configs():
                    matrix.append((name, card.id, card.effect_id, config.trigger.value))
    for name, printed, effect_id, trigger in matrix:
        variants = ["accepted", "mandatory", "wrong-trigger", "owner-override", "provider"]
        if effect_id in {
            "what_the_hell_are_you",
            "reign_of_winter",
            "high_voltage",
            "march_of_nature",
        }:
            variants += ["radius-item", "negative-radius"]
        if effect_id == "rune_mastery":
            variants += ["many-runes", "no-runes", "duplicate-runes", "outside-turn"]
        if effect_id == "legend_of_the_skies":
            variants += ["no-previous"]
        if effect_id in {"a_complicated_profession", "big_sodding_gun"}:
            variants += ["missing-target", "minion-target", "unknown-target", "friendly-target"]
        if effect_id in {"battle_fury", "bullet_time"}:
            variants += ["missing-card-key"]
        if effect_id == "cloak_and_daggers":
            variants += [
                "attack",
                "movement",
                "skill-missing-effect",
                "negative-attack",
                "zero-value",
                "blocked-repeat",
                "unknown-effect",
            ]
        if effect_id == "flurry_of_blows":
            variants += ["already-repeat", "unknown-effect", "missing-source-card"]
        if effect_id == "jinx":
            variants += ["actor-stage"]
        if effect_id == "legend_of_the_skies":
            variants += ["alternate-previous"]
        if effect_id == "dragon_knight":
            variants += ["multiple-skills", "blocked-skill"]
        if effect_id == "a_complicated_profession":
            variants += ["complete"]
        if effect_id == "rune_mastery":
            variants += ["complete-runes"]
        for variant in variants:
            state, hero = build(name)
            card = next(
                c for c in [*hero.deck, hero.ultimate_card] if c is not None and c.id == printed
            )
            hero.level = 8
            card.state, card.is_facedown = CardState.RESOLVED, False
            if card is not hero.ultimate_card:
                hero.played_cards = [card]
            enable(state, hero, card, effect_id)
            enemy = next(team.heroes[0] for team in state.teams.values() if team.color != hero.team)
            target_key = "push_victim_id" if effect_id == "big_sodding_gun" else "marker_target_id"
            if effect_id in {"a_complicated_profession", "big_sodding_gun"}:
                state.execution_context[target_key] = str(enemy.id)
            if variant == "missing-target":
                state.execution_context.pop(target_key, None)
            if variant == "minion-target":
                state.execution_context[target_key] = str(
                    next(iter(state.teams.values())).minions[0].id
                )
            if variant == "unknown-target":
                state.execution_context[target_key] = "unknown"
            if variant == "friendly-target":
                state.execution_context[target_key] = str(hero.id)
            if variant == "radius-item":
                hero.items[StatType.RADIUS] = 3
            if variant == "negative-radius":
                card.radius_value = -2
            if variant in {"many-runes", "complete-runes"}:
                hero.rune_slots = {i + 1: rune for i, rune in enumerate(RuneType)}
            if variant == "no-runes":
                hero.rune_slots = {}
            if variant == "duplicate-runes":
                hero.rune_slots = {
                    1: RuneType.AXE,
                    2: RuneType.BIRD,
                    3: RuneType.BIRD,
                    4: RuneType.HORN,
                }
            if variant == "outside-turn":
                state.turn = 5
                hero.rune_slots = {1: RuneType.HORN, 2: RuneType.AXE}
            if variant == "no-previous":
                hero.played_cards = []
            if variant == "alternate-previous":
                hero.played_cards[1] = hero.hand[0]
            if variant == "multiple-skills":
                skills = [c for c in hero.deck if c.primary_action == ActionType.SKILL][:2]
                for skill in skills:
                    skill.is_facedown = False
                hero.played_cards = skills
            if variant == "blocked-skill":
                state.active_effects.append(
                    effect(
                        state,
                        "skill-block",
                        EffectType.TARGET_PREVENTION,
                        source=str(enemy.id),
                        restrictions=[ActionType.SKILL],
                    )
                )
            if variant == "missing-card-key":
                state.execution_context.pop(
                    "discarded_card_id" if effect_id == "battle_fury" else "basic_action_card_id",
                    None,
                )
            if (
                variant
                in {
                    "attack",
                    "movement",
                    "negative-attack",
                    "zero-value",
                    "blocked-repeat",
                    "unknown-effect",
                    "skill-missing-effect",
                }
                and effect_id == "cloak_and_daggers"
            ):
                state.execution_context.update(
                    basic_action_type=(
                        ActionType.MOVEMENT.value
                        if variant == "movement"
                        else (
                            ActionType.SKILL.value
                            if variant == "skill-missing-effect"
                            else ActionType.ATTACK.value
                        )
                    ),
                    basic_action_value=(
                        -3 if variant == "negative-attack" else 0 if variant == "zero-value" else 3
                    ),
                    basic_action_range=2,
                )
                if variant == "unknown-effect":
                    state.execution_context["basic_action_effect_id"] = "unknown"
                if variant == "blocked-repeat":
                    state.active_effects.append(
                        effect(
                            state,
                            "repeat-block",
                            EffectType.REPEAT_PREVENTION,
                            source=str(enemy.id),
                        )
                    )
            if effect_id == "flurry_of_blows":
                if variant == "already-repeat":
                    state.execution_context["is_flurry_repeat"] = True
                if variant == "unknown-effect":
                    state.execution_context.update(
                        attack_effect_id="unknown", attack_card_id=hero.hand[0].id
                    )
                if variant == "missing-source-card":
                    state.execution_context.update(
                        attack_effect_id=hero.hand[0].effect_id, attack_card_id="missing"
                    )
            step = OfferPassiveStep(
                step_id="fixture/passive-body/offer",
                card_id=printed,
                trigger="after_end_turn" if variant == "wrong-trigger" else trigger,
                prompt="Accepted body",
                pending_input={"selection": "YES"},
                is_optional=variant != "mandatory",
            )
            # Use a valid, unrelated enum value for direct offers; source bodies
            # may intentionally ignore trigger checks, so do not filter them.
            if variant == "wrong-trigger":
                step.trigger = "after_push"
            if variant in {"owner-override", "actor-stage"}:
                state.current_actor_id = str(enemy.id)
                step.hero_id = str(hero.id)
            if variant == "provider":
                carrier = enemy.hand[0]
                provider = effect(
                    state,
                    "passive-body-provider",
                    EffectType.DELAYED_TRIGGER,
                    source=str(hero.id),
                    source_card_id=carrier.id,
                    granted_passive_effect_id=effect_id,
                )
                state.active_effects.append(provider)
                step.card_id, step.active_effect_id = carrier.id, provider.id
            operations = retain(state, step)
            if variant in {"actor-stage", "alternate-previous"}:
                operations = [
                    {"kind": "advance_prefix", "steps": 1},
                    {"kind": "advance_prefix", "steps": 1},
                    {"kind": "advance_prefix", "steps": 0},
                ]
            if variant in {"complete", "complete-runes"}:
                from object_execution_traces import live_choice_operations

                operations = live_choice_operations(state, stop_types=())
                operations.append({"kind": "advance_prefix", "steps": 0})
            yield f"passive-body-{effect_id}-{trigger}-{variant}", state, operations
