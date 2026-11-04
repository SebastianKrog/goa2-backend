"""Physical initiative overrides, lifecycle clearing, aliases and actor ordering."""

from __future__ import annotations


def build(variant="plain", value=11):
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase
    from goa2.engine.steps import SetCardInitiativeStep

    state = setup(["Hanu", "Arien"], ["Razzle", "Brogan"])
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_hanu"
    state.resolution_owner_id = "hero_hanu"
    target = state.get_hero("hero_arien")
    if variant == "self":
        target = state.get_hero("hero_hanu")
    elif variant == "enemy":
        target = state.get_hero("hero_brogan")
    elif variant in ("piece", "piece-offboard"):
        target = state.get_hero("hero_razzle")
    card = target.hand[0]
    target.play_card(card)
    card.is_facedown = False
    card.initiative_override = 3
    state.execution_context = {
        "who": target.id,
        "current-alias": card,
        "deck-alias": target.deck,
        "hand-alias": target.hand,
        "effects-alias": state.active_effects,
    }
    step = SetCardInitiativeStep(hero_key="who", value=value)
    if variant == "no-override":
        card.initiative_override = None
    elif variant.startswith("state-"):
        card.state = CardState(variant.removeprefix("state-"))
    elif variant == "facedown":
        card.is_facedown = True
    elif variant == "missing-card":
        target.current_turn_card = None
    elif variant == "missing-target":
        state.execution_context.pop("who")
    elif variant == "unknown-target":
        state.execution_context["who"] = "unknown"
    elif variant == "numeric-target":
        state.execution_context["who"] = 123
    elif variant == "null-target":
        state.execution_context["who"] = None
    elif variant == "false-target":
        state.execution_context["who"] = False
    elif variant == "empty-target":
        state.execution_context["who"] = ""
    elif variant == "no-actor":
        state.current_actor_id = None
    elif variant == "empty-actor":
        state.current_actor_id = ""
    elif variant == "unknown-actor":
        state.current_actor_id = "unknown"
    elif variant == "active-gate":
        step.active_if_key = "missing"
    elif variant == "skip-gate":
        step.skip_if_key = "who"
    elif variant == "shared-current":
        state.get_hero("hero_brogan").current_turn_card = card
    elif variant == "independent-current":
        other = card.model_copy(deep=True)
        other.metadata["independent"] = True
        state.get_hero("hero_brogan").current_turn_card = other
        state.execution_context["independent-card"] = other
    elif variant in ("piece", "piece-offboard"):
        piece = state.get_piece_ids(target.id)[0]
        state.execution_context["who"] = piece
        if variant == "piece-offboard":
            state.remove_entity(piece)
    elif variant == "custom-key":
        state.execution_context["custom"] = state.execution_context.pop("who")
        step.hero_key = "custom"
    return state, target, card, step


def _scene(name, state, steps, *, stop_types=()):
    from object_execution_traces import live_choice_operations

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import SetContextFlagStep

    steps = [*steps, SetContextFlagStep(key="after", value=True)]
    for index, step in enumerate(steps):
        step.step_id = f"fixture/initiative/{name}/{index}"
    push_steps(state, steps)
    return "initiative-" + name, state, live_choice_operations(state, stop_types=stop_types)


def scenes():
    from goa2.domain.models import CardState, StatType
    from goa2.engine.steps import DiscardCardStep, FindNextActorStep, SetCardInitiativeStep

    variants = [
        "plain",
        "no-override",
        "facedown",
        "missing-card",
        "missing-target",
        "unknown-target",
        "numeric-target",
        "null-target",
        "false-target",
        "empty-target",
        "no-actor",
        "empty-actor",
        "unknown-actor",
        "active-gate",
        "skip-gate",
        "shared-current",
        "independent-current",
        "piece",
        "piece-offboard",
        "custom-key",
        "self",
        "enemy",
        *["state-" + value.value for value in CardState],
    ]
    for variant in variants:
        state, _, _, step = build(variant)
        yield _scene(variant, state, [step])
    for value in (0, -3, 1, 11, 99, 2**53 + 1, 2**63 - 1, -(2**63)):
        state, _, _, step = build(value=value)
        yield _scene(f"value-{value}", state, [step])
    for variant in ("plain", "shared-current", "independent-current", "piece", "self"):
        state, target, card, step = build(variant)
        yield _scene(
            "discard-" + variant,
            state,
            [step, DiscardCardStep(hero_id=target.id, card_id=card.id, source="PLAYED")],
        )
    for value in (0, 6, 11):
        for items in (0, 2):
            state, target, _, step = build(value=value)
            other = state.get_hero("hero_hanu")
            card = other.hand[0]
            other.play_card(card)
            card.is_facedown = False
            card.initiative_override = 6
            target.items[StatType.INITIATIVE] = items
            state.unresolved_hero_ids = [target.id, other.id]
            yield _scene(
                f"order-{value}-items-{items}",
                state,
                [step, FindNextActorStep()],
                stop_types=("resolve_card",),
            )
    state, _, _, first = build()
    yield _scene("replace-twice", state, [first, SetCardInitiativeStep(hero_key="who", value=-2)])
