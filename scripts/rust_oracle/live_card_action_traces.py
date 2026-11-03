"""Live card menus, ordered action recipes and legal piece selection.

Construction-only groups deliberately stack fresh resolve steps over queued
children, so every ordinary card and choice is tested without pretending that
all those unported card bodies can already execute. Complete flows are separate.
"""

from __future__ import annotations

import copy


def _ordinary_scenes():
    from aura_traces import _board
    from live_defeat_traces import operations
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import ActionType, CardState, GamePhase
    from goa2.engine.steps import ChooseActingPieceStep, ResolveCardStep

    def base(name):
        ally = "Swift" if name != "Swift" else "Hanu"
        foe = "Brogan" if name != "Brogan" else "Arien"
        other = next(n for n in ("Hanu", "Tigerclaw", "Arien") if n not in (name, ally, foe))
        state = setup([name, ally], [foe, other])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        state.current_actor_id = "hero_" + name.lower()
        hero = state.get_hero(str(state.current_actor_id))
        if hero.is_multi_piece:
            for i in range(1, hero.piece_supply + 1):
                state.place_entity(f"{hero.id}_piece_{i}", h(1 - i, 0))
        else:
            state.place_entity(hero.id, h(0, 0))
        state.place_entity("hero_" + foe.lower(), h(1, 0))
        state.place_entity("hero_" + ally.lower(), h(0, 1))
        return state, hero, h

    setup()
    for name in sorted(HeroRegistry._heroes):
        original, hero, _h = base(name)
        cards = [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
        seen = set()
        for card in cards:
            if card.id in seen:
                continue
            seen.add(card.id)
            state = copy.deepcopy(original)
            owner = state.get_hero(str(hero.id))
            current = next(
                c
                for c in [*owner.deck, *([owner.ultimate_card] if owner.ultimate_card else [])]
                if c.id == card.id
            )
            current.is_facedown = False
            current.state = CardState.UNRESOLVED
            owner.current_turn_card = current
            probe = copy.deepcopy(state)
            request = (
                ResolveCardStep(hero_id=str(owner.id))
                .resolve(probe, probe.execution_context)
                .input_request
            )
            assert request is not None, (name, card.id)
            selections = [
                None,
                *dict.fromkeys(option.id for option in request.options),
                "invalid-action",
            ]
            groups = []
            for index, selection in enumerate(selections):
                step = ResolveCardStep(hero_id=str(owner.id), step_id=f"fixture/card-menu/{index}")
                if selection is not None:
                    step.pending_input = {"selection": selection}
                groups.append([step])
            state.execution_stack = []
            state.execution_context.update(
                menu_groups=groups,
                current_card_id="before-card",
                current_action_type="before-action",
                hero_alias=owner,
                current_alias=current,
                locations_alias=state.entity_locations,
            )
            ops = []
            for index in range(len(groups)):
                ops.extend(
                    [
                        {
                            "kind": "queue_steps",
                            "path": ["execution_context", "menu_groups", index],
                        },
                        {"kind": "advance_prefix", "steps": 1},
                    ]
                )
            ops.append({"kind": "advance_prefix", "steps": 0})
            yield "card-action-menu-" + name.lower() + "-" + str(card.id), state, ops

    for variant in (
        "ordinary",
        "single",
        "none",
        "bound",
        "valid",
        "invalid",
        "legal-single",
        "none-legal",
        "no-action",
        "skip",
        "inactive",
        "piece-operand",
    ):
        state, hero, _h = base("Razzle")
        card = hero.hand[0]
        hero.current_turn_card = card
        card.is_facedown = False
        step = ChooseActingPieceStep(hero_id=str(hero.id), step_id="fixture/piece-choice")
        pieces = state.get_piece_ids(str(hero.id))
        state.execution_context.update(piece_step=step, current_action_type=ActionType.ATTACK)
        if variant in ("single", "none"):
            for piece in pieces[1 if variant == "single" else 0 :]:
                state.remove_entity(piece)
        if variant == "bound":
            state.acting_piece_id = pieces[-1]
        if variant == "valid":
            step.pending_input = {"selection": pieces[-1]}
        if variant == "invalid":
            step.pending_input = {"selection": "absent"}
        if variant == "no-action":
            state.execution_context.pop("current_action_type")
        if variant == "piece-operand":
            step.hero_id = pieces[0]
        if variant == "skip":
            step.skip_if_key = "blocked"
            state.execution_context["blocked"] = True
        if variant == "inactive":
            step.active_if_key = "enabled"
        if variant in ("legal-single", "none-legal"):
            from goa2.domain.models.effect import (
                ActiveEffect,
                DurationType,
                EffectScope,
                EffectType,
                Shape,
            )

            state.active_effects.append(
                ActiveEffect(
                    id="prevent",
                    source_id="hero_brogan",
                    effect_type=EffectType.MOVEMENT_ZONE,
                    scope=EffectScope(
                        shape=Shape.GLOBAL if variant == "none-legal" else Shape.RADIUS, range=2
                    ),
                    duration=DurationType.PASSIVE,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                    is_active=True,
                    restrictions=[ActionType.ATTACK],
                )
            )
        state.execution_stack = [step]
        yield "card-action-piece-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]

    # Reuse complete primary-text worlds, adding the actual action lifecycle.
    from live_utility_traces import scenes as utility_scenes

    for name, state, _ in utility_scenes():
        if not name.startswith("utility-card-") or not name.endswith("-first"):
            continue
        hero = state.get_hero(str(state.current_actor_id))
        card = hero.current_turn_card
        kind = card.current_primary_action
        if kind == ActionType.DEFENSE_SKILL:
            kind = ActionType.SKILL
        step = ResolveCardStep(hero_id=str(hero.id), step_id="fixture/card-action/main")
        state.execution_stack[-1] = step
        state.execution_context["defeat_steps"] = [step]

        def select(request, mode, kind=kind):
            from goa2.domain.input import selection_value

            if request.request_type.value == "CHOOSE_ACTION":
                return kind.value
            if request.request_type.value == "SELECT_CARD_OR_PASS":
                return "PASS"
            return selection_value(request.options[0])

        ops, _ = operations(state, select=select)
        yield "card-action-flow-" + name.removeprefix("utility-card-"), state, [
            *ops,
            {"kind": "advance_prefix", "steps": 0},
        ]


def edge_scenes():
    """Request submissions and mutated live operands beyond printed card faces."""
    from aura_traces import _board
    from live_defeat_traces import operations
    from model_traces import setup

    from goa2.domain.models import ActionType, GamePhase
    from goa2.engine.steps import ChooseActingPieceStep, ResolveCardStep

    for mode in ("first", "last", "invalid"):
        state = setup(["Razzle", "Swift"], ["Brogan", "Arien"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        state.current_actor_id = "hero_razzle"
        hero = state.get_hero("hero_razzle")
        for i in range(1, hero.piece_supply + 1):
            state.place_entity(f"hero_razzle_piece_{i}", h(1 - i, 0))
        hero.current_turn_card = hero.hand[0]
        state.execution_context["current_action_type"] = ActionType.ATTACK
        step = ChooseActingPieceStep(hero_id="hero_razzle", step_id="fixture/piece-input")
        state.execution_context["choice_alias"] = step
        state.execution_stack = [step]
        ops, _ = operations(state, mode=mode)
        yield "card-action-edge-piece-input-" + mode, state, [
            *ops,
            {"kind": "advance_prefix", "steps": 0},
        ]

    for variant in (
        "raw-defense-skill",
        "negative-attack",
        "facedown",
        "offboard",
        "no-card",
        "missing-hero",
        "piece-operand",
    ):
        multi = variant == "piece-operand"
        name = "Razzle" if multi else "Arien"
        state = setup([name, "Swift"], ["Brogan", "Hanu"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        state.current_actor_id = "hero_" + name.lower()
        hero = state.get_hero(str(state.current_actor_id))
        if multi:
            state.place_entity("hero_razzle_piece_1", h(0, 0))
        else:
            state.place_entity(hero.id, h(0, 0))
        card = hero.hand[0]
        hero.current_turn_card = card
        card.is_facedown = variant == "facedown"
        selected = None
        if variant == "raw-defense-skill":
            card.primary_action = ActionType.DEFENSE_SKILL
            card.secondary_actions = {ActionType.DEFENSE_SKILL: 3}
            selected = "DEFENSE_SKILL"
        if variant == "negative-attack":
            card.primary_action = ActionType.ATTACK
            card.primary_action_value = -3
            selected = "ATTACK"
        if variant == "facedown":
            selected = "HOLD"
        if variant == "offboard":
            state.remove_entity(hero.id)
        if variant == "no-card":
            hero.current_turn_card = None
        operand = (
            "absent"
            if variant == "missing-hero"
            else "hero_razzle_piece_1" if multi else str(hero.id)
        )
        step = ResolveCardStep(hero_id=operand, step_id="fixture/action-edge")
        if selected is not None:
            step.pending_input = {"selection": selected}
        state.execution_context.update(
            current_card_id="old-card",
            current_action_type="old-action",
            card_alias=card,
            action_alias=step,
        )
        state.execution_stack = [step]
        yield "card-action-edge-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]


def scenes():
    yield from _ordinary_scenes()
    yield from edge_scenes()
