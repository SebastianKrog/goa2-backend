"""Frozen effect-aware battle counts, lane pushes and spawn displacement.

Raw steps isolate turn-boundary primitives. Cards and roster definitions remain
registered objects; the real frozen handler supplies every expected record.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from combat_effect_traces import effect
from mine_traces import drive
from oracle import ROOT, digest, verify_sources


def exclusion_states():
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.board import Zone
    from goa2.domain.models import GamePhase, TeamColor, Token, TokenType
    from goa2.domain.models.effect import (
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )

    variants = [
        "ordinary",
        "inactive",
        "expired",
        "offboard",
        "missing-source",
        "empty-source",
        "token-source",
        "blue-source",
        "no-team",
        "same-origin",
        "opposite-caps",
        "overlap",
        "overlap-reversed",
        "both-teams",
        "subject-ignored",
        "split",
        "split-immunity-ignored",
        "isolation",
        "immunity-ignored",
        "outside-zone",
        "token-not-minion",
        "army",
    ]
    variants += [f"cap-{cap}" for cap in [None, -3, 0, 1, 2, 3, 9]]
    variants += [
        "scope-" + shape.value + "-" + affects.value for shape in Shape for affects in AffectsFilter
    ]
    variants += ["duration-" + duration.value for duration in DurationType]
    for variant in variants:
        state = setup(["Wuk", "Arien"], ["Brogan", "Hanu"])
        h = _board(state)
        state.phase = GamePhase.CLEANUP
        state.current_actor_id = "hero_wuk"
        state.round, state.turn = 3, 4
        state.board.zones = {"battle": Zone(id="battle", hexes=set(state.board.tiles))}
        state.battle_zones = {"lane_1": "battle"}
        for uid, point in [
            ("hero_wuk", h(0, 0)),
            ("hero_arien", h(0, 1)),
            ("hero_brogan", h(2, 0)),
            ("hero_hanu", h(-3, 0)),
        ]:
            state.place_entity(uid, point)
        for team, points in [
            (TeamColor.RED, [h(-2, 0), h(-2, 1), h(-1, 2)]),
            (TeamColor.BLUE, [h(1, 0), h(1, -1), h(0, -1), h(-1, 0)]),
        ]:
            for minion, point in zip(state.teams[team].minions, points, strict=False):
                state.place_entity(minion.id, point)
        e = effect(
            state, "exclusion", EffectType.MINION_BATTLE_EXCLUSION, source="hero_wuk", max_value=2
        )
        e.created_at_round, e.created_at_turn = 3, 4
        state.active_effects = [e]
        if variant.startswith("cap-"):
            raw = variant[4:]
            e.max_value = None if raw == "None" else int(raw)
        if variant == "inactive":
            e.is_active = False
        if variant == "expired":
            e.duration, e.created_at_round, e.created_at_turn = DurationType.THIS_TURN, 1, 1
        if variant.startswith("duration-"):
            e.duration = DurationType(variant[9:])
        if variant == "offboard":
            state.remove_entity("hero_wuk")
        if variant == "missing-source":
            e.source_id = "missing"
        if variant == "empty-source":
            e.source_id = ""
        if variant == "blue-source":
            e.source_id = "hero_brogan"
        if variant == "no-team":
            state.get_hero("hero_wuk").team = None
        if variant in ["token-source", "token-not-minion"]:
            token = Token(
                id="exclusion-token",
                name="Exclusion token",
                token_type=TokenType.TOTEM,
                owner_id="hero_wuk",
            )
            state.misc_entities[token.id] = token
            if variant == "token-source":
                state.remove_entity("hero_wuk")
                state.place_entity(token.id, h(0, 0))
                e.source_id = token.id
            else:
                state.remove_entity(state.teams[TeamColor.BLUE].minions[0].id)
                token.owner_id = "hero_hanu"
                state.place_entity(token.id, h(1, 0))
        if variant in ["same-origin", "opposite-caps"]:
            state.active_effects.append(
                e.model_copy(
                    deep=True,
                    update={"id": "second", "max_value": -1 if variant == "opposite-caps" else 2},
                )
            )
        if variant in ["overlap", "overlap-reversed", "both-teams"]:
            state.active_effects.append(
                e.model_copy(
                    deep=True,
                    update={
                        "id": "second",
                        "source_id": "hero_brogan" if variant == "both-teams" else "hero_arien",
                    },
                )
            )
            if variant == "overlap-reversed":
                state.active_effects.reverse()
        if variant == "subject-ignored":
            e.subject_id = "hero_hanu"
        if variant.startswith("scope-"):
            shape, affects = variant[6:].split("-", 1)
            e.scope = EffectScope(
                shape=Shape(shape), affects=AffectsFilter(affects), range=-1, origin_hex=h(4, 0)
            )
        if variant.startswith("split") or variant == "isolation":
            state.active_effects.append(
                effect(
                    state,
                    "topology",
                    (
                        EffectType.TOPOLOGY_ISOLATION
                        if variant == "isolation"
                        else EffectType.TOPOLOGY_SPLIT
                    ),
                    source="hero_hanu",
                    split_axis="q",
                    split_value=0,
                    isolated_hex=h(0, 0),
                )
            )
        if variant in ["immunity-ignored", "split-immunity-ignored"]:
            state.active_effects.append(
                effect(state, "immunity", EffectType.IMMUNITY_ENEMY_ACTIONS, source="hero_hanu")
            )
        if variant == "outside-zone":
            state.board.zones["battle"].hexes.discard(h(1, 0))
        if variant == "army":
            state.get_hero("hero_brogan").level = 8
        yield variant, state


def cases():
    from battle_traces import cases as regression_cases

    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import MinionBattleStep, ResolveDisplacementStep, SetContextFlagStep

    def add_effects(state, name):
        state.active_effects = [
            effect(
                state, "battle-inert", EffectType.MOVEMENT_ZONE, source="hero_arien", max_value=99
            )
        ]
        if name == "spawn-blocked-token":
            from goa2.domain.models.effect import EffectScope, Shape

            for tag, source, token, origin in [
                ("bound", "hero_arien", "battle_token", None),
                ("source", "battle_token", None, None),
                ("origin", "hero_arien", None, "battle_token"),
            ]:
                e = effect(
                    state,
                    "linked-" + tag,
                    EffectType.MOVEMENT_ZONE,
                    source=source,
                    token_id=token,
                    max_value=99,
                )
                e.scope = EffectScope(shape=Shape.GLOBAL, origin_id=origin)
                state.active_effects.append(e)

    advance = {"kind": "advance"}

    def answer(value, request_id="current"):
        return {"kind": "input", "request_id": request_id, "selection": value}

    result = regression_cases(advance, answer, effect_factory=add_effects)
    for case in result:
        case["name"] = "battle-effect-regression-" + case["name"]
    from displacement_effect_traces import states as displacement_states

    for variant, state, h, _, minion in displacement_states():
        # Exercise connected BFS and the subsequently generated mandatory Place
        # step separately. Source placement prevention may abort the tail; do not
        # prefilter it into an apparently successful automatic displacement.
        case, _ = drive(
            "battle-effect-displacement-" + variant,
            state,
            [
                ResolveDisplacementStep(displacements=[(str(minion.id), h(-1, 0))]),
                SetContextFlagStep(key="battle_done", value=True),
            ],
        )
        result.append(case)

    for variant, state in exclusion_states():
        case, final = drive(
            "battle-effect-exclusion-" + variant,
            state,
            [MinionBattleStep(), SetContextFlagStep(key="battle_done", value=True)],
        )
        assert final.execution_context["battle_done"]
        result.append(case)
    return result


def export(path):
    from stack_traces import write_corpus

    m = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": m["oracle_revision"],
        "source_sha256": m["source_sha256"],
        "cases": cases(),
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/artifacts/battle-effects.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        from stack_traces import verify

        print(
            json.dumps(
                verify(args.corpus, args.worker[1:] if args.worker[:1] == ["--"] else args.worker),
                indent=2,
            )
        )
