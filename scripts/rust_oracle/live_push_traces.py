"""Live push paths, actor validation, mine ordering and primary push bodies.

Raw setup preserves position/effect/token aliases to test the authoritative
arena independently of card-specific effect construction helpers.
"""

from __future__ import annotations


def scenes():
    from unittest.mock import patch

    import push_traces
    import stack_traces
    from aura_traces import _board
    from live_defeat_traces import operations, retain
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, Token, TokenType
    from goa2.engine.steps import PushUnitStep, ResolveCardTextStep

    captured = []

    def capture(state):
        captured.append(state)
        return {}

    with (
        patch.object(push_traces, "export_state", capture),
        patch.object(stack_traces, "reference_trace", return_value=[]),
    ):
        old = push_traces.cases(
            {"kind": "advance"},
            lambda value: {"kind": "input", "request_id": "current", "selection": value},
        )
    assert len(old) == len(captured), (len(old), len(captured))
    for case, state in zip(old, captured, strict=True):
        state.execution_context.update(
            push_steps=list(reversed(state.execution_stack)),
            locations_alias=state.entity_locations,
            effects_alias=state.active_effects,
        )
        ops, _ = operations(state)
        yield "arena-" + case["name"], state, [*ops, {"kind": "advance_prefix", "steps": 0}]

    from displacement_effect_traces import states

    for variant, state, h, pieces, heavy in states():
        target = "hero_brogan"
        if variant.startswith("target-"):
            target = {
                "target-token": "restriction-token",
                "target-token-empty-actor": "restriction-token",
                "target-piece": pieces[0],
                "target-heavy": str(heavy.id),
                "target-owner": "hero_razzle",
                "target-unknown": "unknown",
            }.get(variant, target)
        source = h(0, 0)
        if target != "hero_brogan" and target in state.entity_locations:
            location = state.entity_locations[target]
            source = location.neighbor(3)
        step = PushUnitStep(
            target_id=target,
            source_hex=source,
            distance=1,
            collision_output_key="collision",
            is_mandatory=variant == "mandatory",
        )
        if variant == "gate-off":
            step.active_if_key = "absent"
        if variant == "context-skip":
            step.skip_if_key = "blocked"
            state.execution_context["blocked"] = False
        if variant == "keys":
            step.target_id = None
            step.target_key = "unit"
            state.execution_context["unit"] = target
        state.execution_context.update(
            locations_alias=state.entity_locations, effects_alias=state.active_effects
        )
        ops, _ = retain(state, [step])
        yield "arena-push-restriction-" + variant, state, ops

    for variant in (
        "enemy-dud",
        "enemy-blast",
        "ordered-mines",
        "friendly",
        "unowned",
        "unknown-owner",
        "trim-last",
        "trim-ignore-wall",
        "pass-through-wall",
        "topology-anchor",
        "piece",
        "token",
        "zero",
        "negative",
    ):
        state = setup(["Min", "Razzle"], ["Arien", "Hanu"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        state.current_actor_id = "hero_hanu"
        state.place_entity("hero_hanu", h(-1, 0))
        target = "hero_razzle_piece_1" if variant == "piece" else "hero_arien"
        state.place_entity(target, h(0, 0))
        state.place_entity("hero_min", h(-3, 2))
        mines = []
        for i in range(1, 3):
            owner = "hero_min" if variant not in {"friendly", "piece"} else "hero_arien"
            if variant == "unowned":
                owner = None
            if variant == "unknown-owner":
                owner = "absent"
            token = Token(
                id=f"push-mine-{i}",
                name="Mine",
                token_type=TokenType.MINE_BLAST if variant == "enemy-blast" else TokenType.MINE_DUD,
                owner_id=owner,
                is_passable=True,
                is_facedown=True,
            )
            state.misc_entities[token.id] = token
            state.token_pool.setdefault(token.token_type, []).append(token)
            state.place_entity(token.id, h(i, 0))
            mines.append(token)
        if variant == "token":
            state.remove_entity(target)
            token = Token(
                id="pushed-token", name="Pushed", token_type=TokenType.TOTEM, owner_id="hero_arien"
            )
            state.misc_entities[token.id] = token
            state.token_pool.setdefault(token.token_type, []).append(token)
            target = str(token.id)
            state.place_entity(target, h(0, 0))
        distance = 2 if variant == "trim-last" else 3
        if variant == "zero":
            distance = 0
        if variant == "negative":
            distance = -2
        ignore = variant in {"trim-ignore-wall", "pass-through-wall"}
        if ignore:
            state.board.tiles[h(3, 0)].is_terrain = True
        if variant == "pass-through-wall":
            distance = 4
        if variant == "topology-anchor":
            from goa2.domain.models.effect import (
                ActiveEffect,
                DurationType,
                EffectScope,
                EffectType,
                Shape,
            )

            state.active_effects.append(
                ActiveEffect(
                    id="split",
                    source_id="hero_min",
                    effect_type=EffectType.TOPOLOGY_SPLIT,
                    scope=EffectScope(shape=Shape.GLOBAL),
                    duration=DurationType.PASSIVE,
                    created_at_turn=state.turn,
                    created_at_round=state.round,
                    is_active=True,
                    split_axis="q",
                    split_value=1,
                )
            )
        state.execution_context.update(
            old_position=state.entity_locations[target],
            mines_alias=mines,
            locations_alias=state.entity_locations,
            collision="old",
            push_victim_id="old",
        )
        step = PushUnitStep(
            target_id=target,
            distance=distance,
            ignore_obstacles=ignore,
            collision_output_key="collision",
        )
        ops, _ = retain(state, [step])
        yield "arena-push-mines-" + variant, state, ops

    from primary_templates import EFFECTS

    for effect in (
        "rogue_wave",
        "tidal_blast",
        "brutal_jab",
        "mighty_punch",
        "savage_kick",
        "point_blank_shot",
    ):
        for mode in ("first", "last", "skip", "invalid", "positive"):
            name = EFFECTS[effect]
            foe = "Arien" if name == "Brogan" else "Brogan"
            state = setup([name, "Swift"], [foe, "Hanu"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            hero = state.get_hero("hero_" + name.lower())
            state.current_actor_id = str(hero.id)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state = CardState.UNRESOLVED
            card.is_facedown = False
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_" + foe.lower(), h(1, 0))
            state.place_entity("hero_hanu", h(0, 1))
            state.place_entity("hero_swift", h(-1, 1))
            state.execution_context.update(
                hero_alias=hero, current_alias=card, locations_alias=state.entity_locations
            )

            def positive(request, _mode):
                from goa2.domain.input import selection_value

                values = [selection_value(option) for option in request.options]
                if request.request_type.value == "SELECT_NUMBER":
                    return max(values, key=lambda value: int(value))
                if request.request_type.value == "SELECT_HEX" and "move" in request.prompt.lower():
                    for value in values:
                        if value == {"q": 1, "r": -1, "s": 0}:
                            return value
                return values[0]

            ops, _ = retain(
                state,
                [ResolveCardTextStep(hero_id=str(hero.id), card_id="ignored")],
                mode,
                select=positive if mode == "positive" else None,
            )
            yield "arena-push-card-" + effect + "-" + mode, state, ops
