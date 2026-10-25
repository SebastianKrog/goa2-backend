"""Frozen complete targeting verdicts and ordered single/multi/query filter scans."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from combat_effect_traces import effect
from oracle import ROOT, Worker, digest, verify_sources
from reaction_effect_traces import illusion_state
from state_codec import export_state


def states():
    from goa2.domain.board import Zone
    from goa2.domain.hex import Hex
    from goa2.domain.models import MinionType, TeamColor
    from goa2.domain.models.effect import (
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )
    from goa2.domain.models.enums import DisplacementType

    variants = [
        "plain",
        "inactive",
        "other-actor",
        "no-actor",
        "empty-actor",
        "unknown-actor",
        "unknown-source",
        "token-source",
        "blue-source",
        "actor-piece",
        "source-piece",
        "both-pieces",
        "old-active",
        "off-scope",
        "wrong-subject",
        "source-offboard",
        "duplicate",
        "non-illusion",
        "offboard-token",
        "split",
        "isolation",
    ]
    variants += [
        "los-" + v
        for v in [
            "plain",
            "point",
            "actor-end",
            "target-end",
            "off-segment",
            "inactive-card",
            "passive-dormant",
            "cardless-dormant",
            "expired",
            "missing-origin",
            "empty-origin",
            "source-origin",
            "target-immune",
            "actor-immune",
            "attack-origin",
            "attack-except",
            "token-source",
            "friendly",
            "self",
            "allow-enemy",
            "multiple",
        ]
    ]
    variants += [
        "action-" + v for v in ["enemy", "friendly", "subject", "radius", "old", "inactive", "self"]
    ]
    variants += [
        "attack-" + v
        for v in [
            "basic",
            "nonbasic",
            "only-nonbasic",
            "skill",
            "except",
            "facedown",
            "context-flag",
        ]
    ]
    variants += [
        "move-" + v for v in ["move", "place", "both", "inactive", "immune", "radius", "no-actor"]
    ]
    variants += ["heavy", "heavy-unsupported", "topology-owner"]

    def h(q, r):
        return Hex(q=q, r=r, s=-q - r)

    for variant in variants:
        state, _ = illusion_state(
            variant
            if not variant.startswith(("los-", "action-", "attack-", "move-"))
            and variant not in ["heavy", "heavy-unsupported", "topology-owner"]
            else "plain"
        )
        state.execution_context = {"current_action_type": "SKILL", "near": "hero_mrak"}
        if variant.startswith("los-"):
            smoke = effect(state, "smoke", EffectType.LOS_BLOCKER, source="hero_hanu")
            smoke.scope.origin_hex = h(0, 2)
            state.active_effects.append(smoke)
            v = variant[4:]
            if v == "point":
                smoke.scope.shape = Shape.POINT
            if v == "actor-end":
                smoke.scope.origin_hex = h(0, 3)
            if v == "target-end":
                smoke.scope.origin_hex = h(0, 0)
            if v == "off-segment":
                smoke.scope.origin_hex = h(1, 2)
            if v in ["inactive-card", "cardless-dormant", "passive-dormant"]:
                smoke.is_active = False
                if v != "passive-dormant":
                    smoke.duration = DurationType.THIS_ROUND
                if v == "inactive-card":
                    smoke.source_card_id = state.get_hero("hero_hanu").hand[0].id
            if v == "expired":
                smoke.duration = DurationType.THIS_ROUND
                smoke.created_at_round = state.round - 1
            if v in ["missing-origin", "empty-origin", "source-origin"]:
                smoke.scope.origin_hex = None
                smoke.scope.origin_id = "missing" if v == "missing-origin" else ""
                if v == "source-origin":
                    state.remove_entity("illusion-2")
                    state.place_entity("hero_hanu", h(0, 2))
            if v in ["actor-immune", "target-immune"]:
                protected = "hero_arien" if v == "actor-immune" else "hero_mrak"
                state.active_effects.append(
                    effect(state, "immune", EffectType.IMMUNITY_ENEMY_ACTIONS, source=protected)
                )
            if v in ["attack-origin", "attack-except"]:
                smoke.origin_attack_is_basic = True
                state.active_effects.append(
                    effect(
                        state,
                        "immune",
                        EffectType.ATTACK_IMMUNITY,
                        source="hero_arien",
                        basic_attacks_only=True,
                        except_attacker_ids=["hero_hanu"] if v == "attack-except" else [],
                    )
                )
            if v == "token-source":
                smoke.source_id = "illusion-0"
            if v == "friendly":
                smoke.source_id = "hero_razzle"
                smoke.blocks_enemy_actors = False
                smoke.blocks_friendly_actors = True
            if v == "self":
                smoke.source_id = "hero_arien"
                smoke.blocks_self = True
            if v == "allow-enemy":
                smoke.blocks_enemy_actors = False
            if v == "multiple":
                state.active_effects.append(smoke.model_copy(update={"id": "smoke-second"}))
        if variant.startswith("action-"):
            shield = effect(
                state, "action-immunity", EffectType.IMMUNITY_ENEMY_ACTIONS, source="hero_mrak"
            )
            state.active_effects.append(shield)
            v = variant[7:]
            if v == "friendly":
                state.current_actor_id = "hero_hanu"
                shield.blocks_friendly_actors = True
            if v == "subject":
                shield.source_id = "hero_hanu"
                shield.subject_id = "hero_mrak"
            if v == "radius":
                shield.source_id = "hero_hanu"
                shield.scope = EffectScope(
                    shape=Shape.RADIUS, range=4, affects=AffectsFilter.FRIENDLY_HEROES
                )
            if v == "old":
                shield.duration = DurationType.THIS_ROUND
                shield.created_at_round = state.round - 1
            if v == "inactive":
                shield.is_active = False
            if v == "self":
                state.current_actor_id = "hero_mrak"
        if variant.startswith("attack-"):
            state.execution_context["current_action_type"] = (
                "SKILL" if variant == "attack-skill" else "ATTACK"
            )
            hero = state.get_hero("hero_arien")
            basic = variant not in ["attack-nonbasic", "attack-only-nonbasic"]
            card = next(c for c in hero.hand if c.is_basic == basic)
            hero.play_card(card)
            card.is_facedown = variant == "attack-facedown"
            state.execution_context["attack_is_basic"] = False
            state.active_effects.append(
                effect(
                    state,
                    "attack-immunity",
                    EffectType.ATTACK_IMMUNITY,
                    source="hero_mrak",
                    basic_attacks_only=variant != "attack-only-nonbasic",
                    non_basic_attacks_only=variant == "attack-only-nonbasic",
                    except_attacker_ids=["hero_arien"] if variant == "attack-except" else [],
                )
            )
        if variant.startswith("move-"):
            v = variant[5:]
            prevention = effect(
                state, "displacement", EffectType.PLACEMENT_PREVENTION, source="hero_hanu"
            )
            prevention.displacement_blocks = (
                [DisplacementType.PLACE]
                if v == "place"
                else (
                    [DisplacementType.MOVE, DisplacementType.PLACE]
                    if v == "both"
                    else [DisplacementType.MOVE]
                )
            )
            state.active_effects.append(prevention)
            if v == "inactive":
                prevention.duration = DurationType.THIS_ROUND
                prevention.source_card_id = state.get_hero("hero_hanu").hand[0].id
                prevention.is_active = False
            if v == "immune":
                state.active_effects.append(
                    effect(
                        state,
                        "immune",
                        EffectType.IMMUNITY_ENEMY_ACTIONS,
                        source="hero_mrak",
                        blocks_friendly_actors=True,
                    )
                )
            if v == "radius":
                prevention.scope = EffectScope(shape=Shape.RADIUS, range=1, origin_id="hero_mrak")
            if v == "no-actor":
                state.current_actor_id = None
        if variant in ["heavy", "heavy-unsupported"]:
            heavy = next(
                m for m in state.teams[TeamColor.BLUE].minions if m.type == MinionType.HEAVY
            )
            heavy.lane_id = "lane"
            state.place_entity(heavy.id, h(2, -2))
            state.board.zones["battle"] = Zone(id="battle", hexes=set(state.board.tiles))
            state.battle_zones = {"lane": "battle"}
            for m in state.teams[TeamColor.BLUE].minions:
                m.lane_id = "lane"
            if variant == "heavy-unsupported":
                for m in state.teams[TeamColor.BLUE].minions:
                    if m.id != heavy.id:
                        state.remove_entity(m.id)
        if variant == "topology-owner":
            from aura_traces import _pieces

            state.current_actor_id = "hero_razzle"
            state.acting_piece_id = _pieces(state, "hero_razzle")[0]
            split = effect(
                state,
                "split",
                EffectType.TOPOLOGY_SPLIT,
                source="hero_hanu",
                split_axis="r",
                split_value=1,
            )
            immune = effect(
                state,
                "immune",
                EffectType.IMMUNITY_ENEMY_ACTIONS,
                source="hero_razzle",
                blocks_friendly_actors=True,
            )
            state.active_effects += [split, immune]
        yield variant, state


def queries_for(state):
    from goa2.domain.models import MinionType, TargetType
    from goa2.engine.filters_hex import RangeFilter
    from goa2.engine.filters_units import (
        AdjacencyToContextFilter,
        CanBeMovedByActorFilter,
        CanBePlacedByActorFilter,
        ImmunityFilter,
        MinionTypesFilter,
        TeamFilter,
        TokenTypeFilter,
        UnitTypeFilter,
    )

    ids = [*map(str, state.entity_locations), "hero_razzle", "missing", "", None]
    queries = [{"mode": "target", "actor": actor, "unit": unit} for actor in ids for unit in ids]
    filters = [
        [],
        [UnitTypeFilter(unit_type="MINION")],
        [UnitTypeFilter(unit_type="HERO")],
        [UnitTypeFilter(unit_type="TOKEN")],
        [TeamFilter(relation="FRIENDLY")],
        [TeamFilter(relation="ENEMY")],
        [TeamFilter(relation="SELF")],
        [MinionTypesFilter(minion_types=[MinionType.MELEE])],
        [MinionTypesFilter(minion_types=[MinionType.RANGED])],
        [CanBeMovedByActorFilter()],
        [CanBePlacedByActorFilter()],
        [ImmunityFilter()],
        [RangeFilter(max_range=3)],
        [RangeFilter(max_range=3, distance_mode="geometric")],
        [AdjacencyToContextFilter(target_key="near")],
        [TokenTypeFilter()],
    ]
    for mode in ["select", "multi", "count", "collect"]:
        for target_type in [TargetType.UNIT, TargetType.UNIT_OR_TOKEN]:
            for skip in [False, True]:
                for fs in filters:
                    queries.append(
                        {
                            "mode": mode,
                            "target_type": target_type.value,
                            "filters": [f.model_dump(mode="json") for f in fs],
                            "skip_self_filter": False,
                            "skip_immunity_filter": skip,
                        }
                    )
    return queries


def evaluate(state, query):
    from goa2.engine.steps import CollectUnitsStep, CountStep, MultiSelectStep, SelectStep

    if query["mode"] == "target":
        return state.validator.can_be_targeted(state, query["actor"], query["unit"]).model_dump(
            mode="json"
        )
    # The serialization union is registered by bootstrap; actual step classes
    # parse the real filters, then execute their public primitive implementation.
    cls = {
        "select": SelectStep,
        "multi": MultiSelectStep,
        "count": CountStep,
        "collect": CollectUnitsStep,
    }[query["mode"]]
    spec = {k: v for k, v in query.items() if k != "mode"}
    spec["output_key"] = "result"
    if query["mode"] in ["select", "multi"]:
        spec["prompt"] = "Targeting diagnostic"
    if query["mode"] == "select":
        spec.update(is_mandatory=False, auto_select_if_one=False)
    if query["mode"] == "multi":
        spec.update(max_selections=3, min_selections=0)
    step = cls.model_validate(spec)
    context = dict(state.execution_context)
    if query["mode"] == "multi":
        return step._get_candidates(state, context)
    out = step.resolve(state, context)
    if query["mode"] == "select":
        return [o.id for o in out.input_request.options] if out.input_request else []
    return context["result"]


def corpus():
    manifest = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "states": [],
    }
    for name, state in states():
        queries = queries_for(state)
        data["states"].append(
            {
                "name": name,
                "envelope": export_state(state),
                "queries": queries,
                "expected": [evaluate(state, q) for q in queries],
            }
        )
    return {**data, "corpus_sha256": digest(data)}


def verify(path, command):
    data = json.loads(path.read_text())
    identity = data.pop("corpus_sha256")
    assert digest(data) == identity
    assert data["source_sha256"] == verify_sources()["source_sha256"]
    worker = Worker(command)
    count = 0
    try:
        for case in data["states"]:
            actual = worker.request(
                "targeting", envelope=case["envelope"], queries=case["queries"]
            )["results"]
            if actual != case["expected"]:
                fail = ROOT / "rust/artifacts/targeting-mismatch.json"
                fail.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError((case["name"], fail))
            count += len(actual)
    finally:
        worker.close()
    return {
        "states": len(data["states"]),
        "queries": count,
        "corpus_sha256": identity,
        "parity": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/tests/fixtures/targeting.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = corpus()
        args.corpus.write_text(json.dumps(data, separators=(",", ":")) + "\n")
        print(
            json.dumps(
                {
                    "states": len(data["states"]),
                    "queries": sum(len(c["queries"]) for c in data["states"]),
                    "corpus_sha256": data["corpus_sha256"],
                }
            )
        )
    else:
        print(
            json.dumps(
                verify(args.corpus, args.worker[1:] if args.worker[:1] == ["--"] else args.worker),
                indent=2,
            )
        )
