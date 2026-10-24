"""Conditional aura and live nested-card context reference cases."""

from __future__ import annotations


def states():
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.domain.models import ActionType, CardState

    variants = [
        "basic",
        "nonbasic",
        "silver",
        "no-current",
        "level-seven",
        "ultimate-facedown",
        "card-facedown",
        "resolved-ultimate",
        "alias-ultimate",
        "ctx-basic",
        "ctx-nonbasic",
        "ctx-missing",
        "ctx-wrong-owner",
        "ctx-owner-preferred",
        "ctx-global",
        "ctx-piece-owner",
        "ctx-facedown",
        "ctx-empty",
        "ctx-zero",
        "ctx-false",
        "ctx-list",
        "ctx-dict",
        "piece-aura",
    ]
    result = []
    for variant in variants:
        state = setup(["Sabina", "Arien"], ["Razzle", "Brogan"])
        h = _board(state)
        hero = state.get_hero("hero_sabina")
        hero.level = 8
        state.place_entity(hero.id, h(0, 0))
        state.current_actor_id = hero.id
        basic = next(c for c in hero.hand if c.is_basic and c.primary_action == ActionType.ATTACK)
        other = next(c for c in hero.hand if c.is_basic and c.primary_action != ActionType.ATTACK)
        nonbasic = next(
            c for c in hero.hand if not c.is_basic and c.primary_action == ActionType.ATTACK
        )
        current = (
            nonbasic
            if variant in ["nonbasic", "ctx-basic", "ctx-global", "ctx-facedown"]
            else other if variant == "silver" else basic
        )
        if variant != "no-current":
            hero.play_card(current)
            current.is_facedown = variant == "card-facedown"
        if variant == "level-seven":
            hero.level = 7
        if variant == "ultimate-facedown":
            hero.ultimate_card.is_facedown = True
        if variant in ["resolved-ultimate", "alias-ultimate"]:
            hero.played_cards = [hero.ultimate_card]
            hero.ultimate_card.state = CardState.RESOLVED
            if variant == "resolved-ultimate":
                hero.level = 1
        if variant.startswith("ctx-"):
            card = nonbasic if variant == "ctx-nonbasic" else basic
            state.execution_context = {
                "performing_card_id": str(card.id),
                "performing_card_owner_id": str(hero.id),
            }
            if variant == "ctx-missing":
                state.execution_context["performing_card_id"] = "missing"
            if variant == "ctx-wrong-owner":
                state.execution_context["performing_card_owner_id"] = "hero_arien"
            if variant == "ctx-global":
                state.execution_context.pop("performing_card_owner_id")
            if variant == "ctx-facedown":
                card.is_facedown = True
            if variant in ["ctx-owner-preferred", "ctx-piece-owner"]:
                owner = state.get_hero(
                    "hero_arien" if variant == "ctx-owner-preferred" else "hero_razzle"
                )
                clone = basic.model_copy(deep=True)
                clone.primary_action = ActionType.SKILL
                clone.primary_action_value = None
                owner.deck.append(clone)
                state.execution_context["performing_card_owner_id"] = (
                    str(owner.id)
                    if variant == "ctx-owner-preferred"
                    else _pieces(state, owner.id)[0]
                )
            if variant in ["ctx-empty", "ctx-zero", "ctx-false", "ctx-list", "ctx-dict"]:
                state.execution_context["performing_card_id"] = {
                    "ctx-empty": "",
                    "ctx-zero": 0,
                    "ctx-false": False,
                    "ctx-list": [],
                    "ctx-dict": {},
                }[variant]
        if variant == "piece-aura":
            owner = state.get_hero("hero_razzle")
            owner.level = 8
            owner.ultimate_card = hero.ultimate_card.model_copy(deep=True)
            card = basic.model_copy(deep=True)
            card.state = CardState.UNRESOLVED
            owner.deck.append(card)
            owner.current_turn_card = card
            state.place_entity(_pieces(state, owner.id)[0], h(1, 0))
        result.append((f"aura-conditional-{variant}", state))
    return result


def cases(advance, answer):
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import ActionType, GamePhase, StatType, TeamColor
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.stats import get_computed_stat
    from goa2.engine.steps import AttackSequenceStep, ResolveCardStep, SetContextFlagStep

    result = []
    selected = {
        "basic",
        "nonbasic",
        "silver",
        "no-current",
        "level-seven",
        "ultimate-facedown",
        "card-facedown",
        "ctx-basic",
        "ctx-nonbasic",
        "ctx-missing",
        "ctx-wrong-owner",
        "ctx-owner-preferred",
        "ctx-global",
        "ctx-piece-owner",
        "ctx-empty",
    }
    data = [
        (name, state, False)
        for name, state in states()
        if name.removeprefix("aura-conditional-") in selected
    ]
    # Live register changes must be observed by each later attack, not cached at import.
    dynamic = next(state for name, state in states() if name == "aura-conditional-nonbasic")
    data.append(("aura-conditional-changing-context", dynamic, True))
    menu = next(state for name, state in states() if name == "aura-conditional-basic")
    data.append(("aura-conditional-menu", menu, None))
    for variant in ["basic", "level-seven", "ctx-nonbasic"]:
        primary = next(state for name, state in states() if name == f"aura-conditional-{variant}")
        data.append((f"aura-conditional-primary-{variant}", primary, "primary"))
    for variant in ["basic", "ctx-nonbasic"]:
        fallback = next(state for name, state in states() if name == f"aura-conditional-{variant}")
        fallback.get_hero("hero_sabina").current_turn_card.effect_id = (
            "native_unregistered_basic_attack"
        )
        data.append((f"aura-conditional-primary-fallback-{variant}", fallback, "primary"))
    for name, state, dynamic in data:
        hero = state.get_hero("hero_sabina")
        state.phase = GamePhase.RESOLUTION
        targets = [m for m in state.teams[TeamColor.BLUE].minions if not m.is_heavy][:2]
        from goa2.domain.hex import Hex

        state.place_entity(targets[0].id, Hex(q=3, r=0, s=-3))

        def attack(key):
            return AttackSequenceStep(
                damage=1,
                range_val=1,
                damage_stat_type=StatType.ATTACK,
                range_stat_type=StatType.RANGE,
                is_ranged=True,
                target_output_key=key,
                is_mandatory=False,
            )

        if dynamic is True:
            basic = next(
                c for c in hero.deck if c.is_basic and c.primary_action == ActionType.ATTACK
            )
            other = hero.current_turn_card
            state.place_entity(targets[1].id, Hex(q=0, r=3, s=-3))
            steps = [
                SetContextFlagStep(key="performing_card_id", value=str(basic.id)),
                attack("first"),
                SetContextFlagStep(key="performing_card_id", value=str(other.id)),
                attack("second"),
                SetContextFlagStep(key="performing_card_id", value=str(basic.id)),
                attack("third"),
            ]
        elif dynamic is None or dynamic == "primary":
            steps = [ResolveCardStep(hero_id=hero.id)]
        else:
            steps = [attack("victim")]
        expected_kill = get_computed_stat(state, hero.id, StatType.RANGE, 1) >= 3
        if dynamic == "primary" and "fallback" not in name:
            from goa2.engine.stats import compute_card_stats

            expected_kill = compute_card_stats(state, hero.id, hero.current_turn_card).range >= 3
        for i, step in enumerate(steps):
            step.step_id = f"fixture/{name}/{i}"
        push_steps(state, steps)
        envelope = export_state(state)
        probe = state.model_copy(deep=True)
        ops = []
        for _ in range(60):
            ops.append(advance)
            req = process_stack(probe).input_request
            if req is None:
                break
            ids = [o.id for o in req.options]
            if dynamic == "primary" and req.request_type.value == "CHOOSE_ACTION":
                value = "ATTACK"
            elif "HOLD" in ids:
                value = "HOLD"
            elif "CONFIRM" in ids:
                value = "CONFIRM"
            elif "PASS" in ids:
                value = "PASS"
            elif str(targets[0].id) in ids:
                value = str(targets[0].id)
            elif req.options:
                value = selection_value(req.options[0])
            else:
                value = "SKIP"
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=req.id, selection=value))
        else:
            raise AssertionError(name)
        if dynamic is True:
            assert all(probe.get_position(t.id) is None for t in targets), name
        elif dynamic is not None:
            assert (probe.get_position(targets[0].id) is None) == expected_kill, name
        result.append(
            {
                "name": name + "-trace",
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.engine.steps import CheckAdjacencyStep

    for variant in [
        "adjacent",
        "far",
        "same",
        "offboard",
        "literal-first",
        "empty-literal",
        "false-key",
        "true-key",
        "missing-key",
        "multi-owner",
        "multi-piece",
        "falsy-short-circuit",
    ]:
        state = setup(["Sabina", "Arien"], ["Razzle", "Brogan"])
        h = _board(state)
        state.place_entity("hero_sabina", h(0, 0))
        state.place_entity("hero_arien", h(1, 0))
        state.current_actor_id = "hero_sabina"
        state.phase = GamePhase.RESOLUTION
        a, b = "hero_sabina", "hero_arien"
        ak = bk = None
        state.execution_context = {"adjacent": "old"}
        if variant == "far":
            state.move_unit(b, h(3, 0))
        if variant == "same":
            b = a
        if variant == "offboard":
            state.remove_entity(b)
        if variant == "literal-first":
            ak = "bad"
            state.execution_context["bad"] = ["unhashable"]
        if variant in [
            "empty-literal",
            "false-key",
            "true-key",
            "missing-key",
            "falsy-short-circuit",
        ]:
            a = "" if variant == "empty-literal" else None
            ak = "a"
            if variant == "empty-literal":
                state.execution_context["a"] = "hero_sabina"
            elif variant == "false-key":
                state.execution_context["a"] = False
            elif variant == "true-key":
                state.execution_context["a"] = True
            elif variant == "falsy-short-circuit":
                bk = "bad"
                b = None
                state.execution_context["bad"] = ["unhashable"]
        if variant in ["multi-owner", "multi-piece"]:
            piece = _pieces(state, "hero_razzle")[0]
            state.place_entity(piece, h(0, 1))
            state.acting_piece_id = piece
            b = "hero_razzle" if variant == "multi-owner" else piece
        step = CheckAdjacencyStep(
            unit_a_id=a,
            unit_b_id=b,
            unit_a_key=ak,
            unit_b_key=bk,
            output_key="adjacent",
            step_id=f"fixture/adjacency/{variant}",
        )
        push_steps(state, [step])
        envelope = export_state(state)
        ops = [advance]
        result.append(
            {
                "name": f"conditional-adjacency-{variant}",
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    state = next(state for name, state in states() if name == "aura-conditional-level-seven")
    hero = state.get_hero("hero_sabina")
    defender = state.get_hero("hero_brogan")
    from goa2.domain.hex import Hex

    state.phase = GamePhase.RESOLUTION
    state.place_entity(defender.id, Hex(q=1, r=0, s=-1))
    defense = max(defender.hand, key=lambda c: c.secondary_actions.get(ActionType.DEFENSE, -99))
    assert defense.secondary_actions[ActionType.DEFENSE] >= 2
    step = ResolveCardStep(hero_id=hero.id, step_id="fixture/conditional/point-blank-block-push")
    push_steps(state, [step])
    envelope = export_state(state)
    probe = state.model_copy(deep=True)
    ops = []
    for _ in range(40):
        ops.append(advance)
        req = process_stack(probe).input_request
        if req is None:
            break
        ids = [o.id for o in req.options]
        if req.request_type.value == "CHOOSE_ACTION":
            value = "ATTACK"
        elif str(defender.id) in ids:
            value = str(defender.id)
        elif str(defense.id) in ids:
            value = str(defense.id)
        elif "CONFIRM" in ids:
            value = "CONFIRM"
        else:
            value = selection_value(req.options[0])
        ops.append(answer(value))
        submit_input(probe, InputResponse(request_id=req.id, selection=value))
    else:
        raise AssertionError("blocked primary stalled")
    assert probe.get_position(defender.id) == Hex(q=2, r=0, s=-2)
    assert str(defense.id) in [str(c.id) for c in probe.get_hero(defender.id).discard_pile]
    result.append(
        {
            "name": "conditional-primary-blocked-push",
            "envelope": envelope,
            "operations": ops,
            "expected": reference_trace(state, ops),
        }
    )
    return result
