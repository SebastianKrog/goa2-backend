"""Fast-travel arena boundaries, retained coordinates and fresh restrictions.

Raw source construction reuses canonical travel worlds while retaining physical
objects that serialization-only traces cannot distinguish.
"""

from __future__ import annotations

import copy


def captured_worlds():
    from unittest.mock import patch

    import stack_traces
    import travel_traces

    captured = []

    def capture(state):
        captured.append(state)
        return {}

    with (
        patch.object(travel_traces, "export_state", capture),
        patch.object(stack_traces, "reference_trace", return_value=[]),
    ):
        rows = travel_traces.cases(
            {"kind": "advance"},
            lambda value, request_id="current": {
                "kind": "input",
                "request_id": request_id,
                "selection": value,
            },
        )
    assert len(rows) == len(captured)
    return [(row["name"], state) for row, state in zip(rows, captured, strict=True)]


def scenes():
    from live_defeat_traces import operations

    worlds = captured_worlds()
    for name, original in worlds:
        for mode in ("first", "last", "skip", "invalid"):
            state = copy.deepcopy(original)
            state.execution_context.update(
                travel_steps=list(reversed(state.execution_stack)),
                locations_alias=state.entity_locations,
                effects_alias=state.active_effects,
                snapshot_alias=state.last_turn_positions,
            )
            ops, _ = operations(state, mode=mode)
            yield "arena-" + name + "-" + mode, state, [
                *ops,
                {"kind": "advance_prefix", "steps": 0},
            ]

    yield from edge_scenes(dict(worlds)["travel-sequence"])


def edge_scenes(original=None):
    from live_defeat_traces import operations
    from model_traces import setup

    from goa2.domain.hex import Hex
    from goa2.domain.models import ActionType, CardColor, CardState
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.domain.models.enums import DisplacementType
    from goa2.engine.steps import (
        FastTravelSequenceStep,
        FastTravelUnitStep,
        ResolveCardStep,
        SetContextFlagStep,
    )

    if original is None:
        original = dict(captured_worlds())["travel-sequence"]
    for variant in (
        "literal-hex",
        "context-hex",
        "context-dict",
        "literal-wins",
        "snapshot-shared",
        "empty-destination",
        "skip-destination",
        "bad-destination",
        "blocked-movement",
        "blocked-travel",
        "except-current",
        "except-source",
        "except-empty-source",
        "except-physical",
        "physical-null",
        "physical-dict",
        "physical-text",
        "facedown",
        "missing-grant",
        "placement-only",
        "barrier",
        "topology-split",
        "action-gold",
        "action-red",
    ):
        state = copy.deepcopy(original)
        destination = Hex(q=2, r=-1, s=-1)
        step = FastTravelUnitStep(
            unit_id="hero_arien", target_hex_arg=destination, step_id="fixture/travel-edge"
        )
        if variant in (
            "context-hex",
            "context-dict",
            "snapshot-shared",
            "empty-destination",
            "skip-destination",
            "bad-destination",
        ):
            step.target_hex_arg = None
            state.execution_context["target_hex"] = {
                "context-hex": destination,
                "context-dict": destination.model_dump(mode="json"),
                "snapshot-shared": destination,
                "empty-destination": {},
                "skip-destination": "SKIP",
                "bad-destination": "bad",
            }[variant]
        if variant == "literal-wins":
            state.execution_context["target_hex"] = Hex(q=-1, r=-1, s=2)
        if variant == "snapshot-shared":
            state.last_turn_positions = state.entity_locations
        if variant.startswith(("blocked-", "except-", "physical-")) or variant in (
            "facedown",
            "missing-grant",
        ):
            cards = setup().get_hero("hero_arien").hand
            gold = next(card for card in cards if card.color == CardColor.GOLD)
            red = next(card for card in cards if card.color == CardColor.RED)
            gold.is_facedown = False
            red.is_facedown = False
            gold.state = red.state = CardState.UNRESOLVED
            hero = state.get_hero("hero_arien")
            hero.hand = [gold, red]
            hero.deck = [gold, red]
            hero.current_turn_card = gold
            state.execution_context["current_card_id"] = gold.id
            if variant == "except-source":
                step.source_card_id = gold.id
                state.execution_context["current_card_id"] = red.id
            if variant == "except-empty-source":
                step.source_card_id = ""
            if variant.startswith("physical-") or variant == "except-physical":
                step.source_card_id = gold.id
                state.execution_context["card"] = {
                    "except-physical": gold,
                    "physical-null": None,
                    "physical-dict": {"id": gold.id},
                    "physical-text": gold.id,
                }[variant]
            if variant == "facedown":
                gold.is_facedown = True
            if variant == "missing-grant":
                step.source_card_id = "absent-card"
            state.active_effects.append(
                ActiveEffect(
                    id="travel-restriction",
                    source_id="hero_hanu",
                    effect_type=EffectType.MOVEMENT_ZONE,
                    scope=EffectScope(shape=Shape.GLOBAL),
                    duration=DurationType.THIS_TURN,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                    is_active=True,
                    restrictions=[
                        (
                            ActionType.FAST_TRAVEL
                            if variant == "blocked-travel"
                            else ActionType.MOVEMENT
                        )
                    ],
                    except_card_colors=[] if variant.startswith("blocked-") else [CardColor.GOLD],
                )
            )
            if variant in (
                "blocked-movement",
                "except-current",
                "except-source",
                "except-empty-source",
                "except-physical",
                "physical-null",
                "physical-dict",
                "physical-text",
                "facedown",
                "missing-grant",
            ):
                step = FastTravelSequenceStep(
                    unit_id="hero_arien",
                    source_card_id=step.source_card_id,
                    step_id="fixture/travel-edge",
                )
                state.execution_context["target_hex"] = destination
        if variant in ("placement-only", "barrier", "topology-split"):
            state.active_effects.append(
                ActiveEffect(
                    id="travel-spatial",
                    source_id="hero_hanu",
                    effect_type={
                        "placement-only": EffectType.PLACEMENT_PREVENTION,
                        "barrier": EffectType.STATIC_BARRIER,
                        "topology-split": EffectType.TOPOLOGY_SPLIT,
                    }[variant],
                    scope=EffectScope(
                        shape=Shape.RADIUS if variant == "barrier" else Shape.GLOBAL, range=2
                    ),
                    duration=DurationType.THIS_TURN,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                    is_active=True,
                    displacement_blocks=(
                        [DisplacementType.PLACE] if variant == "placement-only" else []
                    ),
                    split_axis="q" if variant == "topology-split" else None,
                    split_value=1 if variant == "topology-split" else 0,
                )
            )
        if variant.startswith("action-"):
            cards = setup().get_hero("hero_arien").hand
            hero = state.get_hero("hero_arien")
            hero.hand = cards
            hero.deck = cards
            card = next(
                card
                for card in cards
                if card.color == (CardColor.GOLD if variant.endswith("gold") else CardColor.RED)
            )
            card.is_facedown = False
            card.state = CardState.UNRESOLVED
            hero.current_turn_card = card
            step = ResolveCardStep(hero_id="hero_arien", step_id="fixture/travel-action")
        tail = SetContextFlagStep(
            key="travel_continued", value=True, step_id="fixture/travel-edge-tail"
        )
        state.execution_stack = [tail, step]
        state.execution_context.update(
            travel_steps=[step, tail],
            destination_alias=destination,
            locations_alias=state.entity_locations,
            snapshot_alias=state.last_turn_positions,
            effects_alias=state.active_effects,
        )

        def select(request, _mode):
            from goa2.domain.input import selection_value

            if request.request_type.value == "CHOOSE_ACTION":
                return "FAST_TRAVEL"
            return selection_value(request.options[0])

        ops, _ = operations(state, select=select)
        yield "arena-travel-edge-" + variant, state, [*ops, {"kind": "advance_prefix", "steps": 0}]
