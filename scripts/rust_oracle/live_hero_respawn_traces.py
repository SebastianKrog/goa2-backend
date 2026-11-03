"""Hero respawn uses real source decisions with retained maps and physical aliases."""

from __future__ import annotations

import copy


def operations(state, mode="first", invalid=None):
    from goa2.domain.input import InputResponse, selection_value
    from goa2.engine.handler import process_stack, submit_input

    probe = copy.deepcopy(state)
    result = []
    injected = False
    for _ in range(12):
        if not probe.execution_stack:
            return result
        remaining = 1

        def stop(_state, _step):
            nonlocal remaining
            if not remaining:
                return True
            remaining -= 1
            return False

        out = process_stack(probe, stop_before_step=stop)
        result.append({"kind": "advance_prefix", "steps": 1})
        if out.input_request:
            req = out.input_request
            if invalid is not None and not injected:
                value = invalid
                injected = True
            elif req.request_type.value == "CHOOSE_RESPAWN":
                value = "PASS" if mode == "pass" else "RESPAWN"
            else:
                value = selection_value(req.options[-1] if mode == "last" else req.options[0])
            submit_input(probe, InputResponse(request_id=req.id, selection=value))
            result.append({"kind": "input", "selection": value})
    raise AssertionError("hero respawn did not finish")


def inputs():
    from combat_effect_traces import effect
    from model_traces import setup
    from oracle import ROOT

    from goa2.domain.board import Board, Zone
    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase, TeamColor, Token, TokenType
    from goa2.domain.models.effect import EffectType
    from goa2.domain.models.spawn import SpawnPoint, SpawnType
    from goa2.domain.tile import Tile
    from goa2.engine.setup import GameSetup
    from goa2.engine.steps import RespawnHeroStep

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def tiny(multi=False):
        state = setup(["Razzle", "Hanu"] if multi else None)
        for uid in list(state.entity_locations):
            state.remove_entity(uid)
        points = [h(q, r) for q in range(-2, 3) for r in range(-2, 3) if abs(q + r) <= 2]
        state.board = Board(
            map_id="hero_respawn_fixture",
            tiles={p: Tile(hex=p, zone_id="home") for p in points},
            zones={"home": Zone(id="home", hexes=set(points))},
            spawn_points=[
                SpawnPoint(location=h(0), team=TeamColor.RED, type=SpawnType.HERO),
                SpawnPoint(location=h(1), team=TeamColor.RED, type=SpawnType.HERO),
            ],
        )
        state.battle_zones = {}
        state.wave_counters = {}
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_razzle" if multi else "hero_arien"
        state.resolution_owner_id = state.current_actor_id
        return state

    variants = (
        "ordinary",
        "present",
        "unknown",
        "no-spawns",
        "no-space",
        "fallback",
        "fallback-second",
        "token",
        "terrain-token",
        "token-effects",
        "token-denied",
        "denied",
        "static-barrier",
        "piece",
        "piece-present",
        "piece-reversed-registry",
        "shared-snapshot",
        "duplicate-spawn",
        "skipped",
        "pending-pass-no-spawns",
        "pending-pass-no-space",
        "pending-respawn",
        "pending-valid",
        "pending-invalid",
        "pending-empty",
        "pending-null",
        "pending-hex-model",
        "pending-numeric",
        "pending-extra",
    )
    for variant in variants:
        multi = variant.startswith("piece")
        state = tiny(multi)
        hero = "hero_razzle" if multi else "hero_arien"
        step = RespawnHeroStep(hero_id="missing" if variant == "unknown" else hero)
        if variant in ("present", "piece-present"):
            state.place_entity(
                f"{hero}_piece_{state.get_hero(hero).piece_supply}" if multi else hero, h(-1)
            )
        if variant in ("no-spawns", "pending-pass-no-spawns"):
            state.board.spawn_points = []
        if variant in ("no-space", "pending-pass-no-space"):
            for tile in state.board.tiles.values():
                tile.is_terrain = True
        if variant in ("fallback", "fallback-second"):
            for sp in state.board.spawn_points:
                state.board.tiles[sp.location].is_terrain = True
            if variant == "fallback-second":
                state.board.spawn_points[0].location = h(20)
        if variant in ("token", "terrain-token", "token-effects", "token-denied"):
            token = Token(
                id="respawn_token", name="Token", token_type=TokenType.TOTEM, owner_id="hero_hanu"
            )
            state.misc_entities[token.id] = token
            state.place_entity(token.id, h(0))
            if variant == "terrain-token":
                state.board.tiles[h(0)].is_terrain = True
            if variant == "token-effects":
                state.active_effects = [
                    effect(state, "source-link", EffectType.MOVEMENT_ZONE, source=token.id),
                    effect(state, "token-link", EffectType.MOVEMENT_ZONE, token_id=token.id),
                    effect(state, "origin-link", EffectType.MOVEMENT_ZONE),
                    effect(state, "unrelated", EffectType.MOVEMENT_ZONE),
                ]
                state.active_effects[2].scope.origin_id = token.id
        if variant in ("denied", "token-denied", "static-barrier"):
            state.place_entity("hero_brogan", h(0, 1))
            e = effect(
                state,
                "spawn-obstacle",
                (
                    EffectType.STATIC_BARRIER
                    if variant == "static-barrier"
                    else EffectType.EMPTY_HEX_OBSTACLE
                ),
            )
            e.scope.range = 1
            e.barrier_radius = 1
            state.active_effects = [e]
        if variant == "piece-reversed-registry":
            state.misc_entities = dict(reversed(list(state.misc_entities.items())))
        if variant == "shared-snapshot":
            state.last_turn_positions = state.entity_locations
        if variant == "duplicate-spawn":
            state.board.spawn_points.append(state.board.spawn_points[0].model_copy())
        if variant == "skipped":
            step.skip_if_key = "skip_respawn"
            state.execution_context["skip_respawn"] = True
        pending = {
            "pending-pass-no-spawns": {"selection": "PASS"},
            "pending-pass-no-space": {"selection": "PASS"},
            "pending-respawn": {"selection": "RESPAWN"},
            "pending-valid": {"selection": h(0).model_dump()},
            "pending-invalid": {"selection": {"q": 0, "r": 0, "s": 1}},
            "pending-empty": {},
            "pending-null": {"selection": None},
            "pending-hex-model": {"selection": h(0)},
            "pending-numeric": {"selection": {"q": "0", "r": 0.0, "s": False}},
            "pending-extra": {"selection": {"q": 0, "r": 0, "s": 0, "ignored": True}},
        }
        if variant in pending:
            step.pending_input = pending[variant]
            state.execution_context["pending_input_alias"] = step.pending_input
            state.execution_context["selected_alias"] = step.pending_input.get("selection")
        yield variant, state, step
    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        for side in ("RED", "BLUE"):
            state = GameSetup.create_game(
                str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
            )
            hero = str(state.teams[side].heroes[0].id)
            state.remove_entity(hero)
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = hero
            state.resolution_owner_id = hero
            yield "map-" + path.stem + "-" + side, state, RespawnHeroStep(hero_id=hero)


def scenes(names=None):
    for label, state, step in inputs():
        for mode in ("first", "last", "pass"):
            name = "hero-respawn-live-" + label + "-" + mode
            if names is not None and name not in names:
                continue
            state.execution_stack = [step]
            initial = copy.deepcopy(state)
            root = initial.execution_stack[-1]
            root.step_id = "fixture/hero-respawn/step"
            initial.execution_stack = [root]
            initial.execution_context.update(
                step_alias=root,
                locations_alias=initial.entity_locations,
                snapshot_alias=initial.last_turn_positions,
                effects_alias=initial.active_effects,
                misc_alias=initial.misc_entities,
            )
            initial.execution_context["spawn_hex_aliases"] = [
                sp.location for sp in initial.board.spawn_points
            ]
            initial.execution_context["tile_aliases"] = list(initial.board.tiles.values())
            yield name, initial, operations(
                initial,
                mode=mode,
                invalid={"q": 0, "r": 0, "s": 1} if label == "ordinary" else None,
            )
