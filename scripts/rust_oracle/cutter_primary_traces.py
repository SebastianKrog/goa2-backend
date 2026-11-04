"""Frozen source coverage for Cutter's remaining primary bodies."""

EFFECTS = (
    "bombardment",
    "barrage",
    "broadside",
    "outmaneuver",
    "outsmart",
    "x_marks_the_spot",
    "a_fistful_of_coins",
    "brace_for_impact",
    "ramming_speed",
    "crashland",
    "evasive_shot",
    "tumble_shot",
    "daring_strike",
    "bold_thrust",
    "fearless_lunge",
)


def board(variant):
    from aura_traces import _board, _pieces
    from combat_effect_traces import effect
    from model_traces import setup

    from goa2.domain.models import GamePhase, StatType
    from goa2.domain.models.effect import EffectType

    state = setup(["Cutter", "Razzle"], ["Brogan", "Min"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = state.resolution_owner_id = "hero_cutter"
    state.battle_zones = {}
    piece = _pieces(state, "hero_razzle")[0]
    state.place_entity("hero_cutter", h(0, 0))
    state.place_entity(piece, h(-2, 1))
    state.place_entity("hero_brogan", h(2, 0))
    state.place_entity("hero_min", h(3, -1))
    minion = next(m for m in state.teams["BLUE"].minions if not m.is_heavy)
    state.place_entity(minion.id, h(0, 1))
    if variant == "empty-hand":
        for hero in state.teams["BLUE"].heroes:
            hero.hand = []
    if variant == "blocked":
        state.board.tiles[h(1, 0)].is_terrain = True
        state.board.tiles[h(-1, 0)].is_terrain = True
    if variant == "split":
        split = effect(state, "cutter-split", EffectType.TOPOLOGY_SPLIT)
        split.scope.range = 1
        state.active_effects = [split]
    if variant == "no-enemies":
        for unit in list(state.entity_locations):
            if unit not in ("hero_cutter", piece):
                state.remove_entity(unit)
    if variant == "negative":
        state.get_hero("hero_cutter").items = {StatType.ATTACK: -12}
    if variant == "win":
        state.get_hero("hero_cutter").gold = 12
    return state, h, piece


def scenes():
    from live_defeat_traces import retain

    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState, TargetType
    from goa2.engine.filters_geometry import SameDirectionFromOriginFilter
    from goa2.engine.steps import (
        CheckSoloWinStep,
        PerformCardActionStep,
        PerformPrimaryActionStep,
        ResolveCardStep,
        ResolveCardTextStep,
        SelectStep,
    )

    for variant in (
        "ordinary",
        "hex-origin",
        "unit-origin",
        "missing",
        "zero",
        "off-axis",
        "split",
    ):
        state, h, _ = board(variant)
        state.execution_context.update(
            reference=(
                h(0, 0)
                if variant == "zero"
                else h(1, 1).model_dump() if variant == "off-axis" else h(1, 0)
            ),
            origin=h(-1, 0).model_dump(),
            source="hero_cutter",
        )
        if variant == "missing":
            del state.execution_context["reference"]
        filt = SameDirectionFromOriginFilter(
            reference_key="reference",
            origin_hex_key="origin" if variant == "hex-origin" else None,
            origin_key="source" if variant == "unit-origin" else None,
        )
        steps = [
            SelectStep(
                target_type=kind, filters=[filt], prompt="Choose along the ray", is_mandatory=False
            )
            for kind in (TargetType.HEX, TargetType.UNIT)
        ]
        ops, _ = retain(state, steps)
        yield f"cutter-primary-ray-{variant}", state, ops

    for variant in ("below", "threshold", "above", "explicit", "missing", "no-actor", "custom"):
        state, _, _ = board("ordinary")
        state.get_hero("hero_cutter").gold = {"below": 12, "above": 14, "custom": 3}.get(
            variant, 13
        )
        state.get_hero("hero_brogan").gold = 13
        if variant in ("explicit", "missing"):
            state.execution_context["winner"] = (
                "hero_brogan" if variant == "explicit" else "missing"
            )
        if variant == "no-actor":
            state.current_actor_id = None
        step = CheckSoloWinStep(hero_key="winner", threshold=3 if variant == "custom" else 13)
        ops, final = retain(state, [step])
        assert (final.phase.value == "GAME_OVER") == (
            variant not in ("below", "missing", "no-actor")
        )
        yield f"cutter-primary-solo-{variant}", state, ops

    for effect_id in EFFECTS:
        entries = [
            (entry, mode, "ordinary")
            for entry in ("text", "action", "perform", "copy")
            for mode in ("first", "last", "skip", "invalid")
        ]
        entries.extend(
            ("text", "first", variant)
            for variant in ("empty-hand", "blocked", "split", "no-enemies")
        )
        if effect_id in (
            "daring_strike",
            "bold_thrust",
            "fearless_lunge",
            "evasive_shot",
            "tumble_shot",
        ):
            entries.append(("text", "first", "negative"))
        if effect_id == "a_fistful_of_coins":
            entries.append(("text", "last", "win"))
        for entry, mode, variant in entries:
            state, h, piece = board(variant)
            cutter = state.get_hero("hero_cutter")
            card = next(c for c in cutter.deck if c.effect_id == effect_id)
            if card in cutter.hand:
                cutter.hand.remove(card)
            cutter.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            state.execution_context["cutter_card_alias"] = card
            if entry == "text":
                step = ResolveCardTextStep(hero_id=cutter.id, card_id=card.id)
            elif entry == "action":
                step = ResolveCardStep(hero_id=cutter.id)
            elif entry == "copy":
                state.current_actor_id = "hero_razzle"
                state.acting_piece_id = piece
                state.execution_context.update(cutter_copy=card.id, cutter_owner=cutter.id)
                step = PerformCardActionStep(
                    card_key="cutter_copy",
                    card_owner_key="cutter_owner",
                    hero_id="hero_razzle",
                    suppress_after_resolve_card=True,
                )
            else:
                step = PerformPrimaryActionStep(hero_id=cutter.id)
            invalid_sent = False
            charge_value = h(1, 0).model_dump()

            def choose(request, mode, primary=card.primary_action.value, charge=charge_value):
                nonlocal invalid_sent
                kind = request.request_type.value
                if mode == "invalid" and not invalid_sent:
                    invalid_sent = True
                    return "not-offered"
                if kind == "CHOOSE_ACTION":
                    return primary
                if kind == "SELECT_CARD_OR_PASS":
                    return "PASS"
                if kind == "CONFIRM_PASSIVE":
                    return "NO"
                if request.can_skip and mode == "skip":
                    return "SKIP"
                if kind == "SELECT_HEX" and request.prompt == "Move in a straight line":
                    for option in request.options:
                        if selection_value(option) == charge:
                            return charge
                return selection_value(
                    request.options[-1] if mode == "last" else request.options[0]
                )

            ops, _ = retain(state, [step], mode=mode, select=choose)
            yield f"cutter-primary-{effect_id}-{entry}-{mode}-{variant}", state, ops
