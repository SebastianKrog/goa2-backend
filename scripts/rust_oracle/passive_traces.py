"""Frozen passive provider/offer/use traces plus real Sabina push chains."""

from __future__ import annotations


def cases(advance, answer):
    from aura_traces import _board, _pieces
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.input import InputResponse
    from goa2.domain.models import ActionType, CardState, GamePhase, TeamColor
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import (
        CheckPassiveAbilitiesStep,
        MarkPassiveUsedStep,
        OfferPassiveStep,
        PushUnitStep,
        ResolveCardStep,
    )

    result = []
    variants = [
        "enemy",
        "empty",
        "friendly",
        "minion",
        "missing",
        "falsy",
        "offboard",
        "unknown",
        "piece",
        "owner",
        "level-seven",
        "facedown",
        "regular",
        "unresolved",
        "alias",
        "spent",
        "override",
        "wrong-trigger",
        "offer-yes",
        "offer-no",
        "offer-skip",
        "offer-low-level",
        "offer-missing-card",
        "offer-facedown",
        "offer-no-owner",
        "offer-wrong-trigger",
        "mark",
        "mark-facedown",
        "mark-missing",
        "mark-override-actor",
        "push",
        "push-empty",
        "push-twice",
        "push-blocked",
        "primary",
    ]
    for variant in variants:
        state = setup(["Sabina", "Arien"], ["Razzle", "Brogan"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_sabina")
        hero.level = 8
        ultimate = hero.ultimate_card
        enemy = state.get_hero("hero_brogan")
        state.current_actor_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(1, 0))
        victim = str(enemy.id)
        if variant in ["empty", "push-empty"]:
            enemy.discard_pile.extend(enemy.hand)
            enemy.hand = []
        if variant == "friendly":
            victim = "hero_arien"
            state.place_entity(victim, h(1, 1))
        if variant == "minion":
            victim = str(state.teams[TeamColor.BLUE].minions[0].id)
        if variant in ["piece", "owner"]:
            owner = state.get_hero("hero_razzle")
            piece = _pieces(state, owner.id)[0]
            state.place_entity(piece, h(1, 1))
            victim = piece if variant == "piece" else str(owner.id)
        if variant == "unknown":
            victim = "missing"
        state.execution_context = {"push_victim_id": victim}
        if variant == "missing":
            state.execution_context.clear()
        if variant == "falsy":
            state.execution_context["push_victim_id"] = []
        if variant == "offboard":
            state.remove_entity(enemy.id)
        if variant in ["level-seven", "offer-low-level"]:
            hero.level = 7
        if variant in ["facedown", "offer-facedown", "mark-facedown"]:
            ultimate.is_facedown = True
        if variant in ["regular", "unresolved", "alias"]:
            hero.played_cards = [ultimate]
            ultimate.state = CardState.UNRESOLVED if variant == "unresolved" else CardState.RESOLVED
            if variant != "alias":
                hero.level = 1
        if variant == "spent":
            ultimate.passive_uses_this_turn = 5
        if variant in ["override", "mark-override-actor"]:
            state.current_actor_id = "hero_arien"
        if variant == "offer-no-owner":
            state.current_actor_id = None
        trigger = (
            "before_action" if variant in ["wrong-trigger", "offer-wrong-trigger"] else "after_push"
        )
        if variant.startswith("offer-"):
            steps = [
                OfferPassiveStep(
                    card_id="missing" if variant == "offer-missing-card" else str(ultimate.id),
                    trigger=trigger,
                    is_optional=True,
                    prompt="Test passive?",
                )
            ]
        elif variant.startswith("mark"):
            steps = [
                MarkPassiveUsedStep(
                    card_id="missing" if variant == "mark-missing" else str(ultimate.id)
                )
            ]
        elif variant.startswith("push"):
            state.execution_context.clear()
            if variant == "push-blocked":
                state.place_entity("hero_arien", h(2, 0))
            steps = [PushUnitStep(target_id=str(enemy.id), distance=1)]
            if variant == "push-twice":
                steps.append(PushUnitStep(target_id=str(enemy.id), distance=1))
        elif variant == "primary":
            current = next(c for c in hero.hand if c.effect_id == "point_blank_shot")
            hero.play_card(current)
            current.is_facedown = False
            steps = [ResolveCardStep(hero_id=hero.id)]
        else:
            steps = [
                CheckPassiveAbilitiesStep(
                    trigger=trigger, hero_id=str(hero.id) if variant == "override" else None
                )
            ]
        for i, step in enumerate(steps):
            step.step_id = f"fixture/passive/{variant}/{i}"
        push_steps(state, steps)
        envelope = export_state(state)
        probe = state.model_copy(deep=True)
        ops = [{"kind": "advance_prefix", "steps": 1}]
        for _ in range(60):
            ops.append(advance)
            req = process_stack(probe).input_request
            if req is None:
                break
            ids = [o.id for o in req.options]
            if req.request_type.value == "CONFIRM_PASSIVE":
                value = (
                    "SKIP" if variant == "offer-skip" else "NO" if variant == "offer-no" else "YES"
                )
            elif req.request_type.value == "CHOOSE_ACTION":
                value = "ATTACK"
            elif req.request_type.value == "SELECT_CARD_OR_PASS":
                defender = probe.get_hero(req.player_id)
                value = str(
                    max(
                        defender.hand,
                        key=lambda c: c.secondary_actions.get(ActionType.DEFENSE, -99),
                    ).id
                )
            elif str(enemy.id) in ids:
                value = str(enemy.id)
            elif "CONFIRM" in ids:
                value = "CONFIRM"
            else:
                value = req.options[0].metadata.get("hex", req.options[0].id)
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=req.id, selection=value))
        else:
            raise AssertionError(variant)
        uses = probe.get_hero(hero.id).ultimate_card.passive_uses_this_turn
        expected = (
            2
            if variant in ["alias", "push-twice"]
            else (
                6
                if variant == "spent"
                else (
                    1
                    if variant
                    in [
                        "enemy",
                        "empty",
                        "offboard",
                        "piece",
                        "owner",
                        "regular",
                        "offer-yes",
                        "offer-low-level",
                        "mark",
                        "mark-facedown",
                        "push",
                        "push-empty",
                        "push-blocked",
                        "primary",
                    ]
                    else 0
                )
            )
        )
        assert uses == expected, (variant, uses, expected)
        if variant == "push-empty":
            assert probe.get_position(enemy.id) is None
        if variant in ["push", "push-twice", "primary"]:
            discarded = 2 if variant in ["push-twice", "primary"] else 1
            assert len(probe.get_hero(enemy.id).hand) == len(enemy.hand) - discarded, variant
        result.append(
            {
                "name": f"passive-{variant}",
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    return result
