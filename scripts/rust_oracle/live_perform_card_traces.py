"""Copied-card menus, physical slot lookup and bounded complete action flows.

Raw setup is intentional: the oracle must retain foreign-card aliases and each
queued source step while checking construction and action-context restoration.
"""

from __future__ import annotations

import copy


def base(name, effect=None, performer_name=None):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase

    performer = performer_name or ("Arien" if name != "Arien" else "Hanu")
    ally = next(n for n in ("Swift", "Tigerclaw") if n not in (name, performer))
    other = next(n for n in ("Brogan", "Tigerclaw", "Hanu") if n not in (name, performer, ally))
    state = setup([performer, ally], [name, other])
    h = _board(state)
    actor = state.get_hero("hero_" + performer.lower())
    owner = state.get_hero("hero_" + name.lower())
    state.current_actor_id = actor.id
    state.phase = GamePhase.RESOLUTION
    state.battle_zones = {}
    if actor.is_multi_piece:
        for index in range(actor.piece_supply):
            state.place_entity(f"{actor.id}_piece_{index+1}", h(0, index))
    else:
        state.place_entity(actor.id, h(0, 0))
    state.place_entity("hero_" + ally.lower(), h(-1, 1))
    if owner.is_multi_piece:
        state.place_entity(str(owner.id) + "_piece_1", h(2, 0))
    else:
        state.place_entity(owner.id, h(2, 0))
    state.place_entity("hero_" + other.lower(), h(3, 0))
    for minion, point in zip(state.teams[owner.team].minions, [h(1, 0), h(1, -1)], strict=False):
        state.place_entity(minion.id, point)
    actor.current_turn_card = actor.hand[0]
    actor.current_turn_card.is_facedown = False
    actor.current_turn_card.state = CardState.UNRESOLVED
    card = next(c for c in owner.deck if c.effect_id == effect) if effect else None
    state.execution_context.update(
        foreign_owner=owner.id,
        current_card_id="outer-card",
        current_action_type="outer-action",
        performing_card_id=actor.current_turn_card.id,
        performing_card_owner_id=actor.id,
        token_type_override=None,
        skip_markers=False,
        substitution_actor_id=actor.id,
        actor_alias=actor,
        owner_alias=owner,
        locations_alias=state.entity_locations,
    )
    if card:
        card.is_facedown = False
        state.execution_context.update(selected_card=card.id, card_alias=card)
    return state, actor, owner, card


def scenes(*, heroes=None, include_common=True):
    from model_traces import setup
    from primary_templates import ALL_EFFECTS as EFFECTS

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import ActionType
    from goa2.engine.effects import CardEffect, CardEffectRegistry
    from goa2.engine.steps import PerformCardActionStep, SetContextFlagStep

    setup()
    for name in sorted(HeroRegistry._heroes):
        if heroes is not None and name not in heroes:
            continue
        original, actor, owner, _ = base(name)
        for printed in [*owner.deck, *([owner.ultimate_card] if owner.ultimate_card else [])]:
            state = copy.deepcopy(original)
            copied_owner = state.get_hero(str(owner.id))
            card = next(
                c
                for c in [
                    *copied_owner.deck,
                    *([copied_owner.ultimate_card] if copied_owner.ultimate_card else []),
                ]
                if c.id == printed.id
            )
            card.is_facedown = False
            copied_owner.current_turn_card = card
            state.execution_context.update(selected_card=card.id, card_alias=card)
            probe = PerformCardActionStep(card_owner_key="foreign_owner").resolve(
                state, state.execution_context
            )
            assert probe.requires_input
            effect = CardEffectRegistry.get_for_card(card)
            body_ready = (
                card.effect_id in EFFECTS
                or effect is None
                or type(effect).build_steps is CardEffect.build_steps
            )
            primary = (
                "SKILL"
                if card.current_primary_action == ActionType.DEFENSE_SKILL
                else card.current_primary_action
            )
            selections = [
                None,
                "invalid-action",
                *dict.fromkeys(
                    o.id for o in probe.input_request.options if o.id != primary or body_ready
                ),
            ]
            groups = []
            for index, selection in enumerate(selections):
                step = PerformCardActionStep(
                    card_owner_key="foreign_owner", step_id=f"fixture/copied-menu/{index}"
                )
                if selection is not None:
                    step.pending_input = {"selection": selection}
                groups.append([step])
            state.execution_context["copied_groups"] = groups
            ops = []
            for index in range(len(groups)):
                ops.extend(
                    [
                        {
                            "kind": "queue_steps",
                            "path": ["execution_context", "copied_groups", index],
                        },
                        {"kind": "advance_prefix", "steps": 1},
                    ]
                )
            ops.append({"kind": "advance_prefix", "steps": 0})
            yield "perform-card-menu-" + name.lower() + "-" + str(card.id), state, ops

    if not include_common:
        return

    variants = (
        "default-actor",
        "explicit-actor",
        "no-actor",
        "missing-actor",
        "missing-key",
        "missing-card",
        "missing-owner",
        "missing-owner-key",
        "facedown",
        "previous",
        "previous-empty",
        "previous-turn-one",
        "previous-hole",
        "previous-four",
        "offboard",
        "skip",
        "inactive",
        "invalid",
        "substitution",
        "suppress",
        "negative-attack",
        "fallback-attack",
        "fallback-movement",
        "fallback-skill",
        "no-effect",
        "owner-hand",
        "owner-discard",
        "owner-played",
        "owner-deck",
        "same-owner",
        "primary-defense",
        "defense-skill",
        "empty-owner-key",
    )
    from goa2.domain.models import ActionType, StatType, TokenType

    for variant in variants:
        state, actor, owner, card = base("Tigerclaw", "poisoned_dagger")
        step = PerformCardActionStep(card_owner_key="foreign_owner", step_id="fixture/copied-edge")
        step.pending_input = {"selection": "HOLD"}
        if variant == "explicit-actor":
            step.hero_id = actor.id
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "missing-actor":
            step.hero_id = "absent"
        if variant == "missing-key":
            state.execution_context.pop("selected_card")
        if variant == "missing-card":
            state.execution_context["selected_card"] = "absent"
        if variant == "missing-owner":
            state.execution_context["foreign_owner"] = "absent"
        if variant == "missing-owner-key":
            state.execution_context.pop("foreign_owner")
        if variant == "empty-owner-key":
            step.card_owner_key = ""
        if variant == "facedown":
            card.is_facedown = True
        if variant.startswith("previous"):
            step.previous_slot = True
            state.turn = 2
            owner.played_cards = [card]
            if variant == "previous-empty":
                owner.played_cards = []
            if variant == "previous-turn-one":
                state.turn = 1
            if variant == "previous-hole":
                owner.played_cards = [None, card]
            if variant == "previous-four":
                state.turn = 4
                owner.played_cards = [None, None, card]
        if variant == "offboard":
            state.remove_entity(actor.id)
            step.pending_input = {"selection": "CLEAR"}
        if variant == "skip":
            step.skip_if_key = "skip"
            state.execution_context["skip"] = True
        if variant == "inactive":
            step.active_if_key = "enabled"
        if variant == "invalid":
            step.pending_input = {"selection": "invalid"}
        if variant == "substitution":
            step.token_type_override = TokenType.ILLUSION
            step.skip_markers = True
        if variant == "suppress":
            step.suppress_after_resolve_card = True
        if variant == "negative-attack":
            actor.items = {StatType.ATTACK: -9}
            step.pending_input = {"selection": "ATTACK"}
        if variant.startswith("fallback-") or variant == "no-effect":
            card.effect_id = None if variant == "no-effect" else "unregistered-copied"
            card.primary_action = (
                ActionType.ATTACK
                if variant == "no-effect"
                else ActionType(variant.split("-")[1].upper())
            )
            card.primary_action_value = -3
            card.range_value = 0
            step.pending_input = {"selection": card.primary_action.value}
        if variant.startswith("owner-"):
            owner.current_turn_card = None
            owner.hand = []
            owner.discard_pile = []
            owner.played_cards = []
            if variant == "owner-hand":
                owner.hand = [card]
            if variant == "owner-discard":
                owner.discard_pile = [card]
            if variant == "owner-played":
                owner.played_cards = [None, card]
        if variant == "same-owner":
            step.card_owner_key = None
            state.execution_context["selected_card"] = actor.current_turn_card.id
        if variant == "primary-defense":
            card.primary_action = ActionType.DEFENSE
            step.pending_input = None
        if variant == "defense-skill":
            card.primary_action = ActionType.DEFENSE_SKILL
            step.pending_input = {"selection": "SKILL"}
        state.execution_stack = [
            SetContextFlagStep(key="copied_tail", value=True, step_id="fixture/copied-tail"),
            step,
        ]
        state.execution_context["copied_step"] = step
        yield "perform-card-edge-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]

    yield from flow_scenes()
    yield from piece_scenes()


def flow_scenes():
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PerformCardActionStep, SetContextFlagStep

    for effect, owner_name in (
        ("noble_blade", "Arien"),
        ("spell_break", "Arien"),
        ("ebb_and_flow", "Arien"),
        ("magical_current", "Arien"),
        ("poisoned_dagger", "Tigerclaw"),
        ("close_support", "Sabina"),
    ):
        for action in ("primary", "HOLD", "MOVEMENT", "FAST_TRAVEL", "CLEAR"):
            for mode in ("first", "last", "skip", "invalid"):
                state, _actor, _owner, card = base(owner_name, effect)
                action_id = card.current_primary_action.value if action == "primary" else action
                step = PerformCardActionStep(
                    card_owner_key="foreign_owner",
                    skip_markers=True,
                    token_type_override=TokenType.ILLUSION,
                    step_id="fixture/copied-flow",
                )
                menu = step.resolve(state, state.execution_context).input_request
                if action_id not in {o.id for o in menu.options}:
                    continue
                state.execution_stack = [
                    SetContextFlagStep(
                        key="copied_tail", value=True, step_id="fixture/copied-tail"
                    ),
                    step,
                ]
                state.execution_context["copied_step"] = step
                bad = False

                def select(request, _mode, action_id=action_id, mode=mode):
                    nonlocal bad
                    if mode == "invalid" and not bad:
                        bad = True
                        return "not-offered"
                    if request.request_type.value == "CHOOSE_ACTION":
                        return action_id
                    if request.request_type.value == "SELECT_CARD_OR_PASS":
                        return "PASS"
                    if mode == "skip" and request.can_skip:
                        return "SKIP"
                    return selection_value(
                        request.options[-1] if mode == "last" else request.options[0]
                    )

                ops, _ = operations(state, select=select)
                yield "perform-card-flow-" + effect + "-" + action + "-" + mode, state, [
                    *ops,
                    {"kind": "advance_prefix", "steps": 0},
                ]


def piece_scenes():
    from goa2.engine.steps import PerformCardActionStep

    for mode in ("primary", "secondary"):
        state, actor, _owner, card = base("Arien", "liquid_leap", performer_name="Razzle")
        step = PerformCardActionStep(
            hero_id=str(actor.id) + "_piece_1",
            card_owner_key="foreign_owner",
            pending_input={
                "selection": card.current_primary_action.value if mode == "primary" else "MOVEMENT"
            },
            step_id="fixture/copied-piece",
        )
        state.execution_stack = [step]
        state.execution_context["copied_step"] = step
        yield "perform-card-edge-piece-" + mode, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]
