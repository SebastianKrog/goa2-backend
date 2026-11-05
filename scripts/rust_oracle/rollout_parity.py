"""Compare first-legal game-turn rollouts with the frozen Python engine."""

import argparse
import json
import os
from collections import Counter, defaultdict
from copy import deepcopy
from importlib.util import module_from_spec, spec_from_file_location
from math import ceil, isclose
from pathlib import Path
from random import Random
from statistics import median
from types import SimpleNamespace
from time import perf_counter

from model_traces import setup
from oracle import ROOT, Worker, WorkerUnavailable, verify_sources
from shared_objects import export_state_with_shared_objects

from goa2.domain.input import InputResponse, selection_value
from goa2.domain.models import CardTier, GamePhase
from goa2.engine.handler import process_stack, submit_input
from goa2.engine.phases import commit_card, finish_planning, pass_turn, planning_open_for_second_card
from goa2.engine.setup import GameSetup


def run(root, turns, *, copy_state=True, policy=None):
    state = deepcopy(root) if copy_state else root
    cutoff = state.round * 4 + state.turn + turns
    count = 0
    req = None
    while state.phase != GamePhase.GAME_OVER and state.round * 4 + state.turn < cutoff:
        if state.phase == GamePhase.PLANNING:
            moves = policy(state, None, count) if policy else None
            for team in state.teams.values():
                for hero in team.heroes:
                    planned = [move for move in moves if move.player_id == hero.id] if moves else ()
                    if planned:
                        for move in planned:
                            if move.kind == "CARD":
                                card = next(card for card in hero.hand if card.id == move.selection)
                                commit_card(state, hero.id, card)
                            elif move.kind == "FINISH_PLANNING":
                                finish_planning(state, hero.id)
                            elif move.kind == "PASS":
                                pass_turn(state, hero.id)
                            else:
                                raise AssertionError("unexpected validated planning move")
                    elif hero.id in state.pending_inputs:
                        if planning_open_for_second_card(state, hero.id):
                            finish_planning(state, hero.id)
                    elif hero.hand:
                        commit_card(state, hero.id, hero.hand[0])
                        if planning_open_for_second_card(state, hero.id):
                            finish_planning(state, hero.id)
                    else:
                        pass_turn(state, hero.id)
            count += 1
        elif req:
            move = policy(state, req, count) if policy else None
            if move is not None:
                choice = move.selection
            elif req.request_type.value == "UPGRADE_PHASE":
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
        assert count < max(512, turns * 128)
    return state, count


def verify_native_run(
    command, map_id, root, turns, *, apply, sample_future_planning=False,
    seed=29, decision_ms=2_000,
):
    from automata.runtime.driver import eligible_hero_ids_for_request
    from planning_validation import validate_commitments
    from public_upgrades import PublicUpgradeRecorder
    from resolution_validation import validate_resolution
    from state_codec import verified_instance_exporter

    export_instances = verified_instance_exporter()
    upgrades = PublicUpgradeRecorder()
    totals = Counter()
    failures = Counter()
    elapsed_by_phase = defaultdict(list)
    worker = Worker(command)

    def native_choice(state, prompt, revision, player):
        nonlocal worker
        started = perf_counter()
        envelope, _ = export_state_with_shared_objects(state, instance_exporter=export_instances)
        wire = dict(envelope=envelope, player_id=player, max_depth=1,
                    max_nodes=512, time_ms=decision_ms, timeout=decision_ms / 1_000 + 2)
        if apply:
            wire["rollout_turns"] = 2
        validation = dict(player_id=player, controlled_ids={player},
                          searched_revision=revision, live_revision=revision)
        phase = "planning" if prompt is None else "resolution"
        if upgrades.batches and (prompt is None or sample_future_planning):
            wire["public_upgrade_batches"] = upgrades.batches
        try:
            if prompt is None:
                result = worker.request(
                    "search_sampled_planning", seed=39, cap=2,
                    bot_team={"mode": "true_rules", "bot_ids": [player], "worlds": 2}, **wire,
                )
                move = validate_commitments(state, envelope, result, **validation)
            else:
                if sample_future_planning:
                    wire["planning_sample_seed"] = 39
                result = worker.request("search_resolution", **wire)
                move = validate_resolution(state, prompt, result, **validation)
        except (TimeoutError, WorkerUnavailable) as error:
            totals[f"{phase}_fallback"] += 1
            failures[str(error)] += 1
            worker.close()
            worker = Worker(command)
        except (RuntimeError, ValueError) as error:
            totals[f"{phase}_fallback"] += 1
            failures[str(error)] += 1
        else:
            totals[f"{phase}_validated"] += 1
            if apply:
                if prompt is None:
                    hand = state.get_hero(player).hand
                    if hand and move[0].selection != hand[0].id:
                        totals["planning_changed_first_choice"] += 1
                elif prompt.options and move.selection != selection_value(prompt.options[0]):
                    totals["resolution_changed_first_option"] += 1
                return move
        finally:
            elapsed_by_phase[phase].append(1_000 * (perf_counter() - started))

    def choose(state, prompt, revision):
        upgrades.observe(state, revision)
        if prompt is not None:
            eligible = eligible_hero_ids_for_request(state, prompt)
            if not eligible:
                return None
            return native_choice(state, prompt, revision, eligible[0])
        return native_choice(state, None, revision, "hero_arien")

    try:
        final, count = run(root, turns, policy=choose)
        upgrades.observe(final, count)
    finally:
        worker.close()
    if not apply:
        expected, expected_count = run(root, turns)
        assert count == expected_count
        assert normalized_state(final.model_dump(mode="json")) == normalized_state(
            expected.model_dump(mode="json")
        )
    else:
        assert final.phase == GamePhase.GAME_OVER or (
            final.round * 4 + final.turn >= root.round * 4 + root.turn + turns
        )
    assert totals["planning_validated"] and totals["resolution_validated"], failures
    if upgrades.batches:
        totals["public_upgrade_batches"] = len(upgrades.batches)
    mode = "play" if apply else "shadow"
    print(f"{map_id} native {mode}, seed {seed}, up to {turns} turns: {dict(totals)}")
    for phase, samples in sorted(elapsed_by_phase.items()):
        samples.sort()
        p50, p95 = (samples[ceil(len(samples) * fraction) - 1] for fraction in (0.5, 0.95))
        print(f"  {phase} latency p50 {p50:.1f} ms, p95 {p95:.1f} ms ({len(samples)} decisions)")
    for reason, count in failures.items():
        print(f"  {count} fallback(s) for {reason}")


def describe_state_difference(expected, actual, path="state"):
    if isinstance(expected, dict) and isinstance(actual, dict):
        for key in expected.keys() | actual.keys():
            if key not in expected:
                return f"{path}.{key} only exists in Rust"
            if key not in actual:
                return f"{path}.{key} only exists in Python"
        for key, value in expected.items():
            if value != actual[key]:
                return describe_state_difference(value, actual[key], f"{path}.{key}")
    elif isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            return f"{path} length differs (Python {len(expected)}, Rust {len(actual)})"
        for index, (left, right) in enumerate(zip(expected, actual, strict=True)):
            if left != right:
                return describe_state_difference(left, right, f"{path}[{index}]")
    return f"{path} differs (Python {repr(expected)[:160]}, Rust {repr(actual)[:160]})"


def verify_rust_match_replay(
    worker, map_id, root, turns, *, require_game_over=False, sweep_policy=None, seed=39,
    decision_ms=1_000,
):
    from automata.runtime.legality import DecisionMove, check_moves, check_sequences
    from goa2.engine.phases import hero_can_play_two_cards

    envelope, _ = export_state_with_shared_objects(root)
    baseline = dict(depth=0, max_nodes=4096, time_ms=decision_ms)
    candidate = dict(baseline, depth=1, rollout_turns=2, evaluation="lab_attack_range_v1")
    if sweep_policy is None:
        pairs = ([candidate, baseline], [baseline, candidate])
    elif sweep_policy == "search":
        pairs = ([candidate, candidate],)
    else:
        policy = dict(baseline, sample_choices=sweep_policy == "sampled")
        pairs = ([policy, policy],)
    for policies in pairs:
        result = worker.request(
            "offline_match", envelope=envelope, turns=turns, seed=seed,
            policies=policies, capture_trace=True,
        )

        def replay(state, prompt, revision):
            row = result["trace"][revision]
            if prompt is not None:
                assert row["kind"] == "resolution"
                move = DecisionMove("INPUT", row["hero"], row["selection"], prompt.id)
                verdict = check_moves(state, [move]).results[0]
                assert verdict.legal is True, (
                    map_id, revision, state.round, state.turn, row,
                    prompt.model_dump(mode="json"), verdict,
                )
                return move

            assert row["kind"] == "planning"
            eligible = {
                hero.id for team in state.teams.values() for hero in team.heroes
                if hero.id not in state.pending_inputs
                or planning_open_for_second_card(state, hero.id)
            }
            commitments = row["commitments"]
            assert len(commitments) == len(eligible) and {
                entry["hero"] for entry in commitments
            } == eligible, (map_id, revision, state.round, state.turn, row, eligible)
            moves = []
            for entry in commitments:
                hero = state.get_hero(entry["hero"])
                first, second = entry["first"], entry["second"]
                second_slot = hero.id in state.pending_inputs
                if first is None:
                    kind = "FINISH_PLANNING" if second_slot else "PASS"
                    moves.append(DecisionMove(kind, hero.id))
                    continue
                moves.append(DecisionMove("CARD", hero.id, first))
                if second is not None:
                    moves.append(DecisionMove("CARD", hero.id, second))
                elif not second_slot and len(hero.hand) > 1 and hero_can_play_two_cards(hero):
                    moves.append(DecisionMove("FINISH_PLANNING", hero.id))
            assert check_sequences(state, [moves]).results[0].legal is True, row
            return moves

        final, count = run(root, turns, policy=replay)
        assert count == len(result["trace"])
        expected = normalized_state(final.model_dump(mode="json"))
        actual = normalized_state(result["state"])
        assert expected == actual, describe_state_difference(expected, actual)
        if require_game_over:
            assert final.phase == GamePhase.GAME_OVER, (map_id, final.round, final.turn)
        if final.phase == GamePhase.GAME_OVER:
            assert result["winner_team"] == (final.winner.value if final.winner else None)
            assert result["winner_hero"] == final.individual_winner_id
        endpoint = (
            f"game over, winner {result['winner_team'] or result['winner_hero']}"
            if final.phase == GamePhase.GAME_OVER
            else f"round {final.round}, turn {final.turn}"
        )
        print(f"{map_id}: Python validated {count} Rust match decisions through {endpoint}")
        for team, (policy, stats) in enumerate(zip(policies, result["policies"], strict=True)):
            if not policy["depth"]:
                continue
            print(
                f"  {('RED', 'BLUE')[team]} search used {stats['fallback']} fallbacks across "
                f"{stats['planning']} planning and {stats['resolution']} resolution decisions "
                f"({stats.get('forced', 0)} forced)"
            )
            reasons = stats.get("resolution_fallbacks", {})
            for reason, frequency in sorted(reasons.items(), key=lambda row: (-row[1], row[0]))[:3]:
                print(f"    {frequency} × {reason}")


def verify_terminal_matches(worker):
    from simulation_bench import PythonRunner, decode, initial_state
    from stack_traces import read_case, read_index
    from state_codec import export_state_with_instances

    path = ROOT / "rust/tests/fixtures/complete-games-hanu.json"
    index, _ = read_index(path)
    for entry in index["cases"]:
        case = read_case(path, entry)
        root, map_id = initial_state(case)
        runner = PythonRunner(root, export_state_with_instances(root)[1])
        operations = case["operations"]
        inputs = [i for i, op in enumerate(operations) if op["kind"] == "input"]
        runner.run(decode(operations[:inputs[-4]]))
        assert runner.request is not None, case["name"]
        verify_rust_match_replay(worker, map_id, root, 4, require_game_over=True)


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


def verify_scoped_planning(worker, map_id, root):
    from planning_validation import validate_commitments
    from state_codec import REFERENCE_KEY

    player, ally, opponent = "hero_arien", "hero_brogan", "hero_tigerclaw"

    def search(state, mode, bots):
        envelope, _ = export_state_with_shared_objects(state)
        result = worker.request(
            "search_sampled_planning", timeout=12, envelope=envelope, player_id=player,
            seed=39, cap=2, max_depth=0, max_nodes=16, time_ms=10_000,
            bot_team={"mode": mode, "bot_ids": bots, "worlds": 2},
        )
        controlled = [player] if mode == "true_rules" else bots
        validate_commitments(
            state, envelope, result, player_id=player, controlled_ids=controlled,
            searched_revision=1, live_revision=1,
        )
        for revision, roster in ((2, controlled), (1, [])):
            try:
                validate_commitments(
                    state, envelope, result, player_id=player, controlled_ids=roster,
                    searched_revision=1, live_revision=revision,
                )
            except ValueError:
                pass
            else:
                raise AssertionError("accepted a stale or unauthorized planning response")
        foreign = next(
            hero["hand"][0][REFERENCE_KEY]
            for team in envelope["state"]["teams"].values()
            for hero in team["heroes"]
            if hero["id"] == ally
        )
        invalid = deepcopy(result)
        invalid["commitments"][0]["first_card"] = foreign
        try:
            validate_commitments(
                state, envelope, invalid, player_id=player, controlled_ids=controlled,
                searched_revision=1, live_revision=1,
            )
        except ValueError as error:
            assert str(error) == "physical card was outside the searched hand"
        else:
            raise AssertionError("accepted a physical card from another hero's hand")
        assert result["evaluated_cells"] == result["total_cells"] == 8
        return result

    def commitment_ids(result):
        return [row["player_id"] for row in result["commitments"]]

    assert commitment_ids(search(root, "true_rules", [player, ally])) == [player]
    assert commitment_ids(search(root, "coordinated", [player, ally])) == [player, ally]

    before = search(root, "coordinated_show", [player])
    committed = deepcopy(root)
    commit_card(committed, ally, committed.get_hero(ally).hand[0])
    assert committed.pending_inputs[ally] is not None
    know_mode = "coordinated_know_same_team_humans"
    know = search(committed, know_mode, [player])
    assert commitment_ids(know) == [player]

    commit_card(committed, opponent, committed.get_hero(opponent).hand[0])
    assert committed.pending_inputs[opponent] is not None
    assert search(committed, "coordinated_show", [player]) == before
    assert search(committed, know_mode, [player]) == know
    print(f"{map_id} turn {root.turn}: scoped plans legal in Python across all four modes")


def verify_scoped_worker_scaling(worker, map_id, root):
    from planning_validation import validate_commitments

    envelope, _ = export_state_with_shared_objects(root)
    player = "hero_arien"
    options = dict(
        envelope=envelope, player_id=player, seed=39, cap=4, max_depth=1,
        max_nodes=100_000, time_ms=10_000, rollout_turns=2, timeout=12,
        bot_team={"mode": "true_rules", "bot_ids": [player], "worlds": 4},
    )
    reference = worker.request("search_sampled_planning", workers=1, **options)
    assert reference["prepared_worlds"] == 4
    assert reference["completed_rows"] == 4
    validate_commitments(
        root, envelope, reference, player_id=player, controlled_ids={player},
        searched_revision=1, live_revision=1,
    )

    elapsed = {workers: [] for workers in (1, 2, 4)}
    for order in ((1, 2, 4), (4, 2, 1), (2, 4, 1), (1, 4, 2), (4, 1, 2)):
        for workers in order:
            start = perf_counter()
            actual = worker.request("search_sampled_planning", workers=workers, **options)
            elapsed[workers].append(1000 * (perf_counter() - start))
            assert actual == reference, (map_id, root.turn, workers)
    timings = ", ".join(f"{workers}: {median(samples):.1f} ms"
                        for workers, samples in elapsed.items())
    print(f"{map_id} turn {root.turn} scoped workers ({reference['nodes']} nodes): {timings}")


def verify_shown_commitments(worker, root):
    state = deepcopy(root)
    player, ally, opponent = "hero_arien", "hero_brogan", "hero_tigerclaw"

    def shown(viewer, mode="coordinated_show"):
        envelope, _ = export_state_with_shared_objects(state)
        return worker.request(
            "show_bot_commitments", envelope=envelope, viewer_id=viewer,
            bot_ids=[player, opponent], mode=mode,
        )["commitments"]

    assert shown(ally) == []
    own_card = state.get_hero(player).hand[0]
    enemy_card = state.get_hero(opponent).hand[0]
    commit_card(state, player, own_card)
    commit_card(state, opponent, enemy_card)
    assert shown(ally) == [{"hero": player, "first": str(own_card.id), "second": None}]
    assert shown("hero_sabina") == [{"hero": opponent, "first": str(enemy_card.id), "second": None}]
    for mode in ("true_rules", "coordinated", "coordinated_know_same_team_humans"):
        assert shown(ally, mode) == []


def verify_public_upgrade_replay(worker, root):
    envelope, _ = export_state_with_shared_objects(root)
    hero = root.get_hero("hero_brogan")
    stats = ("ATTACK", "DEFENSE", "MOVEMENT", "INITIATIVE", "RANGE", "RADIUS")
    items = {stat.value: count for stat, count in hero.items.items()}
    before = {"level": 1, "card_items": [0] * len(stats)}
    after = {"level": hero.level, "card_items": [items.get(stat, 0) for stat in stats]}
    batch = {"revision": 3, "hero_id": str(hero.id), "before": before, "after": after}
    options = dict(
        envelope=envelope, player_id="hero_arien", seed=39, cap=2,
        max_depth=1, max_nodes=512, time_ms=10_000,
    )
    for workers in (1, 2):
        direct = worker.request(
            "search_sampled_planning", workers=workers,
            upgrade_checkpoints={str(hero.id): [before, after]}, **options,
        )
        replay = worker.request(
            "search_sampled_planning", workers=workers,
            public_upgrade_batches=[batch, batch], **options,
        )
        assert replay == direct
    try:
        worker.request("search_sampled_planning", upgrade_checkpoints={"typo": [before]}, **options)
    except RuntimeError as error:
        assert "upgrade checkpoints name an unknown hero" in str(error)
    else:
        raise AssertionError("search ignored an unknown hero in upgrade evidence")


def verify_large_upgrade_batches(worker, opening):
    from goa2.engine.steps import EndPhaseCleanupStep, ResolveUpgradesStep
    from planning_validation import validate_commitments
    from public_upgrades import PublicUpgradeRecorder

    stats = ("ATTACK", "DEFENSE", "MOVEMENT", "INITIATIVE", "RANGE", "RADIUS")

    def checkpoint(hero):
        totals = {stat.value: count for stat, count in hero.items.items()}
        return {"level": hero.level, "card_items": [totals.get(stat, 0) for stat in stats]}

    def apply_batch(state, hero, target_level, revision, recorder, game_id):
        before = checkpoint(hero)
        count = min(target_level, 7) - hero.level
        for team in state.teams.values():
            for other in team.heroes:
                other.gold = 0
        hero.gold = sum(range(hero.level, target_level))
        EndPhaseCleanupStep()._level_up(state)
        assert state.phase == GamePhase.LEVEL_UP and hero.level == target_level
        step = ResolveUpgradesStep()
        state.execution_stack = [step]
        prompt = process_stack(state).input_request
        assert prompt.context["players"][hero.id]["remaining"] == count
        recorded = len(recorder.batches)
        recorder.observe(state, revision)
        assert len(recorder.batches) == recorded
        saved = json.loads(json.dumps(recorder.checkpoint(game_id)))
        resumed = PublicUpgradeRecorder.restore(saved, game_id)
        assert resumed.batches == recorder.batches

        choices = []
        preview = state
        for _ in range(count):
            choice = step._get_upgrade_options(preview, hero.id)[0]["pair"][0]
            choices.append(choice)
            preview = step.preview_choices(preview, hero.id, [choice])
        submit_input(state, InputResponse(
            request_id=prompt.id, selection={"hero_id": hero.id, "card_ids": choices},
        ))
        process_stack(state)
        assert state.phase == GamePhase.PLANNING and not state.upgrade_reveal_snapshot
        recorder.observe(state, revision)
        resumed.observe(state, revision)
        assert resumed.checkpoint(game_id) == recorder.checkpoint(game_id)
        return {"revision": revision, "hero_id": hero.id, "before": before, "after": checkpoint(hero)}

    for levels in ((3, 6), (8,)):
        state = deepcopy(opening)
        hero = state.get_hero("hero_brogan")
        recorder = PublicUpgradeRecorder()
        recorder.observe(state, 0)
        game_id = f"{state.board.map_id}:{levels}"
        batches = [
            apply_batch(state, hero, level, revision, recorder, game_id)
            for revision, level in enumerate(levels, start=1)
        ]
        assert recorder.batches == batches
        persisted = json.loads(json.dumps(recorder.checkpoint(game_id)))
        restored = PublicUpgradeRecorder.restore(persisted, game_id)
        assert restored.batches == batches
        latest = batches[-1]
        assert latest["before"]["level"] == (3 if len(levels) == 2 else 1)
        assert latest["after"]["level"] == levels[-1]
        if levels[-1] == 8:
            assert hero.ultimate_card is not None
        envelope, _ = export_state_with_shared_objects(state)
        player = "hero_arien"
        options = dict(
            envelope=envelope, player_id=player, seed=39, cap=2, max_depth=0,
            max_nodes=16, time_ms=10_000,
            bot_team={"mode": "true_rules", "bot_ids": [player], "worlds": 2},
        )
        direct = worker.request(
            "search_sampled_planning", **options,
            upgrade_checkpoints={hero.id: [batches[0]["before"], *(batch["after"] for batch in batches)]},
        )
        replay = worker.request(
            "search_sampled_planning", **options, public_upgrade_batches=[*reversed(restored.batches), latest],
        )
        assert direct == replay
        validate_commitments(
            state, envelope, replay, player_id=player, controlled_ids={player},
            searched_revision=latest["revision"], live_revision=latest["revision"],
        )
        print(f"{state.board.map_id}: {latest['before']['level']}-to-{levels[-1]} "
              "upgrade batch replay matched direct evidence")


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


def verify_sampled_resolution(
    worker, map_path, opening, decision_ms, sample_count, workers, iterations, verify_deadline,
    transport_reserve_ms,
):
    from automata.runtime.driver import eligible_hero_ids_for_request
    from automata.runtime.legality import DecisionMove
    from public_upgrades import ITEM_STATS
    from resolution_validation import validate_resolution

    map_id = map_path.stem
    cases = [("opening", deepcopy(opening)), ("upgraded", upgraded_opening(opening))]
    if map_id == "forgotten_island":
        cases.extend((label, deepcopy(opening)) for label in ("movement", "fast travel"))
        focus_opening = GameSetup.create_game(
            str(map_path), ["Misa", "Brogan"], ["Tigerclaw", "Sabina"], seed=29,
        )

        def play_focus(_state, _prompt, index):
            if index == 0:
                return [DecisionMove("CARD", "hero_misa", "focus")]
            return None

        focus, _ = run(focus_opening, 1, policy=play_focus)
        assert any(effect.source_card_id == "focus" for effect in focus.active_effects)
        cases.append(("focus", focus))

    for label, root in cases:
        checkpoints = {}
        for team in root.teams.values():
            for hero in team.heroes:
                if hero.level > 1:
                    checkpoints[hero.id] = [{
                        "level": hero.level,
                        "card_items": [hero.items.get(stat, 0) for stat in ITEM_STATS],
                    }]
                commit_card(root, hero.id, hero.hand[int(label in {"movement", "fast travel"})])
        prompt = process_stack(root).input_request
        assert prompt is not None, (map_id, label)
        if label in {"movement", "fast travel"}:
            assert prompt.request_type.value == "CHOOSE_ACTION"
            action = "MOVEMENT" if label == "movement" else "FAST_TRAVEL"
            submit_input(root, InputResponse(request_id=prompt.id, selection=action))
            prompt = process_stack(root).input_request
            assert prompt is not None and prompt.request_type.value == "SELECT_HEX"
        player = eligible_hero_ids_for_request(root, prompt)[0]
        envelope, _ = export_state_with_shared_objects(root)
        options = dict(
            envelope=envelope, player_id=player,
            max_depth=1, max_nodes=8192 if label == "fast travel" else 1024,
            rollout_turns=2,
            resolution_sample_seed=39, resolution_sample_worlds=sample_count,
            planning_sample_seed=71, upgrade_checkpoints=checkpoints or None,
        )
        modes = (1, workers) if workers > 1 else (1,)
        durations = {count: [] for count in modes}
        outcomes = {count: Counter() for count in modes}
        result = None
        for index in range(max(1, iterations)):
            order = modes if index % 2 == 0 else reversed(modes)
            for count in order:
                started = perf_counter()
                try:
                    timing = (
                        dict(decision_deadline=started + decision_ms / 1_000,
                             transport_reserve=transport_reserve_ms / 1_000)
                        if transport_reserve_ms else dict(time_ms=decision_ms)
                    )
                    reply = worker.request(
                        "search_resolution", resolution_sample_workers=count, **options, **timing,
                    )
                except RuntimeError as error:
                    if not verify_deadline or str(error) != (
                        "worker rejected request: search deadline reached during request setup"
                    ):
                        raise
                    durations[count].append(1_000 * (perf_counter() - started))
                    outcomes[count]["setup deadline (no move)"] += 1
                    continue
                durations[count].append(1_000 * (perf_counter() - started))
                if verify_deadline:
                    depth = reply["completed_depth"]
                    assert depth in (0, 1), (map_id, label, reply)
                    if depth == 1:
                        assert reply["score_kind"] == "turn_rollout", (map_id, label, reply)
                        assert reply["score"] is not None, (map_id, label, reply)
                    else:
                        assert reply["stop"] == "deadline", (map_id, label, reply)
                        kind = "unscored" if reply["score"] is None else reply["score_kind"]
                        assert kind in {"unscored", "immediate", "turn_rollout"}, (map_id, label, reply)
                    validate_resolution(
                        root, prompt, reply, player_id=player, controlled_ids={player},
                        searched_revision=0, live_revision=0, require_score=False,
                    )
                    outcome = "completed" if depth else f"deadline ({kind})"
                    outcomes[count][outcome] += 1
                    continue
                if result is None:
                    result = reply
                else:
                    assert reply == result, (map_id, label, count, result, reply)
        if verify_deadline:
            for count in modes:
                samples = durations[count]
                if transport_reserve_ms:
                    assert max(samples) <= decision_ms, (map_id, label, count, samples)
                counts = ", ".join(f"{key}: {value}" for key, value in sorted(outcomes[count].items()))
                print(f"{map_id} {label} {count} worker(s): {counts}; "
                      f"end-to-end median {median(samples):.2f} ms, max {max(samples):.2f} ms")
            continue
        assert result["completed_depth"] == 1 and result["score_kind"] == "turn_rollout", (
            map_id, label, result,
        )
        move = validate_resolution(
            root, prompt, result, player_id=player, controlled_ids={player},
            searched_revision=0, live_revision=0,
        )
        print(f"{map_id} {label} {prompt.request_type.value}: "
              f"Python accepted {move.selection} ({result['nodes']} Rust nodes)")
        if iterations:
            times = ", ".join(
                f"{count} worker(s) {median(samples):.2f} ms"
                for count, samples in durations.items()
            )
            print(f"  end-to-end median over {iterations} runs: {times}")



def verify_sampled_rollout(worker, map_id, root, evaluator, iterations, sample_count=1, workers=1):
    seeds = range(39, 39 + sample_count)
    envelope, _ = export_state_with_shared_objects(root)
    case = (map_id, root.turn)
    expected = []
    for seed in seeds:
        leaf, nodes = run(sampled_python(root, seed), 2, copy_state=False)
        result = worker.request(
            "rollout_game_turns", envelope=envelope, player_id="hero_arien",
            turns=2, max_nodes=512, time_ms=10_000, capture_state=True, sample_seed=seed,
        )
        assert result["complete"] and result["nodes"] == nodes, (case, seed)
        assert normalized_state(result["state"]) == normalized_state(leaf.model_dump(mode="json")), (case, seed)
        expected.append((seed, nodes, lab_value(evaluator, leaf) if evaluator else None))
    print(f"{map_id} turn {root.turn}: sampled rollout state matched ({sample_count} seeds)")
    if iterations or evaluator is not None:
        native = worker.request(
            "rollout_bench", envelope=envelope, player_id="hero_arien",
            iterations=max(sample_count, iterations, workers), sample_seed=seeds.start,
            sample_count=sample_count, lab_evaluation=evaluator is not None,
            **({"workers": workers} if workers > 1 else {}),
        )
        for row, (seed, nodes, value) in zip(native["sample_results"], expected, strict=True):
            assert (row["seed"], row["nodes"]) == (seed, nodes), case
            if value is not None:
                assert isclose(row["lab_value"], value, rel_tol=1e-12, abs_tol=1e-12), case
    if iterations:
        sampling_share = native["sampling_and_restore_seconds"] / native.get("worker_seconds", native["seconds"])
        print(f"{map_id} turn {root.turn}: {native['rollouts_per_second']:.1f} Rust "
              f"sampled rollouts/s ({sampling_share:.1%} in sampling and restoring)")


def upgraded_opening(opening):
    from goa2.engine.steps.cards import apply_hero_upgrade

    upgraded = deepcopy(opening)
    for hero_id in ("hero_brogan", "hero_tigerclaw"):
        hero = upgraded.get_hero(hero_id)
        hero.level = 2
        card = next(card for card in hero.deck if card.tier == CardTier.II)
        apply_hero_upgrade(upgraded, hero_id, card.id)
    return upgraded


def verify_roster_sweep(worker, paths, turns, seed, policy, *, team_size, game_type, decision_ms):
    from goa2.data.heroes import HeroRegistry

    heroes = sorted(HeroRegistry.list_heroes(include_playtest=True))
    failures = []
    roster_size = 2 * team_size
    for path in paths:
        for index in range(0, len(heroes), roster_size):
            roster = (heroes + heroes[:roster_size - 1])[index:index + roster_size]
            case = f"{path.stem} ({', '.join(roster)})"
            try:
                root = GameSetup.create_game(
                    str(path), roster[:team_size], roster[team_size:], seed=seed, game_type=game_type,
                )
                verify_rust_match_replay(
                    worker, case, root, turns, sweep_policy=policy, seed=seed, decision_ms=decision_ms,
                )
            except (AssertionError, RuntimeError, ValueError) as error:
                failures.append(case)
                print(f"{case}: {type(error).__name__}: {error}", flush=True)
    if failures:
        raise AssertionError(f"{len(failures)} roster/map cases failed: {', '.join(failures)}")


def verify_recorded_rollouts(worker, evaluator, iterations, sample_count, workers):
    from simulation_bench import read_case, read_index, workloads

    corpus = ROOT / "rust/tests/fixtures/complete-games-hanu.json"
    index, _ = read_index(corpus)
    for entry in index["cases"]:
        for workload in workloads(read_case(corpus, entry)):
            verify_sampled_rollout(
                worker, workload["name"], workload["state"], evaluator, iterations, sample_count, workers,
            )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--bench-iterations", type=int, default=0)
    parser.add_argument("--lab-evaluator", type=Path, help="Path to the lab's bot_evaluation.py")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--shadow-turns", type=int, choices=range(1, 9),
                      help="Validate native suggestions while Python runs this many turns")
    mode.add_argument("--native-turns", type=int,
                      help="Apply validated native suggestions through Python for this many turns")
    mode.add_argument("--replay-terminal", action="store_true",
                      help="Validate Rust-selected finishing moves from recorded late-game states")
    mode.add_argument("--roster-sweep-turns", type=int,
                      help="Replay Rust decisions for every registered hero")
    parser.add_argument("--sweep-team-size", type=int, choices=range(1, 6), default=2)
    parser.add_argument("--sweep-game-type", choices=("LONG", "QUICK"), default="LONG")
    parser.add_argument("--sweep-policy", choices=("first-legal", "sampled", "search"),
                        default="first-legal", help="Rust policy used by both teams in the sweep")
    mode.add_argument("--recorded-rollouts", action="store_true",
                      help="Validate and benchmark sampled rollouts from recorded game positions")
    mode.add_argument("--sampled-resolution", action="store_true",
                      help="Validate sampled resolution choices and two-turn scores")
    parser.add_argument("--prepare-search", action="store_true",
                        help="Initialize immutable Rust rule programs before timing decisions")
    parser.add_argument("--resolution-transport-reserve-ms", type=int, default=0,
                        help="Use a caller deadline and reserve time for the worker exchange")
    parser.add_argument("--verify-resolution-deadline", action="store_true",
                        help="Validate interrupted choices and report end-to-end latency")
    parser.add_argument("--decision-ms", type=int, default=2_000,
                        help="Configured native search time per decision")
    parser.add_argument(
        "--replay-roster", nargs=4, metavar=("TEAM_A_1", "TEAM_A_2", "TEAM_B_1", "TEAM_B_2"),
        help="Hero names for a dedicated Rust match replay",
    )
    parser.add_argument("--game-seed", type=int, default=29,
                        help="Seed used to initialize the board")
    parser.add_argument("--sample-future-planning", action="store_true",
                        help="Sample future hidden hands in native resolution rollouts")
    parser.add_argument("--rust-replay-turns", type=int,
                        help="Replay a Rust-only match in Python for rule validation")
    parser.add_argument("--bench-planning-workers", action="store_true",
                        help="Compare scoped planning on one, two and four workers")
    parser.add_argument("--planning", action="store_true", help="Also verify sampled planning")
    parser.add_argument("--all-maps", action="store_true", help="Check every bundled map")
    parser.add_argument("--rollout-workers", type=int, choices=range(1, 5), default=1,
                        help="Independent workers for the recorded rollout benchmark")
    parser.add_argument("--sample-workers", type=int, choices=range(1, 5), default=1,
                        help="Parallel world scorers for sampled resolution")
    parser.add_argument("--sample-count", type=int, choices=range(1, 9), default=1,
                        help="Number of worlds for sampled rollout benchmarks or resolution search")
    parser.add_argument("--sampled", action="store_true", help="Also check sampled level-one roots")
    parser.add_argument("--sampled-upgrades", action="store_true", help="Also check upgraded samples")
    args = parser.parse_args()
    if args.roster_sweep_turns is not None and not 1 <= args.roster_sweep_turns <= 64:
        parser.error("--roster-sweep-turns must be in 1..64")
    if args.rust_replay_turns is not None and not 1 <= args.rust_replay_turns <= 64:
        parser.error("--rust-replay-turns must be in 1..64")
    if args.replay_roster and not args.rust_replay_turns:
        parser.error("--replay-roster requires --rust-replay-turns")
    if args.native_turns is not None and not 1 <= args.native_turns <= 40:
        parser.error("--native-turns must be in 1..40")
    if not 1 <= args.decision_ms <= 60_000:
        parser.error("--decision-ms must be in 1..60000")
    if args.sample_future_planning and not args.native_turns:
        parser.error("--sample-future-planning requires --native-turns")
    if args.prepare_search and not args.sampled_resolution:
        parser.error("--prepare-search requires --sampled-resolution")
    if args.verify_resolution_deadline and not args.sampled_resolution:
        parser.error("--verify-resolution-deadline requires --sampled-resolution")
    if not 0 <= args.resolution_transport_reserve_ms < args.decision_ms:
        parser.error("resolution transport reserve must be smaller than --decision-ms")
    if args.resolution_transport_reserve_ms and not args.verify_resolution_deadline:
        parser.error("resolution transport reserve requires --verify-resolution-deadline")
    if args.rollout_workers > 1 and not args.recorded_rollouts:
        parser.error("--rollout-workers requires --recorded-rollouts")
    if args.rollout_workers > 1 and args.bench_iterations < args.rollout_workers:
        parser.error("--bench-iterations must cover every rollout worker")
    if args.sample_workers > args.sample_count:
        parser.error("--sample-workers cannot exceed --sample-count")
    if not args.sampled_resolution and 0 < args.bench_iterations < args.sample_count:
        parser.error("--bench-iterations must cover every sample")
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
    roster = args.replay_roster or ["Arien", "Brogan", "Tigerclaw", "Sabina"]
    worker = Worker([args.worker])
    try:
        if args.prepare_search:
            started = perf_counter()
            prepared = worker.request("prepare_search")
            assert prepared == {"ready": True}, prepared
            print(f"Rust search preparation: {1_000 * (perf_counter() - started):.2f} ms")
        if args.roster_sweep_turns:
            verify_roster_sweep(
                worker, paths, args.roster_sweep_turns, args.game_seed, args.sweep_policy,
                team_size=args.sweep_team_size, game_type=args.sweep_game_type,
                decision_ms=args.decision_ms,
            )
            return
        if args.recorded_rollouts:
            verify_recorded_rollouts(
                worker, evaluator, args.bench_iterations, args.sample_count, args.rollout_workers,
            )
            return
        if args.replay_terminal:
            verify_terminal_matches(worker)
            return
        for path in paths:
            map_id = path.stem
            opening = GameSetup.create_game(
                str(path),
                roster[:2],
                roster[2:],
                seed=args.game_seed,
            )
            if args.sampled_resolution:
                verify_sampled_resolution(
                    worker, path, opening, args.decision_ms, args.sample_count, args.sample_workers,
                    args.bench_iterations, args.verify_resolution_deadline,
                    args.resolution_transport_reserve_ms,
                )
                continue
            if args.shadow_turns:
                verify_native_run(
                    [args.worker], map_id, opening, args.shadow_turns,
                    apply=False, decision_ms=args.decision_ms,
                )
            if args.native_turns:
                verify_native_run(
                    [args.worker], map_id, opening, args.native_turns, apply=True,
                    sample_future_planning=args.sample_future_planning,
                    seed=args.game_seed, decision_ms=args.decision_ms,
                )
            if args.rust_replay_turns:
                verify_rust_match_replay(worker, map_id, opening, args.rust_replay_turns)
                if args.replay_roster:
                    continue
            if args.bench_planning_workers:
                verify_scoped_worker_scaling(worker, map_id, opening)
                midround, _ = run(opening, 1)
                verify_scoped_worker_scaling(worker, map_id, midround)
            if args.planning:
                verify_sampled_planning(worker, map_id, opening)
                verify_scoped_planning(worker, map_id, opening)
                verify_shown_commitments(worker, opening)
                midround, _ = run(opening, 1)
                verify_sampled_planning(worker, map_id, midround)
                verify_scoped_planning(worker, map_id, midround)
            if args.sampled:
                midround, _ = run(opening, 1)
                for root in (opening, midround):
                    verify_sampled_rollout(
                        worker, map_id, root, evaluator, args.bench_iterations, args.sample_count,
                    )
            if args.sampled_upgrades:
                verify_large_upgrade_batches(worker, opening)
                upgraded = upgraded_opening(opening)
                midround, _ = run(upgraded, 1)
                for root in (upgraded, midround):
                    verify_public_upgrade_replay(worker, root)
                    verify_sampled_rollout(worker, f"{map_id} upgraded", root, evaluator,
                                           args.bench_iterations, args.sample_count)
            round_end, _ = run(opening, 3)
            assert (round_end.round, round_end.turn, round_end.phase) == (1, 4, GamePhase.PLANNING)
            if args.sampled:
                verify_sampled_rollout(
                    worker, map_id, round_end, evaluator, args.bench_iterations, args.sample_count,
                )
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
                    print(f"{map_id} {label}: {native['rollouts_per_second']:.1f} Rust rollouts/s")
    finally:
        worker.close()


if __name__ == "__main__":
    main()
