"""Python Card equality, including private state absent from normal JSON."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from oracle import ROOT, Worker, digest, verify_sources
from state_codec import export_state_with_instances


def cases():
    from model_traces import setup

    from goa2.domain.models import CardState

    variants = [
        "equal",
        "same-instance",
        "revision",
        "same-revision",
        "metadata-order",
        "metadata-number",
        "large-float",
        "int-limit",
        "list-order",
        "nested",
        "different-definition",
        "different-name",
        "face",
        "state",
        "active",
        "enraged",
        "uses",
        "initiative",
        "played",
        "null-metadata",
        "effect-text",
        "image",
        "starts-in-deck",
        "spell-equal",
        "spell-revision",
        "spell-card-type",
    ]
    for variant in variants:
        state = (
            setup(red=["Gydion", "Arien"], blue=["Brogan", "Hanu"])
            if variant.startswith("spell-")
            else setup()
        )
        hero = next(iter(state.teams.values())).heroes[0]
        a = hero.spells[0] if variant in ("spell-equal", "spell-revision") else hero.hand[0]
        b = a if variant == "same-instance" else a.model_copy(deep=True)
        if variant == "revision":
            b._definition_revision += 1
        if variant == "same-revision":
            a._definition_revision = 9
            b._definition_revision = 9
        if variant == "metadata-order":
            a.metadata = {"first": 1, "second": [True, {"k": 2}]}
            b.metadata = {"second": [1.0, {"k": 2.0}], "first": True}
        if variant == "metadata-number":
            a.metadata, b.metadata = {"value": False}, {"value": -0.0}
        if variant == "large-float":
            a.metadata, b.metadata = {"value": 9007199254740993}, {"value": 9007199254740992.0}
        if variant == "int-limit":
            a.metadata, b.metadata = {"value": 9223372036854775807}, {
                "value": 9223372036854775808.0
            }
        if variant == "list-order":
            a.metadata, b.metadata = {"value": [1, 2]}, {"value": [2, 1]}
        if variant == "nested":
            a.metadata, b.metadata = {"a": [{"b": 1}]}, {"a": [{"b": True}]}
        if variant == "different-definition":
            b = hero.hand[1]
        if variant == "different-name":
            b.name += " alternate"
        if variant == "face":
            b.is_facedown = not a.is_facedown
        if variant == "state":
            b.state = CardState.DISCARD
        if variant == "active":
            b.is_active = not a.is_active
        if variant == "enraged":
            b.enraged_active_override = not a.enraged_active_override
        if variant == "uses":
            b.passive_uses_this_turn += 1
        if variant == "initiative":
            b.initiative_override = 3
        if variant == "played":
            b.played_this_round = not a.played_this_round
        if variant == "null-metadata":
            b.metadata = None
        if variant == "effect-text":
            b.effect_text += " alternate"
            b._definition_revision = a._definition_revision
        if variant == "image":
            b.image_id += " alternate"
            b._definition_revision = a._definition_revision
        if variant == "starts-in-deck":
            b.starts_in_deck = not a.starts_in_deck
            b._definition_revision = a._definition_revision
        if variant == "spell-revision":
            b._definition_revision += 1
        if variant == "spell-card-type":
            b = hero.spells[0]
        if variant in ("spell-equal", "spell-revision"):
            hero.spells = [a, b]
        elif variant != "spell-card-type":
            hero.hand = [a, b]
        envelope, objects = export_state_with_instances(state)
        ids = [next(i for i, item in enumerate(objects) if item is card) for card in (a, b)]
        queries = [{"a": ids[x], "b": ids[y]} for x, y in [(0, 0), (0, 1), (1, 0), (1, 1)]]
        expected = [left == right for left, right in [(a, a), (a, b), (b, a), (b, b)]]
        yield {"name": variant, "envelope": envelope, "queries": queries, "expected": expected}


def export(path):
    manifest = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "cases": list(cases()),
    }
    data["corpus_sha256"] = digest(data)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")
    return data


def verify(path, command):
    manifest = verify_sources()
    data = json.loads(path.read_text())
    identity = data.pop("corpus_sha256")
    if (
        digest(data) != identity
        or data["source_sha256"] != manifest["source_sha256"]
        or data["oracle_revision"] != manifest["oracle_revision"]
    ):
        raise ValueError("card equality corpus identity mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            result = worker.request(
                "card_equality", envelope=case["envelope"], queries=case["queries"]
            )
            if result["results"] != case["expected"]:
                raise AssertionError(case["name"])
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "comparisons": sum(len(case["queries"]) for case in data["cases"]),
        "corpus_sha256": identity,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=("export", "verify"))
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/tests/fixtures/card-equality.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        result = export(args.corpus)
        print({"cases": len(result["cases"]), "corpus_sha256": result["corpus_sha256"]})
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker command required after --")
        print(verify(args.corpus, command))
