"""Compare first-legal game-turn rollouts with the frozen Python engine."""

import argparse
import json
import os
from collections import Counter, defaultdict
from copy import deepcopy
from hashlib import file_digest
from importlib.util import module_from_spec, spec_from_file_location
from math import ceil, isclose
from pathlib import Path
from random import Random
from shutil import which
from statistics import median
from types import SimpleNamespace
from time import perf_counter

from model_traces import setup
from oracle import ROOT, Worker, WorkerUnavailable, digest, verify_sources
from resolution_validation import is_mine_path_request
from shared_objects import export_state_with_shared_objects

from goa2.domain.input import InputResponse, selection_value
from goa2.domain.models import CardTier, GamePhase
from goa2.engine.handler import process_stack, submit_input
from goa2.engine.phases import commit_card, finish_planning, pass_turn, planning_open_for_second_card
from goa2.engine.setup import GameSetup


def apply_planning_move(state, move):
    hero = state.get_hero(move.player_id)
    if move.kind == "CARD":
        card = next(card for card in hero.hand if card.id == move.selection)
        commit_card(state, hero.id, card)
    elif move.kind == "FINISH_PLANNING":
        finish_planning(state, hero.id)
    elif move.kind == "PASS":
        pass_turn(state, hero.id)
    else:
        raise AssertionError("unexpected validated planning move")


def run(root, turns, *, copy_state=True, policy=None, exact_planning=False, observe_events=None):
    state = deepcopy(root) if copy_state else root
    cutoff = state.round * 4 + state.turn + turns
    count = 0
    req = None
    while state.phase != GamePhase.GAME_OVER and state.round * 4 + state.turn < cutoff:
        if state.phase == GamePhase.PLANNING:
            moves = policy(state, None, count) if policy else None
            if exact_planning:
                assert moves is not None
                for move in moves:
                    apply_planning_move(state, move)
            else:
                for team in state.teams.values():
                    for hero in team.heroes:
                        planned = [move for move in moves if move.player_id == hero.id] if moves else ()
                        if planned:
                            for move in planned:
                                apply_planning_move(state, move)
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
            elif is_mine_path_request(state, req) and req.options:
                choice = req.options[0].id
            else:
                choice = selection_value(req.options[0]) if req.options else "SKIP"
            submit_input(state, InputResponse(request_id=req.id, selection=choice))
            req = None
            count += 1
        else:
            result = process_stack(state)
            if observe_events is not None:
                observe_events(state, result.events)
            req = result.input_request
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
        deadline = started + decision_ms / 1_000
        envelope, _ = export_state_with_shared_objects(state, instance_exporter=export_instances)
        # Reserve 10 ms for Python validation after the worker exchange.
        wire = dict(envelope=envelope, player_id=player, max_depth=1, max_nodes=512,
                    decision_deadline=deadline - 0.010, transport_reserve=0.015)
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
            if perf_counter() >= deadline:
                raise ValueError("recommendation validation exceeded the decision deadline")
        except (TimeoutError, WorkerUnavailable) as error:
            totals[f"{phase}_fallback"] += 1
            failures[str(error)] += 1
            if worker.process.poll() is not None:
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


def trace_policy(map_id, trace, *, observe=None, live_session=None):
    from automata.runtime.legality import DecisionMove, _Cursor, check_moves, check_sequences
    from goa2.engine.phases import hero_can_play_two_cards

    def replay(state, prompt, revision):
        if observe is not None:
            observe(state)
        row = trace[revision]
        if prompt is not None:
            assert row["kind"] == "resolution"
            move = DecisionMove("INPUT", row["hero"], row["selection"], prompt.id)
            # This two-stage step reissues its first (hex) prompt when
            # advanced from a cloned state without the pending response.
            # Validate the already-issued second prompt before submitting it.
            if prompt.prompt == "Choose a minion to respawn.":
                assert move.player_id == prompt.player_id and any(
                    move.selection == selection_value(option) for option in prompt.options
                ), (map_id, revision, row, prompt.model_dump(mode="json"))
                return move
            if is_mine_path_request(state, prompt):
                assert move.player_id == prompt.player_id and any(
                    move.selection == option.id for option in prompt.options
                ), (map_id, revision, row, prompt.model_dump(mode="json"))
                return move
            if live_session is not None:
                _Cursor(live_session, prompt).check_input(move)
                return move
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
        if live_session is None:
            assert check_sequences(state, [moves]).results[0].legal is True, row
        return moves

    return replay


def verify_rust_match_replay(
    worker, map_id, root, turns, *, require_game_over=False, sweep_policy=None, seed=39,
    decision_ms=1_000, policy_pairs=None, planning_workers=None,
):
    envelope, original_objects = export_state_with_shared_objects(root)
    if policy_pairs is not None:
        print(f"  Start snapshot SHA-256 {digest(envelope)}, {root.game_type.value}, "
              f"round {root.round}, turn {root.turn}", flush=True)
    baseline = dict(depth=0, max_nodes=4096, time_ms=decision_ms)
    candidate = dict(baseline, depth=1, rollout_turns=2, evaluation="lab_attack_range_v1")
    if policy_pairs is not None:
        pairs = policy_pairs
    elif sweep_policy is None:
        pairs = ([candidate, baseline], [baseline, candidate])
    elif sweep_policy == "search":
        candidate["planning_workers"] = 2 if planning_workers is None else planning_workers
        pairs = ([candidate, candidate],)
    else:
        policy = dict(baseline, sample_choices=sweep_policy == "sampled")
        pairs = ([policy, policy],)
    from public_upgrades import PublicUpgradeRecorder

    results = []
    for policies in pairs:
        result = worker.request(
            "offline_match", envelope=envelope, turns=turns, seed=seed,
            policies=policies, capture_trace=True,
        )

        evidence = None
        evidence_revision = 0
        if "public_upgrade_batches" in result:
            evidence = PublicUpgradeRecorder()
            evidence.observe(root, evidence_revision)

        lane_progress = {lane: [] for lane in root.wave_counters}
        lane_presence = {lane: {} for lane in lane_progress}
        lane_order = {
            lane: {zone: index for index, zone in enumerate(root.board.lanes[lane])}
            for lane in lane_progress
        }
        push_distances = defaultdict(list)

        def observe(state):
            nonlocal evidence_revision
            if evidence is not None and state.phase in (GamePhase.LEVEL_UP, GamePhase.PLANNING):
                evidence_revision += 1
                evidence.observe(state, evidence_revision)
            if not policy_pairs:
                return
            turn = (state.round, state.turn)
            for lane, history in lane_progress.items():
                position = (state.wave_counters[lane], state.battle_zones.get(lane))
                before = history[-1][2] if history else None
                if not history or history[-1][1:] != position:
                    history.append((turn, *position))
                zone = state.board.zones.get(position[1])
                order = lane_order[lane]
                if before != position[1] and before in order and position[1] in order and zone:
                    side = "RED" if order[position[1]] > order[before] else "BLUE"
                    targets = [
                        spawn.location for spawn in zone.spawn_points
                        if spawn.is_minion_spawn and spawn.team.value != side
                    ] or zone.hexes
                    heroes = next(team.heroes for color, team in state.teams.items() if color.value == side)
                    for hero in heroes:
                        distance = min(
                            (hex.distance(target) for hex in state.get_positions(hero.id) for target in targets),
                            default=None,
                        )
                        push_distances[lane, side].append(distance)
                if state.phase != GamePhase.PLANNING or zone is None or turn in lane_presence[lane]:
                    continue
                lane_presence[lane][turn] = {
                    color.value: (
                        sum(any(hex in zone.hexes for hex in state.get_positions(hero.id))
                            for hero in team.heroes),
                        sum(state.get_position(minion.id) in zone.hexes for minion in team.minions),
                    )
                    for color, team in state.teams.items()
                }

        defeats = Counter()
        minion_teams = {
            minion.id: color.value
            for color, team in root.teams.items()
            for minion in team.minions
        }

        def count_defeats(state, events):
            for event in events:
                if event.event_type.value != "UNIT_DEFEATED":
                    continue
                hero = state.get_hero(event.target_id)
                if hero:
                    defeats[(hero.team.value, "hero")] += 1
                elif event.target_id in minion_teams:
                    defeats[(minion_teams[event.target_id], "minion")] += 1
                else:
                    defeats[("unknown", "unit")] += 1

        replay = trace_policy(
            map_id, result["trace"], observe=observe if policy_pairs or evidence is not None else None,
        )
        final, count = run(
            root, turns, policy=replay, exact_planning=True,
            observe_events=count_defeats if policy_pairs else None,
        )
        assert count == len(result["trace"])
        if evidence is not None:
            if final.phase == GamePhase.PLANNING:
                evidence.observe(final, evidence_revision + 1)

            def public_batches(batches):
                # Revisions are local to each engine's advancement loop.
                return sorted(
                    ({key: value for key, value in row.items() if key != "revision"} for row in batches),
                    key=lambda row: (row["hero_id"], row["after"]["level"]),
                )

            actual_batches = public_batches(result["public_upgrade_batches"])
            expected_batches = public_batches(evidence.batches)
            assert actual_batches == expected_batches, (map_id, seed, actual_batches, expected_batches)
        expected = normalized_state(final.model_dump(mode="json"))
        actual = normalized_state(result["state"])
        if expected != actual:
            # A capped match can retain generated effect steps. Compare typed
            # identities without treating Python's allocation IDs as game data.
            from object_execution_traces import ObjectTraceNormalizer

            final_envelope, final_objects = export_state_with_shared_objects(final)
            bindings = final_envelope["object_bindings"]
            reference_normalizer = ObjectTraceNormalizer()
            reference_normalizer.bind_originals(envelope["object_bindings"], original_objects)
            expected = reference_normalizer.record(
                dict(semantic_state=expected, object_bindings=bindings, request=None),
                final_objects,
            )["semantic_state"]
            actual = ObjectTraceNormalizer().record(
                dict(semantic_state=actual, object_bindings=bindings, request=None),
            )["semantic_state"]
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
        if final.phase == GamePhase.GAME_OVER:
            print(f"  Victory condition: {final.victory_condition}")
        if policy_pairs:
            turns_elapsed = (final.round - root.round) * 4 + final.turn - root.turn
            print(f"  Turn index advanced {turns_elapsed}; defeats suffered: "
                  f"RED {defeats['RED', 'hero']} heroes, {defeats['RED', 'minion']} minions; "
                  f"BLUE {defeats['BLUE', 'hero']} heroes, {defeats['BLUE', 'minion']} minions"
                  + (f"; {defeats['unknown', 'unit']} unclassified" if defeats['unknown', 'unit'] else ""))
            observe(final)
            for lane, history in lane_progress.items():
                order = lane_order[lane]
                red = blue = unknown = 0
                for (_, _, before), (_, _, after) in zip(history, history[1:]):
                    if before == after:
                        continue
                    if before not in order or after not in order:
                        unknown += 1
                    elif order[after] > order[before]:
                        red += 1
                    else:
                        blue += 1
                shifts = red + blue + unknown
                direction = f"RED {red}, BLUE {blue}"
                if unknown:
                    direction += f", unknown {unknown}"
                print(f"  {lane}: {shifts} observed zone shifts ({direction}), "
                      f"{history[-1][1]} waves remaining")
                for side in ("RED", "BLUE"):
                    distances = push_distances[lane, side]
                    if distances:
                        ready = sum(distance is not None and distance <= 1 for distance in distances)
                        off_board = sum(distance is None for distance in distances)
                        print(f"    {side} at first observed post-push state: "
                              f"{ready}/{len(distances)} heroes within one hex, {off_board} off board")
                if final.phase != GamePhase.GAME_OVER:
                    print(f"    Lane history (round/turn, waves, zone): {history}")
                    samples = lane_presence[lane].values()
                    if samples:
                        for color in ("RED", "BLUE"):
                            present = sum(row[color][0] > 0 for row in samples)
                            minimum = min(row[color][1] for row in samples)
                            print(f"    {color}: heroes present at {present}/{len(samples)} "
                                  f"planning boundaries, minimum {minimum} minions in zone")
        for team, (policy, stats) in enumerate(zip(policies, result["policies"], strict=True)):
            if not policy["depth"]:
                continue
            print(
                f"  {('RED', 'BLUE')[team]} search used {stats['fallback']} fallbacks across "
                f"{stats['planning']} planning and {stats['resolution']} resolution decisions "
                f"({stats.get('forced', 0)} forced)"
            )
            if "planning_best_so_far" in stats:
                print(
                    f"    scored best-so-far: {stats['planning_best_so_far']} planning, "
                    f"{stats['resolution_best_so_far']} resolution"
                )
            if stats.get("planning_seed_nodes"):
                print(f"    planning seed pass: {stats['planning_seed_nodes']} nodes, "
                      f"{stats['planning_seed_choices']} choices retained")
                if "planning_seed_scored" in stats:
                    print(f"    seed pass scored {stats['planning_seed_scored']} decisions, "
                          f"deep search agreed on {stats['planning_seed_deep_agreement']}")
            if stats.get("planning_shallow_scores"):
                print(f"    shallow scored planning: {stats['planning_shallow_scores']}")
            if "planning_nodes" in stats:
                print(f"    search nodes: {stats['planning_nodes']} planning, "
                      f"{stats['resolution_nodes']} resolution")
                if "planning_greedy_nodes" in stats:
                    print(f"    planning nodes by stage: {stats['planning_greedy_nodes']} greedy, "
                          f"{stats['planning_probe_nodes']} reply probe, {stats['planning_shallow_nodes']} shallow retry")
                print(f"    planning rows: {stats['planning_completed_rows']} completed, "
                      f"{stats['planning_pruned_rows']} pruned, "
                      f"{stats['planning_total_rows']} total")
            if policy.get("greedy_card_order"):
                print(f"    greedy scores: {stats['planning_greedy_card_scores']} across "
                      f"{stats['planning_with_greedy_scores']}/{stats['planning']} planning decisions")
                print(f"    forecast matched {stats['planning_forecast_matches']}/"
                      f"{stats['planning_forecast_checks']} simulated later commitments, "
                      f"changed {stats['planning_forecast_changes']}")
            for phase in ("planning", "resolution"):
                elapsed = sorted(stats.get(f"{phase}_micros", ()))
                if elapsed:
                    assert len(elapsed) == stats[phase]
                    p95 = elapsed[ceil(len(elapsed) * 0.95) - 1]
                    print(f"    {phase} selection p50 {median(elapsed) / 1000:.2f} ms, "
                          f"p95 {p95 / 1000:.2f} ms")
                reasons = stats.get(f"{phase}_fallbacks", {})
                for reason, frequency in sorted(reasons.items(), key=lambda row: (-row[1], row[0])):
                    print(f"    {frequency} × {phase}: {reason}")
            for index, reasons in stats.get("planning_fallback_trace", {}).items():
                for reason in reasons:
                    print(f"    replay decision {int(index) + 1}: {reason}")
            for index, reason in stats.get("resolution_fallback_trace", {}).items():
                print(f"    replay decision {int(index) + 1}: {reason}")
        results.append(result)
    return results


def verify_work_budget(worker, map_id, root):
    policy = dict(
        depth=2, max_nodes=8192, resolution_max_nodes=2048, work_budget="nodes_v1",
        rollout_turns=2, evaluation="lane_pressure_v3", greedy_card_order=True,
        planning_workers=1, sampled_worlds=2,
    )
    for limits in (policy, dict(policy, max_nodes=1, resolution_max_nodes=1)):
        pair = (limits, limits)
        first, repeated = verify_rust_match_replay(
            worker, map_id, root, 4, seed=39, policy_pairs=(pair, pair),
        )
        assert first == repeated, (map_id, limits["max_nodes"])
        assert first["work_budget"] == "nodes_v1"
        for stats in first["policies"]:
            assert 0 < stats["planning_nodes"] <= stats["planning"] * limits["max_nodes"]
            assert 0 < stats["resolution_nodes"] <= (
                (stats["resolution"] - stats["forced"]) * limits["resolution_max_nodes"]
            )
            assert "planning_micros" not in stats and "resolution_micros" not in stats
            if limits["max_nodes"] == 1:
                assert stats["planning_fallbacks"]
        print(f"{map_id} nodes_v1 {limits['max_nodes']}/{limits['resolution_max_nodes']} "
              f"deterministic result SHA-256: {digest(first)}")


def verify_strength_trial(worker, map_id, root, seed, decision_ms):
    common = dict(depth=1, max_nodes=4096, time_ms=decision_ms, evaluation="lab_attack_range_v1")
    one_turn = dict(common, rollout_turns=1)
    two_turn = dict(common, rollout_turns=2)
    results = verify_rust_match_replay(
        worker, map_id, root, 64, seed=seed, decision_ms=decision_ms,
        policy_pairs=((two_turn, one_turn), (one_turn, two_turn)),
    )
    outcomes = Counter()
    for result, two_turn_side in zip(results, ("RED", "BLUE"), strict=True):
        winner = result["winner_team"]
        if result["phase"] != "GAME_OVER":
            outcome = "unfinished"
        elif winner is None:
            outcome = "draw"
        else:
            outcome = "two turns won" if winner == two_turn_side else "one turn won"
        print(f"{map_id} seed {seed}, two turns on {two_turn_side}: {outcome}; lives {result['lives']}")
        if outcome == "unfinished":
            print(f"  remaining waves {result['state']['wave_counters']}; "
                  f"active zones {result['state']['battle_zones']}")
        outcomes[outcome] += 1
    return results, outcomes


def verify_lane_pressure_trial(
    worker, maps, decision_ms, evaluator, baseline_evaluator, held_out, planning_workers,
    *, attack_targets=False, greedy_card_order=False, greedy_branching_trial=False, planning_branching_only=False, max_nodes=4096, reply_cap=4,
):
    baseline = dict(
        depth=1, max_nodes=max_nodes, time_ms=decision_ms, rollout_turns=2,
        evaluation=baseline_evaluator, planning_workers=planning_workers, reply_cap=reply_cap,
    )
    candidate = dict(baseline, evaluation=evaluator)
    candidate_label = evaluator
    if attack_targets:
        candidate["opponent_policy"] = "adversarial_attack_targets"
        candidate_label += " + attack target extension"
    if greedy_card_order:
        candidate["greedy_card_order"] = True
        candidate_label += " + four-turn greedy card ordering"
    baseline_label = baseline_evaluator
    if greedy_branching_trial:
        baseline.update(greedy_card_order=True, fixed_future_planning=True)
        baseline_label += " + greedy forecasts with fixed future planning"
        candidate_label += " + later planning branching"
    if planning_branching_only:
        candidate["branch_future_planning"] = True
        candidate_label += " + later planning branching"
    cases = (
        (
            ("forgotten_island", 311, ["Arien", "Misa"], ["Bain", "Hanu"]),
            ("magma_and_ice", 463, ["Wasp", "Brogan"], ["Sabina", "Tigerclaw"]),
            ("narrow_passages", 463, ["Wasp", "Brogan"], ["Sabina", "Tigerclaw"]),
            ("vexing_cliffs_v2", 311, ["Arien", "Misa"], ["Bain", "Hanu"]),
        )
        if held_out else
        (
            ("across_the_river", 137, ["Arien", "Hanu"], ["Misa", "Brogan"]),
            ("across_the_river", 251, ["Sabina", "Bain"], ["Tigerclaw", "Wasp"]),
        )
    )
    outcomes = Counter()
    fallbacks = choices = 0
    for map_id, seed, red, blue in cases:
        root = GameSetup.create_game(
            str(maps / f"{map_id}.json"), red, blue, seed=seed, game_type="QUICK",
        )
        results = verify_rust_match_replay(
            worker, map_id, root, 64, seed=seed, decision_ms=decision_ms,
            policy_pairs=((candidate, baseline), (baseline, candidate)),
        )
        for index, (result, side) in enumerate(zip(results, ("RED", "BLUE"), strict=True)):
            winner = result["winner_team"]
            if result["phase"] != "GAME_OVER":
                outcomes["unfinished"] += 1
            elif winner is None:
                outcomes["draw"] += 1
            else:
                outcomes["candidate wins" if winner == side else "baseline wins"] += 1
            stats = result["policies"][index]
            fallbacks += stats["fallback"]
            choices += stats["planning"] + stats["resolution"] - stats["forced"]
            print(f"{map_id} seed {seed}, {candidate_label} on {side}: "
                  f"{winner or result['phase']}; lives {result['lives']}")
    print(f"{candidate_label} versus {baseline_label}: {dict(outcomes)}; "
          f"candidate fallbacks {fallbacks}/{choices}")
    if held_out:
        passed = outcomes["candidate wins"] >= 5 and outcomes["unfinished"] <= 1 and fallbacks / choices < 0.02
        print(f"Held-out evaluator threshold {'met' if passed else 'missed'}")


STRENGTH_SCREEN_STARTS = (
    ("forgotten_island", 71, ["Arien", "Razzle"], ["Emmitt", "Ignatia"]),
    ("across_the_river", 109, ["Takahide", "Bain"], ["Swift", "Wasp"]),
    ("magma_and_ice", 173, ["Min", "Hanu"], ["Gydion", "Silverarrow"]),
    ("narrow_passages", 223, ["Misa", "Tali"], ["Brogan", "Sabina"]),
)


LANE_TIMING_HELD_OUT_STARTS = (
    ("forgotten_island", 571, ["Hanu", "Rowenna"], ["Min", "Xargatha"]),
    ("across_the_river", 607, ["Ignatia", "Sabina"], ["Tali", "Garrus"]),
    ("magma_and_ice", 631, ["Swift", "Takahide"], ["Bain", "Wuk"]),
    ("narrow_passages", 661, ["Emmitt", "Cordelia"], ["Misa", "Brynn"]),
    ("vexing_cliffs", 701, ["Gydion", "Wasp"], ["Brogan", "Dodger"]),
    ("vexing_cliffs_v2", 727, ["Arien", "Razzle"], ["Cutter", "Tigerclaw"]),
    ("across_the_river", 757, ["Whisper", "Mortimer"], ["Hanu", "Silverarrow"]),
    ("magma_and_ice", 797, ["NebKher", "Snorri"], ["Tali", "Trinkets"]),
)


def paired_strength_p_value(margins):
    """One-sided sign-flip probability, treating each starting position as one unit."""
    observed = sum(margins)
    distribution = Counter({0: 1})
    for margin in margins:
        flipped = Counter()
        for total, count in distribution.items():
            flipped[total + margin] += count
            flipped[total - margin] += count
        distribution = flipped
    return sum(count for total, count in distribution.items() if total >= observed) / distribution.total()


STRENGTH_MIDGAME_MAPS = ("forgotten_island", "magma_and_ice")


def strength_start_positions(maps, selected_map, *, held_out, midgame, game_type):
    if midgame:
        from simulation_bench import read_case, read_index, workloads

        corpus = ROOT / "rust/tests/fixtures/complete-games-hanu.json"
        index, _ = read_index(corpus)
        for entry in index["cases"]:
            map_id = next(name for name in STRENGTH_MIDGAME_MAPS if name in entry["name"])
            if selected_map is not None and map_id != selected_map:
                continue
            workload = next(workloads(read_case(corpus, entry), midgame=True))
            assert workload["start"] == (3, 1), workload["source_game"]
            yield map_id, 29, workload["state"]
        return

    starts = LANE_TIMING_HELD_OUT_STARTS if held_out else STRENGTH_SCREEN_STARTS
    for map_id, seed, red, blue in starts:
        if selected_map is not None and map_id != selected_map:
            continue
        root = GameSetup.create_game(
            str(maps / f"{map_id}.json"), red, blue, seed=seed, game_type=game_type,
        )
        yield map_id, seed, root


def verify_strength_baseline(
    worker, maps, decision_ms, *, trial=None, selected_map=None, held_out=False,
    candidate_rollout_turns=1, midgame=False, game_type="QUICK",
):
    policy = dict(
        depth=2, max_nodes=131072, time_ms=decision_ms, rollout_turns=2,
        evaluation="lane_pressure_v3", planning_workers=2, reply_cap=4,
        greedy_card_order=True, forecast_node_share="quarter",
    )
    label = "Baseline self-play"
    policy_names = ("quarter",)
    pairs = ((policy, policy),)
    if trial == "forecast_share":
        label = "Forecast-share trial"
        policy_names = ("eighth", "quarter")
        candidate = dict(policy, forecast_node_share="eighth")
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "forecast_value":
        label = "Fixed-opponent forecast trial"
        policy_names = ("four-turn-forecast", "lowest-defense")
        control = dict(policy, forecast_node_share="zero")
        pairs = ((policy, control), (control, policy))
    elif trial == "followup_width":
        label = "Coordinated follow-up width trial"
        policy_names = ("balanced-four", "legacy-two")
        candidate = dict(policy, coordinated_planning=True)
        control = dict(candidate, legacy_fixed_followup=True)
        pairs = ((candidate, control), (control, candidate))
    elif trial == "horizon":
        label = "Search horizon trial"
        candidate_name = "one-turn" if candidate_rollout_turns == 1 else "three-turn"
        policy_names = (candidate_name, "two-turn")
        candidate = dict(policy, rollout_turns=candidate_rollout_turns)
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "lane_timing":
        label = "Held-out lane timing trial" if held_out else "Lane timing trial"
        policy_names = ("lane_pressure_v4", policy["evaluation"])
        candidate = dict(policy, evaluation=policy_names[0])
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "node_budget":
        label = "Search node-budget trial"
        policy_names = ("262144-nodes", "131072-nodes")
        candidate = dict(policy, max_nodes=262144)
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "decision_depth":
        label = "Decision depth trial"
        policy_names = ("depth-three", "depth-two")
        candidate = dict(policy, depth=3)
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "tactical_forecast":
        label = "Tactical forecast trial"
        policy_names = ("tactical-forecast", "hold-forecast")
        candidate = dict(policy, tactical_forecast=True)
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "planning_opposition":
        label = "Planning opposition trial"
        policy_names = ("adversarial-planning", "forecast-opposition")
        candidate = dict(policy, branch_future_planning=True)
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "attack_extension":
        label = "Attack-target extension trial"
        policy_names = ("attack-extension", "fixed-depth")
        candidate = dict(policy, opponent_policy="adversarial_attack_targets")
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "movement_extension":
        label = "Movement-target extension trial"
        policy_names = ("movement-extension", "fixed-depth")
        candidate = dict(policy, opponent_policy="adversarial_movement_targets")
        pairs = ((candidate, policy), (policy, candidate))
    elif trial == "reply_width":
        label = "Planning reply width trial"
        policy_names = ("eight-replies", "four-replies")
        # Keep later planning fixed while letting the root compare opponent replies.
        control = dict(policy, fixed_future_planning=True)
        candidate = dict(control, reply_cap=8)
        pairs = ((candidate, control), (control, candidate))
    elif trial == "world_count":
        label = "Sampled-world count trial"
        policy_names = ("four-worlds", "two-worlds")
        candidate = dict(policy, sampled_worlds=4)
        control = dict(policy, sampled_worlds=2)
        pairs = ((candidate, control), (control, candidate))
    elif trial == "planning_seed":
        label = "Planning seed-pass trial"
        policy_names = ("seed-pass", "direct-deep")
        candidate = dict(policy, planning_seed_pass=True)
        pairs = ((candidate, policy), (policy, candidate))
    print(f"{label} policies: {json.dumps(dict(zip(policy_names, pairs[0])), sort_keys=True)}")
    outcomes = Counter()
    pair_margins = []
    incomplete_pairs = 0
    decisions = 0
    fallbacks = Counter()
    shallow_scores = Counter()
    non_forced = Counter()
    timings = defaultdict(list)
    search_work = defaultdict(Counter)
    for map_id, seed, root in strength_start_positions(
        maps, selected_map, held_out=held_out, midgame=midgame, game_type=game_type,
    ):
        red, blue = ([hero.name for hero in root.teams[side].heroes] for side in ("RED", "BLUE"))
        print(f"{label} {map_id}, seed {seed}, Red {red}, Blue {blue}", flush=True)
        results = verify_rust_match_replay(
            worker, map_id, root, 64, seed=seed, decision_ms=decision_ms,
            policy_pairs=pairs,
        )
        assert len(results) == len(pairs), (map_id, seed, len(results))
        pair = []
        for index, result in enumerate(results):
            winner = result["winner_team"]
            if result["phase"] != "GAME_OVER":
                outcome = "unfinished"
            elif winner is None:
                outcome = "draw"
            elif trial:
                outcome = policy_names[int(winner != ("RED", "BLUE")[index])]
            else:
                outcome = winner
            pair.append(outcome)
            outcomes[outcome] += 1
            decisions += len(result["trace"])
            for side, stats in enumerate(result["policies"]):
                share = policy_names[int(bool(trial) and side != index)]
                fallbacks[share] += stats["fallback"]
                shallow_scores[share] += stats.get("planning_shallow_scores", 0)
                non_forced[share] += stats["planning"] + stats["resolution"] - stats["forced"]
                for key in (
                    "planning_nodes", "resolution_nodes", "planning_completed_rows",
                    "planning_pruned_rows", "planning_total_rows",
                    "planning_greedy_nodes", "planning_greedy_tactical_probes",
                    "planning_probe_nodes", "planning_shallow_nodes",
                    "planning_seed_nodes", "planning_seed_scored", "planning_seed_choices",
                    "planning_seed_deep_agreement",
                ):
                    if key in stats:
                        search_work[share][key] += stats[key]
                for phase in ("planning", "resolution"):
                    timings[share, phase].extend(stats.get(f"{phase}_micros", ()))
        if trial:
            print(f"{map_id}, seed {seed}, {policy_names[0]} as Red then Blue: {pair}")
            if "unfinished" in pair:
                incomplete_pairs += 1
            else:
                pair_margins.append(pair.count(policy_names[0]) - pair.count(policy_names[1]))
    print(f"{label}: {dict(outcomes)}; {decisions} decisions")
    paired_p = paired_strength_p_value(pair_margins) if pair_margins else None
    if trial:
        print(f"  Completed {len(pair_margins)}/{len(pair_margins) + incomplete_pairs} side-swapped pairs")
        if paired_p is not None:
            candidate_points = len(pair_margins) + sum(pair_margins) / 2
            print(f"  Candidate game points from completed pairs: {candidate_points:g}/{2 * len(pair_margins)}")
            print(f"  Paired sign-flip p for a {policy_names[0]} advantage: "
                  f"{paired_p:.4f} ({len(pair_margins)} starts)")
        else:
            print("  Paired strength estimate unavailable without a completed pair")
    for share in sorted(non_forced):
        print(f"  {share}: {fallbacks[share]}/{non_forced[share]} unscored fallbacks")
        if shallow_scores[share]:
            print(f"    shallow scored planning: {shallow_scores[share]}")
        work = search_work[share]
        if work:
            print(f"    search nodes: {work['planning_nodes']} planning, "
                  f"{work['resolution_nodes']} resolution")
            if "planning_greedy_nodes" in work:
                print(f"    planning nodes by stage: {work['planning_greedy_nodes']} greedy, "
                      f"{work['planning_probe_nodes']} reply probe, {work['planning_shallow_nodes']} shallow retry")
            if work["planning_greedy_tactical_probes"]:
                print(f"    tactical probes in scored greedy forecasts: "
                      f"{work['planning_greedy_tactical_probes']}")
            if work["planning_seed_nodes"]:
                print(f"    planning seed pass: {work['planning_seed_nodes']} nodes, "
                      f"{work['planning_seed_scored']} decisions scored, "
                      f"{work['planning_seed_deep_agreement']} deep agreements, "
                      f"{work['planning_seed_choices']} choices retained")
            print(f"    planning rows: {work['planning_completed_rows']} completed, "
                  f"{work['planning_pruned_rows']} pruned, {work['planning_total_rows']} total")
        for phase in ("planning", "resolution"):
            elapsed = sorted(timings[share, phase])
            if elapsed:
                p95 = elapsed[ceil(len(elapsed) * 0.95) - 1]
                print(f"    {phase} p50 {median(elapsed) / 1000:.2f} ms, "
                      f"p95 {p95 / 1000:.2f} ms ({len(elapsed)} selections)")

    if held_out:
        candidate = policy_names[0]
        latency_ok = all(
            (samples := sorted(timings[candidate, phase]))
            and samples[ceil(len(samples) * 0.95) - 1] <= 5_100_000
            for phase in ("planning", "resolution")
        )
        checks = {
            "at least 11 candidate wins": outcomes[candidate] >= 11,
            "at most two unfinished games": outcomes["unfinished"] <= 2,
            "paired sign-flip p at most 0.05": paired_p is not None and paired_p <= 0.05,
            "candidate fallback rate below 2%": (
                non_forced[candidate] > 0 and fallbacks[candidate] / non_forced[candidate] < 0.02
            ),
            "candidate planning and resolution p95 at most 5.1 s": latency_ok,
        }
        failed = [label for label, passed in checks.items() if not passed]
        print("Held-out lane timing threshold " + (f"missed ({', '.join(failed)})" if failed else "met"))


def verify_held_out_strength(worker, maps):
    outcomes = Counter()
    fallbacks = [0, 0]
    choices = [0, 0]
    for map_id in ("across_the_river", "narrow_passages", "vexing_cliffs", "vexing_cliffs_v2"):
        for seed, red, blue in (
            (137, ["Arien", "Hanu"], ["Misa", "Brogan"]),
            (251, ["Sabina", "Bain"], ["Tigerclaw", "Wasp"]),
        ):
            root = GameSetup.create_game(
                str(maps / f"{map_id}.json"), red, blue, seed=seed, game_type="QUICK",
            )
            results, pair_outcomes = verify_strength_trial(worker, map_id, root, seed, 100)
            outcomes.update(pair_outcomes)
            for two_turn_side, result in enumerate(results):
                for side, stats in enumerate(result["policies"]):
                    index = int(side == two_turn_side)
                    fallbacks[index] += stats["fallback"]
                    choices[index] += stats["planning"] + stats["resolution"] - stats["forced"]
    rates = [count / total for count, total in zip(fallbacks, choices, strict=True)]
    passed = outcomes["two turns won"] >= 10 and outcomes["unfinished"] <= 2 and rates[1] < 0.02
    print(f"Held-out horizon comparison: {dict(outcomes)}; "
          f"fallbacks one turn {fallbacks[0]}/{choices[0]}, two turns {fallbacks[1]}/{choices[1]}; "
          f"two-turn threshold {'met' if passed else 'missed'}")



def verify_greedy_search(worker, map_id, root, decision_ms, *, comparison="cards"):
    envelope, _ = export_state_with_shared_objects(root)
    team = next(iter(root.teams.values()))
    target_reuses = 0
    for controlled in ([team.heroes[0].id], [hero.id for hero in team.heroes]):
        for baseline in (True, False):
            started = perf_counter()
            result = worker.request(
                "search_greedy", envelope=envelope, player_id=controlled[0],
                controlled_ids=controlled, turns=2, seed=17, max_nodes=4096,
                time_ms=decision_ms, fixed_card_order=baseline and comparison == "cards",
                positional_ordering=not (baseline and comparison == "position"),
                position_first=comparison == "routes" and not baseline,
                tactical_replies=comparison == "tactics" and not baseline,
                timeout=decision_ms / 1_000 + 5,
            )
            elapsed = (perf_counter() - started) * 1_000
            target_reuses += result["tactical_target_reuses"]
            replay = result["replay"]
            assert replay and result["completed_sequences"] > 0, (map_id, controlled, result)
            final, count = run(root, 2, policy=trace_policy(map_id, replay["trace"]))
            assert count == len(replay["trace"])
            expected = normalized_state(final.model_dump(mode="json"))
            actual = normalized_state(replay["state"])
            assert expected == actual, describe_state_difference(expected, actual)
            if comparison == "tactics":
                label = "hold" if baseline else "tactical"
            elif comparison == "routes":
                label = "card first" if baseline else "position first"
            elif comparison == "position":
                label = "unordered" if baseline else "position ordered"
            else:
                label = "fixed" if baseline else "search"
            print(f"{map_id} {len(controlled)} controlled {label}: Python validated {count} steps, "
                  f"score {result['score']}, {result['completed_sequences']} completed sequences, "
                  f"{result['nodes']} nodes, {result['position_routes']} routes, "
                  f"{result['position_groups']} groups, {result['tactical_probes']} tactical probes, "
                  f"{result['tactical_target_reuses']} target reuses, "
                  f"{result['resumed_position_routes']} route resumes, "
                  f"{elapsed:.1f} ms")
    return target_reuses


def verify_close_contact_tactics(worker, opening, decision_ms):
    from goa2.domain.models.enums import ActionType

    root = deepcopy(opening)
    friendly, enemy = list(root.teams.values())
    target = friendly.heroes[0]
    attacker = enemy.heroes[0]
    start = root.get_position(target.id)
    adjacent = next(
        hex for hex in root.board.get_neighbors(start)
        if not root.board.tiles[hex].occupant_id and not root.board.tiles[hex].is_terrain
    )
    root.move_unit(attacker.id, adjacent)
    attack = max(
        (card for card in attacker.hand if card.primary_action == ActionType.ATTACK
         or ActionType.ATTACK in card.secondary_actions),
        key=lambda card: card.initiative,
    )
    for team in root.teams.values():
        for hero in team.heroes:
            card = attack if hero.id == attacker.id else min(hero.hand, key=lambda card: card.initiative)
            commit_card(root, hero.id, card)
    reuses = verify_greedy_search(
        worker, "forgotten_island_close_contact", root, decision_ms, comparison="tactics",
    )
    assert reuses > 0, "close-contact search never reused a verified tactical target"


def compact_greedy_board(opening):
    from goa2.domain.board import Board

    state = deepcopy(opening)
    board = state.board
    keep = {spawn.location for spawn in board.spawn_points}
    for zone in board.zones.values():
        keep.update(spawn.location for spawn in zone.spawn_points)
    for position in state.entity_locations.values():
        keep.add(position)
        keep.update(position.neighbors())
    state.board = Board(
        map_id="greedy_compact_fixture",
        zones={
            name: zone.model_copy(update={"hexes": zone.hexes & keep})
            for name, zone in board.zones.items()
        },
        spawn_points=board.spawn_points,
        tiles={hex: tile for hex, tile in board.tiles.items() if hex in keep},
        lanes=board.lanes,
        starting_battle_zones=board.starting_battle_zones,
    )
    assert len(state.board.tiles) < len(board.tiles)
    return state


def verify_greedy_upgrade_search(
    worker, map_id, opening, decision_ms, *, profile=False, time_limited=False, level_eight=False,
):
    root, _ = run(opening, 3)
    hero = next(iter(root.teams.values())).heroes[0]
    hero.gold += 28 if level_eight else 3
    envelope, _ = export_state_with_shared_objects(root)
    for defer, positions, turns in ((False, False, 2), (True, False, 2), (True, True, 2), (True, True, 3)):
        max_nodes = 65_536 if time_limited else (16_384 if turns == 3 else 4_096)
        started = perf_counter()
        result = worker.request(
            "search_greedy", envelope=envelope, player_id=hero.id, turns=turns,
            seed=17, max_nodes=max_nodes, time_ms=decision_ms,
            defer_upgrades=defer, position_first=positions, profile=profile,
            timeout=decision_ms / 1_000 + 5,
        )
        elapsed = (perf_counter() - started) * 1_000
        replay = result["replay"]
        assert replay and result["completed_sequences"] > 0, (map_id, defer, result)
        upgrades = [
            selection for row in replay["trace"]
            if isinstance(selection := row.get("selection"), dict)
            and selection.get("hero_id") == hero.id and "card_id" in selection
        ]
        assert upgrades, replay["trace"]
        if level_eight:
            assert len(upgrades) == 6, upgrades
        final, count = run(root, turns, policy=trace_policy(map_id, replay["trace"]))
        assert count == len(replay["trace"])
        expected = normalized_state(final.model_dump(mode="json"))
        actual = normalized_state(replay["state"])
        assert expected == actual, describe_state_difference(expected, actual)
        if level_eight:
            assert final.get_hero(hero.id).level == 8
        if positions and not time_limited and not level_eight:
            assert result["upgraded_position_groups"] > 0, (map_id, result)
            assert result["shared_card_moves"] > 0, (map_id, result)
            assert result["shared_upgrade_decisions"] > 0, (map_id, result)
        if turns == 3 and not time_limited and not level_eight:
            assert result["shared_upgrade_commitments"] > 0, (map_id, result)
        label = "position-first" if positions else "deferred" if defer else "eager"
        print(f"{map_id} {label} upgrades: Python validated {count} steps, "
              f"score {result['score']}, {result['completed_sequences']} completed sequences, "
              f"{result['nodes']} nodes, {result['upgraded_position_groups']} upgraded groups, "
              f"{result['upgrade_discovery_steps']} discovery steps, "
              f"{result['shared_upgrade_decisions']} shared decisions, "
              f"{result['shared_upgrade_commitments']} shared commitments, "
              f"{result['shared_card_moves']} cross-card moves, {elapsed:.1f} ms")
        if profile:
            stats = result["profile"]
            assert stats["search_ns"] > 0 and stats["first_plan_ns"] is not None
            print(f"{map_id} {label} {turns} turns profile {json.dumps(stats, sort_keys=True)}")



def verify_greedy_movement_cache(worker, map_id, opening, decision_ms):
    root, _ = run(opening, 3)
    hero = next(iter(root.teams.values())).heroes[0]
    hero.gold += 3
    envelope, _ = export_state_with_shared_objects(root)
    baseline = None
    for reuse in (False, True):
        times = []
        for _ in range(5):
            started = perf_counter()
            result = worker.request(
                "search_greedy", envelope=envelope, player_id=hero.id, turns=2,
                seed=17, max_nodes=4096, time_ms=decision_ms,
                position_first=True, reuse_movement=reuse,
                timeout=decision_ms / 1_000 + 5,
            )
            times.append((perf_counter() - started) * 1_000)
            comparable = {key: value for key, value in result.items()
                          if key not in {"movement_cache_hits", "movement_cache_misses",
                                         "travel_cache_hits", "travel_cache_misses"}}
            if baseline is None:
                baseline = comparable
            assert comparable == baseline, (map_id, reuse, result["nodes"], result["score"])
        replay = result["replay"]
        assert replay
        final, count = run(root, 2, policy=trace_policy(map_id, replay["trace"]))
        assert count == len(replay["trace"])
        expected = normalized_state(final.model_dump(mode="json"))
        actual = normalized_state(replay["state"])
        assert expected == actual, describe_state_difference(expected, actual)
        if reuse:
            assert result["movement_cache_hits"] > 0, map_id
        print(f"{map_id} movement reuse={reuse}: {median(times):.1f} ms median, "
              f"{result['movement_cache_hits']} hits, {result['movement_cache_misses']} misses, "
              f"{result['travel_cache_hits']} travel hits, "
              f"{result['travel_cache_misses']} travel misses, "
              f"Python validated {count} steps")


def verify_terminal_matches(worker, decision_ms):
    from simulation_bench import PythonRunner, decode, initial_state
    from stack_traces import read_case, read_index
    from state_codec import export_state_with_instances

    fixtures = ROOT / "rust/tests/fixtures"
    for filename in ("complete-games.json", "complete-games-hanu.json"):
        path = fixtures / filename
        index, _ = read_index(path)
        for entry in index["cases"]:
            case = read_case(path, entry)
            root, _ = initial_state(case)
            runner = PythonRunner(root, export_state_with_instances(root)[1])
            operations = case["operations"]
            inputs = [i for i, op in enumerate(operations) if op["kind"] == "input"]
            runner.run(decode(operations[:inputs[-4]]))
            assert runner.request is not None, case["name"]
            verify_rust_match_replay(
                worker, case["name"], root, 4, require_game_over=True, decision_ms=decision_ms,
            )


def normalized_state(state):
    for zone in state["board"]["zones"].values():
        zone["hexes"].sort(key=lambda point: (point["q"], point["r"], point["s"]))
        zone["neighbors"].sort()
    return state


def verify_sampled_planning(worker, map_id, root, player="hero_arien"):
    from sequence_legality import python_move

    from automata.runtime.legality import check_sequences

    envelope, _ = export_state_with_shared_objects(root)
    options = dict(
        envelope=envelope,
        player_id=player,
        seed=39,
        cap=2,
        max_depth=1,
        max_nodes=512,
        time_ms=10_000,
        rollout_turns=2,
    )
    serial = worker.request("search_sampled_planning", workers=1, **options)
    parallel = worker.request("search_sampled_planning", workers=2, **options)
    case = (map_id, root.round, root.turn, player)
    for result in (serial, parallel):
        assert result["total_cells"] == 4, case
        assert result["completed_rows"] + result["pruned_rows"] == 2, case
        assert result["nodes"] <= options["max_nodes"], case
    assert serial["commitment"] == parallel["commitment"], case
    assert serial["score"] == parallel["score"], case
    card = serial["commitment"]["first_card"]
    move = python_move({"kind": "card", "player_id": player, "card": card}, envelope)
    assert check_sequences(root, [[move]]).results[0].legal is True, case
    print(f"{map_id} turn {root.turn}: {player} sampled planning matched with one and two workers")
    return serial["commitment"], serial["score"]


def verify_concealed_discard_planning(worker):
    root = setup(["Arien", "Brogan"], ["Takahide", "Sabina"])
    takahide = next(
        hero for team in root.teams.values() for hero in team.heroes
        if hero.id == "hero_takahide"
    )
    gold = next(card for card in takahide.hand if card.color.value == "GOLD")
    takahide.discard_card(gold)
    gold.is_facedown = True
    case = "forgotten_island (concealed Takahide discard)"
    players = ("hero_arien", takahide.id)
    expected = [verify_sampled_planning(worker, case, root, player) for player in players]

    # Change only the hidden identity, leaving Takahide's known hand intact.
    alternate = next(
        card for card in takahide.deck
        if card.color == gold.color and card.state.value == "DECK"
    )
    alternate.state, gold.state = gold.state, alternate.state
    alternate.is_facedown = True
    gold.is_facedown = False
    takahide.discard_pile[-1] = alternate
    actual = [verify_sampled_planning(worker, case, root, player) for player in players]
    assert actual == expected, case


def verify_planning_deadline(worker, map_id, opening, decision_ms, iterations, exporter, workers):
    from planning_validation import validate_commitments

    player = "hero_arien"
    for label, root in (("opening", opening), ("upgraded", upgraded_opening(opening))):
        for mode, bots in (
            ("true_rules", [player]),
            ("coordinated", [player, "hero_brogan"]),
        ):
            replies = []
            durations = []
            completed_rows = []
            outcomes = Counter()
            for _ in range(max(1, iterations)):
                started = perf_counter()
                envelope, _ = export_state_with_shared_objects(root, instance_exporter=exporter)
                result = worker.request(
                    "search_sampled_planning", envelope=envelope, player_id=player,
                    seed=39, cap=4, max_depth=1, max_nodes=4096, rollout_turns=2,
                    workers=workers,
                    bot_team={"mode": mode, "bot_ids": bots, "worlds": 3},
                    decision_deadline=started + decision_ms / 1_000, transport_reserve=0.015,
                )
                reply_ms = 1_000 * (perf_counter() - started)
                assert reply_ms <= decision_ms, (map_id, label, mode, reply_ms)
                replies.append(reply_ms)
                assert {row["player_id"] for row in result["commitments"]} == set(bots)
                validate_commitments(
                    root, envelope, result, player_id=player, controlled_ids=set(bots),
                    searched_revision=0, live_revision=0, require_score=False,
                )
                durations.append(1_000 * (perf_counter() - started))
                completed_rows.append(result["completed_rows"])
                evaluated_rows = result["completed_rows"] + result["pruned_rows"]
                assert 0 <= evaluated_rows <= result["total_rows"]
                if result["score"] is None:
                    assert evaluated_rows == 0
                    outcome = (
                        "shallow scored fallback" if result.get("shallow_score") is not None
                        else "unscored fallback"
                    )
                elif evaluated_rows == result["total_rows"]:
                    outcome = "matrix complete"
                else:
                    outcome = "scored best so far"
                outcomes[outcome] += 1
            replies.sort()
            durations.sort()
            p95_index = ceil(len(durations) * 0.95) - 1
            print(f"{map_id} {label} {mode}: {dict(outcomes)}; "
                  f"reply p50 {median(replies):.2f} ms, p95 {replies[p95_index]:.2f} ms, "
                  f"max {replies[-1]:.2f} ms; "
                  f"validated p50 {median(durations):.2f} ms, p95 {durations[p95_index]:.2f} ms, "
                  f"max {max(durations):.2f} ms; "
                  f"over budget {sum(elapsed > decision_ms for elapsed in durations)}; "
                  f"completed rows {min(completed_rows)}–{max(completed_rows)}")


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
    envelope, _ = export_state_with_shared_objects(root)
    for mode, bots in (("true_rules", [player]), ("coordinated", [player, ally])):
        fallback = worker.request(
            "search_sampled_planning", envelope=envelope, player_id=player,
            seed=39, cap=2, max_depth=1, max_nodes=1, time_ms=10_000, rollout_turns=2,
            bot_team={"mode": mode, "bot_ids": bots, "worlds": 2},
        )
        assert fallback["score"] is None and fallback["completed_rows"] == 0
        assert commitment_ids(fallback) == bots
        validate_commitments(
            root, envelope, fallback, player_id=player, controlled_ids=set(bots),
            searched_revision=1, live_revision=1, require_score=False,
        )
    print(f"{map_id} turn {root.turn}: scoped plans legal in Python across all four modes")


def verify_last_card_closes_second_slot(worker):
    from goa2.engine.phases import planning_open_for_second_card
    from planning_validation import validate_commitments

    opening = setup(["Arien", "Emmitt"], ["Brogan", "Tigerclaw"])
    player, ally = "hero_arien", "hero_emmitt"
    bots = [player, ally]
    for hand_size in (2, 1):
        state = deepcopy(opening)
        emmitt = state.get_hero(ally)
        emmitt.level = 8
        for card in emmitt.hand[hand_size:]:
            emmitt.discard_card(card)
        commit_card(state, ally, emmitt.hand[0])
        assert planning_open_for_second_card(state, ally) == (hand_size == 2)

        envelope, _ = export_state_with_shared_objects(state)
        for max_nodes in (1, 8192):
            result = worker.request(
                "search_sampled_planning", envelope=envelope, player_id=player,
                seed=39, cap=2, max_depth=1, max_nodes=max_nodes, time_ms=5000,
                rollout_turns=2, workers=2,
                bot_team={"mode": "coordinated", "bot_ids": bots, "worlds": 2},
            )
            validate_commitments(
                state, envelope, result, player_id=player, controlled_ids=bots,
                searched_revision=1, live_revision=1, require_score=max_nodes > 1,
            )
            expected = bots if hand_size == 2 else [player]
            assert [row["player_id"] for row in result["commitments"]] == expected
            if max_nodes == 1:
                assert result["score"] is None
    print("Python validated scored and fallback plans with an open or exhausted second slot")


def verify_scoped_worker_scaling(worker, map_id, root):
    from planning_validation import validate_commitments

    envelope, _ = export_state_with_shared_objects(root)
    player = "hero_arien"
    options = dict(
        envelope=envelope, player_id=player, seed=39, cap=4, max_depth=1,
        max_nodes=100_000, time_ms=10_000, rollout_turns=2, timeout=12,
        evaluation="lane_pressure_v3",
        bot_team={"mode": "true_rules", "bot_ids": [player], "worlds": 4},
    )
    reference = worker.request("search_sampled_planning", workers=1, **options)
    assert reference["prepared_worlds"] == 4
    assert reference["completed_rows"] + reference["pruned_rows"] == reference["total_rows"] == 4
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
            # Different batch sizes expose the cross-world bound at
            # different points. The final score and stable choice still match.
            for key in ("commitments", "score", "prepared_worlds", "total_rows", "total_cells"):
                assert actual[key] == reference[key], (map_id, root.turn, workers, key)
            assert actual["completed_rows"] + actual["pruned_rows"] == actual["total_rows"]
            assert reference["evaluated_cells"] <= actual["evaluated_cells"] <= actual["total_cells"]
            assert reference["nodes"] <= actual["nodes"] <= options["max_nodes"]
    timings = ", ".join(f"{workers}: {median(samples):.1f} ms"
                        for workers, samples in elapsed.items())
    print(f"{map_id} turn {root.turn} lane-pressure workers ({reference['nodes']} nodes): {timings}")


def verify_visible_ally_affects_plan(worker):
    from planning_validation import validate_commitments

    root = setup(["Arien", "Brogan"], ["Tigerclaw", "Sabina"])
    player, ally, enemy = "hero_arien", "hero_brogan", "hero_tigerclaw"
    start = root.get_position(ally)
    adjacent = next(
        hex for hex in root.board.get_neighbors(start)
        if not root.board.tiles[hex].occupant_id and not root.board.tiles[hex].is_terrain
    )
    root.move_unit(enemy, adjacent)

    def search(ally_index, enemy_index, mode, bots):
        state = deepcopy(root)
        for hero, index in ((ally, ally_index), (enemy, enemy_index)):
            commit_card(state, hero, state.get_hero(hero).hand[index])
        envelope, _ = export_state_with_shared_objects(state)
        result = worker.request(
            "search_sampled_planning", envelope=envelope, player_id=player,
            seed=39, cap=3, max_depth=1, max_nodes=4096, time_ms=10000,
            rollout_turns=2, evaluation="lab_attack_range_v1",
            bot_team={"mode": mode, "bot_ids": bots, "worlds": 2},
        )
        moves = validate_commitments(
            state, envelope, result, player_id=player, controlled_ids=set(bots),
            searched_revision=0, live_revision=0,
        )
        return result["score"], moves

    for mode, bots, visible in (
        ("true_rules", [player], False),
        ("coordinated_show", [player], False),
        ("coordinated_know_same_team_humans", [player], True),
        ("coordinated", [player, ally], True),
    ):
        first_score, first_moves = search(1, 0, mode, bots)
        assert search(1, 1, mode, bots) == (first_score, first_moves), mode
        other_score, other_moves = search(2, 0, mode, bots)
        if visible:
            assert other_score != first_score and other_moves != first_moves, mode
        else:
            assert (other_score, other_moves) == (first_score, first_moves), mode
    print("Close-contact planning: visible allied cards changed the plan; hidden choices did not")


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
    def require_rejection(message, **evidence):
        try:
            worker.request("search_sampled_planning", **options, **evidence)
        except RuntimeError as error:
            assert message in str(error)
        else:
            raise AssertionError(f"search accepted invalid upgrade evidence: {evidence}")

    impossible = {"level": 2, "card_items": [2, 0, 0, 0, 0, 0]}
    require_rejection("invalid public upgrade batch",
                      public_upgrade_batches=[dict(batch, after=impossible)])
    require_rejection("invalid public upgrade checkpoints",
                      upgrade_checkpoints={str(hero.id): [before, impossible]})
    require_rejection("invalid public upgrade checkpoints",
                      upgrade_checkpoints={"hero_arien": [{"level": 1, "card_items": [1, 0, 0, 0, 0, 0]}]})
    for workers in (1, 2):
        direct = worker.request(
            "search_sampled_planning", workers=workers,
            upgrade_checkpoints={str(hero.id): [before, before, after]}, **options,
        )
        replay = worker.request(
            "search_sampled_planning", workers=workers,
            public_upgrade_batches=[batch, batch], **options,
        )
        assert replay == direct
    require_rejection("upgrade checkpoints name an unknown hero",
                      upgrade_checkpoints={"typo": [before]})


def verify_upgrade_batch_sizes(worker, opening):
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
        recorded = len(recorder.batches)
        saved = json.loads(json.dumps(recorder.checkpoint(game_id)))
        prompt = process_stack(state).input_request
        if count:
            assert prompt.context["players"][hero.id]["remaining"] == count
            recorder.observe(state, revision)
            assert len(recorder.batches) == recorded
            saved = json.loads(json.dumps(recorder.checkpoint(game_id)))
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
        else:
            assert prompt is None
        resumed = PublicUpgradeRecorder.restore(saved, game_id)
        assert resumed.batches == recorder.batches
        assert state.phase == GamePhase.PLANNING and not state.upgrade_reveal_snapshot
        recorder.observe(state, revision)
        resumed.observe(state, revision)
        assert resumed.checkpoint(game_id) == recorder.checkpoint(game_id)
        return {"revision": revision, "hero_id": hero.id, "before": before, "after": checkpoint(hero)}

    for levels in ((2,), (3, 6), (8,), (7, 8)):
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
        assert latest["before"]["level"] == (1, *levels)[-2]
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
        sequence = "-to-".join(map(str, (1, *levels)))
        print(f"{state.board.map_id}: {sequence} upgrade batch replay matched direct evidence")


def verify_offline_upgrade_history(worker, map_id, opening):
    root, _ = run(opening, 3)
    assert (root.round, root.turn) == (1, 4)
    for team in root.teams.values():
        for hero in team.heroes:
            hero.gold = 0
    policy = dict(depth=1, max_nodes=64, time_ms=100, rollout_turns=2)
    for gold, turns, expected in ((6, 2, [(1, 4)]), (27, 10, [(1, 7), (7, 8)])):
        root.get_hero("hero_brogan").gold = gold
        (result,) = verify_rust_match_replay(
            worker, map_id, root, turns, seed=29, decision_ms=100,
            policy_pairs=((policy, policy),),
        )
        batches = [row for row in result["public_upgrade_batches"]
                   if row["hero_id"] == "hero_brogan"]
        assert [(row["before"]["level"], row["after"]["level"])
                for row in batches] == expected, (map_id, batches)
        if expected[-1] == (7, 8):
            assert batches[-1]["before"]["card_items"] == batches[-1]["after"]["card_items"]
        print(f"{map_id}: Python matched Brogan's offline upgrade batches {expected}")


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


def verify_sampled_color_guess(worker):
    from resolution_validation import validate_resolution
    from sequence_legality import bain_guess_traces
    from simulation_bench import PythonRunner, decode

    _, state, operations = next(bain_guess_traces())
    runner = PythonRunner(state, ())
    for command in decode(operations):
        runner.step(command)
        if runner.request and runner.request.prompt == "Guess the card's color":
            break
    prompt = runner.request
    assert prompt is not None and prompt.prompt == "Guess the card's color"
    player = prompt.player_id

    def search(root):
        envelope, _ = export_state_with_shared_objects(root)
        result = worker.request(
            "search_resolution", envelope=envelope, player_id=player,
            max_depth=1, max_nodes=512, time_ms=10000,
            resolution_sample_seed=39, resolution_sample_worlds=3,
        )
        assert result["completed_depth"] == 1
        validate_resolution(
            root, prompt, result, player_id=player, controlled_ids={player},
            searched_revision=0, live_revision=0,
        )
        return result

    expected = search(state)
    alternate = deepcopy(state)
    victim = alternate.get_hero(alternate.execution_context["guess_victim"])
    old = alternate.execution_context["chosen_card"]
    card = next(card for card in victim.hand if card.id != old)
    alternate.execution_context["chosen_card"] = card.id
    alternate.card_guess["attempts"][-1]["card_id"] = card.id
    assert search(alternate) == expected
    print("Bain color guess: hidden card substitution preserved the scored selection")


def verify_retained_short_turn(worker, opening):
    from automata.runtime.legality import DecisionMove
    from resolution_validation import validate_resolution

    movement = False
    root = None

    def capture(state, prompt, _revision):
        nonlocal movement, root
        if prompt is None:
            return None
        if movement and state.turn == 3 and prompt.request_type.value == "SELECT_HEX":
            root = deepcopy(state), prompt.model_copy(deep=True)
        movement = (
            not state.unresolved_hero_ids and prompt.request_type.value == "CHOOSE_ACTION"
            and any(option.id == "MOVEMENT" for option in prompt.options)
        )
        if movement:
            return DecisionMove("INPUT", prompt.player_id, "MOVEMENT", prompt.id)
        return None

    run(opening, 3, policy=capture)
    assert root is not None
    state, prompt = root
    assert (state.round, state.turn, prompt.player_id, len(prompt.options)) == (1, 3, "hero_sabina", 16)
    envelope, _ = export_state_with_shared_objects(state)
    options = dict(
        envelope=envelope, player_id=prompt.player_id, max_depth=1, time_ms=5000,
        resolution_sample_seed=39, resolution_sample_worlds=2, planning_sample_seed=71,
    )
    expected = worker.request("search_resolution", rollout_turns=1, max_nodes=8192, **options)
    actual = worker.request("search_resolution", rollout_turns=2, max_nodes=512, **options)
    assert expected["completed_depth"] == 1, expected
    assert (actual["stop"], actual["completed_depth"], actual["score_kind"], actual["nodes"]) == (
        "node_limit", 0, "short_turn_rollout", 512,
    ), actual
    assert (actual["decision"], actual["score"]) == (expected["decision"], expected["score"])
    validate_resolution(
        state, prompt, actual, player_id=prompt.player_id, controlled_ids={prompt.player_id},
        searched_revision=0, live_revision=0,
    )
    print("Forgotten Island: two-turn node limit retained the Python-validated one-turn move")


def verify_sampled_resolution(
    worker, map_path, opening, decision_ms, sample_count, workers, iterations, verify_deadline,
    transport_reserve_ms, instance_exporter=None,
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
        export_started = perf_counter()
        envelope, _ = export_state_with_shared_objects(root)
        export_ms = 1_000 * (perf_counter() - export_started)
        options = dict(
            envelope=envelope, player_id=player,
            max_depth=1, max_nodes=8192 if label == "fast travel" else 1024,
            rollout_turns=2,
            resolution_sample_seed=39, resolution_sample_worlds=sample_count,
            planning_sample_seed=71, upgrade_checkpoints=checkpoints or None,
        )
        modes = (1, workers) if workers > 1 else (1,)
        durations = {count: [] for count in modes}
        validated_durations = {count: [] for count in modes}
        export_durations = {count: [] for count in modes}
        outcomes = {count: Counter() for count in modes}
        result = None
        for index in range(max(1, iterations)):
            order = modes if index % 2 == 0 else reversed(modes)
            for count in order:
                started = perf_counter()
                request_options = options
                if instance_exporter is not None:
                    live_envelope, _ = export_state_with_shared_objects(
                        root, instance_exporter=instance_exporter,
                    )
                    export_durations[count].append(1_000 * (perf_counter() - started))
                    request_options = dict(options, envelope=live_envelope)
                try:
                    timing = (
                        dict(decision_deadline=started + decision_ms / 1_000,
                             transport_reserve=transport_reserve_ms / 1_000)
                        if transport_reserve_ms else dict(time_ms=decision_ms)
                    )
                    reply = worker.request(
                        "search_resolution", resolution_sample_workers=count, **request_options, **timing,
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
                        assert kind in {
                            "unscored", "immediate", "short_turn_rollout", "turn_rollout",
                        }, (map_id, label, reply)
                    validate_resolution(
                        root, prompt, reply, player_id=player, controlled_ids={player},
                        searched_revision=0, live_revision=0, require_score=False,
                    )
                    validated_durations[count].append(1_000 * (perf_counter() - started))
                    outcome = "completed" if depth else f"deadline ({kind})"
                    outcomes[count][outcome] += 1
                    continue
                if result is None:
                    result = reply
                else:
                    assert reply == result, (map_id, label, count, result, reply)
        if verify_deadline:
            for count in modes:
                samples = sorted(durations[count])
                if transport_reserve_ms:
                    assert max(samples) <= decision_ms, (map_id, label, count, samples)
                counts = ", ".join(f"{key}: {value}" for key, value in sorted(outcomes[count].items()))
                p95 = samples[ceil(len(samples) * 0.95) - 1]
                validated = sorted(validated_durations[count])
                validation_report = ""
                if validated:
                    validated_p95 = validated[ceil(len(validated) * 0.95) - 1]
                    validation_report = (
                        f"; reply + Python validation p50 {median(validated):.2f} ms, "
                        f"p95 {validated_p95:.2f} ms, max {max(validated):.2f} ms"
                    )
                export_label = (
                    f"live export p50 {median(export_durations[count]):.2f} ms (included)"
                    if instance_exporter is not None else f"root export {export_ms:.2f} ms (separate)"
                )
                print(f"{map_id} {label} {count} worker(s): {counts}; "
                      f"reply p50 {median(samples):.2f} ms, p95 {p95:.2f} ms, "
                      f"max {max(samples):.2f} ms; {export_label}"
                      f"{validation_report}")
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



def restore_sampled_cards(root, sampled):
    """Install Rust's sampled card choices using the original Python card objects."""
    from goa2.domain.models import CardState

    root = deepcopy(root)
    heroes = {hero.id: hero for team in root.teams.values() for hero in team.heroes}
    for team in sampled["teams"].values():
        for row in team["heroes"]:
            hero = heroes[row["id"]]
            cards = {card.id: card for card in hero.deck}
            assert len(cards) == len(hero.deck), hero.id
            for entry in row["deck"]:
                card = cards[entry["id"]]
                card.state = CardState(entry["state"])
                card.is_facedown = entry["is_facedown"]
                card.played_this_round = entry["played_this_round"]
            hero.hand = [cards[entry["id"]] for entry in row["hand"]]
            hero.discard_pile = [cards[entry["id"]] for entry in row["discard_pile"]]
            hero.played_cards = [
                None if entry is None else cards[entry["id"]] for entry in row["played_cards"]
            ]
    restored = normalized_state(root.model_dump(mode="json"))
    expected = normalized_state(sampled)
    assert restored == expected, describe_state_difference(expected, restored)
    return root


def verify_sampled_rollout(
    worker, map_id, root, evaluator, iterations, sample_count=1, workers=1, *, same_world=False,
):
    seeds = range(39, 39 + sample_count)
    envelope, _ = export_state_with_shared_objects(root)
    case = (map_id, root.turn)
    expected = []
    for seed in seeds:
        result = worker.request(
            "rollout_game_turns", envelope=envelope, player_id="hero_arien",
            turns=2, max_nodes=512, time_ms=10_000, capture_state=True, sample_seed=seed,
            **({"capture_sampled_root": True} if same_world else {}),
        )
        sampled = (
            restore_sampled_cards(root, result["sampled_root"]) if same_world
            else sampled_python(root, seed)
        )
        leaf, nodes = run(sampled, 2, copy_state=False)
        assert result["complete"] and result["nodes"] == nodes, (case, seed)
        actual = normalized_state(result["state"])
        reference = normalized_state(leaf.model_dump(mode="json"))
        assert actual == reference, (case, seed, describe_state_difference(reference, actual))
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


def verify_roster_sweep(
    worker, paths, turns, seed, policy, *, team_size, game_type, decision_ms, planning_workers,
):
    from goa2.data.heroes import HeroRegistry

    heroes = sorted(HeroRegistry.list_heroes(include_playtest=True))
    failures = []
    totals = Counter()
    roster_size = 2 * team_size
    for path in paths:
        for index in range(0, len(heroes), roster_size):
            roster = (heroes + heroes[:roster_size - 1])[index:index + roster_size]
            case = f"{path.stem} ({', '.join(roster)})"
            try:
                root = GameSetup.create_game(
                    str(path), roster[:team_size], roster[team_size:], seed=seed, game_type=game_type,
                )
                (result,) = verify_rust_match_replay(
                    worker, case, root, turns, sweep_policy=policy, seed=seed,
                    decision_ms=decision_ms, planning_workers=planning_workers,
                )
                totals["matches"] += 1
                totals["decisions"] += len(result["trace"])
                totals["terminal"] += result["phase"] == "GAME_OVER"
                for stats in result["policies"]:
                    for key in ("planning", "resolution", "forced", "fallback",
                                "planning_best_so_far", "resolution_best_so_far"):
                        totals[key] += stats.get(key, 0)
            except (AssertionError, RuntimeError, ValueError) as error:
                failures.append(case)
                print(f"{case}: {type(error).__name__}: {error}", flush=True)
    if failures:
        raise AssertionError(f"{len(failures)} roster/map cases failed: {', '.join(failures)}")


    print(
        f"Roster sweep replayed {totals['decisions']} decisions in {totals['matches']} matches "
        f"({totals['terminal']} reached game over)."
    )
    if policy == "search":
        print(
            f"Search returned {totals['fallback']} unscored fallbacks and scored best-so-far "
            f"on {totals['planning_best_so_far']}/{totals['planning']} planning and "
            f"{totals['resolution_best_so_far']}/{totals['resolution'] - totals['forced']} "
            "non-forced resolution decisions."
        )


def verify_recorded_rollouts(worker, evaluator, iterations, sample_count, workers, corpus, *, midgame=False):
    from simulation_bench import read_case, read_index, workloads

    index, _ = read_index(corpus)
    for entry in index["cases"]:
        for workload in workloads(read_case(corpus, entry), midgame=midgame):
            verify_sampled_rollout(
                worker, workload["name"], workload["state"], evaluator, iterations, sample_count, workers,
                same_world=midgame,
            )


def verify_diverse_rollouts(worker, evaluator, iterations, sample_count, workers):
    rosters = (
        ("across_the_river", ("Arien", "Hanu"), ("Garrus", "Tali")),
        ("narrow_passages", ("Arien", "Cutter"), ("Tigerclaw", "Sabina")),
        ("vexing_cliffs", ("Arien", "Brynn"), ("Wasp", "Widget")),
        ("vexing_cliffs_v2", ("Arien", "Bain"), ("Brogan", "Misa")),
    )
    assert worker.request("prepare_search") == {"ready": True}
    for map_id, red, blue in rosters:
        path = ROOT / f"src/goa2/data/maps/{map_id}.json"
        opening = GameSetup.create_game(str(path), list(red), list(blue), seed=73)
        midgame, _ = run(opening, 8)
        assert midgame.phase == GamePhase.PLANNING and midgame.round == 3
        for label, state in (("opening", opening), ("round-3", midgame)):
            verify_sampled_rollout(
                worker, f"{map_id}-{label}", state, evaluator, iterations,
                sample_count, workers, same_world=True,
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
    mode.add_argument("--verify-work-budget", action="store_true",
                      help="Replay fixed-node matches twice and check exact result digests")
    mode.add_argument("--replay-terminal", action="store_true",
                      help="Validate Rust-selected finishing moves from recorded late-game states")
    mode.add_argument("--roster-sweep-turns", type=int,
                      help="Replay Rust decisions for every registered hero")
    parser.add_argument("--sweep-team-size", type=int, choices=range(1, 6), default=2)
    parser.add_argument("--sweep-game-type", choices=("LONG", "QUICK"), default="LONG")
    parser.add_argument("--sweep-policy", choices=("first-legal", "sampled", "search"),
                        default="first-legal", help="Rust policy used by both teams in the sweep")
    parser.add_argument("--sweep-planning-workers", type=int, choices=(1, 2),
                        help="Parallel sampled-world workers in the search roster sweep (default: 2)")
    mode.add_argument("--diverse-rollouts", action="store_true",
                      help="Verify opening and round-three rollouts on four additional maps")
    mode.add_argument("--recorded-rollouts", action="store_true",
                      help="Validate and benchmark sampled rollouts from recorded game positions")
    mode.add_argument("--greedy-tactics", action="store_true",
                      help="Compare the greedy Hold policy with tactical enemy replies")
    mode.add_argument("--greedy-routes", action="store_true",
                      help="Compare position-first and card-first greedy search")
    mode.add_argument("--greedy-position", action="store_true",
                      help="Compare greedy search with and without positional ordering")
    mode.add_argument("--greedy-movement-cache", action="store_true",
                      help="Compare movement traversal reuse against the uncached search")
    mode.add_argument("--greedy-upgrades", action="store_true",
                      help="Compare eager and deferred greedy upgrades across a round boundary")
    parser.add_argument("--greedy-upgrade-compact", action="store_true",
                        help="Use a compact Forgotten Island rules fixture")
    parser.add_argument("--greedy-upgrade-level-eight", action="store_true",
                        help="Verify all six ordinary upgrades plus the automatic ultimate unlock")
    parser.add_argument("--greedy-upgrade-time-limited", action="store_true",
                        help="Use the same high node cap for timed upgrade comparisons")
    mode.add_argument("--greedy-search", action="store_true",
                      help="Replay and benchmark greedy plans against a fixed card order")
    mode.add_argument("--sampled-resolution", action="store_true",
                      help="Validate sampled resolution choices and two-turn scores")
    parser.add_argument("--profile-greedy", action="store_true",
                        help="Report worker phase timings for the greedy upgrade benchmark")
    parser.add_argument("--prepare-search", action="store_true",
                        help="Initialize immutable Rust rule programs before timing decisions")
    parser.add_argument("--resolution-transport-reserve-ms", type=int, default=0,
                        help="Use a caller deadline and reserve time for the worker exchange")
    parser.add_argument("--resolution-live-export", action="store_true",
                        help="Include live state export in each resolution deadline")
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
    parser.add_argument("--planning-workers", type=int, choices=(1, 2, 3), default=1,
                        help="Parallel workers for planning deadline or lane-pressure trials")
    parser.add_argument("--bench-planning-workers", action="store_true",
                        help="Compare scoped planning on one, two and four workers")
    mode.add_argument("--lane-pressure-trial", action="store_true",
                      help="Compare lane-aware and reference scoring on Across the River")
    parser.add_argument("--lane-pressure-evaluator", choices=("lane_pressure_v1", "lane_pressure_v2", "lane_pressure_v3", "lane_pressure_v4", "lane_pressure_v5"),
                        default="lane_pressure_v2", help="Candidate score for --lane-pressure-trial")
    parser.add_argument("--lane-pressure-attack-targets", action="store_true",
                        help="Extend the candidate search at attack target prompts")
    parser.add_argument("--lane-pressure-reply-cap", type=int, default=4,
                        help="Shared cap on opponent planning replies per sampled world")
    parser.add_argument("--lane-pressure-max-nodes", type=int, default=4096,
                        help="Shared per-decision node budget for both trial policies")
    parser.add_argument("--planning-branching-only-trial", action="store_true",
                        help="Compare later planning branching with greedy forecasts off")
    parser.add_argument("--greedy-branching-trial", action="store_true",
                        help="Keep greedy forecasts on both sides and isolate later planning branching")
    parser.add_argument("--greedy-card-order-trial", action="store_true",
                        help="Compare four-turn greedy card ordering at equal search budgets")
    parser.add_argument("--lane-pressure-held-out", action="store_true",
                        help="Use the four-map held-out evaluator trial")
    parser.add_argument("--lane-pressure-baseline",
                        choices=("lab_attack_range_v1", "lane_pressure_v1", "lane_pressure_v2", "lane_pressure_v3", "lane_pressure_v4"),
                        default="lab_attack_range_v1", help="Baseline score for --lane-pressure-trial")
    mode.add_argument("--strength-baseline", action="store_true",
                      help="Measure the fixed Rust policy against itself on the tuning corpus")
    mode.add_argument("--forecast-share-trial", action="store_true",
                      help="Compare quarter and eighth forecast budgets in side-swapped tuning games")
    mode.add_argument("--forecast-value-trial", action="store_true",
                      help="Compare greedy forecasts with the fixed lowest-defense opponent policy")
    mode.add_argument("--followup-width-trial", action="store_true",
                      help="Compare balanced four-row and legacy two-row coordinated follow-ups")
    mode.add_argument("--horizon-trial", action="store_true",
                      help="Compare candidate and reference rollout horizons, swapping sides")
    parser.add_argument("--candidate-rollout-turns", type=int, choices=(1, 3),
                        help="Candidate horizon for --horizon-trial (default: 1)")
    mode.add_argument("--lane-timing-trial", action="store_true",
                      help="Compare Lane Pressure V4 and V3 under the strength screening policy")
    mode.add_argument("--lane-timing-held-out", action="store_true",
                      help="Validate V4 versus V3 at five seconds on eight held-out starts")
    mode.add_argument("--node-budget-trial", action="store_true",
                      help="Compare 262,144 and 131,072 nodes at the same decision deadline")
    mode.add_argument("--decision-depth-trial", action="store_true",
                      help="Compare search depths three and two at a fixed two-turn horizon")
    mode.add_argument("--tactical-forecast-trial", action="store_true",
                      help="Use bounded tactical replies only in the four-turn greedy forecast")
    mode.add_argument("--planning-opposition-trial", action="store_true",
                      help="Compare adversarial planning replies with fixed greedy forecast opposition")
    mode.add_argument("--attack-extension-trial", action="store_true",
                      help="Extend attack-target search at the depth cutoff while holding the horizon fixed")
    mode.add_argument("--movement-extension-trial", action="store_true",
                      help="Compare a bounded movement shortlist at the depth cutoff with first legal")
    mode.add_argument("--reply-width-trial", action="store_true",
                      help="Compare eight and four planning replies at equal budgets")
    mode.add_argument("--world-count-trial", action="store_true",
                      help="Compare four and two sampled hidden worlds at equal budgets")
    mode.add_argument("--planning-seed-trial", action="store_true",
                      help="Compare a bounded depth-zero planning pass with direct deep search")
    parser.add_argument("--strength-map", choices=tuple(case[0] for case in STRENGTH_SCREEN_STARTS),
                        help="Run only this screening start, including both sides for a trial")
    parser.add_argument("--strength-game-type", choices=("QUICK", "LONG"),
                        help="Opening game settings for strength screens (default: QUICK)")
    parser.add_argument("--strength-midgame", action="store_true",
                        help="Use recorded round-three tuning starts on Forgotten Island and Magma and Ice")
    mode.add_argument("--held-out-strength", action="store_true",
                      help="Run the fixed 16-game horizon comparison at 100 ms")
    mode.add_argument("--strength-trial", action="store_true",
                      help="Run side-swapped one-turn versus two-turn Rust search matches")
    mode.add_argument("--verify-planning-deadline", action="store_true",
                      help="Time live sampled planning and validate scored or fallback choices")
    parser.add_argument("--planning", action="store_true", help="Also verify sampled planning")
    parser.add_argument("--all-maps", action="store_true", help="Check every bundled map")
    parser.add_argument("--rollout-midgame", action="store_true",
                        help="Time recorded windows starting in rounds three and four")
    parser.add_argument("--rollout-corpus", type=Path,
                        help="Complete-game fixture index for --recorded-rollouts")
    parser.add_argument("--rollout-workers", type=int, choices=range(1, 5), default=1,
                        help="Independent workers for the rollout benchmark")
    parser.add_argument("--sample-workers", type=int, choices=range(1, 5), default=1,
                        help="Parallel world scorers for sampled resolution")
    parser.add_argument("--sample-count", type=int, choices=range(1, 9), default=1,
                        help="Number of worlds for sampled rollout benchmarks or resolution search")
    parser.add_argument("--sampled", action="store_true", help="Also check sampled level-one roots")
    parser.add_argument("--sampled-upgrades", action="store_true", help="Also check upgraded samples")
    args = parser.parse_args()
    if args.candidate_rollout_turns is not None and not args.horizon_trial:
        parser.error("--candidate-rollout-turns requires --horizon-trial")
    strength_screen = (
        args.strength_baseline or args.forecast_share_trial or args.forecast_value_trial
        or args.followup_width_trial or args.horizon_trial or args.lane_timing_trial
        or args.node_budget_trial or args.decision_depth_trial or args.attack_extension_trial
        or args.planning_opposition_trial or args.tactical_forecast_trial
        or args.movement_extension_trial
        or args.reply_width_trial or args.world_count_trial or args.planning_seed_trial
    )
    if (args.strength_map or args.strength_midgame) and not strength_screen:
        parser.error("--strength-map and --strength-midgame require a strength baseline or screening trial mode")
    if args.strength_game_type and (not strength_screen or args.strength_midgame):
        parser.error("--strength-game-type requires a strength baseline or screening trial at the opening")
    if args.strength_midgame and args.strength_map not in (None, *STRENGTH_MIDGAME_MAPS):
        parser.error("--strength-midgame only supports Forgotten Island and Magma and Ice")
    if args.lane_timing_held_out and args.decision_ms != 5000:
        parser.error("--lane-timing-held-out requires --decision-ms 5000")
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
    if args.greedy_upgrade_compact and (not args.greedy_upgrades or args.all_maps):
        parser.error("--greedy-upgrade-compact requires --greedy-upgrades without --all-maps")
    if args.greedy_upgrade_level_eight and not args.greedy_upgrades:
        parser.error("--greedy-upgrade-level-eight requires --greedy-upgrades")
    if args.greedy_upgrade_time_limited and not args.greedy_upgrades:
        parser.error("--greedy-upgrade-time-limited requires --greedy-upgrades")
    if args.sample_future_planning and not args.native_turns:
        parser.error("--sample-future-planning requires --native-turns")
    if args.lane_pressure_attack_targets and not args.lane_pressure_trial:
        parser.error("--lane-pressure-attack-targets requires --lane-pressure-trial")
    if args.planning_branching_only_trial and (
        not args.lane_pressure_trial or args.greedy_card_order_trial
        or args.lane_pressure_attack_targets
        or args.lane_pressure_evaluator != args.lane_pressure_baseline
    ):
        parser.error("--planning-branching-only-trial requires a lane-pressure trial with matching evaluators and no other policy changes")
    if args.greedy_branching_trial and not args.greedy_card_order_trial:
        parser.error("--greedy-branching-trial requires --greedy-card-order-trial")
    if args.greedy_card_order_trial and (
        not args.lane_pressure_trial or args.lane_pressure_attack_targets
        or args.lane_pressure_evaluator != args.lane_pressure_baseline
    ):
        parser.error("--greedy-card-order-trial requires a lane-pressure trial with matching evaluators and no attack extension")
    if not 1 <= args.lane_pressure_reply_cap <= 4096:
        parser.error("--lane-pressure-reply-cap must be in 1..4096")
    if args.lane_pressure_max_nodes < 1:
        parser.error("--lane-pressure-max-nodes must be positive")
    if args.lane_pressure_held_out and not args.lane_pressure_trial:
        parser.error("--lane-pressure-held-out requires --lane-pressure-trial")
    if args.prepare_search and not (args.lane_pressure_trial or args.held_out_strength or args.verify_planning_deadline or args.sampled_resolution or args.greedy_movement_cache or args.greedy_upgrades or args.greedy_routes or args.greedy_position or args.greedy_tactics or args.greedy_search):
        parser.error("--prepare-search requires a planning, sampled-resolution or greedy search mode")
    if args.planning_workers != 1 and not (args.verify_planning_deadline or args.lane_pressure_trial):
        parser.error("--planning-workers requires --verify-planning-deadline or --lane-pressure-trial")
    if args.lane_pressure_trial and args.planning_workers > 2:
        parser.error("--lane-pressure-trial has two sampled planning worlds")
    if args.verify_resolution_deadline and not args.sampled_resolution:
        parser.error("--verify-resolution-deadline requires --sampled-resolution")
    if not 0 <= args.resolution_transport_reserve_ms < args.decision_ms:
        parser.error("resolution transport reserve must be smaller than --decision-ms")
    if args.resolution_live_export and not args.resolution_transport_reserve_ms:
        parser.error("--resolution-live-export requires --resolution-transport-reserve-ms")
    if args.resolution_transport_reserve_ms and not args.verify_resolution_deadline:
        parser.error("resolution transport reserve requires --verify-resolution-deadline")
    if args.rollout_midgame and not args.recorded_rollouts:
        parser.error("--rollout-midgame requires --recorded-rollouts")
    if args.rollout_corpus and not args.recorded_rollouts:
        parser.error("--rollout-corpus requires --recorded-rollouts")
    if args.rollout_workers > 1 and not (args.recorded_rollouts or args.diverse_rollouts):
        parser.error("--rollout-workers requires --recorded-rollouts or --diverse-rollouts")
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
    instance_exporter = None
    if args.sweep_planning_workers is not None and not (
        args.roster_sweep_turns and args.sweep_policy == "search"
    ):
        parser.error("--sweep-planning-workers requires a search roster sweep")

    if args.resolution_live_export or args.verify_planning_deadline:
        from state_codec import verified_instance_exporter

        instance_exporter = verified_instance_exporter()
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
                decision_ms=args.decision_ms, planning_workers=args.sweep_planning_workers,
            )
            return
        if args.diverse_rollouts:
            verify_diverse_rollouts(
                worker, evaluator, args.bench_iterations, args.sample_count, args.rollout_workers,
            )
            return
        if args.recorded_rollouts:
            verify_recorded_rollouts(
                worker, evaluator, args.bench_iterations, args.sample_count, args.rollout_workers,
                args.rollout_corpus or ROOT / "rust/tests/fixtures/complete-games-hanu.json",
                midgame=args.rollout_midgame,
            )
            return
        if args.lane_pressure_trial:
            verify_lane_pressure_trial(
                worker, maps, args.decision_ms, args.lane_pressure_evaluator,
                args.lane_pressure_baseline, args.lane_pressure_held_out, args.planning_workers,
                attack_targets=args.lane_pressure_attack_targets,
                greedy_card_order=args.greedy_card_order_trial,
                greedy_branching_trial=args.greedy_branching_trial,
                planning_branching_only=args.planning_branching_only_trial,
                max_nodes=args.lane_pressure_max_nodes,
                reply_cap=args.lane_pressure_reply_cap,
            )
            return
        if strength_screen or args.lane_timing_held_out:
            trial = (
                "forecast_share" if args.forecast_share_trial else
                "forecast_value" if args.forecast_value_trial else
                "followup_width" if args.followup_width_trial else
                "horizon" if args.horizon_trial else
                "lane_timing" if args.lane_timing_trial or args.lane_timing_held_out else
                "node_budget" if args.node_budget_trial else
                "decision_depth" if args.decision_depth_trial else
                "attack_extension" if args.attack_extension_trial else
                "tactical_forecast" if args.tactical_forecast_trial else
                "planning_opposition" if args.planning_opposition_trial else
                "movement_extension" if args.movement_extension_trial else
                "reply_width" if args.reply_width_trial else
                "world_count" if args.world_count_trial else
                "planning_seed" if args.planning_seed_trial else None
            )
            worker_path = Path(which(args.worker) or args.worker)
            with worker_path.open("rb") as executable:
                print(f"Strength worker SHA-256: {file_digest(executable, 'sha256').hexdigest()}")
            verify_strength_baseline(
                worker, maps, args.decision_ms, trial=trial, selected_map=args.strength_map,
                held_out=args.lane_timing_held_out,
                candidate_rollout_turns=args.candidate_rollout_turns or 1,
                midgame=args.strength_midgame, game_type=args.strength_game_type or "QUICK",
            )
            return
        if args.held_out_strength:
            verify_held_out_strength(worker, maps)
            return
        if args.replay_terminal:
            verify_terminal_matches(worker, args.decision_ms)
            return
        if args.greedy_upgrade_compact:
            opening = GameSetup.create_game(
                str(maps / "forgotten_island.json"), roster[:2], roster[2:], seed=args.game_seed,
            )
            compact = compact_greedy_board(opening)
            print(f"Compact fixture: {len(compact.board.tiles)} of {len(opening.board.tiles)} tiles")
            verify_greedy_upgrade_search(
                worker, "compact", compact, args.decision_ms, profile=args.profile_greedy,
                time_limited=args.greedy_upgrade_time_limited,
                level_eight=args.greedy_upgrade_level_eight,
            )
            return
        for path in paths:
            map_id = path.stem
            opening = GameSetup.create_game(
                str(path),
                roster[:2],
                roster[2:],
                seed=args.game_seed,
                game_type="QUICK" if args.strength_trial else "LONG",
            )
            if args.verify_work_budget:
                verify_work_budget(worker, map_id, opening)
                continue
            if args.strength_trial:
                verify_strength_trial(worker, map_id, opening, args.game_seed, args.decision_ms)
                continue
            if args.greedy_movement_cache:
                verify_greedy_movement_cache(worker, map_id, opening, args.decision_ms)
                continue
            if args.greedy_upgrades:
                verify_greedy_upgrade_search(
                    worker, map_id, opening, args.decision_ms, profile=args.profile_greedy,
                    time_limited=args.greedy_upgrade_time_limited,
                    level_eight=args.greedy_upgrade_level_eight,
                )
                continue
            if args.greedy_search or args.greedy_position or args.greedy_routes or args.greedy_tactics:
                verify_greedy_search(
                    worker, map_id, opening, args.decision_ms,
                    comparison="tactics" if args.greedy_tactics else
                    "routes" if args.greedy_routes else
                    "position" if args.greedy_position else "cards",
                )
                if args.greedy_tactics and map_id == "forgotten_island":
                    verify_close_contact_tactics(worker, opening, args.decision_ms)
                continue
            if args.verify_planning_deadline:
                verify_planning_deadline(
                    worker, map_id, opening, args.decision_ms, args.bench_iterations, instance_exporter,
                    args.planning_workers,
                )
                continue
            if args.sampled_resolution:
                if map_id == "forgotten_island":
                    if args.verify_resolution_deadline:
                        verify_retained_short_turn(worker, opening)
                    else:
                        verify_sampled_color_guess(worker)
                verify_sampled_resolution(
                    worker, path, opening, args.decision_ms, args.sample_count, args.sample_workers,
                    args.bench_iterations, args.verify_resolution_deadline,
                    args.resolution_transport_reserve_ms, instance_exporter,
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
                if map_id == "forgotten_island":
                    verify_concealed_discard_planning(worker)
                    verify_visible_ally_affects_plan(worker)
                    verify_last_card_closes_second_slot(worker)
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
                verify_upgrade_batch_sizes(worker, opening)
                if map_id in STRENGTH_MIDGAME_MAPS:
                    verify_offline_upgrade_history(worker, map_id, opening)
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
