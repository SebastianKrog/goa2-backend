"""Utility continuations with physical dict/Hex/template identities.

Raw fixtures isolate stack and object identity boundaries; card-specific builder
helpers would hide the literal/key precedence and retained step aliases here.
"""

from __future__ import annotations


def scenes():
    from aura_traces import _board
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.models import GamePhase
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.steps import (
        AddContextValueStep,
        CheckAdjacencyStep,
        ComputeHexStep,
        MayRepeatNTimesStep,
        MayRepeatOnceStep,
        RecordHexStep,
        RecordTargetStep,
        SetContextFlagStep,
    )

    def base():
        state = setup(["Arien", "Swift"], ["Brogan", "Hanu"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        state.battle_zones = {}
        state.place_entity("hero_arien", h(0, 0))
        state.place_entity("hero_brogan", h(1, 0))
        state.execution_context.update(
            target="hero_brogan",
            origin="hero_arien",
            unit="hero_brogan",
            a="hero_arien",
            b="hero_brogan",
            output={"before": True},
            position_alias=state.entity_locations["hero_brogan"],
        )
        state.execution_context["old_output"] = state.execution_context["output"]
        return state, h

    for kind in ("record", "compute", "adjacency"):
        for variant in (
            "plain",
            "keys",
            "literal-wins",
            "missing-key",
            "empty-key",
            "unknown",
            "offboard",
            "numeric",
            "skip",
            "inactive",
            "repeated",
            "fresh",
            "zero",
            "negative",
            "same",
            "far",
        ):
            if variant == "repeated" or (kind != "compute" and variant in {"zero", "negative"}):
                continue
            state, h = base()
            if kind == "record":
                step = RecordHexStep(unit_id="hero_brogan", output_key="output")
                if variant in {"keys", "missing-key", "empty-key", "numeric"}:
                    step.unit_id = None
                    step.unit_key = "unit"
                if variant == "literal-wins":
                    step.unit_key = "unit"
                    state.execution_context["unit"] = "hero_arien"
                if variant == "missing-key":
                    state.execution_context.pop("unit")
                if variant == "empty-key":
                    state.execution_context["unit"] = ""
                if variant == "numeric":
                    state.execution_context["unit"] = 7
                if variant == "unknown":
                    step.unit_id = "absent"
            elif kind == "compute":
                step = ComputeHexStep(target_key="target", output_key="output")
                if variant in {"keys", "literal-wins", "missing-key", "empty-key", "numeric"}:
                    step.origin_key = "origin"
                if variant == "literal-wins":
                    state.current_actor_id = "absent"
                if variant == "missing-key":
                    state.execution_context.pop("origin")
                if variant == "empty-key":
                    state.execution_context["origin"] = ""
                if variant == "numeric":
                    state.execution_context["target"] = 7
                if variant == "unknown":
                    state.execution_context["target"] = "absent"
                if variant == "zero":
                    step.scale = 0
                if variant == "negative":
                    step.scale = -2
            else:
                step = CheckAdjacencyStep(
                    unit_a_id="hero_arien", unit_b_id="hero_brogan", output_key="output"
                )
                if variant in {"keys", "missing-key", "empty-key", "numeric"}:
                    step.unit_a_id = None
                    step.unit_b_id = None
                    step.unit_a_key = "a"
                    step.unit_b_key = "b"
                if variant == "literal-wins":
                    step.unit_a_key = "a"
                    state.execution_context["a"] = "absent"
                if variant == "missing-key":
                    state.execution_context.pop("a")
                if variant == "empty-key":
                    state.execution_context["a"] = ""
                if variant == "numeric":
                    state.execution_context["a"] = 7
                if variant == "unknown":
                    step.unit_a_id = "absent"
            if variant == "offboard":
                state.remove_entity("hero_brogan")
            if variant == "same":
                state.entity_locations["hero_brogan"] = state.entity_locations["hero_arien"]
            if variant == "far":
                state.place_entity("hero_brogan", h(3, 0))
            if variant == "skip":
                step.skip_if_key = "blocked"
                state.execution_context["blocked"] = True
            if variant == "inactive":
                step.active_if_key = "enabled"
            steps = [step]
            if variant in {"repeated", "fresh"}:
                history = RecordTargetStep(input_key="output", output_list_key="history")
                steps = [step, history, step, history]
            ops, _ = retain(state, steps)
            yield "utility-" + kind + "-" + variant, state, ops

    for variant in ("plain", "split", "isolated", "dormant", "reverse"):
        state, h = base()
        state.place_entity(
            "hero_arien", h(1, -1) if variant in {"isolated", "dormant", "reverse"} else h(0, 0)
        )
        state.place_entity("hero_brogan", h(1, 0))
        if variant != "plain":
            state.active_effects.append(
                ActiveEffect(
                    id="topology",
                    source_id="hero_brogan",
                    effect_type=(
                        EffectType.TOPOLOGY_SPLIT
                        if variant == "split"
                        else EffectType.TOPOLOGY_ISOLATION
                    ),
                    scope=EffectScope(shape=Shape.GLOBAL),
                    duration=DurationType.PASSIVE,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                    is_active=variant != "dormant",
                    split_axis="q",
                    split_value=0,
                )
            )
        a, b = (
            ("hero_brogan", "hero_arien") if variant == "reverse" else ("hero_arien", "hero_brogan")
        )
        ops, _ = retain(state, [CheckAdjacencyStep(unit_a_id=a, unit_b_id=b, output_key="output")])
        yield "utility-adjacency-topology-" + variant, state, ops

    for variant in (
        "yes",
        "no",
        "invalid",
        "three",
        "negative-counter",
        "limit",
        "zero-limit",
        "negative-limit",
        "no-actor",
        "empty-actor",
        "skip",
        "inactive",
        "blocked",
        "pending-other",
        "pending-empty",
        "nested",
        "aliases",
        "lane",
    ):
        state, h = base()
        shared = [{"nested": []}]
        first = SetContextFlagStep(key="snapshot_a", value=shared)
        second = SetContextFlagStep(key="snapshot_b", value=shared)
        templates = [
            first,
            second,
            RecordTargetStep(input_key="snapshot_a", output_list_key="history"),
            RecordTargetStep(input_key="snapshot_b", output_list_key="history"),
            AddContextValueStep(key="counter", amount=1),
        ]
        step = MayRepeatOnceStep(steps_template=templates, prompt="Again?")
        if variant == "three":
            step.max_repeats = 3
        if variant == "negative-counter":
            step.repeats_done = -1
        if variant == "limit":
            step.repeats_done = 1
        if variant == "zero-limit":
            step.max_repeats = 0
        if variant == "negative-limit":
            step.max_repeats = -2
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "empty-actor":
            state.current_actor_id = ""
        if variant == "skip":
            step.skip_if_key = "blocked"
            state.execution_context["blocked"] = True
        if variant == "inactive":
            step.active_if_key = "enabled"
        if variant == "blocked":
            state.active_effects.append(
                ActiveEffect(
                    id="no-repeat",
                    source_id="hero_brogan",
                    effect_type=EffectType.REPEAT_PREVENTION,
                    scope=EffectScope(shape=Shape.GLOBAL),
                    duration=DurationType.PASSIVE,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                    is_active=True,
                )
            )
        if variant == "pending-other":
            step.pending_input = {"selection": "MAYBE"}
        if variant == "pending-empty":
            step.pending_input = {}
        if variant == "nested":
            step.steps_template = [
                MayRepeatNTimesStep(max_repeats=2, steps_template=templates, prompt="Nested?")
            ]
        # AnyStep validation can rebuild nested models. Establish the physical
        # aliases on the actual stored roots after that construction boundary.
        stored = (
            step.steps_template[0].steps_template if variant == "nested" else step.steps_template
        )
        stored[1].value = stored[0].value
        shared = stored[0].value
        state.execution_context.update(
            repeat_alias=step, templates_alias=step.steps_template, template_value=shared
        )
        if variant == "lane":
            state.battle_zones = {"lane": "zone"}
            state.execution_stack = [step]
            step.step_id = "fixture/repeat/lane"
            yield "utility-repeat-lane", state, [
                {"kind": "advance_prefix", "steps": 1},
                {"kind": "advance_prefix", "steps": 0},
            ]
            continue

        def choose(request, mode, variant=variant):
            return "NO" if variant == "no" else "MAYBE" if variant == "invalid" else "YES"

        steps = [step]
        if variant == "aliases":
            state.execution_context["extra"] = {"injected": True}
            steps.insert(0, RecordTargetStep(input_key="extra", output_list_key="template_value"))
        ops, _ = retain(state, steps, select=choose)
        yield "utility-repeat-" + variant, state, ops

    from primary_templates import EFFECTS

    from goa2.domain.models import CardState, TeamColor
    from goa2.engine.steps import ResolveCardTextStep

    effects = (
        "noble_blade",
        "onslaught",
        "hit_and_run",
        "leaping_strike",
        "blink_strike",
        "mad_dash",
        "troop_movement",
        "marching_orders",
        "path_to_victory",
        "shootout",
        "bullet_hell",
        "quickdraw",
        "gunslinger",
        "dead_shot",
    )
    for effect in effects:
        for mode in ("first", "skip"):
            name = EFFECTS[effect]
            foe = "Arien" if name == "Brogan" else "Brogan"
            state = setup([name, "Swift"], [foe, "Hanu"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            hero = state.get_hero("hero_" + name.lower())
            state.current_actor_id = str(hero.id)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state = CardState.UNRESOLVED
            card.is_facedown = False
            if effect == "onslaught":
                from goa2.domain.models import StatType

                hero.items[StatType.ATTACK] = 10
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_" + foe.lower(), h(1, 0))
            state.place_entity("hero_swift", h(0, 1))
            state.place_entity("hero_hanu", h(-1, 1))
            for minion, point in zip(
                state.teams[TeamColor.RED].minions, [h(-1, 0), h(0, -1), h(-2, 0)], strict=False
            ):
                state.place_entity(minion.id, point)
            state.place_entity(state.teams[TeamColor.BLUE].minions[0].id, h(1, -1))
            state.execution_context.update(
                hero_alias=hero, current_alias=card, locations_alias=state.entity_locations
            )
            ops, _ = retain(
                state, [ResolveCardTextStep(hero_id=str(hero.id), card_id="ignored")], mode
            )
            yield "utility-card-" + effect + "-" + mode, state, ops

    for kind in ("record", "compute", "adjacency"):
        for piece in (False, True):
            state = setup(["Razzle", "Swift"], ["Brogan", "Hanu"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            state.current_actor_id = "hero_razzle"
            state.acting_piece_id = "hero_razzle_piece_1"
            state.place_entity("hero_razzle_piece_1", h(0, 0))
            state.place_entity("hero_brogan", h(1, 0))
            unit = "hero_razzle_piece_1" if piece else "hero_razzle"
            state.execution_context.update(unit=unit, target="hero_brogan", output="unchanged")
            if kind == "record":
                step = RecordHexStep(unit_key="unit", output_key="output")
            elif kind == "compute":
                step = ComputeHexStep(origin_key="unit", target_key="target", output_key="output")
            else:
                step = CheckAdjacencyStep(
                    unit_a_key="unit", unit_b_id="hero_brogan", output_key="output"
                )
            ops, _ = retain(state, [step])
            yield "utility-multipiece-" + kind + "-" + ("piece" if piece else "owner"), state, ops

    from goa2.engine.steps import ForEachStep

    state, _ = base()
    inner = MayRepeatOnceStep(
        steps_template=[
            SetContextFlagStep(key="snapshot_a", value=[{"nested": []}]),
            SetContextFlagStep(key="snapshot_b", value=[]),
            RecordTargetStep(input_key="snapshot_a", output_list_key="history"),
            RecordTargetStep(input_key="snapshot_b", output_list_key="history"),
            AddContextValueStep(key="counter", amount=1),
        ]
    )
    outer = ForEachStep(list_key="items", item_key="item", steps_template=[inner])
    stored = outer.steps_template[0].steps_template
    stored[1].value = stored[0].value
    state.execution_context.update(
        items=[1], repeat_alias=outer.steps_template[0], template_value=stored[0].value
    )
    ops, _ = retain(state, [outer], select=lambda request, mode: "YES")
    yield "utility-repeat-in-foreach", state, ops
