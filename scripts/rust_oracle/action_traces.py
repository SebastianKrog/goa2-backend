"""Compare native action menus/recipes with actual ResolveCardStep calls.

Each operation resolves one isolated instruction. Generated children are recorded
in execution order, not executed. The reference asserts no persistent game fields
change; execution_context, request and complete child records are compared.
"""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

from model_traces import setup
from oracle import ROOT, Worker, digest, verify_sources
from stack_traces import TraceNormalizer
from state_codec import export_state_with_instances


def normalize(record, live=None):
    result = copy.deepcopy(record)
    normalizer = TraceNormalizer()
    if live is not None:
        normalizer.bind_steps(live)
    if result["request"]:
        result["request"]["id"] = normalizer.request_id(result["request"]["id"])
    for i, step in enumerate(result["steps"]):
        normalizer.step(step, live[i] if live is not None else None)
    return result


def corpus():
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import StatType
    from goa2.engine.steps import ResolveCardStep

    setup()  # Register actual data and effects.
    cases = []

    def add(name, state, hero, cards, *, extras=False):
        envelope, objects = export_state_with_instances(state)
        operations = []
        expected = []

        def resolve(card, selection=None, pending=False, who=None):
            target = hero if who is None else state.get_hero(who)
            if target is not None:
                target.current_turn_card = card
            operation = {
                "hero": who or str(hero.id),
                "card": next((i for i, obj in enumerate(objects) if obj is card), None),
            }
            step = ResolveCardStep(hero_id=operation["hero"])
            if pending:
                step.pending_input = {"selection": selection}
                operation["selection"] = selection
            before = state.model_dump(mode="json")
            before.pop("execution_context")
            result = step.resolve(state, state.execution_context)
            after = state.model_dump(mode="json")
            after.pop("execution_context")
            assert before == after, "ResolveCardStep unexpectedly changed persistent game state"
            expected.append(
                normalize(
                    {
                        "context": state.execution_context,
                        "request": (
                            result.input_request.model_dump(mode="json")
                            if result.input_request
                            else None
                        ),
                        "steps": [s.model_dump(mode="json") for s in result.new_steps or []],
                    },
                    result.new_steps or [],
                )
            )
            operations.append(operation)
            return result

        for card in cards:
            result = resolve(card)
            if result.input_request:
                for option in dict.fromkeys(o.id for o in result.input_request.options):
                    resolve(card, option, True)
        if extras:
            for value in (None, True, 7, "SKIP", "not_an_action"):
                resolve(cards[0], value, True)
            resolve(None)
            resolve(None, who="unknown")
        cases.append(
            {"name": name, "envelope": envelope, "operations": operations, "expected": expected}
        )

    for name in sorted(HeroRegistry._heroes):
        other = [n for n in ["Arien", "Hanu", "Brogan", "Tigerclaw"] if n != name]
        state = setup([name, other[0]], other[1:3])
        hero = next(h for h in state.teams[next(iter(state.teams))].heroes if h.name == name)
        add(name, state, hero, list(hero.deck), extras=name == "Arien")
    for variant in ("items", "facedown", "offboard", "blocked_travel", "unbound_team"):
        state = setup()
        hero = state.get_hero("hero_arien")
        if variant == "items":
            hero.items = {stat: i - 2 for i, stat in enumerate(StatType)}
        elif variant == "facedown":
            for card in hero.deck:
                card.is_facedown = True
        elif variant == "offboard":
            state.remove_entity(hero.id)
        elif variant == "blocked_travel":
            zone = state.board.zones[state.board.get_zone_for_hex(state.get_position(hero.id))]
            free = next(h for h in zone.hexes if h not in state.entity_locations.values())
            state.move_unit("hero_brogan", free)
        else:
            hero.team = None
        add(variant, state, hero, list(hero.deck), extras=True)
    data = {
        "schema_version": 1,
        "source_sha256": verify_sources()["source_sha256"],
        "cases": cases,
        "travel": travel_cases(),
    }
    return {**data, "corpus_sha256": digest(data)}


def travel_cases():
    from goa2.domain.board import Board, Zone
    from goa2.domain.hex import Hex
    from goa2.domain.models import Hero, Team, TeamColor, Token, TokenType
    from goa2.domain.state import GameState
    from goa2.engine.rules import get_safe_zones_for_fast_travel
    from goa2.engine.setup import GameSetup

    states = []
    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        states.append((path.stem, state))
    for variant in ("token", "enemy", "unbound", "unknown", "empty", "zone_identity", "tile_zone"):
        coords = [Hex(q=q, r=-q, s=0) for q in range(5)]
        zones = {
            "start": Zone(
                id="start", hexes=set(coords[:2]), neighbors=["middle", "missing", "middle"]
            ),
            "middle": Zone(id="middle", hexes=set(coords[2:4]), neighbors=["start", "end"]),
            "end": Zone(id="end", hexes={coords[4]}, neighbors=["middle"]),
        }
        red = Hero(id="hero_red", name="Red", deck=[], team=TeamColor.RED)
        blue = Hero(
            id="hero_blue",
            name="Blue",
            deck=[],
            team=None if variant == "unbound" else TeamColor.BLUE,
        )
        state = GameState(
            board=Board(zones=zones),
            teams={
                TeamColor.RED: Team(color=TeamColor.RED, heroes=[red]),
                TeamColor.BLUE: Team(color=TeamColor.BLUE, heroes=[blue]),
            },
            rules_version=9,
        )
        state.place_entity(red.id, coords[0])
        if variant in ("enemy", "unbound"):
            state.place_entity(blue.id, coords[1])
        elif variant == "token":
            token = Token(id="token", name="Token", token_type=TokenType.TOTEM)
            state.misc_entities[token.id] = token
            state.place_entity(token.id, coords[1])
        elif variant == "unknown":
            state.place_entity("unknown", coords[1])
        if variant == "zone_identity":
            state.board.zones["middle"].hexes.add(coords[0])
            state.board.zones["start"].id = "actual_start"
        elif variant == "tile_zone":
            from goa2.domain.tile import Tile

            state.board.tiles = {
                hex: Tile(
                    hex=hex,
                    zone_id=None if i == 0 else "end" if i == 1 else "" if i == 2 else "middle",
                )
                for i, hex in enumerate(coords)
            }
        states.append((f"generated-{variant}", state))
    cases = []
    for name, state in states:
        queries = []
        expected = []
        for team in TeamColor:
            for zone in [*state.board.zones, "missing"]:
                queries.append({"team": team.value, "zone": zone})
                expected.append(get_safe_zones_for_fast_travel(state, team, zone))
        lookups = (
            list(state.board.tiles)[:5]
            if state.board.tiles
            else list(next(iter(state.board.zones.values())).hexes)
        )
        lookups.append(Hex(q=1000, r=-1000, s=0))
        for hex in lookups:
            queries.append({"kind": "zone_at", "hex": hex.model_dump(mode="json")})
            expected.append(state.board.get_zone_for_hex(hex))
        envelope, _ = export_state_with_instances(state)
        cases.append({"name": name, "envelope": envelope, "queries": queries, "expected": expected})
    return cases


def verify(path, command):
    data = json.loads(path.read_text())
    identity = data.pop("corpus_sha256")
    if digest(data) != identity or data["source_sha256"] != verify_sources()["source_sha256"]:
        raise ValueError("action corpus source/identity mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request(
                "action_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            actual = [normalize(record) for record in actual]
            if actual != case["expected"]:
                failure = ROOT / "rust/artifacts/action-mismatch.json"
                failure.parent.mkdir(parents=True, exist_ok=True)
                failure.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError(f"{case['name']}: saved {failure}")
        for case in data["travel"]:
            actual = worker.request(
                "travel_zones", envelope=case["envelope"], queries=case["queries"]
            )["results"]
            if actual != case["expected"]:
                raise AssertionError(f"travel zones differ: {case['name']}")
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "operations": sum(len(c["operations"]) for c in data["cases"]),
        "travel_cases": len(data["travel"]),
        "travel_queries": sum(len(c["queries"]) for c in data["travel"]),
        "corpus_sha256": identity,
    }


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/tests/fixtures/actions.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = corpus()
        args.corpus.write_text(json.dumps(data, separators=(",", ":")) + "\n")
        print(
            json.dumps(
                {
                    "cases": len(data["cases"]),
                    "operations": sum(len(c["operations"]) for c in data["cases"]),
                    "travel_cases": len(data["travel"]),
                    "travel_queries": sum(len(c["queries"]) for c in data["travel"]),
                }
            )
        )
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker command required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))


if __name__ == "__main__":
    main()
