"""Compare first-legal game-turn rollouts with the frozen Python engine."""

import argparse
import os
from copy import deepcopy
from importlib.util import module_from_spec, spec_from_file_location
from math import isclose
from pathlib import Path
from random import Random
from types import SimpleNamespace
from time import perf_counter

from model_traces import setup
from oracle import ROOT, Worker, verify_sources
from shared_objects import export_state_with_shared_objects

from goa2.domain.input import InputResponse, selection_value
from goa2.domain.models import CardTier, GamePhase
from goa2.engine.handler import process_stack, submit_input
from goa2.engine.phases import commit_card, pass_turn
from goa2.engine.setup import GameSetup


def run(root, turns, *, copy_state=True):
    state = deepcopy(root) if copy_state else root
    cutoff = state.round * 4 + state.turn + turns
    count = 0
    req = None
    while state.phase != GamePhase.GAME_OVER and state.round * 4 + state.turn < cutoff:
        if state.phase == GamePhase.PLANNING:
            for team in state.teams.values():
                for hero in team.heroes:
                    if hero.hand:
                        commit_card(state, hero.id, hero.hand[0])
                    else:
                        pass_turn(state, hero.id)
            count += 1
        elif req:
            if req.request_type.value == "UPGRADE_PHASE":
                hero, row = next(
                    (hero, row)
                    for hero, row in req.context["players"].items()
                    if row["options"]
                )
                choice = {"hero_id": hero, "card_id": row["options"][0]["pair"][0]}
            else:
                choice = selection_value(req.options[0]) if req.options else "SKIP"
            submit_input(state, InputResponse(request_id=req.id, selection=choice))
            req = None
            count += 1
        else:
            req = process_stack(state).input_request
            assert req or state.phase in {GamePhase.PLANNING, GamePhase.GAME_OVER}
        assert count < 512
    return state, count


def normalized_state(state):
    for zone in state["board"]["zones"].values():
        zone["hexes"].sort(key=lambda point: (point["q"], point["r"], point["s"]))
        zone["neighbors"].sort()
    return state


def verify_sampled_planning(worker, map_id, root):
    from sequence_legality import python_move

    from automata.runtime.legality import check_sequences

    envelope, _ = export_state_with_shared_objects(root)
    options = dict(
        envelope=envelope,
        player_id="hero_arien",
        seed=39,
        cap=2,
        max_depth=1,
        max_nodes=512,
        time_ms=10_000,
        rollout_turns=2,
    )
    serial = worker.request("search_sampled_planning", workers=1, **options)
    parallel = worker.request("search_sampled_planning", workers=2, **options)
    case = (map_id, root.round, root.turn)
    for result in (serial, parallel):
        assert result["total_cells"] == 4, case
        assert result["completed_rows"] + result["pruned_rows"] == 2, case
        assert result["nodes"] <= options["max_nodes"], case
    assert serial["commitment"] == parallel["commitment"], case
    assert serial["score"] == parallel["score"], case
    card = serial["commitment"]["first_card"]
    move = python_move({"kind": "card", "player_id": "hero_arien", "card": card}, envelope)
    assert check_sequences(root, [[move]]).results[0].legal is True, case
    print(f"{map_id} turn {root.turn}: sampled planning matched with one and two workers")


def load_lab_evaluator(path):
    spec = spec_from_file_location("rollout_reference_lab_evaluator", path)
    if spec is None or spec.loader is None:
        raise ValueError(f"Cannot load the lab evaluator from {path}")
    module = module_from_spec(spec)
    spec.loader.exec_module(module)
    evaluator = module.LabLeafEvaluator()
    if evaluator.recipe_id != "lab-attack-range-v1":
        raise ValueError(f"Unexpected lab recipe {evaluator.recipe_id}")
    return evaluator


def lab_value(evaluator, state):
    hero = state.get_hero("hero_arien")
    context = SimpleNamespace(perspective_team=hero.team, root_viewer_id=str(hero.id))
    return evaluator.evaluate(context, state).value


class AlignedChoices(Random):
    """Align the choice index with the native diagnostic's per-hero seed."""

    def __init__(self, seed, indices):
        super().__init__(seed)
        self.sample_seed = seed
        self.indices = iter(indices)

    def choice(self, choices):
        mask = (1 << 64) - 1
        value = (self.sample_seed + next(self.indices) + 0x9E3779B97F4A7C15) & mask
        value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & mask
        value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & mask
        return choices[(value ^ (value >> 31)) % len(choices)]


def sampled_python(root, seed):
    from automata.runtime.determinize import determinize

    assert not root.pending_inputs
    heroes = [hero for team in root.teams.values() for hero in team.heroes]
    rng = AlignedChoices(seed, [i for i, hero in enumerate(heroes) if hero.id != "hero_arien"])
    sampled = determinize(root, "hero_arien", rng)
    assert next(rng.indices, None) is None
    for team in sampled.teams.values():
        for hero in team.heroes:
            if str(hero.id) == "hero_arien":
                continue
            # Use deck order in both engines for the diagnostic policy.
            order = {str(card.id): index for index, card in enumerate(hero.deck)}
            hero.hand.sort(key=lambda card: order[str(card.id)])
    return sampled


def verify_sampled_rollout(worker, map_id, root, evaluator, iterations):
    seed = 39
    leaf, nodes = run(sampled_python(root, seed), 2, copy_state=False)
    envelope, _ = export_state_with_shared_objects(root)
    case = (map_id, root.turn)
    result = worker.request(
        "rollout_game_turns",
        envelope=envelope,
        player_id="hero_arien",
        turns=2,
        max_nodes=512,
        time_ms=10_000,
        capture_state=True,
        sample_seed=seed,
    )
    assert result["complete"] and result["nodes"] == nodes, case
    assert normalized_state(result["state"]) == normalized_state(leaf.model_dump(mode="json")), case
    print(f"{map_id} turn {root.turn}: sampled rollout state matched")
    if iterations or evaluator is not None:
        native = worker.request(
            "rollout_bench",
            envelope=envelope,
            player_id="hero_arien",
            iterations=max(1, iterations),
            sample_seed=seed,
            lab_evaluation=evaluator is not None,
        )
        assert native["nodes_per_iteration"] == nodes, case
        if evaluator is not None:
            assert isclose(native["lab_value"], lab_value(evaluator, leaf),
                           rel_tol=1e-12, abs_tol=1e-12), case
    if iterations:
        start = perf_counter()
        for _ in range(iterations):
            end, _ = run(sampled_python(root, seed), 2, copy_state=False)
            if evaluator is not None:
                lab_value(evaluator, end)
        python_rate = iterations / (perf_counter() - start)
        print(f"{map_id} turn {root.turn}: {native['rollouts_per_second']:.1f} Rust "
              f"sampled rollouts/s, {python_rate:.1f} Python sampled rollouts/s")


def upgraded_opening(opening):
    from goa2.engine.steps.cards import apply_hero_upgrade

    upgraded = deepcopy(opening)
    for hero_id in ("hero_brogan", "hero_tigerclaw"):
        hero = upgraded.get_hero(hero_id)
        hero.level = 2
        card = next(card for card in hero.deck if card.tier == CardTier.II)
        apply_hero_upgrade(upgraded, hero_id, card.id)
    return upgraded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--bench-iterations", type=int, default=0)
    parser.add_argument("--lab-evaluator", type=Path, help="Path to the lab's bot_evaluation.py")
    parser.add_argument("--planning", action="store_true", help="Also verify sampled planning")
    parser.add_argument("--all-maps", action="store_true", help="Check every bundled map")
    parser.add_argument("--sampled", action="store_true", help="Also check sampled level-one roots")
    parser.add_argument("--sampled-upgrades", action="store_true", help="Also check upgraded samples")
    args = parser.parse_args()
    if not 0 <= args.bench_iterations <= 10_000:
        parser.error("--bench-iterations must be in 0..10000")
    if os.environ.get("PYTHONHASHSEED") != "0":
        parser.error("run with PYTHONHASHSEED=0 for reproducible choices")

    verify_sources()
    evaluator = load_lab_evaluator(args.lab_evaluator) if args.lab_evaluator else None
    setup()
    maps = ROOT / "src/goa2/data/maps"
    paths = (
        sorted(maps.glob("*.json"))
        if args.all_maps
        else [maps / "forgotten_island.json", maps / "magma_and_ice.json"]
    )
    worker = Worker([args.worker])
    try:
        for path in paths:
            map_id = path.stem
            opening = GameSetup.create_game(
                str(path),
                ["Arien", "Brogan"],
                ["Tigerclaw", "Sabina"],
                seed=29,
            )
            if args.planning:
                verify_sampled_planning(worker, map_id, opening)
                midround, _ = run(opening, 1)
                verify_sampled_planning(worker, map_id, midround)
            if args.sampled:
                midround, _ = run(opening, 1)
                for root in (opening, midround):
                    verify_sampled_rollout(worker, map_id, root, evaluator, args.bench_iterations)
            if args.sampled_upgrades:
                upgraded = upgraded_opening(opening)
                midround, _ = run(upgraded, 1)
                for root in (upgraded, midround):
                    verify_sampled_rollout(worker, f"{map_id} upgraded", root, evaluator,
                                           args.bench_iterations)
            round_end, _ = run(opening, 3)
            assert (round_end.round, round_end.turn, round_end.phase) == (1, 4, GamePhase.PLANNING)
            for label, root in (("opening", opening), ("round end", round_end)):
                envelope, _ = export_state_with_shared_objects(root)
                for turns in (1, 2):
                    leaf, nodes = run(root, turns)
                    result = worker.request(
                        "rollout_game_turns",
                        envelope=envelope,
                        player_id="hero_arien",
                        turns=turns,
                        max_nodes=512,
                        time_ms=10_000,
                        capture_state=True,
                    )
                    case = (map_id, label, turns)
                    assert result["complete"], case
                    boundary = tuple(result[key] for key in ("round", "turn", "phase", "nodes"))
                    assert boundary == (leaf.round, leaf.turn, leaf.phase.value, nodes), case
                    expected = normalized_state(leaf.model_dump(mode="json"))
                    assert normalized_state(result["state"]) == expected, case
                    print(f"{map_id} {label} +{turns}: matched round {leaf.round}, turn {leaf.turn}")
                if args.bench_iterations or evaluator is not None:
                    native = worker.request(
                        "rollout_bench",
                        envelope=envelope,
                        player_id="hero_arien",
                        iterations=max(1, args.bench_iterations),
                        lab_evaluation=evaluator is not None,
                    )
                    assert native["nodes_per_iteration"] == nodes
                    if evaluator is not None:
                        expected = lab_value(evaluator, leaf)
                        assert isclose(native["lab_value"], expected, rel_tol=1e-12, abs_tol=1e-12)
                        print(f"{map_id} {label}: lab leaf value matched")
                if args.bench_iterations:
                    start = perf_counter()
                    for _ in range(args.bench_iterations):
                        simulated, _ = run(root, 2)
                        if evaluator is not None:
                            lab_value(evaluator, simulated)
                    python_rate = args.bench_iterations / (perf_counter() - start)
                    print(
                        f"{map_id} {label}: "
                        f"{native['rollouts_per_second']:.1f} Rust rollouts/s, "
                        f"{python_rate:.1f} Python rollouts/s"
                    )
    finally:
        worker.close()


if __name__ == "__main__":
    main()
