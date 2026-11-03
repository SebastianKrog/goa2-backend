"""Mind Grip branches, copied actions and live previous-slot predicates.

Raw states retain physical previous-card slots and nested action-scope aliases;
these boundary tests cannot hide that identity behind a card-test convenience API.
"""

from __future__ import annotations


def base(effect="poisoned_dagger", name="Tigerclaw"):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase

    state = setup(["NebKher", "Hanu"], [name, "Razzle"])
    h = _board(state)
    state.turn = 2
    state.phase = GamePhase.RESOLUTION
    state.battle_zones = {}
    hero = state.get_hero("hero_nebkher")
    owner = state.get_hero("hero_" + name.lower())
    card = next(c for c in hero.deck if c.effect_id == "mind_grip")
    copied = next(c for c in owner.deck if c.effect_id == effect)
    card.is_facedown = copied.is_facedown = False
    card.state = CardState.UNRESOLVED
    copied.state = CardState.RESOLVED
    hero.current_turn_card = card
    owner.current_turn_card = next(c for c in owner.deck if c is not copied)
    owner.current_turn_card.is_facedown = False
    owner.played_cards = [copied]
    owner.resolved_turn_count = 0
    state.current_actor_id = hero.id
    state.place_entity(hero.id, h(0, 0))
    state.place_entity("hero_hanu", h(-1, 1))
    state.place_entity(owner.id, h(2, 0))
    state.place_entity("hero_razzle_piece_1", h(-2, 0))
    state.place_entity("hero_razzle_piece_2", h(0, 2))
    state.place_entity("hero_razzle_piece_3", h(-2, 1))
    for minion, point in zip(state.teams[owner.team].minions, [h(1, 0), h(1, -1)], strict=False):
        state.place_entity(minion.id, point)
    state.execution_context.update(
        hero_alias=hero,
        card_alias=card,
        copied_alias=copied,
        owner_alias=owner,
        locations_alias=state.entity_locations,
    )
    return state, hero, card, copied, owner


def scenes():
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.domain.models import StatType
    from goa2.engine.steps import (
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SetContextFlagStep,
    )

    for effect, name in (
        ("poisoned_dagger", "Tigerclaw"),
        ("liquid_leap", "Arien"),
        ("noble_blade", "Arien"),
    ):
        for entry in ("text", "action", "perform"):
            for action in ("primary", "HOLD", "MOVEMENT", "FAST_TRAVEL", "CLEAR"):
                for mode in ("first", "invalid"):
                    state, hero, card, copied, owner = base(effect, name)
                    # Probe the actual copied menu before freezing a complete flow.
                    from goa2.engine.steps import PerformCardActionStep

                    state.execution_context["copied_owner"] = owner.id
                    menu = (
                        PerformCardActionStep(card_owner_key="copied_owner", previous_slot=True)
                        .resolve(state, state.execution_context)
                        .input_request
                    )
                    chosen = copied.current_primary_action.value if action == "primary" else action
                    if chosen not in {option.id for option in menu.options}:
                        continue
                    step = (
                        ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
                        if entry == "text"
                        else (
                            ResolveCardStep(hero_id=hero.id)
                            if entry == "action"
                            else PerformPrimaryActionStep(hero_id=hero.id)
                        )
                    )
                    state.execution_context["selected_card"] = card.id
                    state.execution_stack = [
                        SetContextFlagStep(
                            key="mind_grip_tail", value=True, step_id="fixture/mind-tail"
                        ),
                        step,
                    ]
                    step.step_id = "fixture/mind-grip"
                    state.execution_context["mind_step"] = step
                    invalid = False

                    def select(request, _mode, mode=mode, chosen=chosen, owner=owner):
                        nonlocal invalid
                        if mode == "invalid" and not invalid:
                            invalid = True
                            return "not-offered"
                        if request.request_type.value == "SELECT_NUMBER":
                            return 1
                        if request.request_type.value == "CHOOSE_ACTION":
                            return "SKILL" if "Choose action for card" in request.prompt else chosen
                        if request.request_type.value == "SELECT_CARD_OR_PASS":
                            return "PASS"
                        if "previous turn slot" in request.prompt:
                            return owner.id
                        return selection_value(request.options[0])

                    ops, _ = operations(state, select=select)
                    yield "mind-grip-copy-" + effect + "-" + entry + "-" + action + "-" + mode, state, [
                        *ops,
                        {"kind": "advance_prefix", "steps": 0},
                    ]

    for entry in ("text", "action", "perform"):
        for mode in ("first", "last", "skip", "invalid"):
            state, hero, card, _copied, _owner = base()
            step = (
                ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
                if entry == "text"
                else (
                    ResolveCardStep(hero_id=hero.id)
                    if entry == "action"
                    else PerformPrimaryActionStep(hero_id=hero.id)
                )
            )
            step.step_id = "fixture/mind-defeat"
            state.execution_context.update(selected_card=card.id, mind_step=step)
            state.execution_stack = [
                SetContextFlagStep(key="mind_grip_tail", value=True, step_id="fixture/mind-tail"),
                step,
            ]
            invalid = False

            def select(request, _mode, mode=mode):
                nonlocal invalid
                if mode == "invalid" and not invalid:
                    invalid = True
                    return "invalid"
                if request.request_type.value == "SELECT_NUMBER":
                    return 2
                if request.request_type.value == "CHOOSE_ACTION":
                    return "SKILL"
                if mode == "skip" and request.can_skip:
                    return "SKIP"
                return selection_value(
                    request.options[-1] if mode == "last" else request.options[0]
                )

            ops, _ = operations(state, select=select)
            yield "mind-grip-defeat-" + entry + "-" + mode, state, [
                *ops,
                {"kind": "advance_prefix", "steps": 0},
            ]

    for variant in (
        "turn-one",
        "missing-slot",
        "slot-hole",
        "facedown",
        "range-zero",
        "range-none",
        "negative-items",
        "missing-current",
        "suppress",
        "removed-current",
    ):
        state, hero, card, copied, owner = base()
        if variant == "turn-one":
            state.turn = 1
        if variant == "missing-slot":
            owner.played_cards = []
        if variant == "slot-hole":
            owner.played_cards = [None, copied]
        if variant == "facedown":
            copied.is_facedown = True
        if variant == "range-zero":
            card.range_value = 0
        if variant == "range-none":
            card.range_value = None
        if variant == "negative-items":
            hero.items = {StatType.ATTACK: -9, StatType.RANGE: -4}
        if variant == "missing-current":
            owner.current_turn_card = None
        if variant == "removed-current":
            owner.discard_pile.append(owner.current_turn_card)
            owner.current_turn_card = None
        if variant == "suppress":
            state.execution_context["skip_markers"] = True
        step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id, step_id="fixture/mind-edge")
        state.execution_stack = [
            SetContextFlagStep(key="mind_grip_tail", value=True, step_id="fixture/mind-tail"),
            step,
        ]

        def select(request, _mode):
            if request.request_type.value == "SELECT_NUMBER":
                return 1
            if request.request_type.value == "CHOOSE_ACTION":
                return "HOLD"
            return selection_value(request.options[0])

        ops, _ = operations(state, select=select)
        yield "mind-grip-edge-" + variant, state, [*ops, {"kind": "advance_prefix", "steps": 0}]

    yield from filter_scenes()


def filter_scenes():
    from goa2.domain.models import TargetType
    from goa2.engine.filters import HasPreviousSlotCardFilter
    from goa2.engine.steps import SelectStep

    for turn in (1, 2, 3, 4, 5):
        for kind in (TargetType.UNIT, TargetType.HEX, TargetType.UNIT_OR_TOKEN):
            state, _hero, _card, copied, owner = base()
            state.turn = turn
            owner.played_cards = [copied, None, copied, None]
            razzle = state.get_hero("hero_razzle")
            razzle.played_cards = [None, copied, None, copied]
            razzle.resolved_turn_count = 4
            step = SelectStep(
                prompt="Previous-slot filter",
                target_type=kind,
                filters=[HasPreviousSlotCardFilter()],
                output_key="eligible_previous",
                is_mandatory=False,
                step_id="fixture/previous-slot",
            )
            state.execution_stack = [step]
            state.execution_context["slot_step"] = step
            yield f"mind-grip-filter-{turn}-{kind.value}", state, [
                {"kind": "advance_prefix", "steps": 1},
                {"kind": "advance_prefix", "steps": 0},
            ]
