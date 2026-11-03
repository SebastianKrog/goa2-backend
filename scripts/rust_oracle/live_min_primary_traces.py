"""Min primary flows retain token, item, deferred-card and effect identities."""

from primary_templates import MIN_EFFECTS as EFFECTS

MINES = ("trip_mine", "cluster_mine", "minefield")
STANCES = ("crane_stance", "tiger_stance", "dragon_stance", "viper_stance", "cobra_stance")
GRENADES = ("death_grenade", "holy_death_grenade")


def base(effect, variant="ordinary"):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, CardTier, GamePhase, StatType, TokenType

    state = setup(["Min", "Wasp"], ["Brogan", "Tigerclaw"])
    h = _board(state)
    hero = state.get_hero("hero_min")
    card = next(c for c in hero.deck if c.effect_id == effect)
    hero.current_turn_card = card
    if card in hero.hand:
        hero.hand.remove(card)
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    state.phase, state.current_actor_id, state.battle_zones = GamePhase.RESOLUTION, hero.id, {}
    for unit, point in (
        (hero.id, h(0, 0)),
        ("hero_wasp", h(-1, 1)),
        ("hero_brogan", h(1, 0)),
        ("hero_tigerclaw", h(2, -1)),
    ):
        state.place_entity(unit, point)
    state.place_entity(state.teams["BLUE"].minions[0].id, h(0, 1))
    if effect in ("viper_stance", "cobra_stance") or variant == "smoke-existing":
        smoke = state.token_pool[TokenType.SMOKE_BOMB][0]
        smoke.owner_id = hero.id
        state.place_entity(smoke.id, h(2, 0))
        state.execution_context["min_smoke_alias"] = smoke
    if effect in GRENADES:
        state.place_entity("hero_brogan", h(1, 1))
        state.place_entity("hero_tigerclaw", h(2, -1))
    if effect == "fast_as_lightning" and variant != "no-red":
        red = next(c for c in hero.deck if c.effect_id == "crane_stance")
        if variant == "after-custom-target":
            from goa2.data.heroes.registry import HeroRegistry

            red = next(
                c for c in HeroRegistry.get("Wasp").deck if c.effect_id == "thunder_boomerang"
            ).model_copy(deep=True)
            state.execution_context["thunder_target_1"] = "hero_tigerclaw"
        if red in hero.hand:
            hero.hand.remove(red)
        red.state, red.is_facedown = CardState.DISCARD, False
        if variant == "after-played":
            red.state = CardState.RESOLVED
            hero.played_cards.append(red)
        else:
            hero.discard_pile.append(red)
        if variant == "after-facedown":
            red.is_facedown = True
        if variant == "after-unregistered":
            red.effect_id = "unregistered-red-effect"
        state.execution_context["min_red_alias"] = red
    if effect == "fast_as_lightning" and variant == "foreign-red":
        actor = state.get_hero("hero_wasp")
        foreign = next(c for c in actor.deck if c.effect_id == "electrocute")
        if foreign in actor.hand:
            actor.hand.remove(foreign)
        foreign.state, foreign.is_facedown = CardState.DISCARD, False
        actor.discard_pile.append(foreign)
        state.execution_context["foreign_red_alias"] = foreign
    if variant.startswith("radius-"):
        card.radius_value = {"radius-none": None, "radius-zero": 0, "radius-negative": -1}[variant]
    if variant == "items":
        hero.items = {
            StatType.ATTACK: -2,
            StatType.MOVEMENT: 2,
            StatType.RADIUS: 1,
            StatType.RANGE: 1,
        }
    if variant == "negative-attack":
        hero.items = {StatType.ATTACK: -12}
    if variant == "survives":
        for other in state.teams["BLUE"].heroes:
            other.items = {StatType.DEFENSE: 20}
    if variant == "blocked":
        occupied = set(state.entity_locations.values())
        for tile in state.board.tiles.values():
            if tile.hex not in occupied:
                tile.is_terrain = True
    if variant == "no-enemies":
        for unit in [*state.teams["BLUE"].heroes, *state.teams["BLUE"].minions]:
            state.remove_entity(unit.id)
    if variant == "empty-hands":
        for other in state.teams["BLUE"].heroes:
            other.hand = []
    if effect == "perfect_self" and variant in (
        "retired-one",
        "retired-many",
        "retired-facedown",
        "option-double",
        "option-item",
        "option-both",
        "retired-skip",
        "foreign-retired",
    ):
        actor = state.get_hero("hero_wasp") if variant == "foreign-retired" else hero
        retired = [c for c in actor.deck if c.tier == CardTier.II]
        for c in retired[: 3 if variant == "retired-many" else 1]:
            c.state = CardState.RETIRED
            c.is_facedown = variant == "retired-facedown"
        state.execution_context["min_retired_alias"] = retired[0]
    state.execution_context.update(
        min_alias=hero,
        min_card_alias=card,
        min_hand_alias=hero.hand,
        min_items_alias=hero.items,
        locations_alias=state.entity_locations,
        effects_alias=state.active_effects,
        selected_card=card.id,
    )
    return state, hero, card


def action_scene(effect, entry="text", mode="first", variant="ordinary"):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
        FindNextActorStep,
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
        state.current_actor_id = "hero_wasp"
        state.execution_context.update(copied_min=card.id, copied_min_owner=hero.id)
        step = PerformCardActionStep(
            card_key="copied_min",
            card_owner_key="copied_min_owner",
            hero_id="hero_wasp",
            suppress_after_resolve_card=True,
        )
    else:
        step = PerformPrimaryActionStep(hero_id=hero.id)
    step.step_id = "fixture/min-primary"
    tail = [SetContextFlagStep(key="min_tail", value=True, step_id="fixture/min-tail")]
    if effect in GRENADES:
        state.unresolved_hero_ids = []
        tail.append(FindNextActorStep(step_id="fixture/min-grenade-expiry"))
    state.execution_stack = list(reversed([step, *tail]))
    state.execution_context["min_step"] = step
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
        if request.can_skip and (mode == "skip" or variant == "retired-skip"):
            return "SKIP"
        if kind == "SELECT_NUMBER":
            if "one, or both" in request.prompt:
                return {"option-double": 1, "option-item": 2, "option-both": 3}.get(variant, 3)
            return 0 if mode == "last" else 3
        if kind == "SELECT_HEX" and effect in GRENADES and "Place Grenade" in request.prompt:
            wanted = {"q": 1, "r": 0, "s": -1}
            if any(selection_value(o) == wanted for o in request.options):
                return wanted
        if kind in ("SELECT_UNIT", "SELECT_UNIT_OR_TOKEN"):
            for target in ("hero_brogan", "hero_tigerclaw"):
                if any(o.id == target for o in request.options):
                    return target
        if any(o.id == "YES" for o in request.options):
            return "NO" if mode in ("skip", "last") else "YES"
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, final = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}], final


def scenes():
    for effect in EFFECTS:
        for entry in ("text", "action", "perform", "copy"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode)
                yield f"min-primary-{effect}-{entry}-{mode}", state, ops
        for variant in ("items", "radius-none", "radius-zero", "radius-negative", "blocked"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"min-primary-{effect}-{variant}", state, ops
    for effect in (*STANCES, "fast_as_lightning"):
        for variant in ("negative-attack", "survives", "no-enemies"):
            state, ops, _ = action_scene(effect, variant=variant)
            yield f"min-primary-{effect}-{variant}", state, ops
    for variant in (
        "retired-one",
        "retired-many",
        "retired-facedown",
        "option-double",
        "option-item",
        "option-both",
        "retired-skip",
    ):
        state, ops, _ = action_scene("perfect_self", variant=variant)
        yield f"min-primary-perfect_self-{variant}", state, ops
    state, ops, _ = action_scene("perfect_self", "copy", variant="foreign-retired")
    yield "min-primary-perfect_self-foreign-retired", state, ops
    for variant in (
        "no-red",
        "after-played",
        "after-facedown",
        "after-unregistered",
        "after-custom-target",
    ):
        state, ops, _ = action_scene("fast_as_lightning", variant=variant)
        yield f"min-primary-fast_as_lightning-{variant}", state, ops
    state, ops, _ = action_scene("fast_as_lightning", "copy", variant="foreign-red")
    yield "min-primary-fast_as_lightning-foreign-red", state, ops
    for effect in GRENADES:
        state, ops, _ = action_scene(effect, variant="empty-hands")
        yield f"min-primary-{effect}-empty-hands", state, ops

    yield from reperform_scenes()


def reperform_scenes():
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.engine.steps import PerformPrimaryActionStep, SetContextFlagStep

    state, hero, _ = base("perfect_self", "option-item")
    first = PerformPrimaryActionStep(hero_id=hero.id, step_id="fixture/min-reperform-first")
    second = PerformPrimaryActionStep(hero_id=hero.id, step_id="fixture/min-reperform-second")
    state.execution_stack = [
        SetContextFlagStep(key="min_repeat_tail", value=True, step_id="fixture/min-reperform-tail"),
        second,
        first,
    ]
    state.execution_context["second_primary_alias"] = second

    def select(request, _mode):
        if request.request_type.value == "SELECT_NUMBER":
            return 2
        if request.request_type.value == "CONFIRM_PASSIVE":
            return "NO"
        return selection_value(request.options[0])

    ops, _ = operations(state, select=select)
    yield "min-primary-perfect_self-reperform-after-convert", state, [
        *ops,
        {"kind": "advance_prefix", "steps": 0},
    ]


def item_scenes():
    from live_defeat_traces import retain

    from goa2.domain.models import CardState, CardTier, StatType
    from goa2.engine.steps import ConvertCardToItemStep

    variants = (
        "ordinary",
        "retired",
        "already-item",
        "facedown",
        "flags",
        "bonus",
        "negative-bonus",
        "shared-items",
        "same-step",
        "duplicate-card",
        "no-item",
        "missing-key",
        "null",
        "false",
        "zero",
        "empty",
        "unknown-card",
        "numeric-id",
        "missing-hero",
        "actor-fallback",
        "no-actor",
        "absent-from-deck",
        "skip",
    )
    for variant in variants:
        state, hero, _ = base("perfect_self")
        card = next(c for c in hero.deck if c.tier == CardTier.II)
        state.execution_context["convert_target"] = card.id
        if variant == "retired":
            card.state = CardState.RETIRED
        if variant == "already-item":
            card.state = CardState.ITEM
        if variant == "facedown":
            card.is_facedown = True
        if variant == "flags":
            card.initiative_override = 19
            card.is_active = True
            card.enraged_active_override = True
            card.passive_uses_this_turn = 2
        if variant in ("bonus", "negative-bonus"):
            hero.items[card.item] = 3 if variant == "bonus" else -2
        if variant == "shared-items":
            state.get_hero("hero_wasp").items = hero.items
            state.execution_context["shared_items_alias"] = hero.items
        if variant == "duplicate-card":
            first = card.model_copy(deep=True, update={"item": StatType.INITIATIVE})
            hero.deck.insert(0, first)
            state.execution_context["first_convert_alias"] = first
        if variant == "no-item":
            card.item = None
        if variant == "missing-key":
            state.execution_context.pop("convert_target")
        if variant in ("null", "false", "zero", "empty", "unknown-card"):
            state.execution_context["convert_target"] = {
                "null": None,
                "false": False,
                "zero": 0,
                "empty": "",
                "unknown-card": "not-a-card",
            }[variant]
        if variant == "numeric-id":
            card.id = "42"
            state.execution_context["convert_target"] = 42
        if variant == "absent-from-deck":
            hero.deck.remove(card)
        step = ConvertCardToItemStep(card_key="convert_target", hero_id=hero.id)
        if variant == "missing-hero":
            step.hero_id = "absent"
        if variant in ("actor-fallback", "no-actor"):
            step.hero_id = ""
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "skip":
            step.active_if_key = "absent"
        state.execution_context["converted_card_alias"] = card
        steps = [step, step] if variant == "same-step" else [step]
        yield f"min-item-{variant}", state, retain(state, steps)[0]


def after_scenes():
    from live_defeat_traces import retain

    from goa2.domain.input import selection_value
    from goa2.domain.models import CardColor, CardState
    from goa2.engine.steps import ApplyAfterAttackCardTextStep, DiscardCardStep

    def select(request, _mode):
        if request.request_type.value == "SELECT_CARD_OR_PASS":
            return "PASS"
        if request.request_type.value == "CONFIRM_PASSIVE":
            return "NO"
        if request.request_type.value == "SELECT_NUMBER":
            return 3
        if any(o.id == "YES" for o in request.options):
            return "YES"
        return selection_value(request.options[0])

    for variant in (
        "ordinary",
        "no-red",
        "played-first",
        "facedown-first",
        "unregistered-first",
        "no-attack-body",
        "attack-only",
        "late-discard",
        "custom-target",
        "missing-hero",
        "skip",
    ):
        base_variant = (
            "no-red"
            if variant
            in ("no-red", "late-discard", "no-attack-body", "attack-only", "custom-target")
            else "ordinary"
        )
        state, hero, _ = base("fast_as_lightning", base_variant)
        state.execution_context["defender_id"] = "hero_brogan"
        red = next(c for c in hero.deck if c.effect_id == "crane_stance")
        if variant in ("played-first", "facedown-first", "unregistered-first"):
            hero.discard_pile.remove(red)
            hero.played_cards.append(red)
            red.state = CardState.RESOLVED
            second = next(c for c in hero.deck if c.effect_id == "tiger_stance")
            second.state, second.is_facedown = CardState.DISCARD, False
            hero.discard_pile.append(second)
            if variant == "facedown-first":
                red.is_facedown = True
            if variant == "unregistered-first":
                red.effect_id = "unknown-red-effect"
        if variant == "no-attack-body":
            red = next(c for c in hero.deck if c.effect_id == "inner_strength")
            red.color = CardColor.RED
            red.state, red.is_facedown = CardState.DISCARD, False
            hero.discard_pile.append(red)
        if variant in ("attack-only", "custom-target"):
            from goa2.data.heroes.registry import HeroRegistry

            red = next(
                c
                for c in HeroRegistry.get("Wasp").deck
                if c.effect_id
                == ("charged_boomerang" if variant == "attack-only" else "thunder_boomerang")
            ).model_copy(deep=True)
            red.state, red.is_facedown = CardState.DISCARD, False
            hero.discard_pile.append(red)
            state.execution_context["thunder_target_1"] = "hero_tigerclaw"
        step = ApplyAfterAttackCardTextStep(hero_id=hero.id)
        if variant == "missing-hero":
            step.hero_id = "absent"
        if variant == "skip":
            step.active_if_key = "absent"
        steps = [step]
        if variant == "late-discard":
            red.state, red.is_facedown = CardState.HAND, False
            if red not in hero.hand:
                hero.hand.append(red)
            steps = [DiscardCardStep(hero_id=hero.id, card_id=red.id), step]
        state.execution_context["after_red_alias"] = red
        yield f"min-after-{variant}", state, retain(state, steps, select=select)[0]
