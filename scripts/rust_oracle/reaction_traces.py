"""Real-handler defense windows and discard orchestration, with registered cards."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import ActionType, CardContainerType, CardState, GamePhase, TeamColor
    from goa2.domain.models.enums import StatType
    from goa2.domain.models.marker import Marker, MarkerType
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        DiscardCardStep,
        ReactionWindowStep,
        ResolveCombatStep,
        ResolveDefenseTextStep,
        ResolveOnBlockEffectStep,
        RestoreActionTypeStep,
    )

    result = []
    setup()

    def build(name="Arien"):
        others = [n for n in ["Arien", "Hanu", "Brogan", "Tigerclaw"] if n != name]
        state = setup([name, others[0]], others[1:3])
        state.phase = GamePhase.RESOLUTION
        hero = state.teams[TeamColor.RED].heroes[0]
        state.current_actor_id = state.teams[TeamColor.BLUE].heroes[0].id
        state.execution_context = {
            "target_id": hero.id,
            "attack_damage": 4,
            "current_action_type": "ATTACK",
            "auto_block": True,
            "defense_invalid": True,
            "defense_bonus": 99,
            "ignore_minion_defense": True,
            "snorri_ult_rune_defense": True,
        }
        return state, hero

    def add(name, state, steps, ops):
        for i, step in enumerate(steps):
            step.step_id = f"fixture/reaction/{name}/{i}"
        push_steps(state, steps)
        result.append(
            {
                "name": f"reaction-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

    for name in sorted(HeroRegistry._heroes):
        state, hero = build(name)
        valid = [
            c
            for c in hero.hand
            if c.current_primary_action in (ActionType.DEFENSE, ActionType.DEFENSE_SKILL)
            or ActionType.DEFENSE in c.current_secondary_actions
        ]
        add(
            f"{name}-prompt-invalid-pass",
            state,
            [ReactionWindowStep()],
            [advance, answer("not_a_card"), advance, answer("PASS"), advance],
        )
        for card in valid:
            state, hero = build(name)
            add(
                f"{name}-{card.id}",
                state,
                [ReactionWindowStep()],
                [advance, answer(str(card.id)), advance],
            )

    for variant in [
        "missing",
        "nonhero",
        "offboard",
        "initiative",
        "blocked-primary",
        "no-hand",
        "facedown",
        "items-markers",
        "false",
        "null",
        "number",
        "restored",
        "pending",
        "secondary-combat",
        "pass-combat",
    ]:
        state, hero = build()
        step = ReactionWindowStep()
        ops = [advance, answer("PASS"), advance]
        steps = [step]
        if variant == "missing":
            state.execution_context.pop("target_id")
            ops = [advance]
        elif variant == "nonhero":
            state.execution_context["target_id"] = state.teams[TeamColor.BLUE].minions[0].id
            ops = [advance]
        elif variant == "offboard":
            state.remove_unit(hero.id)
        elif variant == "initiative":
            state.execution_context["defense_uses_initiative"] = True
        elif variant == "blocked-primary":
            state.execution_context["block_primary_defense"] = True
        elif variant == "no-hand":
            hero.hand = []
        elif variant == "facedown":
            for card in hero.hand:
                card.is_facedown = True
        elif variant == "items-markers":
            hero.items[StatType.DEFENSE] = 2
            state.markers[MarkerType.VENOM] = Marker(
                type=MarkerType.VENOM, target_id=hero.id, value=-1, source_id="other"
            )
        elif variant in ("false", "null", "number"):
            value = {"false": False, "null": None, "number": 17}[variant]
            ops = [advance, answer(value), advance, answer("PASS"), advance]
        elif variant == "restored":
            step.pending_request_id = "fixture/reaction/request"
            ops = [answer("PASS", "fixture/reaction/request"), advance]
        elif variant == "pending":
            step.pending_input = {"selection": "PASS"}
            ops = [advance]
        elif variant in ("secondary-combat", "pass-combat"):
            state.execution_context["attack_damage"] = 2
            card = next(
                c
                for c in hero.hand
                if c.current_primary_action not in (ActionType.DEFENSE, ActionType.DEFENSE_SKILL)
                and ActionType.DEFENSE in c.current_secondary_actions
            )
            selected = str(card.id) if variant == "secondary-combat" else "PASS"
            steps = [
                step,
                ResolveDefenseTextStep(),
                ResolveCombatStep(damage=2, target_key="target_id"),
                ResolveOnBlockEffectStep(),
                RestoreActionTypeStep(),
            ]
            ops = [advance, answer(selected), advance]
        add(variant, state, steps, ops)

    for variant in [
        "hand",
        "played",
        "current",
        "fallback-hand",
        "fallback-played",
        "missing",
        "unknown-hero",
        "keys",
        "piece-owner",
        "repeat",
    ]:
        state, hero = build("Razzle" if variant == "piece-owner" else "Arien")
        card = hero.hand[0]
        step = DiscardCardStep(hero_id=hero.id, card_id=card.id)
        if variant in ("played", "current", "fallback-played"):
            hero.hand.remove(card)
            if variant == "current":
                hero.current_turn_card = card
                card.state = CardState.UNRESOLVED
                card.is_facedown = True
            else:
                hero.played_cards = [card]
                card.state = CardState.RESOLVED
        if variant == "played" or variant == "fallback-hand":
            step.source = CardContainerType.PLAYED
        elif variant == "missing":
            step.card_id = "missing"
        elif variant == "unknown-hero":
            step.hero_id = "unknown"
        elif variant == "keys":
            step.hero_id = None
            step.card_id = None
            step.hero_key = "who"
            step.card_key = "which"
            state.execution_context.update(who=hero.id, which=card.id)
        elif variant == "piece-owner":
            step.hero_id = state.get_piece_ids(hero.id)[0]
        steps = [step]
        if variant == "repeat":
            steps.append(DiscardCardStep(hero_id=hero.id, card_id=card.id))
        add(f"discard-{variant}", state, steps, [advance])
    return result
