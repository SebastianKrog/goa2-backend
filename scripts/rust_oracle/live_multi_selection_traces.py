"""Sequential live selection, list identity and source-built turret consumers."""

from __future__ import annotations


def scenes():
    import copy

    from live_passive_traces import build
    from object_execution_traces import live_choice_operations
    from targeting_rules import states

    from goa2.domain.hex import Hex
    from goa2.domain.models import CardState, TargetType, Turret
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.filters import ContextIdsFilter, ObstacleFilter, RangeFilter
    from goa2.engine.steps import MultiSelectStep, SetContextFlagStep

    def retain(state, steps, mode="first"):
        tail = SetContextFlagStep(step_id="fixture/multi/tail", key="tail", value=True)
        state.execution_stack = list(reversed([*steps, tail]))
        first = steps[0]
        state.execution_context.update(
            selector=first,
            selected_alias=first.selections,
            filters_alias=first.filters,
            pending_alias=first.pending_input,
            old_output=state.execution_context.get("chosen"),
        )
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    def step(**kwargs):
        return MultiSelectStep(
            step_id="fixture/multi/select",
            target_type=TargetType.UNIT,
            prompt="Pick several",
            output_key="chosen",
            max_selections=2,
            **kwargs,
        )

    bases = {}
    for name, state in states():
        if name in {"plain", "actor-piece", "split", "heavy"}:
            bases[name] = copy.deepcopy(state)
        for tokens in (False, True):
            current = copy.deepcopy(state)
            selector = step(min_selections=1)
            if tokens:
                selector.target_type = TargetType.UNIT_OR_TOKEN
            yield f"multi-world-{name}-{tokens}", current, retain(
                current, [selector], "invalid" if tokens else "last"
            )

    for variant in (
        "plain",
        "done",
        "skip",
        "done-under",
        "skip-under",
        "preselected",
        "preselected-max",
        "max-zero",
        "max-negative",
        "min-negative",
        "min-above-max",
        "empty",
        "optional-empty",
        "no-actor",
        "empty-actor",
        "no-actor-preselected",
        "shared-output",
        "output-filter",
        "output-absent",
        "empty-output",
        "skip-gate",
        "active-gate",
        "pending-null",
        "pending-false",
        "pending-int",
        "pending-invalid",
        "pending-empty",
        "repeat",
    ):
        for mode in ("first", "last", "invalid"):
            state = copy.deepcopy(bases["plain"])
            selector = step(min_selections=1)
            candidates = selector._get_candidates(state, state.execution_context)
            state.execution_context["chosen"] = ["old"]
            if variant in {"done", "skip", "done-under", "skip-under"}:
                selector.pending_input = {
                    "selection": "SKIP" if variant.startswith("skip") else "DONE"
                }
                if not variant.endswith("under"):
                    selector.selections = [candidates[0]]
            if variant in {"preselected", "preselected-max", "no-actor-preselected"}:
                selector.selections = candidates[: 2 if variant == "preselected-max" else 1]
            if variant in {"max-zero", "max-negative"}:
                selector.max_selections = 0 if variant == "max-zero" else -1
            if variant == "min-negative":
                selector.min_selections = -1
            if variant == "min-above-max":
                selector.min_selections = 3
                selector.max_selections = 1
            if variant in {"empty", "optional-empty"}:
                selector.filters = [ContextIdsFilter(ids_key="allowed")]
                state.execution_context["allowed"] = []
                selector.is_mandatory = variant == "empty"
            if variant in {"no-actor", "empty-actor", "no-actor-preselected"}:
                state.current_actor_id = "" if variant == "empty-actor" else None
            if variant == "shared-output":
                state.execution_context["chosen"] = selector.selections
            if variant == "output-filter":
                state.execution_context["chosen"] = candidates[:3]
                selector.filters = [ContextIdsFilter(ids_key="chosen")]
                selector.min_selections = 2
            if variant == "output-absent":
                state.execution_context.pop("chosen")
            if variant == "empty-output":
                selector.output_key = ""
            if variant == "skip-gate":
                selector.skip_if_key = "gate"
                state.execution_context["gate"] = False
            if variant == "active-gate":
                selector.active_if_key = "absent"
            if variant.startswith("pending-"):
                value = {
                    "pending-null": None,
                    "pending-false": False,
                    "pending-int": 0,
                    "pending-invalid": "invalid",
                    "pending-empty": "",
                }[variant]
                selector.pending_input = {"selection": value}
            steps = [selector, selector] if variant == "repeat" else [selector]
            yield f"multi-{variant}-{mode}", state, retain(state, steps, mode)

    for base, source in bases.items():
        for filter_name in ("plain", "range", "obstacle"):
            for mode in ("first", "last", "invalid"):
                state = copy.deepcopy(source)
                selector = step(min_selections=1)
                selector.target_type = TargetType.HEX
                selector.filters = (
                    []
                    if filter_name == "plain"
                    else (
                        [RangeFilter(max_range=2)] if filter_name == "range" else [ObstacleFilter()]
                    )
                )
                yield f"multi-hex-{base}-{filter_name}-{mode}", state, retain(
                    state, [selector], mode
                )

    for selection in (None, 0.0, -0.0, 1.5, 1e-5, 1e-4, 1e15, 1e16, 1e20, -1e-20):
        state = copy.deepcopy(bases["plain"])
        selector = step(min_selections=1)
        selector.pending_input = {"selection": selection}
        selector.pending_request_id = "fixture/multi/request"
        yield f"multi-injected-request-{selection}", state, retain(state, [selector])

    for target in (TargetType.CARD, TargetType.NUMBER):
        state = copy.deepcopy(bases["plain"])
        selector = step(min_selections=1)
        selector.target_type = target
        yield f"multi-unhandled-{target.value}", state, retain(state, [selector])

    for effect_id in ("self_destruct", "emergency_protocol"):
        for placed in (False, True):
            for mode in ("first", "last", "invalid"):
                state, hero = build("Trinkets")
                turret = Turret(id="trinkets_turret", name="Turret", owner_id=str(hero.id))
                state.register_entity(turret, "board_entity")
                if placed:
                    state.place_entity(turret.id, Hex(q=1, r=0, s=-1))
                state.place_entity("hero_hanu", Hex(q=2, r=0, s=-2))
                state.place_entity("hero_brogan", Hex(q=1, r=1, s=-2))
                card = next(c for c in [*hero.hand, *hero.deck] if c.effect_id == effect_id)
                hero.current_turn_card = card
                card.state = CardState.UNRESOLVED
                card.is_facedown = False
                body = CardEffectRegistry.get(effect_id).get_steps(state, hero, card)
                for index, item in enumerate(body):
                    item.step_id = f"fixture/multi/body/{index}"
                yield f"multi-card-{effect_id}-{placed}-{mode}", state, retain(state, body, mode)


def export(path):
    from object_execution_traces import reference
    from oracle import verify_sources
    from shared_objects import export_state_with_shared_objects
    from stack_traces import write_corpus_stream

    manifest = verify_sources()

    def cases():
        for name, state, operations in scenes():
            envelope, _ = export_state_with_shared_objects(state)
            yield {
                "name": "object-execution-" + name,
                "envelope": envelope,
                "operations": operations,
                "expected": reference(state, operations),
            }

    return write_corpus_stream(
        path,
        {
            "schema_version": 1,
            "oracle_revision": manifest["oracle_revision"],
            "source_sha256": manifest["source_sha256"],
        },
        cases(),
    )
