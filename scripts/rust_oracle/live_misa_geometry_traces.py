"""Reusable distance and between-segment predicates against the frozen oracle."""


def positioned(split=False):
    from combat_effect_traces import effect
    from live_misa_primary_traces import base

    from goa2.domain.hex import Hex
    from goa2.domain.models.effect import EffectType

    state, hero, _ = base("power_shot")
    for unit in list(state.entity_locations):
        state.remove_entity(unit)
    for unit, q, r in (
        (hero.id, -2, 0),
        ("hero_gydion", -1, 1),
        ("hero_arien", 2, 0),
        ("hero_tigerclaw", 1, 1),
    ):
        state.place_entity(unit, Hex(q=q, r=r, s=-q - r))
    if split:
        state.active_effects.append(
            effect(
                state,
                "misa-split",
                EffectType.TOPOLOGY_SPLIT,
                source=hero.id,
                split_axis="q",
                split_value=0,
            )
        )
    return state, hero


def relative_scenes():
    from live_defeat_traces import retain

    from goa2.engine.filters_geometry import RelativeDistanceFilter
    from goa2.engine.steps import CheckUnitFiltersStep, CountStep

    configs = (
        ("finite", False, "hero_arien", "hero_gydion"),
        ("both-infinite", True, "hero_arien", "hero_tigerclaw"),
        ("reference-infinite", True, "hero_arien", "hero_gydion"),
        ("candidate-infinite", True, "hero_gydion", "hero_arien"),
    )
    for target in ("UNIT", "HEX"):
        for name, split, reference, candidate in configs:
            for operator in (">", ">=", "==", "<=", "<"):
                state, _ = positioned(split)
                predicate = RelativeDistanceFilter(
                    reference_key="relative_reference", operator=operator
                )
                state.execution_context.update(
                    relative_reference=reference,
                    relative_candidate=candidate,
                    relative_alias=predicate,
                )
                step = (
                    CheckUnitFiltersStep(
                        unit_key="relative_candidate",
                        filters=[predicate],
                        output_key="relative_result",
                    )
                    if target == "UNIT"
                    else CountStep(
                        target_type="HEX", filters=[predicate], output_key="relative_result"
                    )
                )
                ops, _ = retain(state, [step])
                yield f"misa-geometry-relative-{target}-{name}-{operator}", state, ops
    for variant in (
        "literal",
        "key",
        "false-key",
        "unknown-key",
        "numeric-key",
        "missing-reference",
        "missing-origin",
        "no-actor",
    ):
        state, hero = positioned()
        predicate = RelativeDistanceFilter(reference_key="relative_reference", operator="<")
        state.execution_context.update(
            relative_reference="hero_arien",
            relative_candidate="hero_gydion",
            relative_origin=hero.id,
        )
        if variant == "literal":
            predicate.origin_id = hero.id
            predicate.origin_key = "bad-key"
            state.execution_context["bad-key"] = []
        elif variant in ("key", "false-key", "unknown-key", "numeric-key"):
            predicate.origin_key = "relative_origin"
            if variant == "false-key":
                state.execution_context["relative_origin"] = 0
            if variant == "unknown-key":
                state.execution_context["relative_origin"] = "absent"
            if variant == "numeric-key":
                state.execution_context["relative_origin"] = 999
        elif variant == "missing-reference":
            state.execution_context.pop("relative_reference")
        elif variant == "missing-origin":
            state.remove_entity(hero.id)
        elif variant == "no-actor":
            state.current_actor_id = None
        state.execution_context["relative_alias"] = predicate
        step = CheckUnitFiltersStep(
            unit_key="relative_candidate", filters=[predicate], output_key="relative_result"
        )
        ops, _ = retain(state, [step])
        yield "misa-geometry-relative-" + variant, state, ops


def distance_scenes():
    from live_defeat_traces import retain

    from goa2.engine.steps import CheckDistanceStep

    for split in (False, True):
        for operator in (">", ">=", "==", "<=", "<", "!=", "invalid"):
            for threshold in (1, 4):
                state, hero = positioned(split)
                step = CheckDistanceStep(
                    unit_a_id=hero.id,
                    unit_b_id="hero_arien",
                    operator=operator,
                    threshold=threshold,
                    output_key="distance_result",
                )
                ops, _ = retain(state, [step])
                yield f"misa-geometry-distance-{split}-{operator}-{threshold}", state, ops
    for variant in (
        "keys",
        "literal-wins",
        "empty-literal",
        "missing-key",
        "false-key",
        "unknown",
        "offboard",
        "skip",
    ):
        state, hero = positioned()
        state.execution_context.update(
            distance_a=hero.id, distance_b="hero_arien", distance_result="retained"
        )
        step = CheckDistanceStep(
            unit_a_key="distance_a",
            unit_b_key="distance_b",
            operator="==",
            threshold=4,
            output_key="distance_result",
        )
        if variant == "literal-wins":
            step.unit_a_id = hero.id
            state.execution_context["distance_a"] = []
        elif variant == "empty-literal":
            step.unit_a_id = ""
        elif variant == "missing-key":
            state.execution_context.pop("distance_b")
        elif variant == "false-key":
            state.execution_context["distance_b"] = 0
        elif variant == "unknown":
            state.execution_context["distance_b"] = 999
        elif variant == "offboard":
            state.remove_entity("hero_arien")
        elif variant == "skip":
            step.skip_if_key = "distance_result"
        state.execution_context["distance_step_alias"] = step
        ops, _ = retain(state, [step])
        yield "misa-geometry-distance-" + variant, state, ops


def between_scenes():
    from live_defeat_traces import retain

    from goa2.domain.hex import Hex
    from goa2.engine.filters_geometry import BetweenHexesFilter
    from goa2.engine.steps import CheckUnitFiltersStep, CountStep

    for target in ("UNIT", "HEX"):
        for variant in (
            "models",
            "dicts",
            "numeric-strings",
            "booleans",
            "missing",
            "bad-sum",
            "not-a-hex",
            "nonstraight",
            "coincident",
            "off-map",
            "endpoint",
            "topology-split",
        ):
            state, hero = positioned(variant == "topology-split")
            start, end = Hex(q=-2, r=0, s=2), Hex(q=2, r=0, s=-2)
            candidate = "hero_gydion"
            state.place_entity(candidate, Hex(q=0, r=0, s=0))
            if variant != "models":
                start, end = start.model_dump(), end.model_dump()
            if variant == "numeric-strings":
                start = {k: str(v) for k, v in start.items()}
            if variant == "booleans":
                start = {"q": False, "r": False, "s": False}
            if variant == "missing":
                start = {"q": 0}
            if variant == "bad-sum":
                start = {"q": 1, "r": 1, "s": 1}
            if variant == "not-a-hex":
                start = "(-2,0,2)"
            if variant == "nonstraight":
                end = {"q": 2, "r": 1, "s": -3}
            if variant == "coincident":
                end = start
            if variant == "off-map":
                start = {"q": -8, "r": 0, "s": 8}
            if variant == "endpoint":
                candidate = hero.id
            predicate = BetweenHexesFilter(from_hex_key="between_from", to_hex_key="between_to")
            state.execution_context.update(
                between_from=start,
                between_to=end,
                between_candidate=candidate,
                between_alias=predicate,
            )
            step = (
                CheckUnitFiltersStep(
                    unit_key="between_candidate", filters=[predicate], output_key="between_result"
                )
                if target == "UNIT"
                else CountStep(target_type="HEX", filters=[predicate], output_key="between_result")
            )
            ops, _ = retain(state, [step])
            yield f"misa-geometry-between-{target}-{variant}", state, ops


def scenes():
    yield from relative_scenes()
    yield from distance_scenes()
    yield from between_scenes()
    yield from adjacency_scenes()


def adjacency_scenes():
    from combat_effect_traces import effect
    from live_defeat_traces import retain

    from goa2.domain.hex import Hex
    from goa2.domain.models.effect import EffectType
    from goa2.engine.filters_units import AdjacencyToContextFilter
    from goa2.engine.steps import CheckUnitFiltersStep, CountStep

    for target in ("UNIT", "HEX"):
        for variant in (
            "ordinary",
            "unknown",
            "falsey",
            "off-map-target",
            "off-map-candidate",
            "isolation-blocked",
            "isolation-source",
        ):
            state, _ = positioned()
            reference, candidate = "hero_arien", "hero_tigerclaw"
            if variant.startswith("isolation-"):
                state.active_effects.append(
                    effect(
                        state,
                        "misa-isolation",
                        EffectType.TOPOLOGY_ISOLATION,
                        source="hero_arien",
                        split_axis="q",
                        split_value=2,
                        isolated_hex=Hex(q=2, r=0, s=-2),
                    )
                )
                if variant == "isolation-source":
                    reference, candidate = candidate, reference
            if variant == "unknown":
                reference = "absent"
            if variant == "falsey":
                reference = 0
            if variant == "off-map-target":
                state.entity_locations["hero_arien"] = Hex(q=8, r=0, s=-8)
            if variant == "off-map-candidate":
                state.entity_locations["hero_tigerclaw"] = Hex(q=7, r=0, s=-7)
            predicate = AdjacencyToContextFilter(target_key="adjacent_target")
            state.execution_context.update(
                adjacent_target=reference, adjacent_candidate=candidate, adjacent_alias=predicate
            )
            step = (
                CheckUnitFiltersStep(
                    unit_key="adjacent_candidate", filters=[predicate], output_key="adjacent_result"
                )
                if target == "UNIT"
                else CountStep(target_type="HEX", filters=[predicate], output_key="adjacent_result")
            )
            ops, _ = retain(state, [step])
            yield f"misa-geometry-adjacency-{target}-{variant}", state, ops
