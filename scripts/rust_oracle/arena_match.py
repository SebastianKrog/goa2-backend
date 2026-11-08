"""Route a node-budget match between two Rust policy workers and a Rust referee."""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from contextlib import ExitStack
from pathlib import Path

from arena_choices import CHOICE_ENCODING, decision_hash
from arena_policy import apply_policy, load_policy, validate_policy
from arena_progress import StallClock
from arena_reports import save_report, sha256_file
from oracle import Worker, digest


SCORER_BUDGETS = {"arena_search_v1": "nodes_v1", "arena_search_v2": "nodes_v2"}


def play_match(referee, workers, envelope, *, turns, seed, planning_nodes,
               resolution_nodes, coordinated=False, watchdog=120, opponent_policies=None,
               observe=None, record=None, observe_request=None, policy_profiles=None,
               work_budget="nodes_v2"):
    if work_budget not in SCORER_BUDGETS.values():
        raise ValueError("unsupported arena work budget")
    selective = work_budget == "nodes_v2"
    options = {"work_budget": work_budget, "rollout_turns": 4 if selective else 2,
               "evaluation": "lane_pressure_v3"}
    opponent_policies = opponent_policies or {}
    policy_profiles = {side: validate_policy(policy) for side, policy in (policy_profiles or {}).items()}
    if policy_profiles.keys() - workers.keys():
        raise ValueError("arena policy settings target an unknown side")
    stall_policies = {side: policy["stall"] for side, policy in policy_profiles.items() if "stall" in policy}
    if stall_policies and not selective:
        raise ValueError("stall policies require the Arena v2 work budget")
    rosters = {
        team: [hero["id"] for hero in row["heroes"]]
        for team, row in envelope["state"]["teams"].items()
    }
    if workers.keys() != rosters.keys() or workers.keys() != {"RED", "BLUE"}:
        raise ValueError("arena matches require a policy worker for each of Red and Blue")
    position = referee.request("arena_start", envelope=envelope, turns=turns, timeout=watchdog)
    stall_clock = StallClock(position) if stall_policies else None
    trace = []
    work = {team: defaultdict(int) for team in workers}

    def recommend(team, phase, operation, **fields):
        work[team][f"{phase}_requests"] += 1
        depth = (3 if phase == "planning" else 5) if selective else 2
        request = dict(options, max_depth=depth, **fields)
        apply_policy(request, phase, policy_profiles.get(team, {}).get(phase, {}))
        stall = stall_policies.get(team)
        if stall is not None and stall_clock.age(stall["progress"]) >= stall["turns"]:
            request["evaluation"] = stall["evaluation"]
            work[team][f"{phase}_stall_activations"] += 1
        if observe_request is not None:
            observe_request(phase, operation, request, position)
        try:
            result = workers[team].request(operation, timeout=watchdog, **request)
        except RuntimeError as error:
            # Engine limitations can use the referee's legal fallback.
            # Transport, protocol and watchdog failures abort the match.
            if not str(error).startswith("worker rejected request: Unsupported("):
                raise
            work[team][f"{phase}_unsupported"] += 1
            return None
        nodes = result["nodes"]
        if type(nodes) is not int or not 0 <= nodes <= fields["max_nodes"]:
            raise ValueError("worker exceeded the match node budget")
        work[team][f"{phase}_nodes"] += nodes
        if selective and nodes == fields["max_nodes"]:
            work[team][f"{phase}_node_cutoffs"] += 1
        if result["score"] is None and result.get("shallow_score") is None:
            work[team][f"{phase}_unscored"] += 1
            return None
        return result

    while position["kind"] != "finished":
        if position["revision"] != len(trace):
            raise ValueError("arena referee skipped a revision")
        epoch_seed = (seed + position["round"] * 4 + position["turn"]) % (1 << 64)
        shared = dict(envelope=position["envelope"],
                      upgrade_checkpoints=position["upgrade_checkpoints"])
        if position["kind"] == "planning":
            groups = defaultdict(list)
            for slot in position["slots"]:
                key = slot["team"] if coordinated else slot["player_id"]
                groups[key].append(slot)
            commitments = []
            for group in groups.values():
                first = group[0]
                team, player = first["team"], first["player_id"]
                response = recommend(
                    team, "planning", "search_sampled_planning", player_id=player,
                    seed=epoch_seed, cap=8 if selective else 4, reply_cap=4, max_nodes=planning_nodes,
                    greedy_card_order=True,
                    bot_team={"mode": "coordinated" if coordinated else "true_rules",
                              "bot_ids": rosters[team] if coordinated else [player],
                              "worlds": 2},
                    **shared,
                )
                if response is None:
                    commitments.extend(slot["fallback"] for slot in group)
                    continue
                rows = response["commitments"]
                expected = {slot["player_id"] for slot in group}
                if len(rows) != len(expected) or {row["player_id"] for row in rows} != expected:
                    raise ValueError("worker did not answer exactly its open planning slots")
                commitments.extend(rows)
            choice = {"kind": "planning", "commitments": commitments}
        elif position["kind"] == "resolution":
            choice = position["fallback"]
            team = position["team"]
            if position["forced"]:
                work[team]["forced"] += 1
            else:
                response = recommend(
                    team, "resolution", "search_resolution", player_id=choice["player_id"],
                    max_nodes=resolution_nodes, resolution_sample_seed=epoch_seed,
                    resolution_sample_worlds=2,
                    opponent_policy=opponent_policies.get(team, "adversarial"), **shared,
                    **({"arena_bot_ids": rosters[team] if coordinated else [choice["player_id"]]}
                       if selective else {"planning_sample_seed": epoch_seed}),
                )
                if response is not None:
                    move = response["decision"]
                    if (move["kind"] != "input" or move["player_id"] != choice["player_id"]
                            or move["request_id"] != choice["request_id"]):
                        raise ValueError("policy did not answer the referee's live prompt")
                    choice = dict(move, kind="resolution")
        else:
            raise ValueError(f"unknown referee boundary {position['kind']!r}")
        response = referee.request(
            "arena_submit", revision=position["revision"], choice=choice, timeout=watchdog,
        )
        legal_choices = position["legal_choices"]
        if observe is not None:
            observe(legal_choices)
        decision = dict(response["decision"], legal_choices_sha256=decision_hash(legal_choices))
        if record is not None:
            record(decision)
        trace.append(decision)
        position = response["position"]
        if stall_clock is not None:
            stall_clock.observe(position)
    result = {"choice_encoding": CHOICE_ENCODING, "trace": trace,
              "work": {team: dict(stats) for team, stats in work.items()}, "endpoint": position}
    if stall_clock is not None:
        result["stall"] = stall_clock.result()
    return result


def score_match(paths, envelope, *, record=None, expected_binaries=None, **parameters):
    with ExitStack() as stack:
        workers = {}
        binaries = {}
        for role, path in paths.items():
            binaries[role] = sha256_file(path)
            if expected_binaries is not None and binaries[role] != expected_binaries[role]:
                raise ValueError(f"arena worker changed for {role}")
            worker = Worker([str(path.resolve())])
            stack.callback(worker.close)
            workers[role] = worker
        referee = workers.pop("referee")
        result = play_match(
            referee, workers, envelope, record=record, **parameters,
        )
    state = result["endpoint"].pop("state")
    result["endpoint"]["rust_state_sha256"] = digest(state)
    result["binaries"] = binaries
    result["parameters"] = {"work_budget": "nodes_v2", **parameters}
    result["rules"] = {
        key: envelope[key] for key in ("schema_version", "oracle_revision", "source_sha256", "rules_version")
    }
    return result


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--referee", type=Path, required=True)
    parser.add_argument("--red", type=Path, required=True)
    parser.add_argument("--blue", type=Path, required=True)
    parser.add_argument("--red-policy", type=Path)
    parser.add_argument("--blue-policy", type=Path)
    parser.add_argument("--envelope", type=Path, required=True)
    parser.add_argument("--store", type=Path, required=True)
    parser.add_argument("--turns", type=int, default=64, choices=range(1, 65))
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--planning-nodes", type=int, required=True)
    parser.add_argument("--resolution-nodes", type=int, required=True)
    parser.add_argument("--coordinated", action="store_true")
    parser.add_argument("--work-budget", choices=tuple(SCORER_BUDGETS.values()), default="nodes_v2")
    args = parser.parse_args()
    if min(args.planning_nodes, args.resolution_nodes) < 1 or not 0 <= args.seed < 1 << 64:
        parser.error("node caps must be positive and seed must be an unsigned 64-bit integer")

    try:
        policies = {side: load_policy(path) for side, path in
                    (("RED", args.red_policy), ("BLUE", args.blue_policy)) if path is not None}
    except (ValueError, OSError) as error:
        parser.error(str(error))
    paths = {"referee": args.referee, "RED": args.red, "BLUE": args.blue}
    envelope = json.loads(args.envelope.read_text(encoding="utf-8"))
    result = score_match(
        paths, envelope, turns=args.turns, seed=args.seed,
        planning_nodes=args.planning_nodes, resolution_nodes=args.resolution_nodes,
        coordinated=args.coordinated, work_budget=args.work_budget,
        **({"policy_profiles": policies} if policies else {}),
    )
    report_id = save_report(args.store, envelope, result)
    print(json.dumps(dict(result, report_id=report_id, start_sha256=digest(envelope)),
                     separators=(",", ":")))


if __name__ == "__main__":
    main()
