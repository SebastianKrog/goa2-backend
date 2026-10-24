"""Count/collect/played-card and exact scalar comparison reference traces."""

from __future__ import annotations


def cases(advance, answer):
    from aura_traces import _board, _pieces
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.board import Zone
    from goa2.domain.models import (
        ActionType,
        GamePhase,
        TargetType,
        TeamColor,
        Token,
        TokenType,
    )
    from goa2.engine.filters import (
        ExcludeIdentityFilter,
        ImmunityFilter,
        PlayedCardFilter,
        RangeFilter,
        TeamFilter,
        UnitTypeFilter,
    )
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        CheckContextConditionStep,
        CollectUnitsStep,
        CountStep,
        SwapUnitsStep,
    )

    result = []
    variants = [
        "all",
        "self",
        "token",
        "token-immune",
        "collect-immune-token",
        "collect-immunity-default",
        "hex",
        "hex-range",
        "no-actor",
        "offboard",
        "piece-owner",
        "piece-actor",
        "friendly",
        "immunity-on",
        "immunity-off",
        "self-explicit",
        "skip-active",
        "skip-if-false",
        "skip-if-null",
        "other-target",
        "card-target",
        "token-only",
        "collect",
        "collect-token",
        "collect-hex",
        "collect-token-only",
        "collect-skipped",
        "collect-order",
        "played-none",
        "played-current",
        "played-resolved",
        "played-prior-slot",
        "played-current-facedown",
        "played-resolved-facedown",
        "played-both",
        "played-color",
        "played-combined",
        "played-any-facedown",
        "played-piece",
        "played-hand-current",
    ]
    for variant in variants:
        state = setup(["Sabina", "Arien"], ["Brogan", "Razzle"])
        h = _board(state)
        state.current_actor_id = "hero_sabina"
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        piece = _pieces(state, "hero_razzle")[0]
        red = next(m for m in state.teams[TeamColor.RED].minions if not m.is_heavy)
        blue = next(m for m in state.teams[TeamColor.BLUE].minions if not m.is_heavy)
        for unit, location in [
            ("hero_sabina", h(0, 0)),
            ("hero_arien", h(1, 0)),
            ("hero_brogan", h(-1, 0)),
            (piece, h(-1, 1)),
            (str(red.id), h(1, -1)),
            (str(blue.id), h(0, -1)),
        ]:
            state.place_entity(unit, location)
        token = Token(
            id="query-token",
            name="Query token",
            token_type=TokenType.GRENADE,
            owner_id="hero_sabina",
        )
        state.token_pool.setdefault(TokenType.GRENADE, []).append(token)
        state.misc_entities[str(token.id)] = token
        if variant in ["token-immune", "collect-immune-token"]:
            token.is_immune_to_enemy_actions = True
            token.owner_id = "hero_brogan"
        state.place_entity(token.id, h(0, 1))
        collect = variant.startswith("collect")
        cls = CollectUnitsStep if collect else CountStep
        target_type = (
            TargetType.UNIT_OR_TOKEN
            if variant in ["token", "collect-token", "token-immune", "collect-immune-token"]
            else (
                TargetType.HEX
                if variant in ["hex", "hex-range", "collect-hex"]
                else (
                    TargetType.NUMBER
                    if variant == "other-target"
                    else (
                        TargetType.CARD
                        if variant == "card-target"
                        else (
                            TargetType.TOKEN
                            if variant in ["token-only", "collect-token-only"]
                            else TargetType.UNIT
                        )
                    )
                )
            )
        )
        filters = []
        options = {}
        state.execution_context = {"result": 99, "go": False, "stop": False}
        if variant == "self":
            options["skip_self_filter"] = True
        if variant in ["hex-range"]:
            filters = [RangeFilter(max_range=1)]
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "offboard":
            state.remove_entity("hero_sabina")
        if variant == "piece-owner":
            state.current_actor_id = "hero_razzle"
            state.acting_piece_id = piece
        if variant == "piece-actor":
            state.current_actor_id = piece
            state.acting_piece_id = piece
        if variant == "friendly":
            filters = [TeamFilter(relation="FRIENDLY")]
        if variant == "self-explicit":
            filters = [ExcludeIdentityFilter(exclude_self=True)]
        if variant in ["immunity-on", "immunity-off", "collect-immunity-default"]:
            heavy = next(m for m in state.teams[TeamColor.BLUE].minions if m.is_heavy)
            blue.lane_id = heavy.lane_id = "query-lane"
            state.board.zones = {"query-zone": Zone(id="query-zone", hexes=set(state.board.tiles))}
            state.board.lanes = {"query-lane": ["query-zone"]}
            state.battle_zones = {"query-lane": "query-zone"}
            state.place_entity(heavy.id, h(-2, 0))
            options["skip_immunity_filter"] = variant == "immunity-off"
            filters = [UnitTypeFilter(unit_type="MINION")]
            if variant == "immunity-on":
                filters.append(ImmunityFilter())
        if variant in ["skip-active", "collect-skipped"]:
            options["active_if_key"] = "go"
        if variant in ["skip-if-false", "skip-if-null"]:
            options["skip_if_key"] = "stop"
            if variant == "skip-if-null":
                state.execution_context["stop"] = None
        if variant.startswith("played-"):
            hero = state.get_hero("hero_razzle" if variant == "played-piece" else "hero_brogan")
            attack = next(c for c in hero.hand if c.primary_action == ActionType.ATTACK)
            color = attack.color
            if variant != "played-none":
                if variant == "played-hand-current":
                    hero.current_turn_card = attack
                else:
                    hero.play_card(attack)
                attack.is_facedown = variant in ["played-current-facedown", "played-any-facedown"]
                if variant in [
                    "played-resolved",
                    "played-prior-slot",
                    "played-resolved-facedown",
                    "played-both",
                    "played-combined",
                ]:
                    hero.resolve_current_card(turn_number=1)
                    attack.is_facedown = variant == "played-resolved-facedown"
                if variant == "played-prior-slot":
                    state.turn = 2
                if variant in ["played-both", "played-combined"]:
                    other = next(
                        c
                        for c in hero.hand
                        if c.primary_action != ActionType.ATTACK and c.color != color
                    )
                    hero.play_card(other)
                    other.is_facedown = False
                    if variant == "played-combined":
                        color = other.color
            kwargs = {} if variant == "played-any-facedown" else {"action_type": ActionType.ATTACK}
            if variant in ["played-color", "played-combined"]:
                kwargs["card_color"] = color
            filters = [PlayedCardFilter(**kwargs)]
            if variant == "played-piece":
                other = _pieces(state, hero.id)[1]
                state.place_entity(other, h(-2, 1))
        step = cls(
            target_type=target_type,
            filters=filters,
            output_key="result",
            step_id=f"fixture/query/{variant}",
            **options,
        )
        steps = [step]
        if variant == "collect-order":
            steps.insert(
                0,
                SwapUnitsStep(
                    unit_a_id="hero_arien",
                    unit_b_id="hero_brogan",
                    step_id="fixture/query/order/swap",
                ),
            )
        push_steps(state, steps)
        envelope = export_state(state)
        ops = [advance]
        expected = reference_trace(state, ops)
        value = expected[-1]["semantic_state"]["execution_context"]["result"]
        if variant in ["skip-active", "skip-if-false"]:
            assert value == 99
        if variant == "collect-skipped":
            assert value == 99
        if variant in [
            "played-current",
            "played-resolved",
            "played-both",
            "played-color",
            "played-any-facedown",
            "played-hand-current",
        ]:
            assert value == 1, (variant, value)
        if variant in [
            "played-none",
            "played-prior-slot",
            "played-current-facedown",
            "played-resolved-facedown",
            "played-combined",
        ]:
            assert value == 0, (variant, value)
        if variant == "collect-immunity-default":
            assert len(value) == 2
        if variant == "token-immune":
            assert value == 6
        if variant == "collect-immune-token":
            assert len(value) == 5
        if variant == "played-piece":
            assert value == 2
        if variant == "offboard":
            assert value == 5
        result.append(
            {
                "name": f"query-{variant}",
                "envelope": envelope,
                "operations": ops,
                "expected": expected,
            }
        )
    state = setup()
    state.phase = GamePhase.RESOLUTION
    values = [
        None,
        False,
        True,
        0,
        1,
        -1,
        0.5,
        -0.5,
        -0.0,
        float(2**53),
        float(2**53 + 2),
        float(2**63),
        float(-(2**63)),
        1e300,
        -1e300,
        "17",
        [],
        {},
    ]
    thresholds = [0, 1, -1, 2**53 + 1, 2**53 - 1, 2**63 - 1, -(2**63)]
    operators = [">=", ">", "==", "<=", "<", "!=", "unknown"]
    state.execution_context = {f"value_{i}": v for i, v in enumerate(values)}
    steps = []
    for i, _ in enumerate(values):
        for j, threshold in enumerate(thresholds):
            for k, operator in enumerate(operators):
                steps.append(
                    CheckContextConditionStep(
                        input_key=f"value_{i}",
                        operator=operator,
                        threshold=threshold,
                        output_key=f"result_{i}_{j}_{k}",
                        step_id=f"fixture/compare/{i}/{j}/{k}",
                    )
                )
    push_steps(state, steps)
    result.append(
        {
            "name": "query-scalar-comparisons",
            "envelope": export_state(state),
            "operations": [advance],
            "expected": reference_trace(state, [advance]),
        }
    )
    return result
