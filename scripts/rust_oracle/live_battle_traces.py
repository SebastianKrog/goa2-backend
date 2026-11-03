"""Authoritative battle flows reuse source scenarios and retain physical aliases."""

from __future__ import annotations

import copy


def alter(state, label, variant):
    from combat_effect_traces import effect

    from goa2.domain.hex import Hex
    from goa2.domain.models.effect import EffectType
    from goa2.domain.models.spawn import SpawnType

    step = state.execution_stack[-1]
    if variant == "same-hex-respawn":
        old = []
        for lane, zone_id in state.battle_zones.items():
            zone = state.board.zones[zone_id]
            unit = next(
                m
                for team in state.teams.values()
                for m in team.minions
                if m.lane_id == lane and state.get_position(m.id) is not None
            )
            spawn = next(sp for sp in zone.spawn_points if sp.team == unit.team)
            previous = spawn.location.model_copy()
            state.move_unit(unit.id, previous)
            old.append((str(unit.id), previous))
        state.execution_context["old_position_aliases"] = old
    elif variant == "linked-effects":
        token = "battle_token"
        state.active_effects = [
            effect(state, "source-linked", EffectType.MOVEMENT_ZONE, source=token),
            effect(state, "token-linked", EffectType.MOVEMENT_ZONE, token_id=token),
            effect(state, "origin-linked", EffectType.MOVEMENT_ZONE),
            effect(state, "unrelated", EffectType.MOVEMENT_ZONE),
        ]
        state.active_effects[2].scope.origin_id = token
    elif variant == "pending-forced":
        step.pending_input = {"selection": "stale"}
    elif variant == "pending-valid":
        chosen = next(
            m
            for team in state.teams.values()
            for m in team.minions
            if str(m.team) == step.losing_team
            and not m.is_heavy
            and state.get_position(m.id) is not None
        )
        step.pending_input = {"selection": str(chosen.id)}
    elif variant == "numeric-id":
        unit = next(
            m for m in state.teams["RED"].minions if state.get_position(m.id) and not m.is_heavy
        )
        previous = unit.id
        point = state.entity_locations.pop(previous)
        unit.id = "1"
        state.entity_locations[unit.id] = point
        state.board.tiles[point].occupant_id = unit.id
        step.pending_input = {"selection": 1}
    elif variant == "pending-invalid":
        step.pending_input = {"selection": "bad"}
    elif variant == "pending-origin":
        step.pending_input = {"selection": step.displacements[0][1].model_dump(mode="json")}
    elif variant == "pending-hex":
        chosen = Hex(q=1, r=0, s=-1)
        state.execution_context["selected_hex_alias"] = chosen
        step.pending_input = {"selection": chosen}
    elif variant == "duplicate-unit":
        uid, origin = step.displacements[0]
        step.displacements = [(uid, origin), (uid, origin.model_copy())]
        state.execution_context["duplicate_origin_aliases"] = [p for _, p in step.displacements]
    elif variant == "null-coin":
        state.tie_breaker_team = None
    elif variant == "hero-default-lane":
        step.displacements[0] = ("hero_arien", step.displacements[0][1])
    elif variant == "non-minion-prefix":
        zone = state.board.zones["lane_1-3"]
        zone.spawn_points.insert(
            0, zone.spawn_points[0].model_copy(update={"type": SpawnType.HERO, "minion_type": None})
        )
    elif variant == "battle-exclusion":
        origin = "hero_arien"
        target = next(m for m in state.teams["BLUE"].minions if state.get_position(m.id))
        hex = next(
            h
            for h in state.get_position(target.id).neighbors()
            if h in state.board.tiles and not state.board.tiles[h].is_occupied
        )
        state.place_entity(origin, hex)
        state.active_effects.append(
            effect(
                state,
                "battle-exclusion",
                EffectType.MINION_BATTLE_EXCLUSION,
                source=origin,
                max_value=1,
            )
        )


def scenes(names=None):
    from battle_traces import cases
    from object_execution_traces import live_choice_operations

    retained = []
    cases(
        {"kind": "advance"},
        lambda value: {"kind": "input", "selection": value},
        live_sink=lambda name, state, _ops: retained.append((name, state)),
    )
    extras = {
        "multi-both-thrones": ("same-hex-respawn",),
        "spawn-blocked-token": ("linked-effects",),
        "casualties-force-over": ("pending-forced",),
        "casualties-choice": ("pending-valid", "numeric-id"),
        "displace-one-choice": ("pending-invalid", "pending-hex"),
        "displace-two-red": ("pending-origin", "duplicate-unit"),
        "displace-two-teams": ("null-coin", "hero-default-lane"),
        "spawn-blocked-one": ("non-minion-prefix",),
        "minions-tie": ("battle-exclusion",),
    }
    for label, original in retained:
        for variant in ("ordinary", *extras.get(label, ())):
            for mode in ("first", "last", "invalid"):
                suffix = "" if variant == "ordinary" else f"-{variant}"
                name = f"live-battle-{label}{suffix}-{mode}"
                if names is not None and name not in names:
                    continue
                state = copy.deepcopy(original)
                alter(state, label, variant)
                context = state.execution_context
                context.update(
                    battle_zone_alias=state.battle_zones,
                    wave_alias=state.wave_counters,
                    location_alias=state.entity_locations,
                    effects_alias=state.active_effects,
                    battle_step_alias=state.execution_stack[-1],
                    pending_alias=state.execution_stack[-1].pending_input,
                    spawn_hex_aliases=[
                        sp.location
                        for zone in state.board.zones.values()
                        for sp in zone.spawn_points
                    ],
                )
                operations = live_choice_operations(state, mode=mode, stop_types=(), max_steps=256)
                operations.append({"kind": "advance_prefix", "steps": 0})
                yield name, state, operations
