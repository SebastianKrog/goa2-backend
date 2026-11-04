"""Journey manager creation and literal end-of-turn programs."""

from __future__ import annotations


def build(variant="plain", move_after=False):
    from action_control_traces import place_heroes
    from initiative_traces import build as base

    from goa2.domain.models import CardState
    from goa2.engine.steps import ScheduleJourneyReturnStep

    state, _, _, _ = base()
    pieces = state.get_piece_ids("hero_razzle")
    place_heroes(state)
    free = [
        point
        for point, tile in state.board.tiles.items()
        if not tile.is_obstacle and point not in state.entity_locations.values()
    ]
    for piece, point in zip(pieces, free, strict=False):
        state.place_entity(piece, point)
    actor = state.get_hero("hero_hanu")
    card = actor.hand[0]
    actor.play_card(card)
    card.is_facedown = False
    state.unresolved_hero_ids = []
    state.execution_context = {
        "enemy": "hero_brogan",
        "card-alias": card,
        "effects-alias": state.active_effects,
        "locations-alias": state.entity_locations,
    }
    step = ScheduleJourneyReturnStep(
        enemy_key="enemy", move_after=move_after, source_card_id=card.id
    )
    if variant == "unbound":
        step.source_card_id = None
    elif variant == "empty-card":
        step.source_card_id = ""
    elif variant == "unknown-card":
        step.source_card_id = "unknown-card"
    elif variant == "resolved-card":
        card.state = CardState.RESOLVED
    elif variant == "copied":
        card.state = CardState.RESOLVED
        actor.current_turn_card = None
        state.current_actor_id = "hero_arien"
        state.resolution_owner_id = "hero_arien"
    elif variant == "missing-enemy":
        state.execution_context.pop("enemy")
    elif variant in (
        "null-enemy",
        "false-enemy",
        "empty-enemy",
        "numeric-enemy",
        "unknown-enemy",
        "self",
    ):
        state.execution_context["enemy"] = {
            "null-enemy": None,
            "false-enemy": False,
            "empty-enemy": "",
            "numeric-enemy": 123,
            "unknown-enemy": "unknown",
            "self": actor.id,
        }[variant]
    elif variant == "no-actor":
        state.current_actor_id = None
    elif variant == "empty-actor":
        state.current_actor_id = ""
    elif variant == "unknown-actor":
        state.current_actor_id = "unknown"
    elif variant == "actor-piece":
        state.current_actor_id = state.get_piece_ids("hero_razzle")[0]
    elif variant in ("enemy-piece", "offboard-piece"):
        piece = state.get_piece_ids("hero_razzle")[0]
        state.execution_context["enemy"] = piece
        if variant == "offboard-piece":
            state.remove_entity(piece)
    elif variant == "custom-key":
        state.execution_context["target"] = state.execution_context.pop("enemy")
        step.enemy_key = "target"
    elif variant == "active-gate":
        step.active_if_key = "missing"
    elif variant == "skip-gate":
        step.skip_if_key = "enemy"
    elif variant == "context-card":
        state.execution_context.update(current_card_id="unknown-card", current_action_type="ATTACK")
    elif variant == "large-counter":
        state.next_entity_id = 2**53 + 1
    if variant.startswith("existing-") or variant == "counter-collision":
        from goa2.domain.models.effect import DurationType, EffectScope, EffectType, Shape
        from goa2.engine.effect_manager import EffectManager

        kinds = (
            [EffectType.IMMUNITY_ENEMY_ACTIONS]
            if variant == "existing-immunity"
            else (
                [EffectType.DELAYED_TRIGGER]
                if variant == "existing-delayed"
                else [EffectType.IMMUNITY_ENEMY_ACTIONS, EffectType.DELAYED_TRIGGER]
            )
        )
        for kind in kinds:
            old = EffectManager.create_effect(
                state=state,
                source_id="hero_arien",
                source_card_id=card.id,
                effect_type=kind,
                scope=EffectScope(shape=Shape.GLOBAL),
                duration=DurationType.THIS_TURN,
                is_active=variant != "existing-inactive",
            )
            if variant == "counter-collision":
                old.source_card_id = None
        if variant == "counter-collision":
            for offset, old in enumerate(state.active_effects):
                old.id = f"eff_e_{state.next_entity_id + offset}"
    return state, card, step


def scene(name, state, steps, *, take_move=False):
    from action_control_traces import operations
    from object_execution_traces import live_choice_operations

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import SetContextFlagStep

    steps = [*steps, SetContextFlagStep(key="after", value=True)]
    for index, step in enumerate(steps):
        step.step_id = f"fixture/journey/{name}/{index}"
    push_steps(state, steps)
    ops = (
        live_choice_operations(state, stop_types=(), max_steps=200)
        if take_move
        else operations(state)
    )
    return "journey-" + name, state, ops


def scenes():
    from goa2.engine.steps import (
        DiscardCardStep,
        FinalizeHeroTurnStep,
        FindNextActorStep,
        ScheduleJourneyReturnStep,
        SetContextFlagStep,
        SwapUnitsStep,
    )

    variants = (
        "plain",
        "unbound",
        "empty-card",
        "unknown-card",
        "resolved-card",
        "copied",
        "missing-enemy",
        "null-enemy",
        "false-enemy",
        "empty-enemy",
        "numeric-enemy",
        "unknown-enemy",
        "self",
        "no-actor",
        "empty-actor",
        "unknown-actor",
        "actor-piece",
        "enemy-piece",
        "offboard-piece",
        "custom-key",
        "active-gate",
        "skip-gate",
        "context-card",
        "large-counter",
        "existing-immunity",
        "existing-delayed",
        "existing-both",
        "existing-inactive",
        "counter-collision",
    )
    for move in (False, True):
        for variant in variants:
            state, _, step = build(variant, move)
            yield scene(f"create-{variant}-move-{move}", state, [step])
        for variant in (
            "plain",
            "unbound",
            "copied",
            "enemy-piece",
            "unknown-enemy",
            "offboard-piece",
        ):
            state, _, step = build(variant, move)
            actor = state.current_actor_id
            enemy = state.execution_context["enemy"]
            yield scene(
                f"end-turn-{variant}-move-{move}",
                state,
                [
                    SwapUnitsStep(unit_a_id=actor, unit_b_id=enemy),
                    step,
                    SetContextFlagStep(key="enemy", value="not-the-target"),
                    FinalizeHeroTurnStep(hero_id=state.resolution_owner_id),
                ],
            )
        state, card, step = build("plain", move)
        yield scene(
            f"discard-cancels-move-{move}",
            state,
            [
                step,
                DiscardCardStep(hero_id="hero_hanu", card_id=card.id, source="PLAYED"),
                FindNextActorStep(),
            ],
        )
        for bound in (False, True):
            state, _, step = build("plain" if bound else "unbound", move)
            yield scene(
                f"repeat-bound-{bound}-move-{move}",
                state,
                [
                    step,
                    ScheduleJourneyReturnStep(
                        enemy_key="enemy", move_after=move, source_card_id=step.source_card_id
                    ),
                ],
            )
    state, _, step = build("plain", True)
    yield scene(
        "take-return-move",
        state,
        [
            SwapUnitsStep(unit_a_id="hero_hanu", unit_b_id="hero_brogan"),
            step,
            FinalizeHeroTurnStep(hero_id="hero_hanu"),
        ],
        take_move=True,
    )
