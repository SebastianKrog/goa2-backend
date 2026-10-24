"""Mandatory discard-or-defeat reference traces, including owner/piece identity."""

from __future__ import annotations


def cases(advance, answer):
    from aura_traces import _board, _pieces
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.input import InputResponse
    from goa2.domain.models import GamePhase
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps.cards import ForceDiscardOrDefeatStep

    variants = [
        "hand",
        "one-card",
        "empty",
        "offboard",
        "missing",
        "falsy",
        "unknown",
        "minion",
        "killer-literal",
        "killer-key",
        "killer-fallback",
        "no-actor",
        "multi-owner",
        "multi-piece",
        "multi-offboard",
        "multi-empty",
        "immunity-source",
        "prefix-hand",
        "prefix-empty",
        "prefix-multi-piece",
    ]
    result = []
    for name in variants:
        variant = name.removeprefix("prefix-")
        state = setup(["Sabina", "Arien"], ["Razzle", "Brogan"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_sabina"
        state.place_entity("hero_sabina", h(0, 0))
        victim = "hero_brogan"
        state.place_entity(victim, h(1, 0))
        if variant.startswith("multi-"):
            state.remove_entity("hero_brogan")
            victim = "hero_razzle"
            piece = _pieces(state, victim)[0]
            if variant != "multi-offboard":
                state.place_entity(piece, h(1, 0))
            if variant == "multi-piece":
                victim = piece
        hero = state.get_hero(victim)
        if variant in [
            "empty",
            "killer-literal",
            "killer-key",
            "killer-fallback",
            "no-actor",
            "multi-empty",
        ]:
            hero.discard_pile.extend(hero.hand)
            hero.hand = []
        if variant == "one-card":
            hero.discard_pile.extend(hero.hand[1:])
            hero.hand = hero.hand[:1]
        if variant == "offboard":
            state.remove_entity(victim)
        state.execution_context = {"victim": victim, "credit": "hero_arien"}
        if variant == "missing":
            state.execution_context.pop("victim")
        if variant == "falsy":
            state.execution_context["victim"] = []
        if variant == "unknown":
            state.execution_context["victim"] = "missing"
        if variant == "minion":
            state.execution_context["victim"] = str(next(iter(state.teams.values())).minions[0].id)
        if variant == "killer-fallback":
            state.execution_context["credit"] = False
        if variant == "no-actor":
            state.current_actor_id = None
        step = ForceDiscardOrDefeatStep(
            victim_key="victim",
            killer_id="hero_arien" if variant == "killer-literal" else None,
            killer_key=(
                "credit" if variant in ["killer-key", "killer-fallback", "killer-literal"] else None
            ),
            immunity_source_id="hero_sabina" if variant == "immunity-source" else None,
            step_id=f"fixture/discard-or-defeat/{name}",
        )
        push_steps(state, [step])
        envelope = export_state(state)
        probe = state.model_copy(deep=True)
        ops = [{"kind": "advance_prefix", "steps": 1}] if name.startswith("prefix-") else []
        for _ in range(30):
            ops.append(advance)
            request = process_stack(probe).input_request
            if request is None:
                break
            value = request.options[0].id
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=request.id, selection=value))
        else:
            raise AssertionError(variant)
        acted = variant not in [
            "offboard",
            "missing",
            "falsy",
            "unknown",
            "minion",
            "multi-offboard",
        ]
        if acted and hero.hand:
            assert len(probe.get_hero(victim).hand) == len(hero.hand) - 1, name
            assert str(probe.get_hero(victim).discard_pile[-1].id) == str(hero.hand[0].id), name
            assert probe.has_board_presence(victim), name
        elif acted:
            # The frozen engine records an owner-level defeat but removes only
            # that owner identity; its surviving pieces are part of the oracle.
            assert probe.has_board_presence(victim) == (variant == "multi-empty"), name
            credit = (
                "hero_arien"
                if variant in ["killer-literal", "killer-key"]
                else None if variant == "no-actor" else "hero_sabina"
            )
            assert probe.execution_context["defeat_log"][-1][1] == credit, name
        else:
            assert probe.get_hero(victim).hand == hero.hand, name
        result.append(
            {
                "name": f"discard-or-defeat-{name}",
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    return result
