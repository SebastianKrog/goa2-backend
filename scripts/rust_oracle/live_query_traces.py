"""Source handler query/condition boundaries, aliases, and real card prefixes."""

from __future__ import annotations


def scenes():
    import copy

    from live_passive_traces import build
    from live_spatial_traces import fast_travel_scenes
    from object_execution_traces import live_choice_operations
    from targeting_rules import states

    from goa2.domain.models import CardState, TargetType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.filters import (
        ContextIdsFilter,
        HasTurnStartPositionFilter,
        RangeFilter,
        UnitTypeFilter,
    )
    from goa2.engine.steps import (
        CheckContextConditionStep,
        CollectUnitsStep,
        CountStep,
        ForEachStep,
        RecordTargetStep,
        SetActorStep,
    )

    def retain(state, steps):
        state.execution_stack = list(reversed(steps))
        state.execution_context["query_steps"] = steps
        return [
            *live_choice_operations(state, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    bases = {}
    for name, base in states():
        if name in {"plain", "heavy", "actor-piece"}:
            bases[name] = copy.deepcopy(base)
        for target in (TargetType.UNIT, TargetType.UNIT_OR_TOKEN):
            state = copy.deepcopy(base)
            filters = [UnitTypeFilter(unit_type="MINION")]
            count = CountStep(
                step_id="fixture/query/count",
                target_type=target,
                filters=filters,
                output_key="count",
            )
            collect = CollectUnitsStep(
                step_id="fixture/query/collect", target_type=target, output_key="units"
            )
            collect.filters = count.filters
            state.execution_context.update(count="old", units=[], filters_alias=count.filters)
            state.execution_context["old_units"] = state.execution_context["units"]
            yield f"query-world-{name}-{target.value}", state, retain(state, [count, collect])

    for name, base in bases.items():
        for flags in range(4):
            state = copy.deepcopy(base)
            steps = []
            for i, cls in enumerate((CountStep, CollectUnitsStep)):
                steps.append(
                    cls(
                        step_id=f"fixture/query/flags/{i}",
                        target_type=TargetType.UNIT_OR_TOKEN,
                        skip_self_filter=bool(flags & 1),
                        skip_immunity_filter=bool(flags & 2),
                        output_key="count" if i == 0 else "units",
                    )
                )
            yield f"query-flags-{name}-{flags}", state, retain(state, steps)

    for target in (TargetType.HEX, TargetType.CARD, TargetType.NUMBER, TargetType.TOKEN):
        state = copy.deepcopy(bases["plain"])
        filters = (
            [RangeFilter(max_range=2)]
            if target == TargetType.HEX
            else [HasTurnStartPositionFilter()]
        )
        steps = [
            CountStep(
                step_id="fixture/query/empty/count",
                target_type=target,
                filters=filters,
                output_key="count",
            ),
            CollectUnitsStep(
                step_id="fixture/query/empty/collect",
                target_type=target,
                filters=filters,
                output_key="units",
            ),
        ]
        yield "query-target-" + target.value, state, retain(state, steps)

    for name, state, _operations in fast_travel_scenes():
        selector = state.execution_stack[-1]
        step = CountStep(
            step_id="fixture/query/hex-travel",
            target_type=TargetType.HEX,
            filters=selector.filters,
            output_key="count",
        )
        yield "query-" + name, state, retain(state, [step])

    for kind in ("count", "collect"):
        for gate in ("active", "skip-false", "skip-null"):
            state = copy.deepcopy(bases["plain"])
            old = [] if kind == "collect" else 17
            state.execution_context.update(
                result=old, old_result=old, skip=False if gate == "skip-false" else None
            )
            cls = CollectUnitsStep if kind == "collect" else CountStep
            step = cls(
                step_id="fixture/query/gated",
                output_key="result",
                active_if_key="missing" if gate == "active" else None,
                skip_if_key="skip" if gate.startswith("skip") else None,
                filters=[HasTurnStartPositionFilter()] if gate != "skip-null" else [],
            )
            yield f"query-gated-{kind}-{gate}", state, retain(state, [step])

    for variant in ("collect-repeat", "collect-loop", "count-history"):
        state = copy.deepcopy(bases["plain"])
        state.execution_context.update(
            history=["hero_mrak"], target="hero_hanu", snapshots=[], seen=[]
        )
        if variant == "count-history":
            step = CountStep(
                step_id="fixture/query/history",
                filters=[ContextIdsFilter(ids_key="history")],
                output_key="count",
            )
            record = RecordTargetStep(
                step_id="fixture/query/save-count", input_key="count", output_list_key="snapshots"
            )
            append = RecordTargetStep(
                step_id="fixture/query/append", input_key="target", output_list_key="history"
            )
            steps = [step, record, append, step]
        else:
            step = CollectUnitsStep(step_id="fixture/query/repeated", output_key="units")
            record = RecordTargetStep(
                step_id="fixture/query/save-units", input_key="units", output_list_key="snapshots"
            )
            steps = [
                step,
                record,
                SetActorStep(step_id="fixture/query/actor", actor_id="hero_mrak"),
                step,
            ]
            if variant == "collect-loop":
                steps.append(
                    ForEachStep(
                        step_id="fixture/query/loop",
                        list_key="units",
                        item_key="item",
                        steps_template=[
                            RecordTargetStep(
                                step_id="fixture/query/seen",
                                input_key="item",
                                output_list_key="seen",
                            )
                        ],
                    )
                )
        yield "query-live-" + variant, state, retain(state, steps)

    values = [
        None,
        False,
        True,
        -3,
        0,
        1,
        1.25,
        -0.0,
        9007199254740992.0,
        9223372036854775808.0,
        "2",
        [],
        {},
        [1],
    ]
    for index, value in enumerate(values):
        state = copy.deepcopy(bases["plain"])
        state.execution_context["value"] = value
        threshold = 9007199254740993 if index == 8 else 9223372036854775807 if index == 9 else 1
        steps = [
            CheckContextConditionStep(
                step_id=f"fixture/query/condition/{i}",
                input_key="value",
                operator=operator,
                threshold=threshold,
                output_key=f"result{i}",
            )
            for i, operator in enumerate((">=", ">", "==", "<=", "<", "!=", "unknown"))
        ]
        yield f"query-condition-{index}", state, retain(state, steps)

    for hero_name, effect_id in (
        ("Xargatha", "fresh_converts"),
        ("Xargatha", "devoted_followers"),
        ("Sabina", "steady_advance"),
    ):
        for population in range(3):
            state, hero = build(hero_name)
            card = next(c for c in [*hero.hand, *hero.deck] if c.effect_id == effect_id)
            card.is_facedown = False
            card.state = CardState.UNRESOLVED
            hero.current_turn_card = card
            if hero_name == "Sabina":
                team = state.teams[hero.team]
            else:
                team = next(t for t in state.teams.values() if t.color != hero.team)
            origin = state.get_position(hero.id)
            for minion, point in zip(
                team.minions[:population], list(origin.neighbors())[:population], strict=True
            ):
                state.place_entity(minion.id, point)
            body = CardEffectRegistry.get(effect_id).get_steps(state, hero, card)
            assert [step.type.value for step in body[:2]] == ["count", "check_context_condition"]
            prefix = body[:2]
            for i, step in enumerate(prefix):
                step.step_id = f"fixture/query/card/{i}"
            yield f"query-card-{effect_id}-{population}", state, retain(state, prefix)
