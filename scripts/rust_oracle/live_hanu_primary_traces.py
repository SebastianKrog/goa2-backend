"""Hanu primary flows, including delegated movement and delayed returns."""

from primary_templates import HANU_EFFECTS as EFFECTS

ATTACKS = (
    "helping_hand",
    "even_the_odds",
    "trusted_sidekick",
    "outnumber",
    "pile_on",
    "fight_and_flight",
)
JOURNEYS = ("unexpected_journey", "there_and_back_again", "safe_travels")
CO_MOVES = ("this_way", "that_way")


def base(effect, variant="ordinary"):
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, StatType

    state = setup(["Hanu", "Arien"], ["Brogan", "Razzle"])
    h = _board(state)
    hero = state.get_hero("hero_hanu")
    card = next(c for c in [*hero.deck, hero.ultimate_card] if c.effect_id == effect)
    if card in hero.hand:
        hero.hand.remove(card)
    hero.current_turn_card = card
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    state.phase, state.current_actor_id, state.resolution_owner_id = (
        GamePhase.RESOLUTION,
        hero.id,
        hero.id,
    )
    state.battle_zones, state.unresolved_hero_ids = {}, []
    pieces = _pieces(state, "hero_razzle")
    for unit, point in [
        (hero.id, h(0, 0)),
        ("hero_arien", h(0, 2) if effect in CO_MOVES else h(2, 0)),
        ("hero_brogan", h(1, 0)),
        (pieces[0], h(2, -1)),
        (pieces[1], h(3, -1)),
    ]:
        state.place_entity(unit, point)
    friendly = next(m for m in state.teams["RED"].minions if not m.is_heavy)
    enemy = next(m for m in state.teams["BLUE"].minions if not m.is_heavy)
    state.place_entity(friendly.id, h(-1, 0))
    state.place_entity(enemy.id, h(1, 1))
    for owner_id in ("hero_arien", "hero_brogan"):
        owner = state.get_hero(owner_id)
        owner.play_card(owner.hand[0])
        owner.current_turn_card.is_facedown = False
    if variant == "items":
        hero.items = {
            StatType.ATTACK: -2,
            StatType.RANGE: 1,
            StatType.RADIUS: 1,
            StatType.MOVEMENT: 2,
        }
    if variant == "negative-attack":
        hero.items = {StatType.ATTACK: -12}
    if variant in ("radius-none", "radius-zero", "radius-negative"):
        card.radius_value = {"radius-none": None, "radius-zero": 0, "radius-negative": -1}[variant]
    if variant in ("range-zero", "range-negative", "range-none"):
        card.range_value = {"range-zero": 0, "range-negative": -1, "range-none": None}[variant]
    if variant == "level-eight":
        hero.level = 8
    if variant == "facedown":
        card.is_facedown = True
    if variant == "survives":
        state.get_hero("hero_brogan").items = {StatType.DEFENSE: 20}
    if variant == "empty-hand":
        state.get_hero("hero_brogan").hand = []
    if variant == "no-targets":
        for unit in list(state.entity_locations):
            if unit != hero.id:
                state.remove_entity(unit)
    if variant == "blocked":
        occupied = set(state.entity_locations.values())
        for tile in state.board.tiles.values():
            if tile.hex not in occupied:
                tile.is_terrain = True
    if variant == "piece-target":
        state.remove_entity("hero_brogan")
        state.move_unit(pieces[0], h(1, 0))
    state.execution_context.update(
        hanu_alias=hero,
        hanu_card_alias=card,
        hanu_hand_alias=hero.hand,
        locations_alias=state.entity_locations,
        effects_alias=state.active_effects,
    )
    return state, hero, card


def action_scene(effect, entry="text", mode="first", variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
        FinalizeHeroTurnStep,
        PerformCardActionStep,
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SetContextFlagStep,
    )

    state, hero, card = base(effect, variant)
    if entry == "text" or entry == "turn":
        step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
    elif entry == "action":
        step = ResolveCardStep(hero_id=hero.id)
    elif entry == "copy":
        state.current_actor_id = "hero_arien"
        state.execution_context.update(copied_hanu=card.id, copied_hanu_owner=hero.id)
        step = PerformCardActionStep(
            card_key="copied_hanu",
            card_owner_key="copied_hanu_owner",
            hero_id="hero_arien",
            suppress_after_resolve_card=True,
        )
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/hanu-primary"
    tail = SetContextFlagStep(key="hanu_tail", value=True, step_id="fixture/hanu-tail")
    state.execution_stack = [tail, step]
    if entry == "turn":
        state.execution_stack.insert(
            1, FinalizeHeroTurnStep(hero_id=hero.id, step_id="fixture/hanu-finalize")
        )
    state.execution_context["hanu_step"] = step
    bad = False

    def select(request, _mode):
        nonlocal bad
        if mode == "invalid" and not bad:
            bad = True
            return "not-offered"
        kind = request.request_type.value
        if kind == "CHOOSE_ACTION":
            primary = card.primary_action.value
            primary = "SKILL" if primary == "DEFENSE_SKILL" else primary
            return (
                primary if any(o.id == primary for o in request.options) else request.options[0].id
            )
        if kind == "CONFIRM_PASSIVE":
            return "NO"
        if kind == "SELECT_CARD_OR_PASS":
            return (
                next(o.id for o in request.options if o.id != "PASS")
                if variant == "survives"
                else "PASS"
            )
        if request.can_skip and mode == "skip":
            return "SKIP"
        if kind in ("SELECT_UNIT", "SELECT_UNIT_OR_TOKEN"):
            for target in ("hero_brogan", "hero_arien"):
                if any(o.id == target for o in request.options):
                    return target
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, final = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}], final


def scenes():
    for effect in EFFECTS:
        for entry in ("text", "action", "perform", "copy"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode)
                yield f"hanu-primary-{effect}-{entry}-{mode}", state, ops
        for variant in (
            "items",
            "radius-none",
            "radius-zero",
            "radius-negative",
            "no-targets",
            "facedown",
        ):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"hanu-primary-{effect}-{variant}", state, ops
    for effect in ATTACKS:
        for variant in ("negative-attack", "survives", "empty-hand", "piece-target"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"hanu-primary-{effect}-{variant}", state, ops
    for effect in (*JOURNEYS, *CO_MOVES, "hurry_up"):
        for variant in ("ordinary", "piece-target", "blocked", "level-eight"):
            for mode in ("first", "last"):
                state, ops, _ = action_scene(effect, "turn", mode, variant)
                yield f"hanu-primary-{effect}-turn-{variant}-{mode}", state, ops
    for variant in ("range-zero", "range-negative", "level-eight"):
        state, ops, _ = action_scene("hurry_up", variant=variant)
        yield f"hanu-primary-hurry_up-{variant}", state, ops

    from live_dynamic_passive_traces import scenes as passive_scenes

    for name, state, ops in passive_scenes(effects=("trusted_sidekick", "pile_on")):
        yield "hanu-primary-" + name, state, ops

    for effect in ATTACKS:
        for variant in ("range-zero", "range-negative", "range-none"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"hanu-primary-{effect}-{variant}", state, ops
