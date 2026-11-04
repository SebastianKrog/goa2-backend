"""Frozen source traces for independent sight blockers and Bain's line attacks."""

from itertools import product


def board(variant):
    from aura_traces import _board, _pieces
    from combat_effect_traces import effect
    from model_traces import setup

    from goa2.domain.models import GamePhase, TokenType
    from goa2.domain.models.effect import EffectType

    state = setup(["Bain", "Min"], ["Brogan", "Razzle"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = state.resolution_owner_id = "hero_bain"
    piece = _pieces(state, "hero_razzle")[0]
    destination = h(2, 1) if variant == "off-axis" else h(3, 0)
    for unit, point in [
        ("hero_bain", h(0, 0)),
        ("hero_min", h(-1, 0)),
        ("hero_brogan", destination),
        (piece, h(0, 2)),
    ]:
        state.place_entity(unit, point)
    middle = h(1, 0)
    if variant in ("unit", "petrify"):
        minion = next(m for m in state.teams["BLUE"].minions if not m.is_heavy)
        state.place_entity(minion.id, middle)
    if variant == "petrify":
        state.active_effects = [effect(state, "sight-petrify", EffectType.PETRIFY)]
    if variant == "barrier":
        state.active_effects = [
            effect(state, "sight-barrier", EffectType.STATIC_BARRIER, barrier_radius=2)
        ]
    if variant == "token":
        smoke = state.token_pool[TokenType.SMOKE_BOMB][0]
        smoke.owner_id = "hero_min"
        state.place_entity(smoke.id, middle)
    if variant == "terrain":
        state.board.tiles[middle].is_terrain = True
    if variant == "destination":
        state.board.tiles[destination].is_terrain = True
    if variant == "gap":
        state.board.tiles.pop(middle)
    if variant == "missing-origin":
        state.remove_entity("hero_bain")
    if variant == "piece-origin":
        state.current_actor_id = "hero_razzle"
        state.acting_piece_id = piece
    return state, destination


def scenes():
    from live_defeat_traces import retain

    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState, TargetType
    from goa2.engine.filters_geometry import ClearLineOfSightFilter
    from goa2.engine.steps import (
        PerformCardActionStep,
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SelectStep,
    )

    variants = (
        "clear",
        "unit",
        "terrain",
        "token",
        "petrify",
        "barrier",
        "gap",
        "destination",
        "off-axis",
        "missing-origin",
        "piece-origin",
    )
    for variant in variants:
        for units, terrain, obstacles in product((False, True), repeat=3):
            state, destination = board(variant)
            sight = ClearLineOfSightFilter(
                blocked_by_units=units,
                blocked_by_terrain=terrain,
                blocked_by_obstacles=obstacles,
            )
            if variant in ("clear", "destination", "gap", "off-axis", "missing-origin"):
                expected = variant in ("clear", "destination")
                assert sight.apply(destination, state, {}) == expected
                assert sight.apply("hero_brogan", state, {}) == expected
            steps = [
                SelectStep(
                    target_type=kind,
                    filters=[sight],
                    output_key=key,
                    prompt="Choose a visible target",
                    is_mandatory=False,
                )
                for kind, key in [
                    (TargetType.HEX, "sight_hex"),
                    (TargetType.UNIT_OR_TOKEN, "sight_unit"),
                ]
            ]
            ops, _ = retain(state, steps)
            flags = f"{int(units)}{int(terrain)}{int(obstacles)}"
            yield f"line-of-sight-rule-{variant}-{flags}", state, ops

    for literal, value in [
        (None, "hero_min"),
        ("hero_bain", "missing"),
        (None, False),
        (None, "missing"),
        (None, 13),
    ]:
        state, _ = board("clear")
        state.execution_context["sight_origin"] = value
        step = SelectStep(
            target_type=TargetType.UNIT,
            filters=[ClearLineOfSightFilter(origin_id=literal, origin_key="sight_origin")],
            output_key="sight_target",
            prompt="Choose a visible unit",
            is_mandatory=False,
        )
        ops, _ = retain(state, [step])
        yield f"line-of-sight-origin-{literal}-{value}", state, ops

    for effect_id in ("light_crossbow", "heavy_crossbow", "arbalest", "get_over_here"):
        entries = [
            (entry, mode, "clear")
            for entry in ("text", "action", "perform", "copy")
            for mode in ("first", "last", "invalid")
        ]
        entries.extend(("text", "first", variant) for variant in variants if variant != "clear")
        for entry, mode, variant in entries:
            state, _ = board(variant)
            bain = state.get_hero("hero_bain")
            card = next(c for c in bain.deck if c.effect_id == effect_id)
            if card in bain.hand:
                bain.hand.remove(card)
            bain.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            state.execution_context["sight_card_alias"] = card
            if entry == "text":
                step = ResolveCardTextStep(hero_id=bain.id, card_id=card.id)
            elif entry == "action":
                step = ResolveCardStep(hero_id=bain.id)
            elif entry == "copy":
                state.current_actor_id = "hero_min"
                state.execution_context.update(sight_copy=card.id, sight_owner=bain.id)
                step = PerformCardActionStep(
                    card_key="sight_copy",
                    card_owner_key="sight_owner",
                    hero_id="hero_min",
                    suppress_after_resolve_card=True,
                )
            else:
                step = PerformPrimaryActionStep(hero_id=bain.id)
            invalid_sent = False

            def choose(request, mode, primary=card.primary_action.value):
                nonlocal invalid_sent
                if mode == "invalid" and not invalid_sent:
                    invalid_sent = True
                    return "not-offered"
                if request.request_type.value == "CHOOSE_ACTION":
                    return primary
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                if request.request_type.value == "CONFIRM_PASSIVE":
                    return "NO"
                return selection_value(
                    request.options[-1] if mode == "last" else request.options[0]
                )

            ops, _ = retain(state, [step], mode=mode, select=choose)
            yield f"line-of-sight-card-{effect_id}-{entry}-{mode}-{variant}", state, ops

    yield from grappling_scenes()
    yield from plank_scenes()


def grappling_scenes():
    from aura_traces import _board, _pieces
    from combat_effect_traces import effect
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState, GamePhase, TokenType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import (
        MoveTowardTargetStep,
        PerformCardActionStep,
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
    )

    def setup_board(variant):
        state = setup(["Cutter", "Razzle"], ["Brogan", "Min"])
        h = _board(state)
        piece = _pieces(state, "hero_razzle")[0]
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = state.resolution_owner_id = "hero_cutter"
        anchor = h(3, 0)
        state.place_entity("hero_cutter", h(0, 0))
        state.place_entity(piece, h(-1, 0))
        state.place_entity("hero_brogan", h(2, 1))
        state.place_entity("hero_min", h(1, 2))
        if variant == "unit":
            state.place_entity("hero_brogan", anchor)
        elif variant == "token":
            smoke = state.token_pool[TokenType.SMOKE_BOMB][0]
            smoke.owner_id = "hero_min"
            state.place_entity(smoke.id, anchor)
        else:
            state.board.tiles[anchor].is_terrain = True
        if variant == "blocked":
            state.board.tiles[h(1, 0)].is_terrain = True
        if variant == "split":
            split = effect(state, "grapple-split", EffectType.TOPOLOGY_SPLIT)
            split.scope.range = 1
            state.active_effects = [split]
        return state, h, piece

    for variant in (
        "clear",
        "adjacent",
        "same",
        "off-axis",
        "missing",
        "null",
        "dict",
        "no-actor",
        "off-board",
        "piece",
        "explicit",
        "blocked",
        "split",
    ):
        state, h, piece = setup_board(variant)
        destination = {
            "adjacent": h(1, 0),
            "same": h(0, 0),
            "off-axis": h(2, 1),
        }.get(variant, h(3, 0))
        if variant != "missing":
            state.execution_context["anchor"] = (
                None
                if variant == "null"
                else destination.model_dump() if variant == "dict" else destination
            )
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "off-board":
            state.remove_entity("hero_cutter")
        if variant == "piece":
            state.current_actor_id = "hero_razzle"
            state.acting_piece_id = piece
        unit = piece if variant == "explicit" else None
        ops, _ = retain(state, [MoveTowardTargetStep(target_hex_key="anchor", unit_id=unit)])
        yield f"grappling-pull-{variant}", state, ops

    for variant in ("clear", "unit", "token", "blocked", "split"):
        for entry in ("text", "action", "perform", "copy"):
            state, _, piece = setup_board(variant)
            cutter = state.get_hero("hero_cutter")
            card = next(c for c in cutter.deck if c.effect_id == "grappling_bolt")
            if card in cutter.hand:
                cutter.hand.remove(card)
            cutter.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            state.execution_context["grapple_card_alias"] = card
            if entry == "text":
                step = ResolveCardTextStep(hero_id=cutter.id, card_id=card.id)
            elif entry == "action":
                step = ResolveCardStep(hero_id=cutter.id)
            elif entry == "copy":
                state.current_actor_id = "hero_razzle"
                state.acting_piece_id = piece
                state.execution_context.update(grapple_copy=card.id, grapple_owner=cutter.id)
                step = PerformCardActionStep(
                    card_key="grapple_copy",
                    card_owner_key="grapple_owner",
                    hero_id="hero_razzle",
                    suppress_after_resolve_card=True,
                )
            else:
                step = PerformPrimaryActionStep(hero_id=cutter.id)

            def choose(request, _mode, primary=card.primary_action.value):
                if request.request_type.value == "CHOOSE_ACTION":
                    return primary
                return selection_value(request.options[0])

            ops, _ = retain(state, [step], select=choose)
            yield f"grappling-card-{variant}-{entry}", state, ops


def plank_scenes():
    from aura_traces import _board
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.board import Zone
    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState, GamePhase
    from goa2.engine.steps import CheckZoneChangeStep, ResolveCardStep, ResolveCardTextStep

    def setup_board():
        state = setup(["Cutter", "Arien"], ["Brogan", "Min"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = state.resolution_owner_id = "hero_cutter"
        state.battle_zones = {}
        state.place_entity("hero_cutter", h(0, 0))
        state.place_entity("hero_brogan", h(1, 0))
        minion = next(m for m in state.teams["BLUE"].minions if not m.is_heavy)
        state.place_entity(minion.id, h(0, 1))
        state.board.zones = {
            "origin": Zone(id="origin", hexes={h(0, 0), h(1, 0)}),
            "landing": Zone(id="landing", hexes={h(2, 0), h(3, 0), h(4, 0)}),
        }
        for zone in state.board.zones.values():
            for point in zone.hexes:
                state.board.tiles[point].zone_id = zone.id
        return state, h

    for variant in (
        "same",
        "changed",
        "dict",
        "missing",
        "off-board",
        "fallback",
        "tile-null",
        "both-null",
    ):
        state, h = setup_board()
        before = h(1, 0) if variant == "same" else h(2, 0)
        if variant == "fallback":
            state.board.tiles.pop(before)
        if variant in ("tile-null", "both-null"):
            state.board.tiles[before].zone_id = None
        if variant == "both-null":
            state.board.tiles[h(1, 0)].zone_id = None
        if variant == "off-board":
            state.remove_entity("hero_brogan")
        state.execution_context.update(subject="hero_brogan", changed="stale")
        if variant != "missing":
            state.execution_context["before"] = before.model_dump() if variant == "dict" else before
        step = CheckZoneChangeStep(
            unit_key="subject", before_hex_key="before", output_key="changed"
        )
        ops, final = retain(state, [step])
        assert final.execution_context["changed"] is (
            None if variant in ("same", "missing", "off-board", "both-null") else True
        )
        yield f"plank-zone-{variant}", state, ops

    for entry in ("text", "action"):
        for variant in ("push", "hold", "blocked", "same-zone", "empty-hand", "minion"):
            state, h = setup_board()
            cutter = state.get_hero("hero_cutter")
            card = next(c for c in cutter.deck if c.effect_id == "walk_the_plank")
            if card in cutter.hand:
                cutter.hand.remove(card)
            cutter.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            if variant == "blocked":
                state.board.tiles[h(2, 0)].is_terrain = True
            if variant == "same-zone":
                for point in (h(2, 0), h(3, 0), h(4, 0)):
                    state.board.tiles[point].zone_id = "origin"
            if variant == "empty-hand":
                state.get_hero("hero_brogan").hand = []
            step = (
                ResolveCardTextStep(hero_id=cutter.id, card_id=card.id)
                if entry == "text"
                else ResolveCardStep(hero_id=cutter.id)
            )

            def choose(request, _mode, variant=variant, primary=card.primary_action.value):
                if request.request_type.value == "CHOOSE_ACTION":
                    return primary
                if request.prompt == "Choose one" and variant == "minion":
                    return selection_value(request.options[-1])
                if request.prompt == "Push how far?" and variant != "hold":
                    return selection_value(request.options[-1])
                return selection_value(request.options[0])

            ops, _ = retain(state, [step], select=choose)
            yield f"plank-card-{entry}-{variant}", state, ops
