"""Frozen Python statistics and Hero model lifecycle fixtures (no neural runtime)."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from oracle import ROOT, Worker, digest, verify_sources
from state_codec import export_state_with_instances

sys.path.insert(0, str(ROOT / "src"))


def setup(red=None, blue=None):
    from goa2.bootstrap import register_all_effects
    from goa2.engine.setup import GameSetup

    register_all_effects()
    return GameSetup.create_game(
        str(ROOT / "src/goa2/data/maps/forgotten_island.json"),
        red or ["Arien", "Hanu"],
        blue or ["Brogan", "Tigerclaw"],
        seed=29,
    )


def stats_cases():
    from goa2.domain.models import Card, StatType
    from goa2.domain.models.marker import MarkerType
    from goa2.engine.stats import (
        calculate_minion_defense_modifier,
        compute_card_stats,
        get_computed_stat,
    )

    states = []
    state = setup()
    for team in state.teams.values():
        for hero in team.heroes:
            hero.items = {stat: index + 1 for index, stat in enumerate(StatType)}
    state.place_marker(MarkerType.VENOM, "hero_arien", -2, "hero_hanu")
    state.place_marker(MarkerType.POISON, "hero_arien", -1, "hero_tigerclaw")
    states.append(("items-markers-and-minions", state))
    for offboard in (False, True):
        state = setup(["Razzle", "Min"], ["Gydion", "Emmitt"])
        hero = state.get_hero("hero_razzle")
        hero.items = {stat: index - 2 for index, stat in enumerate(StatType)}
        pieces = state.get_piece_ids(hero.id)
        state.acting_piece_id = pieces[0]
        state.place_marker(MarkerType.VENOM, pieces[0], -2, "hero_min")
        state.place_marker(MarkerType.POISON, pieces[-1], -1, "hero_emmitt")
        if offboard:
            state.remove_entity(pieces[0])
        states.append((f"razzle-shared-markers-offboard-{offboard}", state))
    result = []
    for name, state in states:
        envelope, objects = export_state_with_instances(state)
        units = [str(hero.id) for team in state.teams.values() for hero in team.heroes]
        units.extend(str(uid) for uid in state.misc_entities)
        units.extend(str(m.id) for team in state.teams.values() for m in team.minions)
        units.append("unknown_unit")
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
            owner = next(
                (
                    hero
                    for team in state.teams.values()
                    for hero in team.heroes
                    if any(
                        candidate is card
                        for candidate in [*hero.deck, *hero.spells, hero.ultimate_card]
                    )
                ),
                None,
            )
            if owner is None:
                continue
            queries.append({"kind": "card", "unit": str(owner.id), "card": index})
            expected.append(vars(compute_card_stats(state, owner.id, card)))
        result.append(
            {"name": name, "envelope": envelope, "queries": queries, "expected": expected}
        )
    return result


def lifecycle_cases():
    result = []

    def add(name, build, actions):
        state = build()
        envelope, objects = export_state_with_instances(state)
        hero = state.get_hero(
            "hero_gydion" if name == "spell-lifecycle-rejection" else "hero_arien"
        )
        a, b, c = hero.hand[:3]
        values = {"a": a, "b": b, "c": c}
        if hero.spells:
            values["spell"] = hero.spells[0]
        indices = {
            key: next(i for i, value in enumerate(objects) if value is card)
            for key, card in values.items()
        }
        operations = []
        expected = []
        for action in actions:
            operation = {**action, "hero": str(hero.id)}
            for key in ("card", "a", "b"):
                if key in operation:
                    operation[key] = indices[operation[key]]
            operations.append(operation)
            error = None
            try:
                kind = action["kind"]
                if kind == "play":
                    hero.play_card(values[action["card"]])
                elif kind == "unplay":
                    hero.unplay_card(values[action["card"]])
                elif kind == "resolve":
                    hero.resolve_current_card(turn_number=action.get("turn"))
                elif kind == "discard":
                    hero.discard_card(
                        values[action["card"]], from_hand=action.get("from_hand", True)
                    )
                elif kind == "return_hand":
                    hero.return_card_to_hand(values[action["card"]])
                elif kind == "return_deck":
                    hero.return_card_to_deck(values[action["card"]])
                elif kind == "retrieve":
                    hero.retrieve_cards()
                elif kind == "swap":
                    hero.swap_cards(values[action["a"]], values[action["b"]])
                else:
                    raise AssertionError(kind)
            except ValueError:
                error = "invalid_card_action"
            expected.append({"semantic_state": state.model_dump(mode="json"), "error": error})
        result.append(
            {"name": name, "envelope": envelope, "operations": operations, "expected": expected}
        )

    add(
        "play-discard-swap-and-retrieve",
        setup,
        [
            {"kind": "play", "card": "a"},
            {"kind": "play", "card": "a"},
            {"kind": "unplay", "card": "a"},
            {"kind": "play", "card": "a"},
            {"kind": "resolve", "turn": 2},
            {"kind": "discard", "card": "a", "from_hand": False},
            {"kind": "return_hand", "card": "a"},
            {"kind": "discard", "card": "b"},
            {"kind": "swap", "a": "a", "b": "b"},
            {"kind": "resolve", "turn": 1},
            {"kind": "retrieve"},
        ],
    )
    add(
        "turn-slots-and-current-retrieval",
        setup,
        [
            {"kind": "play", "card": "a"},
            {"kind": "resolve", "turn": 0},
            {"kind": "resolve"},
            {"kind": "play", "card": "b"},
            {"kind": "resolve", "turn": 4},
            {"kind": "discard", "card": "a", "from_hand": False},
            {"kind": "play", "card": "c"},
            {"kind": "retrieve"},
            {"kind": "resolve", "turn": 4},
        ],
    )

    def overrides():
        state = setup()
        hero = state.get_hero("hero_arien")
        hero.hand[0].initiative_override = 19
        hero.hand[1].initiative_override = 7
        state.pending_inputs[hero.id] = hero.hand[0]
        return state

    add(
        "sparse-override-and-buffer-aliases",
        overrides,
        [
            {"kind": "play", "card": "a"},
            {"kind": "swap", "a": "a", "b": "b"},
            {"kind": "resolve", "turn": 3},
            {"kind": "return_hand", "card": "b"},
            {"kind": "return_hand", "card": "b"},
            {"kind": "return_deck", "card": "a"},
        ],
    )
    add(
        "spell-lifecycle-rejection",
        lambda: setup(["Gydion", "Hanu"], ["Arien", "Tigerclaw"]),
        [
            {"kind": kind, "card": "spell"}
            for kind in ("play", "unplay", "discard", "return_hand", "return_deck")
        ]
        + [{"kind": "swap", "a": "spell", "b": "a"}, {"kind": "retrieve"}],
    )
    add(
        "same-instance-alias-and-partial-error",
        setup,
        [
            {"kind": "unplay", "card": "a"},
            {"kind": "play", "card": "a"},
            {"kind": "discard", "card": "a"},
            {"kind": "retrieve"},
        ],
    )
    return result


def corpus():
    from aura_traces import aura_stats_cases

    manifest = verify_sources()
    payload = {
        "schema_version": 1,
        "source_sha256": manifest["source_sha256"],
        "stats": stats_cases() + aura_stats_cases(),
        "cards": lifecycle_cases(),
    }
    return {**payload, "corpus_sha256": digest(payload)}


def verify(path, command):
    manifest = verify_sources()
    data = json.loads(path.read_text())
    identity = data.pop("corpus_sha256")
    if digest(data) != identity or data["source_sha256"] != manifest["source_sha256"]:
        raise ValueError("model corpus identity mismatch")
    worker = Worker(command)
    try:
        for case in data["stats"]:
            actual = worker.request("stats", envelope=case["envelope"], queries=case["queries"])[
                "results"
            ]
            if actual != case["expected"]:
                for i, (got, want) in enumerate(zip(actual, case["expected"], strict=True)):
                    if got != want:
                        raise AssertionError(
                            f"{case['name']} query {case['queries'][i]}: {got} != {want}"
                        )
                raise AssertionError("stat response shape mismatch")
        for case in data["cards"]:
            actual = worker.request(
                "card_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            if actual != case["expected"]:
                failure = ROOT / "rust/artifacts/card-model-mismatch.json"
                failure.parent.mkdir(parents=True, exist_ok=True)
                failure.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError(f"{case['name']}: saved {failure}")
    finally:
        worker.close()
    return {
        "stat_cases": len(data["stats"]),
        "stat_queries": sum(len(case["queries"]) for case in data["stats"]),
        "card_cases": len(data["cards"]),
        "card_operations": sum(len(case["operations"]) for case in data["cards"]),
        "corpus_sha256": identity,
    }


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/tests/fixtures/models.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = corpus()
        args.corpus.write_text(json.dumps(data, separators=(",", ":")) + "\n")
        print(
            json.dumps(
                {
                    "stats": len(data["stats"]),
                    "cards": len(data["cards"]),
                    "corpus_sha256": data["corpus_sha256"],
                }
            )
        )
    else:
        command = args.worker
        if command and command[0] == "--":
            command = command[1:]
        if not command:
            parser.error("worker command required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))


if __name__ == "__main__":
    main()
