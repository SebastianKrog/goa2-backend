"""Real retrieval support skills, including prerequisites, skips and follow-up moves."""

from __future__ import annotations


def cases(advance, answer):
    from aura_traces import _board, _pieces
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import CardState, GamePhase, StatType, TeamColor
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import ResolveCardStep, ResolveCardTextStep

    result = []
    for effect, name in [
        ("steady_advance", "Sabina"),
        ("unwavering_resolve", "Sabina"),
        ("devoted_followers", "Xargatha"),
        ("fresh_converts", "Xargatha"),
    ]:
        variants = [
            "ordinary",
            "action",
            "empty",
            "empty-discard",
            "one-discard",
            "skip-card",
            "skip-move",
            "invalid",
            "items",
            "atomic",
            "stale-selection",
        ]
        variants += (
            ["one-friendly", "far-friendly"]
            if name == "Sabina"
            else ["enemy-hero", "enemy-minion", "enemy-piece"]
        )
        for variant in variants:
            state = setup([name, "Hanu"], ["Brogan", "Razzle"])
            h = _board(state)
            state.battle_zones = {}
            state.phase = GamePhase.RESOLUTION
            actor = state.teams[TeamColor.RED].heroes[0]
            state.current_actor_id = actor.id
            card = next(c for c in actor.deck if c.effect_id == effect)
            if card in actor.hand:
                actor.hand.remove(card)
            card.state = CardState.UNRESOLVED
            card.is_facedown = False
            actor.current_turn_card = card
            state.place_entity(actor.id, h(0, 0))
            discards = []
            if variant != "empty-discard":
                discards = (
                    [actor.hand[0]] if variant == "one-discard" else [actor.hand[0], actor.hand[-1]]
                )
                for discarded in discards:
                    discarded.is_active_base = True
                    actor.discard_card(discarded)
            prerequisite = variant not in ["empty", "stale-selection"]
            if name == "Sabina":
                friendly = [m for m in state.teams[TeamColor.RED].minions if not m.is_heavy][:2]
                if prerequisite:
                    state.place_entity(friendly[0].id, h(1, 0))
                    if variant != "one-friendly":
                        state.place_entity(
                            friendly[1].id, h(0, 4) if variant == "far-friendly" else h(0, 1)
                        )
                if variant in ["one-friendly", "far-friendly"]:
                    prerequisite = False
            else:
                if prerequisite:
                    if variant == "enemy-hero":
                        target = "hero_brogan"
                    elif variant == "enemy-piece":
                        target = _pieces(state, "hero_razzle")[0]
                    else:
                        target = str(
                            next(
                                m for m in state.teams[TeamColor.BLUE].minions if not m.is_heavy
                            ).id
                        )
                    state.place_entity(target, h(1, 0))
                if effect == "fresh_converts" and variant in ["enemy-hero", "enemy-piece"]:
                    prerequisite = False
            if variant == "items":
                actor.items = {StatType.RADIUS: 1, StatType.MOVEMENT: 2}
            state.execution_context = (
                {"retrieved_card": str(discards[0].id)} if variant == "stale-selection" else {}
            )
            step = (
                ResolveCardStep(hero_id=actor.id)
                if variant == "action"
                else ResolveCardTextStep(hero_id=actor.id, card_id=card.id)
            )
            step.step_id = f"fixture/retrieval-primary/{effect}/{variant}"
            push_steps(state, [step])
            envelope = export_state(state)
            probe = state.model_copy(deep=True)
            ops = []
            invalid = variant == "invalid"
            for _ in range(100):
                if variant == "atomic":
                    remaining = 1

                    def boundary(_state, _step):
                        nonlocal remaining
                        if remaining == 0:
                            return True
                        remaining -= 1
                        return False

                    resolution = process_stack(probe, stop_before_step=boundary)
                    op = {"kind": "advance_prefix", "steps": 1}
                else:
                    resolution = process_stack(probe)
                    op = advance
                ops.append(op)
                req = resolution.input_request
                if req is None:
                    if not probe.execution_stack:
                        break
                    continue
                if invalid:
                    value = "not-a-choice"
                    invalid = False
                elif req.request_type.value == "CHOOSE_ACTION":
                    value = "SKILL"
                elif (variant == "skip-card" and req.request_type.value == "SELECT_CARD") or (
                    variant == "skip-move" and req.request_type.value == "SELECT_HEX"
                ):
                    value = "SKIP"
                elif "CONFIRM" in [o.id for o in req.options]:
                    value = "CONFIRM"
                else:
                    value = selection_value(req.options[0])
                ops.append(answer(value))
                submit_input(probe, InputResponse(request_id=req.id, selection=value))
            else:
                raise AssertionError((effect, variant))
            expected = reference_trace(state, ops)
            retrievals = [
                event
                for record in expected
                for event in record["events"]
                if event["event_type"] == "CARD_RETRIEVED"
            ]
            should_retrieve = (
                prerequisite and bool(discards) and variant != "skip-card"
            ) or variant == "stale-selection"
            assert len(retrievals) == int(should_retrieve), (effect, variant)
            if should_retrieve:
                assert not probe.get_hero(actor.id).hand[-1].is_active_base, (effect, variant)
            moves = [
                event
                for record in expected
                for event in record["events"]
                if event["event_type"] == "UNIT_MOVED"
            ]
            assert bool(moves) == (
                name == "Sabina" and should_retrieve and variant != "skip-move"
            ), (effect, variant, moves)
            result.append(
                {
                    "name": f"retrieval-primary-{effect}-{variant}",
                    "envelope": envelope,
                    "operations": ops,
                    "expected": expected,
                }
            )
    return result
