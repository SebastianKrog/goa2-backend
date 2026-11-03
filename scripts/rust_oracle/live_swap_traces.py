"""Frozen swap execution with retained spatial and step identities.

Raw steps are intentional: these contracts test primitive boundaries and aliases
rather than card authoring. All expected traces come from the frozen handler.
"""

from __future__ import annotations


def scenes():
    from displacement_effect_traces import states
    from live_defeat_traces import retain

    from goa2.engine.steps import PlaceUnitStep, SetActorStep, SetContextFlagStep, SwapUnitsStep

    for variant, state, h, pieces, heavy in states():
        step = SwapUnitsStep(
            unit_a_id="hero_brogan", unit_b_id="hero_hanu", is_mandatory=variant == "mandatory"
        )
        if variant == "same-swap":
            step.unit_b_id = step.unit_a_id
        if variant == "keys":
            step.unit_a_id = step.unit_b_id = None
            step.unit_a_key, step.unit_b_key = "a", "b"
            state.execution_context.update(a="hero_brogan", b="hero_hanu")
        if variant == "context-skip":
            step.unit_a_id, step.unit_a_key = None, "a"
            state.execution_context["a"] = "SKIP"
        if variant == "gate-off":
            step.active_if_key = "absent"
        if variant.startswith("target-"):
            step.unit_a_id = {
                "target-token": "restriction-token",
                "target-token-empty-actor": "restriction-token",
                "target-piece": pieces[0],
                "target-heavy": str(heavy.id),
                "target-owner": "hero_razzle",
                "target-unknown": "unknown",
            }[variant]
        steps = [step]
        if variant in ("live-origin", "live-piece-origin"):
            steps[:0] = [
                PlaceUnitStep(
                    unit_id="hero_arien" if variant == "live-origin" else pieces[0],
                    target_hex_arg=h(-3, 0),
                ),
                SetActorStep(actor_id="hero_brogan"),
            ]
        state.execution_context.update(
            swap_alias=step,
            tiles_alias=state.board.tiles,
            hex_a_alias=state.entity_locations.get(step.unit_a_id),
            hex_b_alias=state.entity_locations.get(step.unit_b_id),
        )
        ops, _ = retain(state, steps)
        yield "live-swap-matrix-" + variant, state, ops

    for variant in (
        "repeated",
        "self-repeated",
        "live-keys",
        "literal-priority",
        "empty-literal",
        "missing-a",
        "missing-b",
        "skip-b",
        "empty-key",
        "offboard-a",
        "offboard-b",
        "snapshot-alias",
        "no-actor",
        "empty-actor",
        "skip-malformed",
        "inactive-malformed",
    ):
        from combat_effect_traces import state_for

        state, _, _ = state_for()
        step = SwapUnitsStep(unit_a_id="hero_arien", unit_b_id="hero_brogan")
        state.execution_context.update(a="hero_arien", b="hero_brogan")
        steps = [step]
        if variant in {"repeated", "self-repeated"}:
            steps.append(step)
        if variant == "self-repeated":
            step.unit_b_id = step.unit_a_id
        if variant in {"live-keys", "empty-literal"}:
            step.unit_a_id, step.unit_b_id = ("" if variant == "empty-literal" else None), None
            step.unit_a_key, step.unit_b_key = "a", "b"
        if variant == "live-keys":
            steps = [
                step,
                SetContextFlagStep(key="a", value="hero_brogan"),
                SetContextFlagStep(key="b", value="hero_arien"),
                step,
            ]
        if variant == "literal-priority":
            step.unit_a_key, step.unit_b_key = "b", "a"
        if variant == "missing-a":
            step.unit_a_id = None
        if variant == "missing-b":
            step.unit_b_id = None
        if variant == "skip-b":
            step.unit_b_id = "SKIP"
        if variant == "empty-key":
            step.unit_a_id, step.unit_a_key = None, ""
            state.execution_context[""] = "hero_arien"
        if variant.startswith("offboard-"):
            state.remove_entity("hero_arien" if variant.endswith("a") else "hero_brogan")
        if variant == "snapshot-alias":
            state.last_turn_positions = state.entity_locations
        if variant in {"no-actor", "empty-actor"}:
            state.current_actor_id = None if variant == "no-actor" else ""
        if variant in {"skip-malformed", "inactive-malformed"}:
            step.unit_a_id = ["malformed"]
            if variant == "skip-malformed":
                step.skip_if_key = "skip"
                state.execution_context["skip"] = True
            else:
                step.active_if_key = "missing"
        state.execution_context.update(
            swap_alias=step,
            tiles_alias=state.board.tiles,
            hex_a_alias=state.entity_locations.get("hero_arien"),
            hex_b_alias=state.entity_locations.get("hero_brogan"),
        )
        ops, _ = retain(state, steps)
        yield "live-swap-alias-" + variant, state, ops
