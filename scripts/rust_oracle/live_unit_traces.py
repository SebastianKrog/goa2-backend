"""Live unit selectors reuse the frozen targeting/equivalence state matrix."""

from __future__ import annotations


def scenes():
    import copy

    from object_execution_traces import live_choice_operations
    from targeting_rules import states

    from goa2.domain.models import ActionType, CardColor, MinionType, TargetType
    from goa2.engine.filters import (
        AdjacencyFilter,
        AdjacencyToContextFilter,
        AndFilter,
        CanBeMovedByActorFilter,
        CanBePlacedByActorFilter,
        ContextIdsFilter,
        ExcludeIdentityFilter,
        HasEmptyNeighborFilter,
        HeroPieceFilter,
        ImmunityFilter,
        MinionTypesFilter,
        OrFilter,
        PlayedCardFilter,
        RangeFilter,
        SpaceBehindEmptyFilter,
        TeamFilter,
        TokenTypeFilter,
        UnitTypeFilter,
    )
    from goa2.engine.steps import RecordTargetStep, SelectStep, SetActorStep, SetContextFlagStep

    def retain(state, step, mode="first", steps=None):
        state.execution_stack = list(reversed(steps or [step]))
        state.execution_context.update(
            selector=step, filters_alias=step.filters, pending_alias=step.pending_input
        )
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    bases = {}
    for name, state in states():
        if name in {"plain", "actor-piece", "split", "heavy", "attack-basic"}:
            bases[name] = copy.deepcopy(state)
        for mode in ("unit", "tokens", "override"):
            current = copy.deepcopy(state)
            step = SelectStep(
                step_id="fixture/live-unit/select",
                target_type=TargetType.UNIT_OR_TOKEN if mode == "tokens" else TargetType.UNIT,
                prompt="Choose unit",
                output_key="chosen",
                is_mandatory=False,
                override_player_id="hero_mrak" if mode == "override" else None,
            )
            yield f"unit-world-{name}-{mode}", current, retain(
                current, step, "invalid" if mode == "override" else "last"
            )

    for name, base in bases.items():
        for index, condition in enumerate(
            [
                TeamFilter(relation="FRIENDLY"),
                TeamFilter(relation="ENEMY"),
                TeamFilter(relation="SELF"),
                UnitTypeFilter(unit_type="HERO"),
                UnitTypeFilter(unit_type="MINION"),
                UnitTypeFilter(unit_type="TOKEN"),
                RangeFilter(origin_key="near", max_range=2),
                RangeFilter(origin_hex_key="from_hex", max_range=3),
                ExcludeIdentityFilter(exclude_self=False, exclude_keys=["excluded", "history"]),
                ImmunityFilter(),
                ContextIdsFilter(ids_key="allowed"),
                HeroPieceFilter(exclude_acting=False),
                TokenTypeFilter(),
                MinionTypesFilter(minion_types=[MinionType.MELEE, MinionType.RANGED]),
                PlayedCardFilter(action_type=ActionType.ATTACK),
                PlayedCardFilter(card_color=CardColor.GOLD),
                AdjacencyFilter(target_tags=["ENEMY", "MINION"]),
                AdjacencyToContextFilter(target_key="near"),
                SpaceBehindEmptyFilter(origin_key="near"),
                HasEmptyNeighborFilter(),
                CanBeMovedByActorFilter(),
                CanBePlacedByActorFilter(),
                OrFilter(
                    filters=[TeamFilter(relation="ENEMY"), UnitTypeFilter(unit_type="MINION")]
                ),
                AndFilter(
                    filters=[
                        UnitTypeFilter(unit_type="HERO"),
                        RangeFilter(origin_key="near", max_range=3),
                    ]
                ),
                OrFilter(),
                AndFilter(),
            ]
        ):
            state = copy.deepcopy(base)
            state.execution_context.update(
                excluded="hero_mrak",
                history=["hero_hanu"],
                allowed=["hero_mrak", "hero_hanu", "illusion-0"],
                from_hex=next(iter(state.board.tiles)),
            )
            step = SelectStep(
                step_id="fixture/live-unit/filter",
                target_type=TargetType.UNIT_OR_TOKEN,
                prompt="Unit predicate",
                output_key="chosen",
                filters=[condition],
                skip_self_filter=True,
                is_mandatory=False,
            )
            yield f"unit-filter-{name}-{index}", state, retain(state, step)

    for variant in (
        "auto",
        "dirty-auto",
        "pending",
        "invalid",
        "skip",
        "no-candidates",
        "mandatory-empty",
        "owner-key",
        "empty-literal",
        "piece-owner",
        "no-actor",
        "null",
        "bool",
        "dict",
    ):
        state = copy.deepcopy(bases["plain"])
        step = SelectStep(
            step_id="fixture/live-unit/input",
            target_type=TargetType.UNIT,
            prompt="Unit input",
            output_key="chosen",
            is_mandatory=variant in {"auto", "dirty-auto", "mandatory-empty"},
            auto_select_if_one=variant in {"auto", "dirty-auto"},
        )
        state.execution_context.update(chosen="old", who="hero_mrak")
        if variant in {"auto", "dirty-auto"}:
            step.filters = [ContextIdsFilter(ids_key="allowed")]
            state.execution_context["allowed"] = ["hero_mrak"]
        if variant in {"no-candidates", "mandatory-empty"}:
            step.filters = [ContextIdsFilter(ids_key="allowed")]
            state.execution_context["allowed"] = []
        if variant in {"owner-key", "empty-literal"}:
            step.override_player_id_key = "who"
        if variant == "empty-literal":
            step.override_player_id = ""
        if variant == "piece-owner":
            step.override_player_id = "hero_razzle_piece_1"
        if variant == "no-actor":
            state.current_actor_id = None
        pending = {
            "dirty-auto": "invalid",
            "pending": "hero_mrak",
            "skip": "SKIP",
            "null": None,
            "bool": False,
            "dict": {"id": "hero_mrak"},
        }
        if variant in pending:
            step.pending_input = {"selection": pending[variant]}
        step.pending_request_id = "fixture/live-unit/old-request"
        yield "unit-input-" + variant, state, retain(
            state, step, "invalid" if variant == "invalid" else "first"
        )

    for variant in ("history", "actor", "replace"):
        state = copy.deepcopy(bases["plain"])
        step = SelectStep(
            step_id="fixture/live-unit/repeated",
            target_type=TargetType.UNIT,
            prompt="Repeated unit",
            output_key="chosen",
            is_mandatory=False,
            filters=[ExcludeIdentityFilter(exclude_keys=["history"])],
        )
        state.execution_context["history"] = []
        state.execution_context["history_alias"] = state.execution_context["history"]
        record = RecordTargetStep(
            step_id="fixture/live-unit/record", input_key="chosen", output_list_key="history"
        )
        middle = []
        if variant == "actor":
            middle = [SetActorStep(step_id="fixture/live-unit/actor", actor_id="hero_mrak")]
        if variant == "replace":
            middle = [
                SetContextFlagStep(
                    step_id="fixture/live-unit/replace", key="history", value=["hero_hanu"]
                )
            ]
        yield "unit-live-" + variant, state, retain(
            state, step, steps=[step, record, *middle, step]
        )
