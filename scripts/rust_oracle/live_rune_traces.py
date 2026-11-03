"""Source rune selector contracts, including metadata and retry lifecycles."""

from __future__ import annotations


def scenes():
    from live_passive_traces import build
    from object_execution_traces import live_choice_operations

    from goa2.domain.models import RuneType
    from goa2.engine.steps import ChooseRuneStep

    variants = (
        "request",
        "single",
        "empty",
        "duplicates",
        "pending-valid",
        "pending-invalid",
        "pending-null",
        "pending-missing",
        "pending-empty",
        "single-invalid",
        "mapped",
        "unmapped",
        "matches",
        "not-matching",
        "skip-invalid",
        "numeric-invalid",
        "no-actor",
        "piece-actor",
        "skipped",
    )
    for variant in variants:
        state, hero = build("Razzle" if variant == "piece-actor" else "Snorri")
        step = ChooseRuneStep(
            step_id="fixture/rune/main",
            output_key="rune",
            options=[RuneType.BIRD, RuneType.AXE, RuneType.HORN],
            prompt="Choose a rune",
            matches_output_key="matched",
            matching_options=[RuneType.AXE],
            value_output_key="mapped",
            value_map={"axe": "A", "horn": "H"},
        )
        state.execution_context.update(rune="old", matched="old", mapped="old")
        if variant in {"single", "single-invalid"}:
            step.options = [RuneType.AXE]
        if variant == "empty":
            step.options = []
        if variant == "duplicates":
            step.options = [RuneType.AXE, RuneType.AXE]
        if variant in {"pending-valid", "mapped", "matches"}:
            step.pending_input = {"selection": "axe"}
        if variant in {"unmapped", "not-matching"}:
            step.pending_input = {"selection": "bird"}
        if variant in {"pending-invalid", "single-invalid"}:
            step.pending_input = {"selection": "wrong"}
        if variant == "pending-null":
            step.pending_input = {"selection": None}
        if variant == "pending-missing":
            step.pending_input = {"other": True}
        if variant == "pending-empty":
            step.pending_input = {}
        if variant == "skip-invalid":
            step.is_mandatory = False
            step.pending_input = {"selection": "SKIP"}
        if variant == "numeric-invalid":
            step.pending_input = {"selection": 0}
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "piece-actor":
            state.current_actor_id = (
                hero.get_piece_ids()[0] if hasattr(hero, "get_piece_ids") else f"{hero.id}_piece_1"
            )
        if variant == "skipped":
            step.skip_if_key = "skip"
            state.execution_context["skip"] = True
            step.pending_input = {"selection": "axe"}
        step.pending_request_id = "fixture/rune/old-request"
        state.execution_context.update(
            step_alias=step,
            pending_alias=step.pending_input,
            options_alias=step.options,
            matching_alias=step.matching_options,
            value_map_alias=step.value_map,
        )
        state.execution_stack = [step]
        operations = (
            [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance_prefix", "steps": 0}]
            if variant == "empty"
            else live_choice_operations(state, stop_types=())
        )
        if operations[-1] != {"kind": "advance_prefix", "steps": 0}:
            operations.append({"kind": "advance_prefix", "steps": 0})
        yield "rune-" + variant, state, operations
