"""Real conditional attacks/discards against current/resolved/hidden target cards."""

from __future__ import annotations


def cases(advance, answer):
    from aura_traces import _board, _pieces
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import ActionType, CardState, GamePhase, StatType
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.stats import compute_card_stats
    from goa2.engine.steps import ResolveCardStep, ResolveCardTextStep

    result = []
    for effect, hero_id in [
        ("quickdraw", "hero_sabina"),
        ("gunslinger", "hero_sabina"),
        ("dead_shot", "hero_sabina"),
        ("shield_bash", "hero_brogan"),
        ("counterattack", "hero_brogan"),
    ]:
        for variant in [
            "attack-current",
            "nonattack-current",
            "attack-resolved",
            "attack-facedown",
            "attack-old-slot",
            "attack-current-resolved",
            "no-current",
            "empty-hand",
            "piece-attack",
            "piece-nonattack",
            "skip",
            "invalid",
            "atomic",
            "action",
            "items",
            "stale-bonus",
        ]:
            state = setup(["Sabina", "Brogan"], ["Arien", "Razzle"])
            h = _board(state)
            state.battle_zones = {}
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = hero_id
            actor = state.get_hero(hero_id)
            card = next(c for c in actor.deck if c.effect_id == effect)
            if card in actor.hand:
                actor.hand.remove(card)
            card.state = CardState.UNRESOLVED
            card.is_facedown = False
            actor.current_turn_card = card
            state.place_entity(hero_id, h(0, 0))
            victim = "hero_arien"
            if variant.startswith("piece-"):
                victim = _pieces(state, "hero_razzle")[0]
            target = state.get_hero(victim)
            state.place_entity(victim, h(1, 0))
            if variant != "no-current":
                attack = variant not in ["nonattack-current", "piece-nonattack", "stale-bonus"]
                played = next(
                    c for c in target.hand if (c.primary_action == ActionType.ATTACK) == attack
                )
                target.play_card(played)
                played.is_facedown = variant == "attack-facedown"
                if variant in ["attack-resolved", "attack-old-slot"]:
                    target.resolve_current_card(turn_number=1)
                    if variant == "attack-old-slot":
                        state.turn = 2
                if variant == "attack-current-resolved":
                    played.state = CardState.RESOLVED
            if variant == "empty-hand":
                target.discard_pile.extend(target.hand)
                target.hand = []
            if variant == "items":
                actor.items = {StatType.ATTACK: 2, StatType.RANGE: 1}
            state.execution_context = {"atk_bonus": 2} if variant == "stale-bonus" else {}
            stats = compute_card_stats(state, actor.id, card)
            step = (
                ResolveCardStep(hero_id=hero_id)
                if variant == "action"
                else ResolveCardTextStep(hero_id=hero_id, card_id=str(card.id))
            )
            step.step_id = f"fixture/conditional-primary/{effect}/{variant}"
            push_steps(state, [step])
            envelope = export_state(state)
            probe = state.model_copy(deep=True)
            ops = []
            invalid = variant == "invalid"
            for _ in range(100):
                if variant == "atomic":
                    operation = {"kind": "advance_prefix", "steps": 1}
                    remaining = 1

                    def boundary(_state, _step):
                        nonlocal remaining
                        if remaining == 0:
                            return True
                        remaining -= 1
                        return False

                    resolution = process_stack(probe, stop_before_step=boundary)
                else:
                    operation = advance
                    resolution = process_stack(probe)
                ops.append(operation)
                req = resolution.input_request
                if req is None:
                    if not probe.execution_stack:
                        break
                    continue
                ids = [o.id for o in req.options]
                if invalid:
                    value = "not-a-choice"
                    invalid = False
                elif req.request_type.value == "CHOOSE_ACTION":
                    value = "ATTACK" if hero_id == "hero_sabina" else "SKILL"
                elif req.request_type.value == "SELECT_CARD_OR_PASS":
                    value = "PASS"
                elif variant == "skip" and req.can_skip:
                    value = "SKIP"
                elif victim in ids:
                    value = victim
                elif "CONFIRM" in ids:
                    value = "CONFIRM"
                else:
                    value = selection_value(req.options[0])
                ops.append(answer(value))
                submit_input(probe, InputResponse(request_id=req.id, selection=value))
            else:
                raise AssertionError((effect, variant))
            expected_bonus = variant not in [
                "nonattack-current",
                "piece-nonattack",
                "attack-facedown",
                "attack-old-slot",
                "no-current",
                "stale-bonus",
            ]
            if hero_id == "hero_sabina":
                bonus = (
                    (4 if effect == "dead_shot" else 3)
                    if expected_bonus
                    else 2 if variant == "stale-bonus" else 0
                )
                assert probe.execution_context["played_attack_count"] == int(expected_bonus), (
                    effect,
                    variant,
                )
                assert probe.execution_context["attack_damage"] == stats.primary_value + bonus, (
                    effect,
                    variant,
                    probe.execution_context,
                )
            elif expected_bonus and variant != "skip":
                if variant == "empty-hand" and effect == "counterattack":
                    assert probe.get_position(victim) is None
                elif variant != "empty-hand":
                    assert probe.turn_discard_log.get(target.id), (effect, variant)
            result.append(
                {
                    "name": f"conditional-primary-{effect}-{variant}",
                    "envelope": envelope,
                    "operations": ops,
                    "expected": reference_trace(state, ops),
                }
            )
    return result
