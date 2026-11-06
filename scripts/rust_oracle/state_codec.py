"""Lossless outer-boundary export, distinct from the native simulation state.

Keep physical aliases without treating printed card IDs as instance IDs. This
format is private offline tooling; no backend API or persistence format changes.
"""

from __future__ import annotations

from enum import Enum

from fixture_cache import read_text, write_text

from oracle import ORACLE_REVISION, digest, verify_sources

REFERENCE_KEY = "$goa2_instance"


def reject_reserved(value):
    if isinstance(value, dict):
        if REFERENCE_KEY in value:
            raise ValueError(f"reserved boundary key in source data: {REFERENCE_KEY}")
        for child in value.values():
            reject_reserved(child)
    elif isinstance(value, list):
        for child in value:
            reject_reserved(child)


class InstanceEncoder:
    def __init__(self):
        from pydantic import BaseModel

        from goa2.domain.models import Card, Token

        self._base_model = BaseModel
        self._card = Card
        self._token = Token
        self.references = {}
        self.objects = []  # Keep originals alive: Python may otherwise reuse id().
        self.definitions = []
        self.definition_indices = {}
        self.instances = []

    def pack(self, value, kind, dynamic_fields):
        key = id(value)
        if key in self.references:
            return {REFERENCE_KEY: self.references[key]}
        instance = len(self.instances)
        self.references[key] = instance
        self.objects.append(value)
        fields = value.model_dump(mode="json", context={})
        reject_reserved(fields)
        definition = {
            "kind": kind,
            "fields": {key: item for key, item in fields.items() if key not in dynamic_fields},
        }
        # Intern scenario overrides by content, never by printed identity.
        definition_key = digest(definition)
        if definition_key not in self.definition_indices:
            self.definition_indices[definition_key] = len(self.definitions)
            self.definitions.append(definition)
        self.instances.append(
            {
                "definition": self.definition_indices[definition_key],
                "fields": {key: item for key, item in fields.items() if key in dynamic_fields},
            }
        )
        return {REFERENCE_KEY: instance}

    def walk(self, value, serialized):
        if type(value) in (str, int, float, bool, type(None)):
            return serialized
        if isinstance(value, self._card):
            return self.pack_card(value)
        if isinstance(value, self._token):
            return self.pack_token(value)
        if isinstance(value, self._base_model) and isinstance(serialized, dict):
            result = dict(serialized)
            for name, field in type(value).model_fields.items():
                key = field.serialization_alias or name
                if key in result:
                    result[key] = self.walk(getattr(value, name), result[key])
            return result
        if isinstance(value, dict) and isinstance(serialized, dict):
            result = dict(serialized)
            for key, item in value.items():
                serialized_key = str(key.value) if isinstance(key, Enum) else str(key)
                if serialized_key in result:
                    result[serialized_key] = self.walk(item, result[serialized_key])
            return result
        if isinstance(value, (list, tuple)) and isinstance(serialized, list):
            return [self.walk(item, raw) for item, raw in zip(value, serialized, strict=True)]
        return serialized

    def pack_card(self, card):
        from math import isfinite

        from goa2.domain.models.spell import SpellCard

        def json_equality(value):
            if value is None or type(value) in (bool, int, str):
                return True
            if type(value) is float:
                return isfinite(value)
            if isinstance(value, Enum) and isinstance(value, (str, int)):
                return value == value.value and json_equality(value.value)
            if type(value) is list:
                return all(json_equality(item) for item in value)
            if type(value) is dict:
                return all(type(key) is str and json_equality(item) for key, item in value.items())
            return False

        private = card.__pydantic_private__ or {}
        if (
            set(private) != {"_definition_revision"}
            or type(private["_definition_revision"]) is not int
        ):
            raise ValueError("unsupported private card equality state")
        kind = "spell" if isinstance(card, SpellCard) else "card"
        reference = self.pack(card, kind, card._search_dynamic_fields)
        self.instances[reference[REFERENCE_KEY]]["equality"] = {
            "model": f"{type(card).__module__}.{type(card).__qualname__}",
            "definition_revision": private["_definition_revision"],
            "json_metadata": json_equality(card.metadata),
        }
        return reference

    def pack_token(self, token):
        fixed = {"id", "name", "token_type"}
        dynamic = set(type(token).model_fields) - fixed
        return self.pack(token, "token", dynamic)


def export_state_with_instances(state):
    return _export_state_with_instances(state, verify_sources()["source_sha256"])


def verified_instance_exporter():
    """Verify the frozen checkout once for a single running oracle session."""
    source_sha256 = verify_sources()["source_sha256"]

    def export(state):
        return _export_state_with_instances(state, source_sha256)

    return export


def _export_state_with_instances(state, source_sha256):
    plain = state.model_dump(mode="json")
    reject_reserved(plain)
    encoder = InstanceEncoder()
    compact = encoder.walk(state, plain)
    envelope = {
        "schema_version": 1,
        "oracle_revision": ORACLE_REVISION,
        "source_sha256": source_sha256,
        "rules_version": state.rules_version,
        "definitions": encoder.definitions,
        "instances": encoder.instances,
        "state": compact,
    }
    if materialize(envelope) != plain:
        raise AssertionError("export omitted or altered semantic state")
    return envelope, tuple(encoder.objects)


def export_state(state):
    return export_state_with_instances(state)[0]


def materialize(envelope):
    definitions = envelope["definitions"]
    instances = envelope["instances"]

    def expand(value):
        if isinstance(value, dict):
            if REFERENCE_KEY in value:
                if set(value) != {REFERENCE_KEY} or type(value[REFERENCE_KEY]) is not int:
                    raise ValueError("invalid instance reference")
                index = value[REFERENCE_KEY]
                if not 0 <= index < len(instances):
                    raise ValueError("instance reference outside table")
                instance = instances[index]
                definition_index = instance["definition"]
                if type(definition_index) is not int or not 0 <= definition_index < len(
                    definitions
                ):
                    raise ValueError("definition reference outside table")
                definition = definitions[definition_index]
                equality = instance.get("equality")
                if equality is not None and (
                    type(equality) is not dict
                    or set(equality) != {"model", "definition_revision", "json_metadata"}
                    or definition["kind"] == "token"
                    or type(equality["model"]) is not str
                    or not equality["model"]
                    or type(equality["definition_revision"]) is not int
                    or not -(2**63) <= equality["definition_revision"] < 2**63
                    or type(equality["json_metadata"]) is not bool
                ):
                    raise ValueError("invalid physical equality metadata")
                if definition["fields"].keys() & instance["fields"].keys():
                    raise ValueError("definition and instance fields overlap")
                return {**definition["fields"], **instance["fields"]}
            return {key: expand(item) for key, item in value.items()}
        if isinstance(value, list):
            return [expand(item) for item in value]
        return value

    return expand(envelope["state"])


def state_corpus():
    from oracle import ROOT

    from goa2.bootstrap import register_all_effects
    from goa2.domain.models import GamePhase, TargetType
    from goa2.engine.setup import GameSetup
    from goa2.engine.steps.selection import SelectStep

    register_all_effects()
    cases = []
    rosters = [
        ("standard", ["Arien", "Hanu"], ["Brogan", "Tigerclaw"]),
        ("tokens-spells-pieces", ["Razzle", "Trinkets"], ["Gydion", "Emmitt"]),
    ]
    for name, red, blue in rosters:
        state = GameSetup.create_game(
            str(ROOT / "src/goa2/data/maps/forgotten_island.json"), red, blue, seed=29
        )
        cases.append(
            {
                "name": name,
                "envelope": export_state(state),
                "expected": state.model_dump(mode="json"),
            }
        )
    state = GameSetup.create_game(
        str(ROOT / "src/goa2/data/maps/forgotten_island.json"),
        ["Arien", "Hanu"],
        ["Brogan", "Tigerclaw"],
        seed=7,
    )
    hero = next(iter(state.teams.values())).heroes[0]
    original = hero.hand[0]
    independent = original.model_copy(deep=True)
    independent.is_facedown = not original.is_facedown
    scenario = original.model_copy(deep=True)
    scenario.range_value = 17
    hero.deck.extend([independent, scenario])
    hero.hand.extend([independent, scenario])
    state.pending_inputs[hero.id] = original
    state.phase = GamePhase.RESOLUTION
    state.execution_stack = [
        SelectStep(
            step_id="fixture/continuation/0",
            target_type=TargetType.NUMBER,
            number_options=[1, 2],
            prompt="Codec continuation",
            output_key="answer",
        )
    ]
    state.execution_context = {
        "target_id": "hero_hanu",
        "pending_input": None,
        "nested": {"card": original},
    }
    cases.append(
        {
            "name": "mid-action-aliases-and-scenario-overrides",
            "envelope": export_state(state),
            "expected": state.model_dump(mode="json"),
        }
    )
    payload = {"schema_version": 1, "oracle_revision": ORACLE_REVISION, "cases": cases}
    return {**payload, "corpus_sha256": digest(payload)}


def main():
    import argparse
    import json
    from pathlib import Path

    from oracle import ROOT, Worker

    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/tests/fixtures/states.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        corpus = state_corpus()
        args.corpus.parent.mkdir(parents=True, exist_ok=True)
        write_text(args.corpus, json.dumps(corpus, separators=(",", ":")) + "\n")
        print(json.dumps({"cases": len(corpus["cases"]), "corpus_sha256": corpus["corpus_sha256"]}))
        return
    verify_sources()
    corpus = json.loads(read_text(args.corpus))
    identity = corpus.pop("corpus_sha256")
    if digest(corpus) != identity:
        raise ValueError("state corpus identity mismatch")
    command = args.worker
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("worker command required after --")
    worker = Worker(command)
    try:
        for case in corpus["cases"]:
            response = worker.request("roundtrip_state", envelope=case["envelope"])
            if response["semantic_state"] != case["expected"]:
                raise AssertionError(f"state roundtrip mismatch: {case['name']}")
            if response["envelope"] != case["envelope"]:
                raise AssertionError(f"instance alias or field mismatch: {case['name']}")
            try:
                worker.request("validate_execution", envelope=case["envelope"])
            except RuntimeError as error:
                if "unsupported native transition" not in str(error):
                    raise
            else:
                raise AssertionError("unsupported native execution was accepted")
    finally:
        worker.close()
    print(
        json.dumps(
            {
                "cases": len(corpus["cases"]),
                "corpus_sha256": identity,
                "semantic_and_alias_roundtrips": True,
            }
        )
    )


if __name__ == "__main__":
    import sys
    from pathlib import Path

    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
    main()
