"""Wasp primary flows and the exact boolean/type query contracts they use.

Raw setup preserves physical cards, nested steps, tokens and retained aliases.
"""

from primary_templates import WASP_EFFECTS as EFFECTS

ATTACKS = (
    "magnetic_dagger",
    "charged_boomerang",
    "shock",
    "electrocute",
    "electroblast",
    "thunder_boomerang",
)
ORBITS = ("lift_up", "control_gravity", "center_of_mass")
TELEPORTS = ("telekinesis", "mass_telekinesis")
KINETIC = ("kinetic_repulse", "kinetic_blast")


def base(effect, variant="ordinary"):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, StatType, TokenType

    state = setup(["Wasp", "Wuk"], ["Brogan", "Tigerclaw"])
    h = _board(state)
    hero = state.get_hero("hero_wasp")
    card = next(c for c in hero.deck if c.effect_id == effect)
    hero.current_turn_card = card
    if card in hero.hand:
        hero.hand.remove(card)
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    state.phase, state.current_actor_id, state.battle_zones = GamePhase.RESOLUTION, hero.id, {}
    for unit, point in (
        (hero.id, h(0, 0)),
        ("hero_wuk", h(-1, 1)),
        ("hero_brogan", h(1, 0)),
        ("hero_tigerclaw", h(2, -1)),
    ):
        state.place_entity(unit, point)
    foe = state.teams["BLUE"].minions[0]
    state.place_entity(foe.id, h(0, 1))
    if effect in (*TELEPORTS, "charged_boomerang", "thunder_boomerang"):
        state.place_entity("hero_brogan", h(1, 1))
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
    if variant == "survives":
        state.get_hero("hero_brogan").items = {StatType.DEFENSE: 20}
        state.get_hero("hero_tigerclaw").items = {StatType.DEFENSE: 20}
    if variant in ("collision", "empty-hand", "minion-only"):
        state.board.tiles[h(2, 0)].is_terrain = True
        state.board.tiles[h(0, 2)].is_terrain = True
    if variant == "empty-hand":
        state.get_hero("hero_brogan").hand = []
        state.get_hero("hero_tigerclaw").hand = []
    if variant == "minion-only":
        state.remove_entity("hero_brogan")
    if variant == "token":
        token = state.token_pool[TokenType.TREE][0]
        token.owner_id = hero.id
        token.is_immune_to_enemy_actions = False
        state.remove_entity("hero_brogan")
        state.place_entity(token.id, h(1, 1) if effect in TELEPORTS else h(1, 0))
        state.execution_context["wasp_token_alias"] = token
        state.execution_context["wasp_token_id"] = str(token.id)
    if variant == "blocked":
        occupied = set(state.entity_locations.values())
        for tile in state.board.tiles.values():
            if tile.hex not in occupied:
                tile.is_terrain = True
    if variant == "no-enemies":
        for unit in [*state.teams["BLUE"].heroes, *state.teams["BLUE"].minions]:
            state.remove_entity(unit.id)
    state.execution_context.update(
        wasp_alias=hero,
        wasp_card_alias=card,
        wasp_hand_alias=hero.hand,
        locations_alias=state.entity_locations,
        effects_alias=state.active_effects,
        selected_card=card.id,
    )
    return state, hero, card


def action_scene(effect, entry="text", mode="first", variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
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
        state.current_actor_id = "hero_wuk"
        state.execution_context.update(copied_wasp=card.id, copied_wasp_owner=hero.id)
        step = PerformCardActionStep(
            card_key="copied_wasp",
            card_owner_key="copied_wasp_owner",
            hero_id="hero_wuk",
            suppress_after_resolve_card=True,
        )
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/wasp-primary"
    state.execution_stack = [
        SetContextFlagStep(key="wasp_tail", value=True, step_id="fixture/wasp-tail"),
        step,
    ]
    state.execution_context["wasp_step"] = step
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
        if request.can_skip and (mode == "skip" or variant == "decline"):
            return "SKIP"
        if any(o.id == "YES" for o in request.options):
            return "NO" if mode in ("last", "skip") or variant == "decline" else "YES"
        if kind == "SELECT_NUMBER":
            return 4 if variant == "distance-four" or mode == "last" else 3
        if kind in ("SELECT_UNIT", "SELECT_UNIT_OR_TOKEN"):
            token = state.execution_context.get("wasp_token_id")
            preferred = [token, "hero_brogan", "hero_tigerclaw"]
            for target in preferred:
                if target and any(o.id == target for o in request.options):
                    return target
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, final = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}], final


def scenes():
    for effect in EFFECTS:
        for entry in ("text", "action", "perform", "copy"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode)
                yield f"wasp-primary-{effect}-{entry}-{mode}", state, ops
        for variant in ("items", "radius-none", "radius-zero", "radius-negative", "no-enemies"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"wasp-primary-{effect}-{variant}", state, ops
    for effect in ATTACKS:
        for variant in ("negative-attack", "survives"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"wasp-primary-{effect}-{variant}", state, ops
    for effect in (*ORBITS, *TELEPORTS):
        for variant in ("token", "blocked", "decline"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"wasp-primary-{effect}-{variant}", state, ops
    for effect in KINETIC:
        for variant in ("collision", "empty-hand", "minion-only", "decline", "distance-four"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"wasp-primary-{effect}-{variant}", state, ops

    for effect in ("shock", "electrocute", "electroblast"):
        state, ops, _ = action_scene(effect, variant="empty-hand")
        yield f"wasp-primary-{effect}-empty-hand", state, ops


def query_scenes():
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.domain.models import GamePhase, TokenType
    from goa2.engine.steps import CheckUnitTypeStep, CombineBooleanContextStep, SetContextFlagStep

    def world():
        state = setup(["Wasp", "Razzle"], ["Brogan", "Wuk"])
        h = _board(state)
        state.phase, state.current_actor_id, state.battle_zones = (
            GamePhase.RESOLUTION,
            "hero_wasp",
            {},
        )
        state.place_entity("hero_wasp", h(0, 0))
        return state

    def retain(state, step):
        step.step_id = "fixture/wasp-query"
        state.execution_stack = [
            SetContextFlagStep(
                key="wasp_query_tail", value=True, step_id="fixture/wasp-query-tail"
            ),
            step,
        ]
        state.execution_context["query_alias"] = step
        return [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance_prefix", "steps": 0}]

    values = (
        "hero",
        "minion",
        "token",
        "piece",
        "unknown",
        "missing",
        "null",
        "empty",
        "false",
        "zero",
        "numeric",
    )
    for expected in ("HERO", "MINION", "UNKNOWN"):
        for variant in values:
            state = world()
            value = {
                "hero": "hero_wasp",
                "minion": str(state.teams["BLUE"].minions[0].id),
                "token": str(state.token_pool[TokenType.TREE][0].id),
                "piece": _pieces(state, "hero_razzle")[0],
                "unknown": "not-a-unit",
                "null": None,
                "empty": "",
                "false": False,
                "zero": 0,
                "numeric": 42,
            }.get(variant)
            if variant == "numeric":
                state.get_hero("hero_wasp").id = "42"
            if variant != "missing":
                state.execution_context["subject"] = value
            step = CheckUnitTypeStep(
                unit_key="subject", expected_type=expected, output_key="query_result"
            )
            yield f"wasp-query-type-{expected}-{variant}", state, retain(state, step)
        for literal in ("hero_wasp", "not-a-unit", ""):
            state = world()
            state.execution_context["subject"] = "hero_brogan"
            step = CheckUnitTypeStep(
                unit_id=literal,
                unit_key="subject",
                expected_type=expected,
                output_key="query_result",
            )
            yield f"wasp-query-type-{expected}-literal-{literal or 'empty'}", state, retain(
                state, step
            )
    pairs = (
        (False, False),
        (False, True),
        (True, False),
        (True, True),
        (None, 1),
        ([], True),
        ([0], False),
        ({}, True),
        ({"a": 0}, True),
        (0, -1),
        ("", "false"),
        ("false", 1),
    )
    for operation in ("AND", "OR", "unknown", "or"):
        for i, (left, right) in enumerate(pairs):
            state = world()
            state.execution_context.update(left=left, right=right)
            step = CombineBooleanContextStep(
                key_a="left", key_b="right", operation=operation, output_key="query_result"
            )
            yield f"wasp-query-boolean-{operation}-{i}", state, retain(state, step)
        state = world()
        step = CombineBooleanContextStep(
            key_a="absent", key_b="absent", operation=operation, output_key="query_result"
        )
        yield f"wasp-query-boolean-{operation}-missing", state, retain(state, step)
