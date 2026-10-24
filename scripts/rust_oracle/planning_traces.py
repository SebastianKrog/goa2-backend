"""Frozen real planning phase calls, including the queued resolution handoff.

These traces deliberately do not drain the resulting action stack. Full session
execution is a separate coverage gate; no neural-network modules are imported.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from model_traces import setup
from oracle import ROOT, Worker, digest, verify_sources
from stack_traces import TraceNormalizer
from state_codec import export_state_with_instances


def corpus():
    from goa2.domain.models import GamePhase, StatType
    from goa2.engine import phases

    cases = []

    def add(name, actions, *, emmitt=False, prepare=None):
        state = setup(["Emmitt", "Arien"] if emmitt else None)
        if emmitt:
            state.get_hero("hero_emmitt").level = 8
        if prepare:
            prepare(state)
        envelope, objects = export_state_with_instances(state)
        cards = {
            (str(h.id), i): card
            for team in state.teams.values()
            for h in team.heroes
            for i, card in enumerate(h.hand)
        }
        operations, expected = [], []
        normalizer = TraceNormalizer()
        for kind, hero, *args in actions:
            operation = {"kind": kind, "hero": hero}
            card = None
            if kind == "commit":
                card = cards[(hero, args[0])]
                operation["card"] = next(i for i, obj in enumerate(objects) if obj is card)
            operations.append(operation)
            error = None
            try:
                if kind == "commit":
                    phases.commit_card(state, hero, card)
                elif kind == "uncommit":
                    phases.uncommit_card(state, hero)
                elif kind == "pass":
                    phases.pass_turn(state, hero)
                elif kind == "finish":
                    phases.finish_planning(state, hero)
                else:
                    raise AssertionError(kind)
            except ValueError:
                error = "invalid_planning_action"
            normalizer.bind_steps(state.execution_stack)
            record = normalizer.record(
                {"semantic_state": state.model_dump(mode="json"), "error": error, "request": None},
                state.execution_stack,
            )
            record.pop("request")
            expected.append(record)
        cases.append(
            {"name": name, "envelope": envelope, "operations": operations, "expected": expected}
        )

    ids = ["hero_arien", "hero_hanu", "hero_brogan", "hero_tigerclaw"]
    commits = [("commit", h, 0) for h in ids]
    add(
        "commit-takeback-recommit-and-illegal-actions",
        [
            ("uncommit", ids[0]),
            ("finish", ids[0]),
            ("pass", ids[0]),
            commits[0],
            commits[0],
            ("finish", ids[0]),
            ("uncommit", ids[0]),
            ("uncommit", ids[0]),
            *commits,
            ("uncommit", ids[0]),
            ("pass", ids[0]),
            ("finish", ids[0]),
            commits[0],
        ],
    )
    add("reverse-commit-order", list(reversed(commits)))

    def tied(state):
        for team in state.teams.values():
            for hero in team.heroes:
                hero.hand[0].initiative = 7

    add("all-heroes-tied-preserve-commit-order", list(reversed(commits)), prepare=tied)

    def modifiers(state):
        tied(state)
        state.get_hero(ids[1]).items[StatType.INITIATIVE] = 3
        state.get_hero(ids[0]).hand[0].initiative_override = 18

    add("items-and-initiative-override", commits, prepare=modifiers)

    def offboard(state):
        modifiers(state)
        state.remove_entity(ids[0])

    add("winning-hero-respawns-first", commits, prepare=offboard)

    def passed(state):
        hero = state.get_hero(ids[1])
        for card in list(hero.hand):
            hero.discard_card(card)
        state.turn = 3
        state.starting_position_requests = {
            "clear-from": {"from": ids[0], "to": ids[1]},
            "clear-to": {"from": ids[2], "to": ids[0]},
            "keep": {"from": ids[2], "to": ids[3]},
        }

    add(
        "pass-slot-gaps-and-starting-request-cleanup",
        [commits[0], ("pass", ids[1]), ("pass", ids[1]), ("uncommit", ids[1]), *commits[2:]],
        prepare=passed,
    )

    emmitt = "hero_emmitt"
    other = [("commit", h, 0) for h in [ids[0], ids[2], ids[3]]]
    add(
        "two-card-takeback-lifo-and-retrieval-handoff",
        [
            ("commit", emmitt, 0),
            ("commit", emmitt, 1),
            ("commit", emmitt, 2),
            ("uncommit", emmitt),
            ("finish", emmitt),
            ("commit", emmitt, 2),
            ("uncommit", emmitt),
            ("commit", emmitt, 1),
            ("commit", emmitt, 2),
            *other,
        ],
        emmitt=True,
    )
    add(
        "two-card-hero-keeps-last-commit-open",
        [*other, ("commit", emmitt, 0), ("commit", emmitt, 1)],
        emmitt=True,
    )
    add(
        "two-card-done-then-takeback",
        [
            ("commit", emmitt, 0),
            ("finish", emmitt),
            ("finish", emmitt),
            ("commit", emmitt, 1),
            ("uncommit", emmitt),
            ("commit", emmitt, 1),
        ],
        emmitt=True,
    )

    def resolution(state):
        state.phase = GamePhase.RESOLUTION

    add(
        "nonplanning-noops-and-errors",
        [commits[0], ("finish", ids[0]), ("pass", ids[0]), ("uncommit", ids[0])],
        prepare=resolution,
    )

    def precommitted(state):
        h = state.get_hero(ids[0])
        phases.commit_card(state, h.id, h.hand[0])
        state.record_public_revealed_card(h.id, h.current_turn_card.id)

    add(
        "restore-pending-takeback-and-public-reveal-deduplication",
        [("uncommit", ids[0]), ("commit", ids[0], 0), *commits[1:]],
        prepare=precommitted,
    )

    def precommitted_two(state):
        h = state.get_hero(emmitt)
        phases.commit_card(state, h.id, h.hand[0])
        phases.commit_card(state, h.id, h.hand[0])

    add(
        "restore-two-card-pending-buffers",
        [
            ("uncommit", emmitt),
            ("uncommit", emmitt),
            ("commit", emmitt, 0),
            ("commit", emmitt, 1),
            *other,
        ],
        emmitt=True,
        prepare=precommitted_two,
    )

    manifest = verify_sources()
    data = {
        "schema_version": 1,
        "source_sha256": manifest["source_sha256"],
        "cases": cases,
        "ties": tie_cases(),
    }
    return {**data, "corpus_sha256": digest(data)}


def tie_cases():
    from goa2.domain.models import TeamColor
    from goa2.engine import phases
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import ResolveTieBreakerStep

    ids = ["hero_arien", "hero_hanu", "hero_brogan", "hero_tigerclaw"]
    cases = []
    scenarios = [
        ("empty", [], TeamColor.RED, [{}]),
        ("single", [ids[0]], TeamColor.BLUE, [{}]),
        (
            "same-team-choice",
            ids[:2],
            TeamColor.BLUE,
            [
                {},
                {"selection": ids[2]},
                {"selection": None},
                {"selection": True},
                {"selection": ids[1]},
            ],
        ),
        ("cross-red", [ids[2], ids[0]], TeamColor.RED, [{}]),
        ("cross-blue", [ids[0], ids[2]], TeamColor.BLUE, [{}]),
        (
            "cross-team-choice",
            ids,
            TeamColor.RED,
            [{}, {"selection": ids[2]}, {"selection": ids[0]}],
        ),
        (
            "cross-blue-choice",
            list(reversed(ids)),
            TeamColor.BLUE,
            [{}, {"selection": ids[0]}, {"selection": ids[3]}],
        ),
        ("unknown-among-candidates", ["unknown", ids[2]], TeamColor.RED, [{}]),
        ("unbound-team", [ids[0], ids[2]], TeamColor.RED, [{}]),
        ("offboard", [ids[0], ids[2]], TeamColor.RED, [{}]),
        ("razzle", ["hero_razzle", ids[2]], TeamColor.RED, [{}]),
    ]
    for name, tied, coin, operations in scenarios:
        state = setup(["Razzle", "Hanu"] if name == "razzle" else None)
        for team in state.teams.values():
            for hero in team.heroes:
                phases.commit_card(state, hero.id, hero.hand[0])
        state.execution_stack = []
        state.current_actor_id = None
        state.resolution_owner_id = None
        state.unresolved_hero_ids = [h for h in tied if state.get_hero(h)]
        state.tie_breaker_team = coin
        if name == "offboard":
            state.remove_entity(ids[0])
        if name == "unbound-team":
            state.get_hero(ids[0]).team = None
        envelope, _ = export_state_with_instances(state)
        step = ResolveTieBreakerStep(step_id="fixture/tie", tied_hero_ids=tied)
        expected = []
        normalizer = TraceNormalizer()
        for operation in operations:
            if "selection" in operation:
                step.pending_input = {"selection": operation["selection"]}
            result = step.resolve(state, state.execution_context)
            if result.new_steps:
                push_steps(state, result.new_steps)
            normalizer.bind_steps(state.execution_stack)
            expected.append(
                normalizer.record(
                    {
                        "semantic_state": state.model_dump(mode="json"),
                        "request": (
                            result.input_request.model_dump(mode="json")
                            if result.input_request
                            else None
                        ),
                    },
                    state.execution_stack,
                )
            )
        cases.append(
            {
                "name": name,
                "envelope": envelope,
                "tied": tied,
                "operations": operations,
                "expected": expected,
            }
        )
    return cases


def verify(path, command):
    data = json.loads(path.read_text())
    identity = data.pop("corpus_sha256")
    if digest(data) != identity or data["source_sha256"] != verify_sources()["source_sha256"]:
        raise ValueError("phase corpus identity mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request(
                "planning_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            normalizer = TraceNormalizer()
            actual = [normalizer.record({**record, "request": None}) for record in actual]
            for record in actual:
                record.pop("request")
            if actual != case["expected"]:
                failure = ROOT / "rust/artifacts/planning-mismatch.json"
                failure.parent.mkdir(parents=True, exist_ok=True)
                failure.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError(f"{case['name']}: saved {failure}")
        for case in data["ties"]:
            actual = worker.request(
                "tie_trace",
                envelope=case["envelope"],
                tied=case["tied"],
                operations=case["operations"],
            )["records"]
            normalizer = TraceNormalizer()
            actual = [normalizer.record(record) for record in actual]
            if actual != case["expected"]:
                failure = ROOT / "rust/artifacts/tie-mismatch.json"
                failure.parent.mkdir(parents=True, exist_ok=True)
                failure.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError(f"{case['name']}: saved {failure}")
    finally:
        worker.close()
    return {
        "tie_cases": len(data["ties"]),
        "tie_operations": sum(len(c["operations"]) for c in data["ties"]),
        "cases": len(data["cases"]),
        "operations": sum(len(c["operations"]) for c in data["cases"]),
        "corpus_sha256": identity,
    }


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/tests/fixtures/planning.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = corpus()
        args.corpus.write_text(json.dumps(data, separators=(",", ":")) + "\n")
        print(
            json.dumps(
                {
                    "tie_cases": len(data["ties"]),
                    "tie_operations": sum(len(c["operations"]) for c in data["ties"]),
                    "cases": len(data["cases"]),
                    "operations": sum(len(c["operations"]) for c in data["cases"]),
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
