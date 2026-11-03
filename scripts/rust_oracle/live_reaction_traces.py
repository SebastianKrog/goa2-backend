"""Live defense windows, shared action stacks and discard/passive continuations.

Raw boundary fixtures retain aliases that normal card-author helpers do not expose.
All expected states, requests and events are produced by the frozen source handler.
"""

from __future__ import annotations


def scenes():
    from unittest.mock import patch

    import reaction_traces
    import stack_traces
    from live_defeat_traces import retain

    from goa2.domain.input import InputRequestType, selection_value
    from goa2.engine.steps import ReactionWindowStep

    captured = []

    def capture(state):
        captured.append(state)
        return {}

    def answer(selection, request_id="current"):
        return {"kind": "input", "selection": selection, "request_id": request_id}

    with (
        patch.object(reaction_traces, "export_state", capture),
        patch.object(stack_traces, "reference_trace", return_value=[]),
    ):
        cases = reaction_traces.cases({"kind": "advance"}, answer)
    assert len(cases) == len(captured)
    for case, state in zip(cases, captured, strict=True):
        if len(state.execution_stack) != 1 or not isinstance(
            state.execution_stack[0], ReactionWindowStep
        ):
            continue
        step = state.execution_stack[0]
        choices = [op.get("selection") for op in case["operations"] if op["kind"] == "input"]
        chosen = 0

        def choose(request, _mode, choices=choices):
            nonlocal chosen
            if request.request_type == InputRequestType.SELECT_CARD_OR_PASS:
                result = choices[chosen] if chosen < len(choices) else "PASS"
                chosen += 1
                return result
            return selection_value(request.options[0])

        stack = []
        state.execution_context.update(
            action_type_stack=stack, stack_alias=stack, reaction_alias=step
        )
        target = state.get_hero(state.execution_context.get(step.target_player_key))
        if target:
            state.execution_context["defense_hand_alias"] = target.hand
        ops, _ = retain(state, [step], select=choose)
        yield "live-" + case["name"], state, ops

    from combat_effect_traces import state_for

    from goa2.domain.models import ActionType

    variants = [
        "no-target",
        "false-target",
        "zero-target",
        "numeric-target",
        "empty-target",
        "reset-target",
        "custom-target",
        "attack-none",
        "attack-false",
        "attack-true",
        "attack-float",
        "attack-negative",
        "stack-missing",
        "stack-empty-current",
        "stack-false-current",
        "stack-null-current",
        "stack-tuple-no-current",
        "stack-null-no-current",
        "stack-aggregate-current",
        "pending-empty",
        "pending-missing-selection",
        "pending-invalid",
        "pending-pass",
        "skip",
        "active",
    ]
    for variant in variants:
        state, _, _ = state_for()
        hero = state.get_hero("hero_brogan")
        card = next(
            c
            for c in hero.hand
            if c.current_primary_action in (ActionType.DEFENSE, ActionType.DEFENSE_SKILL)
            or ActionType.DEFENSE in c.current_secondary_actions
        )
        step = ReactionWindowStep()
        stack = ["previous"]
        context = state.execution_context
        context.update(
            target_id=str(hero.id),
            attack_damage=4,
            current_action_type="ATTACK",
            action_type_stack=stack,
            stack_alias=stack,
            reaction_alias=step,
            defense_hand_alias=hero.hand,
        )
        context.update(
            auto_block=True,
            defense_invalid=True,
            defense_bonus=99,
            ignore_minion_defense=True,
            snorri_ult_rune_defense=True,
        )
        if variant == "no-target":
            context.pop("target_id")
        if variant == "false-target":
            context["target_id"] = False
        if variant == "zero-target":
            context["target_id"] = 0
        if variant == "numeric-target":
            context["target_id"] = 17
        if variant == "empty-target":
            context["target_id"] = ""
        if variant == "reset-target":
            step.target_player_key = "auto_block"
            context["auto_block"] = str(hero.id)
        if variant == "custom-target":
            step.target_player_key = "custom"
            context["custom"] = str(hero.id)
        if variant.startswith("attack-"):
            context["attack_damage"] = {
                "attack-none": None,
                "attack-false": False,
                "attack-true": True,
                "attack-float": 2.5,
                "attack-negative": -16,
            }[variant]
        if variant == "stack-missing":
            context.pop("action_type_stack")
        if variant == "stack-empty-current":
            context["current_action_type"] = ""
        if variant == "stack-false-current":
            context["current_action_type"] = False
        if variant in {"stack-null-current", "stack-tuple-no-current", "stack-null-no-current"}:
            context["current_action_type"] = None
        if variant == "stack-tuple-no-current":
            context["action_type_stack"] = ("previous",)
        if variant == "stack-null-no-current":
            context["action_type_stack"] = None
        if variant == "stack-aggregate-current":
            current = {"action": "ATTACK"}
            context.update(current_action_type=current, current_alias=current)
        if variant.startswith("pending-"):
            step.pending_input = {
                "pending-empty": {},
                "pending-missing-selection": {"extra": 1},
                "pending-invalid": {"selection": "bogus"},
                "pending-pass": {"selection": "PASS"},
            }[variant]
            context["pending_alias"] = step.pending_input
        if variant == "skip":
            step.skip_if_key = "gate"
            context["gate"] = False
        if variant == "active":
            step.active_if_key = "missing"

        def choose(request, _mode, card=card):
            return (
                card.id
                if request.request_type == InputRequestType.SELECT_CARD_OR_PASS
                else selection_value(request.options[0])
            )

        ops, _ = retain(state, [step], select=choose)
        yield "live-reaction-context-" + variant, state, ops

    yield from shield_scenes()
    yield from piece_scenes()
    yield from integration_scenes()
    yield from actor_scenes()


def shield_scenes():
    from aura_traces import _board
    from combat_effect_traces import effect
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.input import InputRequestType, selection_value
    from goa2.domain.models import ActionType, CardState, GamePhase
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import ReactionWindowStep

    for variant in (
        "played",
        "current",
        "alias-hand",
        "equal-hand",
        "unequal-hand",
        "inactive",
        "duplicate-printed",
        "facedown",
        "blocked-primary",
    ):
        state = setup(["Mrak", "Arien"], ["Brogan", "Swift"])
        h = _board(state)
        hero = state.get_hero("hero_mrak")
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_brogan"
        state.place_entity(hero.id, h(0, 0))
        state.place_entity("hero_brogan", h(2, 0))
        shield = next(
            c
            for c in hero.hand
            if c.current_primary_action in (ActionType.DEFENSE, ActionType.DEFENSE_SKILL)
            or ActionType.DEFENSE in c.current_secondary_actions
        )
        hero.hand.remove(shield)
        shield.state = CardState.RESOLVED
        if variant == "current":
            hero.current_turn_card = shield
        else:
            hero.played_cards = [shield]
        if variant == "alias-hand":
            hero.hand.append(shield)
        if variant in {"equal-hand", "unequal-hand"}:
            clone = shield.model_copy(deep=True)
            if variant == "unequal-hand":
                clone.metadata = {"different": 1}
            hero.hand.append(clone)
            state.execution_context["clone_alias"] = clone
        if variant == "duplicate-printed":
            hero.played_cards.append(shield.model_copy(deep=True))
        if variant == "facedown":
            shield.is_facedown = True
        active = effect(
            state,
            "reaction-shield",
            EffectType.DISCARD_SHIELD,
            source=str(hero.id),
            source_card_id=str(shield.id),
        )
        if variant == "inactive":
            active.is_active = False
        state.active_effects = [active]
        stack = ["older"]
        state.execution_context.update(
            target_id=str(hero.id),
            attack_damage=4,
            current_action_type="ATTACK",
            action_type_stack=stack,
            stack_alias=stack,
            shield_alias=shield,
            hand_alias=hero.hand,
            played_alias=hero.played_cards,
        )
        if variant == "blocked-primary":
            state.execution_context["block_primary_defense"] = True
        step = ReactionWindowStep()
        state.execution_context["reaction_alias"] = step

        def choose(request, _mode, shield=shield):
            if request.request_type == InputRequestType.SELECT_CARD_OR_PASS:
                return (
                    str(shield.id)
                    if any(option.id == str(shield.id) for option in request.options)
                    else "PASS"
                )
            return selection_value(request.options[0])

        ops, _ = retain(state, [step], select=choose)
        yield "live-reaction-shield-" + variant, state, ops


def piece_scenes():
    from aura_traces import _board, _pieces
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.input import InputRequestType, selection_value
    from goa2.domain.models import ActionType, GamePhase, StatType
    from goa2.engine.steps import ReactionWindowStep

    for variant in ("owner", "piece-0", "piece-1", "piece-2", "piece-3", "initiative", "pass"):
        state = setup(["Razzle", "Arien"], ["Brogan", "Swift"])
        h = _board(state)
        hero = state.get_hero("hero_razzle")
        pieces = _pieces(state, hero.id)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_brogan"
        for i, piece in enumerate(pieces):
            state.place_entity(piece, h(i, 0))
        state.place_entity("hero_brogan", h(-1, 0))
        hero.items = {StatType.DEFENSE: 2, StatType.INITIATIVE: 1}
        target = (
            str(hero.id)
            if variant == "owner"
            else pieces[int(variant[-1]) if variant.startswith("piece-") else 1]
        )
        state.execution_context.update(
            target_id=target,
            attack_damage=4,
            current_action_type="ATTACK",
            defense_uses_initiative=variant == "initiative",
        )
        step = ReactionWindowStep()
        state.execution_context.update(reaction_alias=step, hand_alias=hero.hand)
        card = next(
            c
            for c in hero.hand
            if c.current_primary_action in (ActionType.DEFENSE, ActionType.DEFENSE_SKILL)
            or ActionType.DEFENSE in c.current_secondary_actions
        )

        def choose(request, _mode, variant=variant, card=card):
            if request.request_type == InputRequestType.SELECT_CARD_OR_PASS:
                return "PASS" if variant == "pass" else str(card.id)
            return selection_value(request.options[0])

        ops, _ = retain(state, [step], select=choose)
        yield "live-reaction-piece-" + variant, state, ops


def integration_scenes():
    from combat_effect_traces import state_for
    from live_defeat_traces import retain
    from object_execution_traces import live_choice_operations

    from goa2.domain.input import InputRequestType, selection_value
    from goa2.engine.steps import AttackSequenceStep, ReactionWindowStep, SetContextFlagStep

    for mode in ("pass", "card"):
        state, _, _ = state_for()
        step = ReactionWindowStep()
        stack = []
        state.execution_context.update(
            target_id="hero_brogan",
            attack_damage=4,
            current_action_type="ATTACK",
            action_type_stack=stack,
            stack_alias=stack,
            reaction_alias=step,
        )
        selected = False

        def choose(request, _mode, mode=mode):
            nonlocal selected
            if request.request_type == InputRequestType.SELECT_CARD_OR_PASS:
                if mode == "pass" or selected:
                    return "PASS"
                selected = True
            return selection_value(request.options[0])

        flag = SetContextFlagStep(key="auto_block", value=True)
        ops, _ = retain(state, [step, flag, step], select=choose)
        yield "live-reaction-reused-" + mode, state, ops

    for variant in ("minion", "hero-pass", "hero-card"):
        state, _, minion = state_for()
        attack = AttackSequenceStep(
            step_id="fixture/reaction/attack", damage=4, range_val=3, target_id_key="chosen"
        )
        state.execution_context.update(
            chosen=str(minion.id) if variant == "minion" else "hero_brogan", attack_alias=attack
        )
        state.execution_stack = [attack]
        ops = live_choice_operations(
            state,
            mode="last" if variant == "hero-pass" else "first",
            stop_types=(
                "resolve_pre_action_movement",
                "resolve_pre_action_discard",
                "resolve_defense_text",
            ),
        )
        yield "live-reaction-attack-" + variant, state, ops


def actor_scenes():
    from combat_effect_traces import state_for
    from live_defeat_traces import retain

    from goa2.engine.steps import SetActorStep

    for variant in (
        "literal",
        "key-wins",
        "save-then-read",
        "missing-key",
        "numeric",
        "boolean",
        "skip",
        "active",
    ):
        state, _, _ = state_for()
        step = SetActorStep(actor_id="hero_hanu", save_key="saved_custom")
        state.execution_context["actor_step_alias"] = step
        if variant in {"key-wins", "missing-key", "numeric", "boolean"}:
            step.actor_key = "chosen"
        if variant == "key-wins":
            state.execution_context["chosen"] = "hero_brogan"
        if variant == "save-then-read":
            step.actor_key = step.save_key
            state.execution_context[step.save_key] = "hero_hanu"
        if variant == "numeric":
            state.execution_context["chosen"] = 17
        if variant == "boolean":
            state.execution_context["chosen"] = True
        if variant == "skip":
            step.skip_if_key = "gate"
            state.execution_context["gate"] = False
        if variant == "active":
            step.active_if_key = "absent"
        ops, _ = retain(state, [step])
        yield "live-reaction-actor-" + variant, state, ops
