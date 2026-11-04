"""Xargatha's live primary cards and board-cache adjacency counter contracts."""

from primary_templates import XARGATHA_EFFECTS as EFFECTS

ATTACKS = (
    "cleave",
    "threatening_slash",
    "deadly_swipe",
    "lethal_spin",
    "long_thrust",
    "rapid_thrusts",
)
DELAYED = ("final_embrace", "constrict")
MINION_MOVES = ("charm", "control", "dominate")
PETRIFY = ("turn_into_statues", "petrifying_stare", "stone_gaze")


def base(effect, variant="ordinary"):
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, MinionType, StatType

    state = setup(["Xargatha", "Arien"], ["Brogan", "Razzle"])
    h = _board(state)
    hero = state.get_hero("hero_xargatha")
    card = next(c for c in [*hero.deck, hero.ultimate_card] if c.effect_id == effect)
    if card in hero.hand:
        hero.hand.remove(card)
    hero.current_turn_card = card
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    if effect == "metamorphosis":
        hero.level = 8
    state.phase, state.current_actor_id, state.resolution_owner_id = (
        GamePhase.RESOLUTION,
        hero.id,
        hero.id,
    )
    state.battle_zones, state.unresolved_hero_ids = {}, []
    pieces = _pieces(state, "hero_razzle")
    for unit, point in [
        (hero.id, h(0, 0)),
        ("hero_arien", h(-2, 0)),
        ("hero_brogan", h(3, 0) if effect == "sirens_call" else h(1, 0)),
        (pieces[0], h(0, 1)),
        (pieces[1], h(1, -1)),
    ]:
        state.place_entity(unit, point)
    for kind, point in [
        (MinionType.MELEE, h(-1, 1)),
        (MinionType.RANGED, h(-1, 0)),
        (MinionType.HEAVY, h(0, -1)),
    ]:
        minion = next(m for m in state.teams["BLUE"].minions if m.type == kind)
        state.place_entity(minion.id, point)
    for owner_id in ("hero_arien", "hero_brogan"):
        owner = state.get_hero(owner_id)
        owner.play_card(owner.hand[0])
        owner.current_turn_card.is_facedown = False
    if effect in ("devoted_followers", "fresh_converts") and variant != "empty-discard":
        discard = next(c for c in hero.deck if c.effect_id == "charm")
        if discard in hero.hand:
            hero.hand.remove(discard)
        discard.state, discard.is_facedown = CardState.DISCARD, False
        hero.discard_pile.append(discard)
    if variant == "items":
        hero.items = {
            StatType.ATTACK: -2,
            StatType.RANGE: 1,
            StatType.RADIUS: 1,
            StatType.MOVEMENT: 2,
        }
    if variant == "negative-attack":
        hero.items = {StatType.ATTACK: -12}
    if variant.startswith("radius-"):
        card.radius_value = {"radius-none": None, "radius-zero": 0, "radius-negative": -1}[variant]
    if variant.startswith("range-"):
        card.range_value = {"range-none": None, "range-zero": 0, "range-negative": -1}[variant]
    if variant == "facedown":
        card.is_facedown = True
    if variant == "survives":
        state.get_hero("hero_brogan").items = {StatType.DEFENSE: 20}
    if variant == "empty-hand":
        state.get_hero("hero_brogan").hand = []
    if variant in ("no-enemies", "off-board"):
        for unit in list(state.entity_locations):
            if variant == "off-board" or unit != hero.id:
                state.remove_entity(unit)
    if variant == "no-ranged":
        for minion in state.teams["BLUE"].minions:
            if minion.type == MinionType.RANGED:
                state.remove_entity(minion.id)
    if variant == "heavy-only":
        for unit in [*state.teams["BLUE"].heroes, *state.teams["BLUE"].minions]:
            if getattr(unit, "type", None) != MinionType.HEAVY:
                state.remove_entity(unit.id)
        for piece in pieces:
            state.remove_entity(piece)
    if variant == "blocked":
        occupied = set(state.entity_locations.values())
        for tile in state.board.tiles.values():
            if tile.hex not in occupied:
                tile.is_terrain = True
    state.execution_context.update(
        xargatha_alias=hero,
        xargatha_card_alias=card,
        xargatha_hand_alias=hero.hand,
        locations_alias=state.entity_locations,
        effects_alias=state.active_effects,
    )
    return state, hero, card


def action_scene(effect, entry="text", mode="first", variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
        EndPhaseStep,
        PerformCardActionStep,
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SetContextFlagStep,
    )

    state, hero, card = base(effect, variant)
    if entry in ("text", "round"):
        step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
    elif entry == "action":
        step = ResolveCardStep(hero_id=hero.id)
    elif entry == "copy":
        state.current_actor_id = "hero_arien"
        state.execution_context.update(copied_xargatha=card.id, copied_xargatha_owner=hero.id)
        step = PerformCardActionStep(
            card_key="copied_xargatha",
            card_owner_key="copied_xargatha_owner",
            hero_id="hero_arien",
            suppress_after_resolve_card=True,
        )
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/xargatha-primary"
    tail = SetContextFlagStep(key="xargatha_tail", value=True, step_id="fixture/xargatha-tail")
    state.execution_stack = [tail, step]
    if entry == "round":
        state.turn = 4
        state.execution_stack.insert(1, EndPhaseStep(step_id="fixture/xargatha-round-end"))
    state.execution_context["xargatha_step"] = step
    bad = False

    def select(request, _mode):
        nonlocal bad
        kind = request.request_type.value
        if mode == "invalid" and not bad:
            bad = True
            return "not-offered"
        if kind == "UPGRADE_PHASE":
            owner, row = next(
                (owner, row) for owner, row in request.context["players"].items() if row["options"]
            )
            return {
                "hero_id": owner,
                "card_id": row["options"][0]["pair"][1 if mode == "last" else 0],
            }
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
        if kind == "SELECT_OPTION" and any(o.id == "YES" for o in request.options):
            return "NO" if mode == "skip" else "YES"
        if kind in ("SELECT_UNIT", "SELECT_UNIT_OR_TOKEN") and any(
            o.id == "hero_brogan" for o in request.options
        ):
            return "hero_brogan"
        assert request.options, (effect, entry, variant, kind, request.prompt)
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, final = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}], final


def count_scene(variant="ordinary", multiplier=1, subtract=0):
    from live_defeat_traces import retain

    from goa2.engine.steps import CountAdjacentEnemiesStep

    state, hero, _ = base("threatening_slash")
    if variant == "no-actor":
        state.current_actor_id = None
    if variant == "unknown-actor":
        state.current_actor_id = "missing-unit"
    if variant == "off-board":
        state.remove_entity(hero.id)
    if variant == "tiles-only":
        state.entity_locations.pop("hero_brogan")
    if variant == "locations-only":
        state.board.get_tile(state.get_position("hero_brogan")).occupant_id = None
    if variant == "owner-with-acting-piece":
        state.current_actor_id = "hero_razzle"
        state.acting_piece_id = state.get_piece_ids("hero_razzle")[0]
    if variant == "piece-actor":
        state.current_actor_id = state.get_piece_ids("hero_razzle")[0]
    if variant == "stale-occupant":
        state.board.get_tile(state.get_position("hero_brogan")).occupant_id = "missing-unit"
    step = CountAdjacentEnemiesStep(
        output_key="adjacency_probe", multiplier=multiplier, subtract=subtract
    )
    ops, final = retain(state, [step])
    return state, ops, final


def scenes():
    for effect in EFFECTS:
        for entry in ("text", "action", "perform", "copy"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode)
                yield f"xargatha-primary-{effect}-{entry}-{mode}", state, ops
        for variant in (
            "items",
            "radius-none",
            "radius-zero",
            "radius-negative",
            "no-enemies",
            "facedown",
        ):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"xargatha-primary-{effect}-{variant}", state, ops
    for effect in ATTACKS:
        for variant in (
            "negative-attack",
            "survives",
            "empty-hand",
            "range-none",
            "range-zero",
            "range-negative",
        ):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"xargatha-primary-{effect}-{variant}", state, ops
    for effect in MINION_MOVES:
        for variant in ("heavy-only", "no-ranged", "blocked", "off-board"):
            for mode in ("first", "skip"):
                state, ops, _ = action_scene(effect, mode=mode, variant=variant)
                yield f"xargatha-primary-{effect}-{variant}-{mode}", state, ops
    for effect in DELAYED:
        for variant in ("ordinary", "no-enemies", "heavy-only"):
            for mode in ("first", "last"):
                state, ops, _ = action_scene(effect, "round", mode, variant)
                yield f"xargatha-primary-{effect}-round-{variant}-{mode}", state, ops
    for variant in (
        "ordinary",
        "no-actor",
        "unknown-actor",
        "off-board",
        "tiles-only",
        "locations-only",
        "owner-with-acting-piece",
        "piece-actor",
        "stale-occupant",
    ):
        state, ops, _ = count_scene(variant)
        yield f"xargatha-count-{variant}", state, ops
    for multiplier, subtract in [(2, 1), (3, 1), (-2, 1), (2, 8), (2, -1)]:
        state, ops, _ = count_scene(multiplier=multiplier, subtract=subtract)
        yield f"xargatha-count-arithmetic-{multiplier}-{subtract}", state, ops
