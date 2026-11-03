"""Misa primary bodies, including live ultimate variants and foreign performers.

Raw setup deliberately retains model, card, effect and position identities.
"""

from primary_templates import MISA_EFFECTS as EFFECTS

BLUE = ("dash_and_slash", "death_from_above", "sudden_breeze", "gust_of_wind", "crushing_squall")
RED = ("challenge_accepted", "matter_of_honor", "worthy_opponent", "power_shot", "thunder_shot")
DYNAMIC = ("swoop_in", "watch_how_i_soar")


def base(effect, variant="ordinary"):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, StatType

    state = setup(["Misa", "Gydion"], ["Arien", "Tigerclaw"])
    h = _board(state)
    hero = state.get_hero("hero_misa")
    card = next(c for c in [*hero.deck, hero.ultimate_card] if c and c.effect_id == effect)
    hero.current_turn_card = card
    if card in hero.hand:
        hero.hand.remove(card)
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = hero.id
    state.battle_zones = {}
    for unit, point in (
        (hero.id, h(0, 0)),
        ("hero_gydion", h(-1, 1)),
        ("hero_arien", h(1, 0)),
        ("hero_tigerclaw", h(2, -1)),
    ):
        state.place_entity(unit, point)
    foe = state.teams["BLUE"].minions[0]
    friend = state.teams["RED"].minions[0]
    state.place_entity(foe.id, h(0, 1))
    state.place_entity(friend.id, h(-1, 0))
    if effect in ("living_tornado", "storm_spirit"):
        radius = card.radius_value or 3
        state.place_entity("hero_arien", h(radius, 0))
        state.place_entity("hero_gydion", h(0, radius))
    if effect == "watch_how_i_soar":
        state.place_entity("hero_arien", h(3, 0))
        state.place_entity("hero_tigerclaw", h(3, -1))
        state.place_entity(state.teams["BLUE"].minions[1].id, h(3, 1))
    if effect == "swoop_in":
        spare = next(c for c in hero.hand if c is not card)
        hero.hand.remove(spare)
        spare.state = CardState.DISCARD
        spare.is_facedown = False
        hero.discard_pile.append(spare)
    if variant in (
        "ultimate",
        "ultimate-place",
        "ultimate-defeat",
        "ultimate-decline",
        "level-seven",
        "no-ultimate",
        "foreign-ultimate",
    ):
        hero.level = 7 if variant == "level-seven" else 8
        hero.ultimate_card.state = CardState.PASSIVE
        hero.ultimate_card.is_facedown = False
        if variant == "no-ultimate":
            hero.ultimate_card = None
        if variant == "foreign-ultimate":
            hero.level = 1
            hero.ultimate_card = None
    if variant.startswith("radius-"):
        card.radius_value = {"radius-none": None, "radius-zero": 0, "radius-negative": -1}[variant]
    if variant == "negative-attack":
        hero.items = {StatType.ATTACK: -12}
    if variant == "items":
        hero.items = {StatType.MOVEMENT: 3, StatType.RANGE: 1, StatType.RADIUS: 1}
    if variant in ("max-range-survives", "adjacent-survives"):
        state.get_hero("hero_arien").items = {StatType.DEFENSE: 20}
        state.place_entity("hero_arien", h(2 if variant == "max-range-survives" else 1, 0))
    if variant == "out-of-range":
        state.place_entity("hero_arien", h(4, 0))
        state.remove_entity("hero_tigerclaw")
        state.remove_entity(foe.id)
    if variant == "cross-minion":
        state.remove_entity("hero_arien")
        state.place_entity(foe.id, h(1, 0))
    if variant == "no-crossed":
        state.place_entity("hero_arien", h(-3, 1))
        state.place_entity(foe.id, h(-3, 0))
    if variant == "long-line":
        state.place_entity(hero.id, h(-2, 0))
        state.place_entity("hero_arien", h(0, 0))
    if variant == "blocked":
        for tile in state.board.tiles.values():
            if tile.hex != h(0, 0) and tile.hex not in state.entity_locations.values():
                tile.is_terrain = True
    if variant == "no-enemies":
        for unit in [*state.teams["BLUE"].heroes, *state.teams["BLUE"].minions]:
            state.remove_entity(unit.id)
    if variant == "null-count-origin":
        state.execution_context["_cmf_origin_hex"] = None
    else:
        state.execution_context["_cmf_origin_hex"] = {"retained": True}
    state.execution_context.update(
        misa_alias=hero,
        misa_card_alias=card,
        misa_hand_alias=hero.hand,
        misa_discard_alias=hero.discard_pile,
        locations_alias=state.entity_locations,
        effects_alias=state.active_effects,
        misa_ultimate_alias=hero.ultimate_card,
        selected_card=card.id,
    )
    return state, hero, card


def action_scene(effect, entry="text", mode="first", variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState
    from goa2.engine.steps import (
        PerformCardActionStep,
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SetContextFlagStep,
    )

    state, hero, card = base(effect, variant)
    if entry == "text":
        step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
    elif entry == "action":
        step = ResolveCardStep(hero_id=hero.id)
    elif entry == "copy":
        actor = state.get_hero("hero_gydion")
        state.current_actor_id = actor.id
        state.execution_context.update(copied_misa=card.id, copied_misa_owner=hero.id)
        if variant == "foreign-ultimate":
            actor.level = 8
        if effect == "swoop_in":
            spare = actor.hand.pop(0)
            spare.state, spare.is_facedown = CardState.DISCARD, False
            actor.discard_pile.append(spare)
        step = PerformCardActionStep(
            card_key="copied_misa",
            card_owner_key="copied_misa_owner",
            hero_id=actor.id,
            suppress_after_resolve_card=True,
        )
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/misa-primary"
    state.execution_stack = [
        SetContextFlagStep(key="misa_tail", value=True, step_id="fixture/misa-tail"),
        step,
    ]
    state.execution_context["misa_step"] = step
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
            if variant.endswith("survives"):
                return next(o.id for o in request.options if o.id != "PASS")
            return "PASS"
        if request.can_skip and (mode == "skip" or variant == "ultimate-decline"):
            return "SKIP"
        if kind == "SELECT_NUMBER" and "Choose one" in request.prompt:
            return 2 if mode == "last" or variant == "ultimate-defeat" else 1
        if kind == "SELECT_HEX":
            wanted = None
            if "straight line, pass through" in request.prompt:
                wanted = {
                    "q": 3 if variant == "long-line" else 2,
                    "r": 0,
                    "s": -3 if variant == "long-line" else -2,
                }
            elif "2+ enemy" in request.prompt:
                wanted = {"q": 1, "r": -1, "s": 0}
            if effect == "watch_how_i_soar" and (
                "maximum range" in request.prompt or "max range" in request.prompt
            ):
                from goa2.domain.hex import Hex

                def score(option):
                    point = Hex(**selection_value(option))
                    heroes = sum(
                        state.get_position(unit.id) is not None
                        and state.get_position(unit.id).distance(point) == 1
                        for unit in state.teams["BLUE"].heroes
                    )
                    minions = sum(
                        state.get_position(unit.id) is not None
                        and state.get_position(unit.id).distance(point) == 1
                        for unit in state.teams["BLUE"].minions
                    )
                    return (bool(minions), heroes, minions)

                return selection_value(max(request.options, key=score))
            if wanted is not None:
                for option in request.options:
                    if selection_value(option) == wanted:
                        return wanted
        if kind in ("SELECT_UNIT", "SELECT_UNIT_OR_TOKEN"):
            for target in ("hero_arien", "hero_gydion", "hero_tigerclaw"):
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
                yield f"misa-primary-{effect}-{entry}-{mode}", state, ops
    for effect in DYNAMIC:
        for entry in ("text", "action", "perform", "copy"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode, "ultimate")
                yield f"misa-primary-{effect}-{entry}-{mode}-ultimate", state, ops
        for variant in (
            "ultimate-place",
            "ultimate-defeat",
            "ultimate-decline",
            "level-seven",
            "no-ultimate",
            "foreign-ultimate",
            "radius-none",
            "radius-zero",
            "radius-negative",
            "no-enemies",
            "null-count-origin",
        ):
            state, ops, _ = action_scene(
                effect, "copy" if variant == "foreign-ultimate" else "text", variant=variant
            )
            yield f"misa-primary-{effect}-{variant}", state, ops
    for effect in RED:
        for variant in ("negative-attack", "no-enemies"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"misa-primary-{effect}-{variant}", state, ops
    for effect in ("power_shot", "thunder_shot"):
        for variant in ("max-range-survives", "adjacent-survives", "out-of-range"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"misa-primary-{effect}-{variant}", state, ops
    for effect in BLUE:
        for variant in ("cross-minion", "no-crossed", "long-line", "blocked", "items"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"misa-primary-{effect}-{variant}", state, ops
    for effect in ("living_tornado", "storm_spirit"):
        for variant in ("radius-none", "radius-zero", "radius-negative"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"misa-primary-{effect}-{variant}", state, ops
