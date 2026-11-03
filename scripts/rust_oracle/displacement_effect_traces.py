"""Ground-truth displacement validators and real placement/swap execution."""

from __future__ import annotations

from fixture_cache import read_text, write_text

import copy
import json

from oracle import ROOT, digest, verify_sources


def states():
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.domain.board import Zone
    from goa2.domain.models import GamePhase, TeamColor, Token, TokenType
    from goa2.domain.models.effect import (
        ActiveEffect,
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )
    from goa2.domain.models.enums import DisplacementType

    variants = [
        "plain",
        "blocked",
        "unrelated-type",
        "second-blocker",
        "duplicate-id",
        "offboard",
        "bound-piece",
        "missing-source",
        "source-token",
        "source-piece",
        "token-immune",
        "heavy-immunity",
        "heavy-alone",
        "attack-shield",
        "attack-except",
        "actor-immunity",
        "actor-immunity-friendly",
        "actor-immunity-radius",
        "no-actor",
        "system-actor",
        "empty-actor",
        "target-token",
        "target-piece",
        "target-heavy",
        "target-owner",
        "target-unknown",
        "target-token-empty-actor",
        "unknown-actor",
        "mandatory",
        "keys",
        "same-place",
        "same-swap",
        "swap-second",
        "gate-off",
        "context-skip",
        "live-origin",
        "fixed-origin",
        "live-piece-origin",
        "topology-split",
        "topology-isolation",
    ]
    variants += ["flags-" + str(flags) for flags in range(8)]
    variants += [
        "scope-" + shape.value + "-" + affects.value for shape in Shape for affects in AffectsFilter
    ]
    variants += [
        "duration-" + duration.value + "-" + which
        for duration in DurationType
        for which in [
            "current",
            "past-turn",
            "past-round",
            "future",
            "dormant",
            "cardless",
            "empty-card",
        ]
    ]
    variants += ["type-" + kind.value for kind in EffectType]
    for variant in variants:
        state = setup(["Razzle", "Arien"], ["Brogan", "Hanu"])
        h = _board(state)
        state.round = 3
        state.turn = 2
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_brogan"
        pieces = _pieces(state, "hero_razzle")
        state.acting_piece_id = None
        for uid, point in [
            ("hero_arien", h(0, 0)),
            ("hero_brogan", h(1, 0)),
            ("hero_hanu", h(3, -1)),
            (pieces[0], h(-1, 0)),
            (pieces[1], h(0, 2)),
        ]:
            state.place_entity(uid, point)
        token = Token(
            id="restriction-token",
            name="Restriction token",
            token_type=TokenType.TOTEM,
            owner_id="hero_arien",
        )
        state.misc_entities[token.id] = token
        state.token_pool.setdefault(TokenType.TOTEM, []).append(token)
        state.place_entity(token.id, h(-1, 1))
        red = state.teams[TeamColor.RED].minions
        heavy = next(m for m in red if m.is_heavy)
        support = next(m for m in red if not m.is_heavy)
        for m in red:
            m.lane_id = "laneA"
        state.place_entity(heavy.id, h(-2, 1))
        state.board.zones = {
            "area": Zone(id="area", hexes={h(-2, 1), h(-2, 2), h(0, 0), h(1, 0)}, neighbors=[])
        }
        for point in state.board.zones["area"].hexes:
            state.board.tiles[point].zone_id = "area"
        state.battle_zones = {"laneA": "area"}
        e = ActiveEffect(
            id="restriction",
            source_id="hero_arien",
            effect_type=EffectType.PLACEMENT_PREVENTION,
            scope=EffectScope(shape=Shape.RADIUS, range=2),
            duration=DurationType.PASSIVE,
            created_at_turn=2,
            created_at_round=3,
            is_active=True,
            displacement_blocks=list(DisplacementType),
        )
        if variant == "plain" or variant.startswith("target-"):
            e.displacement_blocks = []
        if variant.startswith("type-"):
            e.effect_type = EffectType(variant[5:])
        if variant == "unrelated-type":
            e.effect_type = EffectType.AREA_STAT_MODIFIER
        if variant in ("second-blocker", "duplicate-id"):
            first = e.model_copy(
                deep=True,
                update={
                    "id": e.id if variant == "duplicate-id" else "first-nonblocking",
                    "displacement_blocks": [DisplacementType.MOVE],
                },
            )
            state.active_effects.append(first)
        if variant.startswith("flags-"):
            flags = int(variant[6:])
            e.blocks_enemy_actors = bool(flags & 1)
            e.blocks_friendly_actors = bool(flags & 2)
            e.blocks_self = bool(flags & 4)
        if variant.startswith("scope-"):
            _, shape, affects = variant.split("-", 2)
            e.scope = EffectScope(
                shape=Shape(shape), range=2, affects=AffectsFilter(affects), direction=4
            )
        if variant.startswith("duration-"):
            _, duration, which = variant.split("-", 2)
            e.duration = DurationType(duration)
            e.source_card_id = str(state.get_hero("hero_arien").hand[0].id)
            if which == "past-turn":
                e.created_at_turn = 1
            if which == "past-round":
                e.created_at_round = 2
            if which == "future":
                e.created_at_turn = 3
            if which == "dormant":
                e.is_active = False
            if which == "cardless":
                e.source_card_id = None
                e.is_active = False
            if which == "empty-card":
                e.source_card_id = ""
                e.is_active = False
        if variant == "offboard":
            state.remove_entity("hero_brogan")
        if variant == "bound-piece":
            state.acting_piece_id = pieces[0]
        if variant == "missing-source":
            e.source_id = "unknown"
        if variant == "source-token":
            e.source_id = str(token.id)
        if variant == "source-piece":
            e.source_id = pieces[0]
        if variant == "token-immune":
            token.is_immune_to_enemy_actions = True
        if variant == "heavy-immunity":
            state.place_entity(support.id, h(-2, 2))
        if variant in (
            "attack-shield",
            "attack-except",
            "actor-immunity",
            "actor-immunity-friendly",
            "actor-immunity-radius",
        ):
            from goa2.domain.models.effect import AffectsFilter

            shield = e.model_copy(
                deep=True,
                update={
                    "id": "shield",
                    "source_id": "hero_brogan",
                    "effect_type": (
                        EffectType.ATTACK_IMMUNITY
                        if variant.startswith("attack")
                        else EffectType.IMMUNITY_ENEMY_ACTIONS
                    ),
                },
            )
            if variant.startswith("attack"):
                e.origin_attack_is_basic = True
            if variant == "attack-except":
                shield.except_attacker_ids = ["hero_arien"]
            if variant == "actor-immunity-friendly":
                shield.blocks_friendly_actors = True
            if variant == "actor-immunity-radius":
                shield.source_id = "hero_hanu"
                shield.scope = EffectScope(
                    shape=Shape.RADIUS, range=3, affects=AffectsFilter.FRIENDLY_HEROES
                )
            else:
                shield.scope = EffectScope(shape=Shape.GLOBAL)
            state.active_effects.append(shield)
        if variant in ("no-actor", "system-actor"):
            state.current_actor_id = None
        if variant == "system-actor":
            system = Token(
                id="system",
                name="System actor",
                token_type=TokenType.TOTEM,
                owner_id="hero_brogan",
                team=TeamColor.BLUE,
            )
            state.misc_entities[system.id] = system
            state.place_entity(system.id, h(-3, 0))
        if variant in ("empty-actor", "target-token-empty-actor"):
            state.current_actor_id = ""
        if variant == "unknown-actor":
            state.current_actor_id = "unknown"
        if variant == "swap-second":
            e.scope = EffectScope(shape=Shape.POINT, origin_id="hero_hanu")
        if variant in ("live-origin", "live-piece-origin"):
            state.current_actor_id = "hero_arien"
        if variant == "live-piece-origin":
            e.scope.origin_id = "hero_razzle"
            state.acting_piece_id = pieces[0]
        if variant == "fixed-origin":
            e.scope.origin_hex = h(3, -1)
        if variant.startswith("topology-"):
            state.active_effects.append(
                e.model_copy(
                    deep=True,
                    update={
                        "id": "topology",
                        "source_id": "hero_brogan",
                        "effect_type": (
                            EffectType.TOPOLOGY_SPLIT
                            if variant == "topology-split"
                            else EffectType.TOPOLOGY_ISOLATION
                        ),
                        "split_axis": "q",
                        "split_value": 0,
                    },
                )
            )
        state.active_effects.append(e)
        yield variant, state, h, pieces, heavy


def validation_cases():
    from state_codec import export_state

    from goa2.domain.models.enums import DisplacementType

    result = []
    for name, state, h, pieces, heavy in states():
        units = [
            "hero_arien",
            "hero_brogan",
            "hero_hanu",
            "hero_razzle",
            *pieces,
            str(heavy.id),
            "restriction-token",
            "unknown",
        ]
        actors = [
            "hero_arien",
            "hero_brogan",
            "hero_hanu",
            pieces[0],
            "restriction-token",
            None,
            "unknown",
        ]
        queries = []
        expected = []
        for unit in units:
            for actor in actors:
                for kind in DisplacementType:
                    destinations = (
                        [None, h(2, -1), h(0, 0), h(8, 0)]
                        if kind == DisplacementType.PLACE
                        else [None]
                    )
                    for dest in destinations:
                        query = {
                            "kind": kind.value,
                            "unit": unit,
                            "actor": actor,
                            "destination": dest.model_dump() if dest else None,
                        }
                        method = getattr(
                            state.validator,
                            {
                                "MOVE": "can_be_moved",
                                "PUSH": "can_be_pushed",
                                "SWAP": "can_be_swapped",
                                "PLACE": "can_be_placed",
                            }[kind.value],
                        )
                        verdict = method(
                            state,
                            unit,
                            actor,
                            **({"destination": dest} if kind == DisplacementType.PLACE else {}),
                        )
                        queries.append(query)
                        expected.append(verdict.model_dump(mode="json"))
        result.append(
            {
                "name": "displacement-" + name,
                "envelope": export_state(state),
                "queries": queries,
                "expected": expected,
            }
        )
    return result


def cases(advance, answer):
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import PlaceUnitStep, SetActorStep, SetContextFlagStep, SwapUnitsStep

    result = []
    for variant, original, h, pieces, heavy in states():
        for kind in ["place", "swap"]:
            state = copy.deepcopy(original)
            if kind == "place":
                step = PlaceUnitStep(
                    unit_id="hero_brogan",
                    target_hex_arg=h(2, -1),
                    is_mandatory=variant == "mandatory",
                    step_id="fixture/displacement/main",
                )
                if variant == "same-place":
                    step.target_hex_arg = h(1, 0)
                if variant == "keys":
                    step.unit_id = None
                    step.unit_key = "unit"
                    step.target_hex_arg = None
                    state.execution_context = {
                        "unit": "hero_brogan",
                        "target_hex": h(2, -1).model_dump(),
                    }
                if variant == "context-skip":
                    step.target_hex_arg = None
                    state.execution_context = {"target_hex": "SKIP"}
            else:
                step = SwapUnitsStep(
                    unit_a_id="hero_brogan",
                    unit_b_id="hero_hanu",
                    is_mandatory=variant == "mandatory",
                    step_id="fixture/displacement/main",
                )
                if variant == "same-swap":
                    step.unit_b_id = "hero_brogan"
                if variant == "keys":
                    step.unit_a_id = None
                    step.unit_a_key = "a"
                    step.unit_b_id = None
                    step.unit_b_key = "b"
                    state.execution_context = {"a": "hero_brogan", "b": "hero_hanu"}
                if variant == "context-skip":
                    step.unit_a_id = None
                    step.unit_a_key = "a"
                    state.execution_context = {"a": "SKIP"}
            if variant == "gate-off":
                step.active_if_key = "absent"
            if variant.startswith("target-"):
                target = {
                    "target-token": "restriction-token",
                    "target-token-empty-actor": "restriction-token",
                    "target-piece": pieces[0],
                    "target-heavy": str(heavy.id),
                    "target-owner": "hero_razzle",
                    "target-unknown": "unknown",
                }[variant]
                if kind == "place":
                    step.unit_id = target
                else:
                    step.unit_a_id = target
            steps = [
                step,
                SetContextFlagStep(key="after", value=True, step_id="fixture/displacement/after"),
            ]
            if variant in ("live-origin", "live-piece-origin"):
                steps.insert(
                    0, SetActorStep(actor_id="hero_brogan", step_id="fixture/displacement/actor")
                )
                steps.insert(
                    0,
                    PlaceUnitStep(
                        unit_id="hero_arien" if variant == "live-origin" else pieces[0],
                        target_hex_arg=h(-3, 0),
                        step_id="fixture/displacement/origin",
                    ),
                )
            push_steps(state, steps)
            envelope = export_state(state)
            ops = [{"kind": "advance_prefix", "steps": 1}, advance]
            expected = reference_trace(state, ops)
            if variant == "mandatory":
                assert "after" not in state.execution_context
            if variant == "plain":
                assert "after" in state.execution_context
            result.append(
                {
                    "name": "displacement-" + kind + "-" + variant,
                    "envelope": envelope,
                    "operations": ops,
                    "expected": expected,
                }
            )
    return result


def export_validation(path=None):
    m = verify_sources()
    data = {"schema_version": 1, "source_sha256": m["source_sha256"], "cases": validation_cases()}
    data["corpus_sha256"] = digest(data)
    write_text(path or ROOT / "rust/tests/fixtures/displacement-validation.json", json.dumps(data, separators=(",", ":")) + "\n")
    return data


def verify_validation(path, command):
    from oracle import Worker

    manifest = verify_sources()
    data = json.loads(read_text(path))
    identity = data.pop("corpus_sha256")
    if (
        data["schema_version"] != 1
        or digest(data) != identity
        or data["source_sha256"] != manifest["source_sha256"]
    ):
        raise ValueError("displacement corpus identity/source mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request(
                "displacement_legality", envelope=case["envelope"], queries=case["queries"]
            )["results"]
            if len(actual) != len(case["expected"]):
                raise AssertionError("displacement response shape mismatch")
            for index, (got, want) in enumerate(zip(actual, case["expected"], strict=True)):
                if got != want:
                    raise AssertionError((case["name"], case["queries"][index], got, want))
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "queries": sum(len(c["queries"]) for c in data["cases"]),
        "corpus_sha256": identity,
        "full_verdict_parity": True,
    }


if __name__ == "__main__":
    import argparse
    from pathlib import Path

    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/tests/fixtures/displacement-validation.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export_validation(args.corpus)
        print(
            len(data["cases"]), sum(len(c["queries"]) for c in data["cases"]), data["corpus_sha256"]
        )
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker executable required after --")
        print(json.dumps(verify_validation(args.corpus, command), indent=2))
