"""Frozen pre-action movement and reentrant disruptor state/alias traces.

Raw engine boundaries deliberately retain mutable step lists and world aliases.
Real card effects are seeded through their registered source builders/handler.
"""

from __future__ import annotations


def scenes():
    from contextlib import suppress
    from unittest.mock import patch

    import targeting_effect_traces
    from live_defeat_traces import retain

    emitted = []

    class MatrixComplete(Exception):
        pass

    def capture(name, state, steps, mode="first", unordered_keys=(), select=None):
        if name.startswith("targeting-attack-pre-"):
            raise MatrixComplete
        if not name.startswith(("targeting-pre-move-", "targeting-pre-discard-")):
            return {}, state
        step = steps[0]
        state.execution_context["pre_action_alias"] = step
        if hasattr(step, "processed_effect_ids"):
            state.execution_context["processed_alias"] = step.processed_effect_ids
        ops, final = retain(state, steps, mode, select)
        emitted.append(("live-" + name, state, ops))
        return {"name": name}, final

    with patch.object(targeting_effect_traces, "drive", capture), suppress(MatrixComplete):
        targeting_effect_traces.cases()
    assert len(emitted) == 100, len(emitted)
    yield from emitted
    yield from card_scenes()
    yield from alias_scenes()
    yield from attack_scenes()


def card_scenes():
    from aura_traces import _board
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, Turret
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.handler import process_stack, push_steps
    from goa2.engine.steps import ResolvePreActionDiscardStep, ResolvePreActionMovementStep

    for name in ("focus", "discipline", "mastery"):
        for variant in ("first", "skip", "inactive", "expired"):
            state = setup(["Misa", "Arien"], ["Brogan", "Trinkets"])
            h = _board(state)
            hero = state.get_hero("hero_misa")
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_arien", h(-3, 0))
            state.place_entity("hero_brogan", h(3, 0))
            state.place_entity("hero_trinkets", h(3, -1))
            card = next(card for card in hero.deck if card.effect_id == name)
            card.state, card.is_facedown = CardState.RESOLVED, False
            hero.played_cards = [card]
            hero.hand = [item for item in hero.hand if item is not card]
            push_steps(state, CardEffectRegistry.get(name).get_steps(state, hero, card))
            assert process_stack(state).input_request is None
            for index, effect in enumerate(state.active_effects):
                effect.id = f"fixture/{name}/{index}"
            state.turn += 1
            if variant == "inactive":
                state.active_effects[0].is_active = False
            if variant == "expired":
                state.turn += 1
            step = ResolvePreActionMovementStep(hero_id=str(hero.id))
            state.execution_context.update(pre_action_alias=step, carrier_alias=card)
            ops, _ = retain(state, [step], "skip" if variant == "skip" else "first")
            yield f"pre-action-card-{name}-{variant}", state, ops

    for name in ("disruptor_jolt", "disruptor_pulse", "disruptor_grid"):
        for variant in ("hand", "empty", "defender", "out-of-radius"):
            state = setup(["Trinkets", "Arien"], ["Brogan", "Misa"])
            h = _board(state)
            hero = state.get_hero("hero_trinkets")
            victim = state.get_hero("hero_brogan")
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(victim.id, h(2, 0))
            state.place_entity("hero_arien", h(-1, 0))
            state.place_entity("hero_misa", h(3, -1))
            state.remove_entity("trinkets_turret")
            state.misc_entities.pop("trinkets_turret", None)
            turret = Turret(id="trinkets_turret", name="Turret", owner_id=str(hero.id))
            state.register_entity(turret, "board_entity")
            state.place_entity(turret.id, h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == name)
            card.state, card.is_facedown = CardState.RESOLVED, False
            hero.played_cards = [card]
            hero.hand = [item for item in hero.hand if item is not card]
            push_steps(state, CardEffectRegistry.get(name).get_steps(state, hero, card))
            assert process_stack(state).input_request is None
            for index, effect in enumerate(state.active_effects):
                effect.id = f"fixture/{name}/{index}"
            state.current_actor_id = "hero_arien" if variant == "defender" else str(victim.id)
            if variant in {"empty", "defender"}:
                victim.hand = []
            if variant == "out-of-radius":
                state.move_unit(victim.id, h(-4, 0))
            step = ResolvePreActionDiscardStep(
                hero_id=str(victim.id), abort_on_defeat=variant != "defender"
            )
            state.execution_context.update(
                pre_action_alias=step,
                processed_alias=step.processed_effect_ids,
                carrier_alias=card,
                hand_alias=victim.hand,
            )
            ops, _ = retain(state, [step])
            yield f"pre-action-card-{name}-{variant}", state, ops


def alias_scenes():
    from combat_effect_traces import effect, state_for
    from live_defeat_traces import retain

    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import ResolvePreActionDiscardStep, ResolvePreActionMovementStep

    for variant in (
        "shared-step",
        "shared-processed",
        "empty-safe-many",
        "duplicate-safe",
        "processed-tuple",
        "no-effects-null",
        "unknown",
        "missing",
        "numeric",
        "skip",
        "active",
    ):
        state, _, _ = state_for()
        hero = state.get_hero("hero_brogan")
        row = effect(state, "disrupt", EffectType.PRE_ACTION_DISCARD, source="hero_swift")
        state.active_effects = [row]
        step = ResolvePreActionDiscardStep(hero_id=str(hero.id))
        steps = [step]
        if variant == "shared-step":
            steps = [step, step]
        if variant == "shared-processed":
            other = ResolvePreActionDiscardStep(hero_id=str(hero.id))
            other.processed_effect_ids = step.processed_effect_ids
            steps = [step, other]
        if variant in {"empty-safe-many", "duplicate-safe"}:
            hero.hand = []
            state.active_effects.extend(
                row.model_copy(
                    update={"id": "disrupt" if variant == "duplicate-safe" else f"disrupt-{i}"}
                )
                for i in (2, 3)
            )
        if variant == "processed-tuple":
            step.processed_effect_ids = ("disrupt",)
        if variant == "no-effects-null":
            state.active_effects = []
            step.processed_effect_ids = None
        if variant == "unknown":
            step.hero_id = "missing"
        if variant in {"missing", "numeric"}:
            step.hero_id = None
            step.hero_key = "who"
            if variant == "numeric":
                state.execution_context["who"] = 17
        if variant == "skip":
            step.skip_if_key = "gate"
            state.execution_context["gate"] = False
        if variant == "active":
            step.active_if_key = "missing"
        state.execution_context.update(
            pre_action_alias=step, processed_alias=step.processed_effect_ids, hand_alias=hero.hand
        )
        ops, _ = retain(state, steps)
        yield "pre-action-discard-alias-" + variant, state, ops

    for variant in (
        "first-active",
        "after-inactive",
        "wrong-source",
        "numeric",
        "zero",
        "negative",
    ):
        state, _, _ = state_for()
        rows = [
            effect(
                state, "move-a", EffectType.PRE_ACTION_MOVEMENT, source="hero_swift", max_value=2
            ),
            effect(
                state, "move-b", EffectType.PRE_ACTION_MOVEMENT, source="hero_swift", max_value=3
            ),
        ]
        if variant == "after-inactive":
            rows[0].is_active = False
        if variant == "wrong-source":
            rows[0].source_id = "hero_brogan"
        if variant == "zero":
            rows[0].max_value = 0
        if variant == "negative":
            rows[0].max_value = -2
        state.active_effects = rows
        step = ResolvePreActionMovementStep(hero_key="who")
        state.execution_context.update(
            who=17 if variant == "numeric" else "hero_swift", pre_action_alias=step
        )
        ops, _ = retain(state, [step])
        yield "pre-action-move-order-" + variant, state, ops


def attack_scenes():
    import copy
    from unittest.mock import patch

    import card_selection_traces
    import targeting_effect_traces

    from goa2.domain.input import InputResponse
    from goa2.engine.handler import process_stack, submit_input

    emitted = []

    def capture(name, state, steps, mode="first", unordered_keys=(), select=None):
        if not name.startswith("targeting-attack-pre-"):
            return {}, state
        for index, step in enumerate(steps):
            step.step_id = f"fixture/pre-action/attack/{index}"
        state.execution_stack = list(reversed(steps))
        state.execution_context.update(attack_alias=steps[0], effects_alias=state.active_effects)
        probe = copy.deepcopy(state)
        ops = []
        for _ in range(100):
            if (
                not probe.execution_stack
                or probe.execution_stack[-1].type.value == "resolve_defense_text"
                or probe.phase.value == "GAME_OVER"
            ):
                break
            remaining = 1

            def boundary(_state, _step):
                nonlocal remaining
                if not remaining:
                    return True
                remaining -= 1
                return False

            output = process_stack(probe, stop_before_step=boundary)
            ops.append({"kind": "advance_prefix", "steps": 1})
            if output.input_request:
                request = output.input_request
                value = select(request, mode)
                submit_input(probe, InputResponse(request_id=request.id, selection=value))
                ops.append({"kind": "input", "request_id": "current", "selection": value})
        else:
            raise AssertionError("attack pre-action scene did not reach defense text")
        emitted.append(("live-" + name, state, ops))
        return {"name": name}, probe

    with (
        patch.object(targeting_effect_traces, "drive", capture),
        patch.object(card_selection_traces, "cases", return_value=[]),
    ):
        targeting_effect_traces.cases()
    assert len(emitted) == 32, len(emitted)
    yield from emitted
