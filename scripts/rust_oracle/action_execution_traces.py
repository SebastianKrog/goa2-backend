"""Drain real ResolveCardStep secondary actions and shared multi-select/token rules."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.input import InputResponse
    from goa2.domain.models import CardState, GamePhase, TeamColor
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import ResolveCardStep, SetContextFlagStep

    setup()
    result = []

    def drain(state, kind, *, invalid=False, atomic=False, skip=False, target=None):
        probe = state.model_copy(deep=True)
        ops = []
        first = True
        for _ in range(30):
            ops.append(advance)
            request = process_stack(probe).input_request
            if request is None:
                return ops
            if first:
                if invalid:
                    ops.extend([answer("not_an_action"), advance])
                    submit_input(
                        probe, InputResponse(request_id=request.id, selection="not_an_action")
                    )
                    request = process_stack(probe).input_request
                value = kind
                first = False
            elif request.request_type.value == "SELECT_HEX":
                value = (
                    "SKIP"
                    if skip and request.can_skip
                    else request.options[0].metadata.get("raw", request.options[0].id)
                )
            elif request.request_type.value == "SELECT_CARD_OR_PASS":
                value = "PASS"
            elif target is not None and any(option.id == target for option in request.options):
                value = target
            else:
                value = request.options[0].id
            if hasattr(value, "model_dump"):
                value = value.model_dump(mode="json")
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=request.id, selection=value))
            if atomic and not first:
                # Snapshot the queued action recipe immediately after ResolveCard.
                ops.append({"kind": "advance_prefix", "steps": 1})
                process_stack(probe, stop_before_step=_one_step_boundary())
                atomic = False
        raise AssertionError("secondary action did not finish")

    def add(name, state, hero, kind, **kwargs):
        steps = [
            ResolveCardStep(hero_id=hero.id),
            SetContextFlagStep(key="action_done", value=True),
        ]
        for i, step in enumerate(steps):
            step.step_id = f"fixture/action-execution/{name}/{i}"
        push_steps(state, steps)
        ops = drain(state, kind, **kwargs)
        result.append(
            {
                "name": f"action-execution-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

    def prepare(name, kind, *, upgraded=False):
        other = [n for n in ["Arien", "Hanu", "Brogan", "Tigerclaw"] if n != name]
        state = setup([name, other[0]], other[1:3])
        hero = state.teams[TeamColor.RED].heroes[0]
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = hero.id
        card = next(
            (
                c
                for c in hero.deck
                if (c.is_basic or upgraded)
                and c.primary_action != kind
                and kind in c.secondary_actions
            ),
            None,
        )
        if card is None:
            return None
        if card in hero.hand:
            hero.hand.remove(card)
        card.state = CardState.UNRESOLVED
        card.is_facedown = False
        hero.current_turn_card = card
        return state, hero

    from goa2.domain.models import ActionType

    for name in sorted(HeroRegistry._heroes):
        for kind in [
            ActionType.HOLD,
            ActionType.MOVEMENT,
            ActionType.ATTACK,
            ActionType.FAST_TRAVEL,
            ActionType.CLEAR,
        ]:
            prepared = prepare(name, kind)
            if prepared:
                state, hero = prepared
                add(f"{name}-{kind.value}", state, hero, kind.value)
    for variant in [
        "invalid",
        "atomic",
        "skip",
        "items",
        "facedown",
        "offboard",
        "unknown",
        "no-card",
    ]:
        state, hero = prepare("Arien", ActionType.MOVEMENT)
        if variant == "items":
            from goa2.domain.models import StatType

            hero.items = {StatType.MOVEMENT: 2, StatType.ATTACK: 1}
        if variant == "facedown":
            hero.current_turn_card.is_facedown = True
        if variant == "offboard":
            state.remove_unit(hero.id)
        if variant == "no-card":
            hero.current_turn_card = None
        if variant == "unknown":
            hero = hero.model_copy(update={"id": "unknown"})
        add(
            variant,
            state,
            hero,
            "HOLD" if variant == "facedown" else "MOVEMENT",
            invalid=variant == "invalid",
            atomic=variant == "atomic",
            skip=variant == "skip",
        )
    for variant in ["attack-hero", "attack-minion", "attack-items"]:
        state, hero = prepare("Mrak", ActionType.ATTACK, upgraded=True)
        origin = state.get_position(hero.id)
        free = next(
            h
            for h in state.board.tiles
            if origin.distance(h) == 1 and h not in state.entity_locations.values()
        )
        victim = (
            next(m for m in state.teams[TeamColor.BLUE].minions if not m.is_heavy)
            if variant == "attack-minion"
            else state.get_hero("hero_brogan")
        )
        state.move_unit(victim.id, free)
        if variant == "attack-items":
            from goa2.domain.models import StatType

            hero.items = {StatType.ATTACK: 2, StatType.RANGE: 1}
        add(variant, state, hero, "ATTACK", target=str(victim.id))
        assert any(
            event["event_type"] == "UNIT_DEFEATED" and event["target_id"] == victim.id
            for record in result[-1]["expected"]
            for event in record["events"]
        ), "attack fixture must execute an actual defeat"
        assert (
            result[-1]["expected"][-1]["semantic_state"]["execution_context"]["action_done"] is True
        )

    from goa2.domain.models import TargetType, Token, TokenType
    from goa2.engine.filters import RangeFilter, UnitTypeFilter
    from goa2.engine.steps import MultiSelectStep, RemoveTokenStep

    def token_state():
        state, hero = prepare("Arien", ActionType.CLEAR)
        origin = state.get_position(hero.id)
        free = [
            h
            for h in state.board.tiles
            if origin.distance(h) <= 1 and h not in state.entity_locations.values()
        ]
        for i, hex in enumerate(free[:3]):
            token = Token(
                id=f"native_token_{i}",
                name=f"Token {i}",
                token_type=TokenType.TOTEM,
                owner_id=hero.id,
            )
            state.misc_entities[token.id] = token
            state.place_entity(token.id, hex)
        assert len(free) >= 3
        return state, hero

    for variant in ["tokens", "tokens-atomic"]:
        state, hero = token_state()
        add(variant, state, hero, "CLEAR", atomic=variant == "tokens-atomic")

    def direct(name, state, steps, ops):
        for i, step in enumerate(steps):
            step.step_id = f"fixture/action-execution/{name}/{i}"
        push_steps(state, steps)
        result.append(
            {
                "name": f"action-execution-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

    for variant in [
        "maximum",
        "done",
        "skip",
        "invalid",
        "duplicate",
        "saved",
        "no-actor",
        "empty",
        "minimum",
        "inactive",
        "zero-max",
    ]:
        state, hero = token_state()
        step = MultiSelectStep(
            target_type=TargetType.UNIT_OR_TOKEN,
            prompt="Choose tokens",
            output_key="picked",
            max_selections=2,
            min_selections=1 if variant in ("done", "minimum") else 0,
            filters=[UnitTypeFilter(unit_type="TOKEN"), RangeFilter(max_range=1)],
        )
        ops = [advance]
        if variant in ("empty", "minimum"):
            for token in ["native_token_0", "native_token_1", "native_token_2"]:
                state.remove_entity(token)
        elif variant == "no-actor":
            state.current_actor_id = None
        elif variant == "inactive":
            step.active_if_key = "enabled"
        elif variant == "saved":
            step.selections = ["native_token_0"]
            ops += [answer("native_token_1"), advance]
        else:
            if variant == "zero-max":
                step.max_selections = 0
            if variant == "done":
                ops += [answer("DONE"), advance]
            if variant == "invalid":
                ops += [answer("unknown"), advance, answer(None), advance]
            ops += [answer("native_token_0"), advance]
            if variant == "duplicate":
                ops += [answer("native_token_0"), advance]
            if variant != "zero-max":
                ops += [
                    answer(
                        "SKIP"
                        if variant == "skip"
                        else "DONE" if variant == "done" else "native_token_1"
                    ),
                    advance,
                ]
        direct(
            f"multi-{variant}", state, [step, SetContextFlagStep(key="multi_done", value=True)], ops
        )
    for variant in [
        "literal",
        "key",
        "precedence",
        "unknown",
        "missing",
        "offboard",
        "hero",
        "twice",
    ]:
        state, hero = token_state()
        target = (
            "hero_arien"
            if variant == "hero"
            else "unknown" if variant == "unknown" else "native_token_0"
        )
        state.execution_context["token"] = target
        if variant == "offboard":
            state.remove_entity(target)
        step = RemoveTokenStep(
            token_id=target if variant not in ("key", "missing") else None,
            token_key="token" if variant in ("key", "precedence") else None,
        )
        if variant == "precedence":
            step.token_id = "native_token_1"
        steps = [step]
        if variant == "twice":
            steps.append(step.model_copy(deep=True))
        direct(f"remove-token-{variant}", state, steps, [advance])
    return result


def _one_step_boundary():
    remaining = 1

    def boundary(_state, _step):
        nonlocal remaining
        if remaining == 0:
            return True
        remaining -= 1
        return False

    return boundary
