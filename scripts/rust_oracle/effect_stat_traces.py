"""Frozen native-stat consumers: spatial effects, immunity and topology."""

from __future__ import annotations

import json

from oracle import ROOT, digest, verify_sources


def cases():
    from aura_traces import _board, _pieces
    from model_traces import setup
    from state_codec import export_state_with_instances

    from goa2.domain.board import Zone
    from goa2.domain.models import Card, GamePhase, StatType, TeamColor
    from goa2.domain.models.effect import (
        ActiveEffect,
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )
    from goa2.domain.models.marker import MarkerType
    from goa2.engine.stats import (
        calculate_minion_defense_modifier,
        compute_card_stats,
        get_computed_stat,
    )

    variants = [
        "double",
        "double-twice",
        "double-negative",
        "double-zero",
        "double-foreign",
        "double-piece",
        "basic",
        "basic-nonbasic",
        "basic-all-stats",
        "conditional",
        "conditional-reverse",
        "conditional-items-markers",
        "conditional-positive",
        "minion-immunity",
        "source-self",
        "missing-source",
        "missing-origin",
        "fixed-origin",
        "unbound-pieces",
        "offboard-piece",
        "bound-piece",
        "nested-basic",
        "nested-nonbasic",
        "immunity-token-source",
        "immunity-token-own",
        "immunity",
        "immunity-friendly",
        "immunity-subject",
        "immunity-radius",
        "immunity-radius-self",
        "immunity-dormant",
        "immunity-past",
        "attack-immunity",
        "attack-nonbasic",
        "attack-basic-only",
        "attack-nonbasic-only",
        "attack-except",
        "attack-own",
        "attack-dormant",
        "attack-past",
        "split-q",
        "split-r",
        "split-s",
        "split-invalid",
        "split-dormant",
        "split-past",
        "split-immune",
        "isolation-live",
        "isolation-fallback",
        "isolation-piece",
        "multiple-splits",
        "zone-alias",
        "zone-tile-priority",
        "zone-tile-missing",
    ]
    variants += [
        "scope-" + shape.value + "-" + affects.value for shape in Shape for affects in AffectsFilter
    ]
    variants += [
        "duration-" + duration.value + "-" + variant
        for duration in DurationType
        for variant in [
            "current",
            "future",
            "past-round",
            "past-turn",
            "dormant",
            "cardless",
            "empty-card",
        ]
    ]
    result = []
    for variant in variants:
        state = setup(["Razzle", "Arien"], ["Brogan", "Hanu"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.round = 3
        state.turn = 2
        state.current_actor_id = "hero_arien"
        pieces = _pieces(state, "hero_razzle")
        for who, point in [
            ("hero_arien", h(0, 0)),
            ("hero_brogan", h(1, 0)),
            ("hero_hanu", h(3, -1)),
            (pieces[0], h(-1, 0)),
            (pieces[1], h(0, 2)),
        ]:
            state.place_entity(who, point)
        state.acting_piece_id = None
        arien = state.get_hero("hero_arien")
        for team in state.teams.values():
            for hero in team.heroes:
                hero.items = {stat: 2 for stat in StatType}
                hero.play_card(next(c for c in hero.hand if c.is_basic))
        state.board.zones = {
            "area": Zone(id="area", hexes={h(0, 0), h(1, 0), h(-1, 0)}, neighbors=[])
        }
        if variant != "zone-alias":
            for point in state.board.zones["area"].hexes:
                state.board.tiles[point].zone_id = "area"
        for team in state.teams.values():
            for m in team.minions:
                m.lane_id = "laneA"
        state.battle_zones = {"laneA": "area"}
        red = state.teams[TeamColor.RED].minions
        blue = state.teams[TeamColor.BLUE].minions
        for m, point in [(red[0], h(-2, 1)), (red[-1], h(0, -1)), (blue[0], h(1, -1))]:
            state.place_entity(m.id, point)

        def effect(kind=EffectType.AREA_STAT_MODIFIER, **kwargs):
            fields = dict(
                id="stat-effect",
                source_id="hero_arien",
                effect_type=kind,
                scope=EffectScope(shape=Shape.GLOBAL, affects=AffectsFilter.ALL_UNITS),
                duration=DurationType.PASSIVE,
                created_at_turn=2,
                created_at_round=3,
                is_active=True,
                stat_type=StatType.ATTACK,
                stat_value=3,
            )
            fields.update(kwargs)
            return ActiveEffect(**fields)

        e = effect()
        if variant.startswith("scope-"):
            _, shape, affects = variant.split("-", 2)
            e.scope = EffectScope(
                shape=Shape(shape), range=2, affects=AffectsFilter(affects), direction=3
            )
        if variant.startswith("double"):
            e = effect(EffectType.DOUBLE_ITEMS)
            if variant == "double-twice":
                state.active_effects.append(e.model_copy(deep=True, update={"id": "second"}))
            if variant in ("double-negative", "double-zero"):
                arien.items = {stat: -2 if variant == "double-negative" else 0 for stat in StatType}
            if variant == "double-foreign":
                e.source_id = "hero_hanu"
            if variant == "double-piece":
                e.source_id = pieces[0]
        if variant.startswith("basic") or variant.startswith("nested"):
            e = effect(EffectType.BASIC_ACTION_STAT_BONUS)
            if variant == "basic-nonbasic":
                arien.current_turn_card = next(c for c in arien.hand if not c.is_basic)
            if variant.startswith("nested"):
                other = state.get_hero("hero_brogan")
                card = (
                    other.current_turn_card
                    if variant == "nested-basic"
                    else next(c for c in other.hand if not c.is_basic)
                )
                state.execution_context = {
                    "performing_card_owner_id": str(other.id),
                    "performing_card_id": str(card.id),
                }
        if variant.startswith("conditional"):
            e = effect(stat_value=-2, apply_stat_value_only_if_result_at_least=2)
            state.active_effects.append(
                effect(id="second", stat_value=3, apply_stat_value_only_if_result_at_least=5)
            )
            if variant == "conditional-reverse":
                state.active_effects.reverse()
            if variant == "conditional-positive":
                e.stat_value = 5
                e.apply_stat_value_only_if_result_at_least = 4
            if variant == "conditional-items-markers":
                state.place_marker(MarkerType.VENOM, "hero_arien", -1, "hero_hanu")
        if variant == "minion-immunity":
            # Keep a friendly minion in the heavy's active battle zone.
            state.board.zones["area"].hexes.add(h(-1, 1))
            state.place_entity(red[0].id, h(-1, 1))
        if variant == "source-self":
            e.source_id = "hero_brogan"
        if variant == "missing-source":
            e.source_id = "unknown"
        if variant == "missing-origin":
            e.scope = EffectScope(shape=Shape.RADIUS, origin_id="unknown", range=2)
        if variant == "fixed-origin":
            e.scope = EffectScope(shape=Shape.RADIUS, origin_hex=h(3, -1), range=1)
        if variant == "bound-piece":
            state.acting_piece_id = pieces[0]
        if variant == "offboard-piece":
            state.remove_entity(pieces[0])
        if variant in ("immunity-token-source", "immunity-token-own"):
            from goa2.domain.models import Token, TokenType

            token = Token(
                id="stat-token",
                name="Stat token",
                token_type=TokenType.TOTEM,
                owner_id="hero_brogan" if variant.endswith("own") else "hero_arien",
            )
            state.misc_entities[token.id] = token
            state.place_entity(token.id, h(-1, 1))
            e.source_id = str(token.id)
        if (
            variant.startswith("immunity")
            or variant.startswith("attack")
            or variant == "split-immune"
        ):
            target = "hero_brogan"
            is_attack = variant.startswith("attack")
            shield = effect(
                EffectType.ATTACK_IMMUNITY if is_attack else EffectType.IMMUNITY_ENEMY_ACTIONS,
                id="shield",
                source_id=target,
            )
            if variant == "immunity-friendly":
                e.source_id = "hero_hanu"
                shield.blocks_friendly_actors = True
            if variant == "immunity-subject":
                shield.source_id = "hero_hanu"
                shield.subject_id = target
            if variant in ("immunity-radius", "immunity-radius-self"):
                shield.source_id = "hero_hanu"
                shield.scope = EffectScope(
                    shape=Shape.RADIUS, range=3, affects=AffectsFilter.FRIENDLY_HEROES
                )
                if variant == "immunity-radius-self":
                    shield.scope.affects = AffectsFilter.FRIENDLY_UNITS
            if variant.endswith("dormant"):
                shield.is_active = False
            if variant.endswith("past"):
                shield.duration = DurationType.THIS_TURN
                shield.created_at_turn = 1
            if is_attack:
                e.origin_attack_is_basic = variant != "attack-nonbasic"
                if variant == "attack-basic-only":
                    shield.basic_attacks_only = True
                    e.origin_attack_is_basic = False
                if variant == "attack-nonbasic-only":
                    shield.non_basic_attacks_only = True
                if variant == "attack-except":
                    shield.except_attacker_ids = ["hero_arien"]
                if variant == "attack-own":
                    e.source_id = target
            state.active_effects.append(shield)
        if (
            variant.startswith("split")
            or variant.startswith("isolation")
            or variant == "multiple-splits"
        ):
            split = effect(
                (
                    EffectType.TOPOLOGY_ISOLATION
                    if variant.startswith("isolation")
                    else EffectType.TOPOLOGY_SPLIT
                ),
                id="topology",
                source_id="hero_brogan",
                split_axis="q",
                split_value=0,
            )
            if variant in ("split-r", "split-s"):
                split.split_axis = variant[-1]
            if variant == "split-invalid":
                split.split_axis = "invalid"
            if variant == "split-dormant":
                split.is_active = False
            if variant == "split-past":
                split.duration = DurationType.THIS_TURN
                split.created_at_turn = 0
            if variant == "isolation-fallback":
                state.remove_entity("hero_brogan")
                split.isolated_hex = h(1, 0)
            if variant == "isolation-piece":
                split.source_id = pieces[0]
            if variant == "multiple-splits":
                state.active_effects.append(
                    split.model_copy(deep=True, update={"id": "topology2", "split_axis": "r"})
                )
            state.active_effects.append(split)
            e.scope = EffectScope(shape=Shape.RADIUS, origin_hex=h(-1, 0), range=4)
        if variant == "zone-alias":
            state.board.zones["area"].id = "different-id"
            e.scope = EffectScope(shape=Shape.ZONE)
        if variant == "zone-tile-priority":
            state.board.zones["area"].id = "different-id"
            state.board.tiles[h(0, 0)].zone_id = "tile-area"
            state.board.tiles[h(1, 0)].zone_id = "tile-area"
            e.scope = EffectScope(shape=Shape.ZONE)
        if variant == "zone-tile-missing":
            state.board.tiles[h(0, 0)].zone_id = None
            e.scope = EffectScope(shape=Shape.ZONE)
        if variant.startswith("duration-"):
            _, duration, which = variant.split("-", 2)
            e.duration = DurationType(duration)
            e.source_card_id = str(arien.current_turn_card.id)
            if which == "future":
                e.created_at_turn = 3
            if which == "past-round":
                e.created_at_round = 2
            if which == "past-turn":
                e.created_at_turn = 1
            if which == "dormant":
                e.is_active = False
            if which == "cardless":
                e.source_card_id = None
                e.is_active = False
            if which == "empty-card":
                e.source_card_id = ""
                e.is_active = False
        if variant == "conditional-reverse":
            state.active_effects.insert(0, e)
        else:
            state.active_effects.append(e)
        envelope, objects = export_state_with_instances(state)
        units = [
            "hero_arien",
            "hero_brogan",
            "hero_hanu",
            "hero_razzle",
            *pieces,
            str(red[-1].id),
            str(blue[0].id),
            "unknown",
        ]
        queries = []
        expected = []
        for unit in units:
            for stat in StatType:
                for base in (-2, 0, 4):
                    queries.append({"kind": "stat", "unit": unit, "stat": stat.value, "base": base})
                    expected.append(get_computed_stat(state, unit, stat, base))
            queries.append({"kind": "minion_defense", "unit": unit})
            expected.append(calculate_minion_defense_modifier(state, unit))
        for index, card in enumerate(objects):
            if not isinstance(card, Card):
                continue
            if any(card is c for c in [*arien.hand, arien.current_turn_card]):
                queries.append({"kind": "card", "unit": str(arien.id), "card": index})
                expected.append(vars(compute_card_stats(state, arien.id, card)))
        result.append(
            {
                "name": "effect-stat-" + variant,
                "envelope": envelope,
                "queries": queries,
                "expected": expected,
            }
        )
    return result


def export(path=None):
    manifest = verify_sources()
    data = {"schema_version": 1, "source_sha256": manifest["source_sha256"], "cases": cases()}
    data["corpus_sha256"] = digest(data)
    target = path or ROOT / "rust/tests/fixtures/effect-stats.json"
    target.write_text(json.dumps(data, separators=(",", ":")) + "\n")
    return data


def verify(path, command):
    from oracle import Worker

    manifest = verify_sources()
    data = json.loads(path.read_text())
    identity = data.pop("corpus_sha256")
    if (
        data["schema_version"] != 1
        or digest(data) != identity
        or data["source_sha256"] != manifest["source_sha256"]
    ):
        raise ValueError("effect stat corpus identity/source mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request("stats", envelope=case["envelope"], queries=case["queries"])[
                "results"
            ]
            if len(actual) != len(case["expected"]):
                raise AssertionError("stat response shape mismatch")
            for index, (got, want) in enumerate(zip(actual, case["expected"], strict=True)):
                if got != want:
                    raise AssertionError((case["name"], case["queries"][index], got, want))
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "queries": sum(len(c["queries"]) for c in data["cases"]),
        "corpus_sha256": identity,
        "stat_parity": True,
    }


if __name__ == "__main__":
    import argparse
    from pathlib import Path

    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/tests/fixtures/effect-stats.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(
            len(data["cases"]), sum(len(c["queries"]) for c in data["cases"]), data["corpus_sha256"]
        )
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker executable required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))
