"""Replay a persisted Rust arena game and verify it with the Python oracle."""

import argparse
import json
from collections import Counter
from copy import deepcopy
from enum import Enum
from pathlib import Path
from time import perf_counter

from arena_choices import CHOICE_ENCODING, decision_hash, python_legal_choices
from arena_reports import load_report, save_verification, sha256_file
from oracle import digest, verify_sources

VERIFIER_VERSION = 4


class ReplayMismatch(ValueError):
    def __init__(self, message, **details):
        super().__init__(message)
        self.details = details


class ReplayUnsupported(ValueError):
    pass


def restore_python_start(envelope):
    from pydantic import BaseModel, TypeAdapter

    from goa2.bootstrap import register_all_effects
    from goa2.domain.hex import Hex
    from goa2.domain.models import Card, Token
    from goa2.domain.models.spell import SpellCard
    from goa2.domain.state import GameState
    from goa2.engine.step_types import AnyStep

    from shared_objects import instance_bindings
    from state_codec import REFERENCE_KEY, materialize

    model_fields = {}
    mapping_keys = {}

    def slot(container, part):
        if isinstance(container, BaseModel):
            model = type(container)
            names = model_fields.get(model)
            if names is None:
                names = {}
                for name, info in model.model_fields.items():
                    names.setdefault(info.serialization_alias or name, name)
                model_fields[model] = names
            if part in names:
                return names[part]
            raise ReplayUnsupported(f"cannot restore Python model field {part!r}")
        if isinstance(container, dict) and part not in container:
            identity = id(container)
            entry = mapping_keys.get(identity)
            if entry is None:
                names = {}
                for key in container:
                    names.setdefault(str(key.value if isinstance(key, Enum) else key), key)
                # Keep detached containers alive so their IDs cannot be reused.
                entry = mapping_keys[identity] = (container, names)
            if part in entry[1]:
                return entry[1][part]
            raise ReplayUnsupported(f"cannot restore Python mapping key {part!r}")
        return part

    def get(container, key):
        return getattr(container, key) if isinstance(container, BaseModel) else container[key]

    def resolve(path):
        value = state
        for part in path:
            value = get(value, slot(value, part))
        return value

    def contains_model(value):
        if isinstance(value, BaseModel):
            return True
        if isinstance(value, dict):
            return any(contains_model(child) for child in value.values())
        if isinstance(value, (list, tuple)):
            return any(contains_model(child) for child in value)
        return False

    register_all_effects()
    semantic_state = materialize(envelope)
    # Pydantic Any fields can retain their input containers. Keep the
    # expected wire state independent of the alias rewrites below.
    state = GameState.model_validate(deepcopy(semantic_state))
    bindings = envelope.get("object_bindings", [])
    # The wire codec marks Hex models as shared during deepcopy.
    groups = [("hex" if binding.get("deepcopy_shared") else binding["kind"], binding["paths"])
              for binding in bindings]
    groups.extend(("physical", paths) for paths in instance_bindings(envelope["state"]))
    step_adapter = None
    for kind, paths in groups:
        if len(paths) < 2 and kind not in ("hex", "step", "physical"):
            continue
        if kind == "physical":
            # The save-file validators can merge distinct cards by printed ID.
            # Rebuild one model per physical wire instance instead.
            reference, raw = envelope["state"], semantic_state
            for part in paths[0]:
                reference, raw = reference[part], raw[part]
            instance = envelope["instances"][reference[REFERENCE_KEY]]
            definition = envelope["definitions"][instance["definition"]]
            model = {"card": Card, "spell": SpellCard, "token": Token}.get(definition["kind"])
            if model is None:
                raise ReplayUnsupported("unknown Python physical instance kind")
            original = model.model_validate(deepcopy(raw))
            if equality := instance.get("equality"):
                model_name = f"{type(original).__module__}.{type(original).__qualname__}"
                if (not isinstance(original, Card) or model_name != equality["model"]
                        or not equality["json_metadata"]):
                    raise ReplayUnsupported("cannot restore Python card equality from the snapshot")
                original._definition_revision = equality["definition_revision"]
        else:
            values = [resolve(path) for path in paths]
            # Context and other Any fields deserialize as dictionaries. Keep
            # the typed peer, including typed values in shared containers.
            original = next((value for value in values if contains_model(value)), values[0])
            if isinstance(original, dict) and kind == "hex":
                original = Hex.model_validate(original)
            if isinstance(original, dict) and kind == "step":
                if step_adapter is None:
                    step_adapter = TypeAdapter(AnyStep)
                original = step_adapter.validate_python(original)
        for path in paths:
            parent = resolve(path[:-1])
            key = slot(parent, path[-1])
            other = get(parent, key)
            if original is other:
                continue
            if type(other) is not type(original) and not (
                isinstance(original, BaseModel) and isinstance(other, dict)
            ):
                raise ReplayUnsupported(f"incompatible aliases at {paths[0]!r} and {path!r}")
            if isinstance(parent, BaseModel):
                object.__setattr__(parent, key, original)
            else:
                parent[key] = original
    if "board_keys" in envelope:
        tiles = {}
        for row in envelope["board_keys"]:
            point = Hex.model_validate(row["hex"])
            key = resolve(row["path"]) if row["path"] is not None else point
            if key != point:
                raise ReplayUnsupported("incompatible Python board key binding")
            tiles[key] = state.board.tiles[point]
        state.board.tiles.clear()
        state.board.tiles.update(tiles)
    if any(resolve(path) is not resolve(paths[0]) for _, paths in groups for path in paths[1:]):
        raise ReplayUnsupported("Python could not restore all starting aliases")
    from rollout_parity import describe_state_difference, normalized_state

    expected = normalized_state(semantic_state)
    actual = normalized_state(state.model_dump(mode="json"))
    if expected != actual:
        raise ReplayUnsupported("Python start changes during model restore: "
                                + describe_state_difference(expected, actual))
    return state, tuple(resolve(binding["paths"][0]) for binding in bindings)


class ReplayProgress:
    def __init__(self, root):
        self.start_turn = root.round * 4 + root.turn
        self.waves = dict(root.wave_counters)
        self.zones = dict(root.battle_zones)
        self.lane_order = {lane: {zone: index for index, zone in enumerate(zones)}
                           for lane, zones in root.board.lanes.items()}
        self.advances = {lane: Counter() for lane in self.lane_order}
        self.units = {
            unit.id: (color.value, kind)
            for color, team in root.teams.items()
            for kind, units in (("hero", team.heroes), ("minion", team.minions))
            for unit in units
        }
        self.defeats = {side: Counter() for side in ("RED", "BLUE")}
        self.planning_boundaries = Counter()
        self.hero_presence = {lane: Counter() for lane in self.lane_order}
        self.minion_presence = {lane: Counter() for lane in self.lane_order}

    def observe_planning(self, state):
        for lane, zone_id in state.battle_zones.items():
            zone = state.board.zones.get(zone_id)
            if zone is None or lane not in self.lane_order:
                continue
            self.planning_boundaries[lane] += 1
            for side, team in state.teams.items():
                self.hero_presence[lane][side.value] += any(
                    position in zone.hexes for hero in team.heroes
                    for position in state.get_positions(hero.id)
                )
                self.minion_presence[lane][side.value] += sum(
                    state.get_position(minion.id) in zone.hexes for minion in team.minions
                )

    def observe(self, state, events):
        for lane, zone in state.battle_zones.items():
            previous = self.zones.get(lane)
            order = self.lane_order.get(lane, {})
            if previous != zone and previous in order and zone in order:
                side = "RED" if order[zone] > order[previous] else "BLUE"
                self.advances[lane][side] += 1
            self.zones[lane] = zone
        for event in events:
            if event.event_type.value == "UNIT_DEFEATED":
                side, kind = self.units.get(event.target_id, ("unknown", "unit"))
                self.defeats.setdefault(side, Counter())[kind] += 1

    def result(self, state):
        return {
            "turns_elapsed": state.round * 4 + state.turn - self.start_turn,
            "victory_condition": state.victory_condition,
            "battle_zone_advances": {lane: dict(counts) for lane, counts in self.advances.items()},
            "wave_counters_spent": {lane: before - state.wave_counters[lane]
                                    for lane, before in self.waves.items()},
            "defeats_suffered": {side: dict(counts) for side, counts in self.defeats.items()},
            "planning_lane_presence": {
                lane: {"samples": count, "hero_present": dict(self.hero_presence[lane]),
                       "minion_count_sum": dict(self.minion_presence[lane])}
                for lane, count in self.planning_boundaries.items()
            },
        }


def verified_oracle():
    if not __debug__:
        raise ReplayUnsupported("run Python without -O to enable oracle validation")
    try:
        return verify_sources()
    except (ValueError, OSError) as error:
        raise ReplayUnsupported(str(error)) from error


def verify_report(report, envelope, referee):
    from automata.runtime.legality import _UnverifiableInput
    from goa2.engine.session import GameSession

    from object_execution_traces import ObjectTraceNormalizer
    from rollout_parity import describe_state_difference, normalized_state, run, trace_policy
    from shared_objects import export_state_with_shared_objects
    from streamed_games import StreamWorker

    if report["choice_encoding"] != CHOICE_ENCODING:
        raise ReplayUnsupported("unknown arena choice encoding")
    if any(envelope.get(key) != value for key, value in report["rules"].items()):
        raise ReplayMismatch("report rules identity differs from the stored start")
    manifest = verified_oracle()
    try:
        referee_hash = sha256_file(referee)
    except (ValueError, OSError) as error:
        raise ReplayUnsupported(str(error)) from error
    if any(envelope[key] != manifest[key] for key in ("oracle_revision", "source_sha256", "rules_version")):
        raise ReplayUnsupported("the pinned Python oracle is unavailable")
    if referee_hash != report["binaries"]["referee"]:
        raise ReplayUnsupported("the pinned Rust referee is unavailable")

    start_time = perf_counter()
    try:
        root, original_objects = restore_python_start(envelope)
    except (KeyError, TypeError, ValueError) as error:
        raise ReplayUnsupported(str(error)) from error
    restore_seconds = perf_counter() - start_time
    progress = ReplayProgress(root)
    trace = report["trace"]
    turns = report["parameters"]["turns"]
    session = GameSession(root, create_rollback_snapshots=False)
    replay_choice = trace_policy(root.board.map_id, trace, live_session=session)
    worker = StreamWorker([str(referee.resolve())])
    stream = worker.operation_records("arena_replay", len(trace) + 1,
                                      envelope=envelope, turns=turns, choices=trace)
    stream_seconds = 0
    hash_seconds = 0

    def native_record():
        nonlocal stream_seconds
        started = perf_counter()
        try:
            return next(stream)
        finally:
            stream_seconds += perf_counter() - started

    def check_choice(state, prompt, index):
        nonlocal hash_seconds
        if index >= len(trace):
            raise ReplayMismatch("Python issued an extra decision", decision=index)
        native = native_record()
        expected = trace[index]["legal_choices_sha256"]
        started = perf_counter()
        python_choices = python_legal_choices(state, prompt)
        rust_hash = decision_hash(native["legal_choices"])
        python_hash = decision_hash(python_choices)
        hash_seconds += perf_counter() - started
        if native["recorded_sha256"] != expected or rust_hash != expected or python_hash != expected:
            raise ReplayMismatch(
                "legal choices differ", decision=index, expected_sha256=expected,
                rust_sha256=rust_hash, python_sha256=python_hash,
                rust_choices=native["legal_choices"], python_choices=python_choices,
            )
        if prompt is None:
            progress.observe_planning(state)
            offered = {player["player"]: player["moves"] for player in python_choices["players"]}
            if any([row["first"], row["second"]] not in offered.get(row["hero"], ())
                   for row in trace[index].get("commitments", ())):
                raise ReplayMismatch("Python rejected a recorded commitment", decision=index,
                                     choice=trace[index])
        try:
            return replay_choice(state, prompt, index)
        except _UnverifiableInput as error:
            raise ReplayUnsupported(f"decision {index}: {error}") from error
        except (AssertionError, StopIteration, ValueError) as error:
            raise ReplayMismatch("Python rejected a recorded choice", decision=index,
                                 choice=trace[index], oracle_detail=str(error)) from error

    try:
        started = perf_counter()
        final, count = run(root, turns, copy_state=False, policy=check_choice, exact_planning=True,
                           observe_events=progress.observe)
        python_seconds = perf_counter() - started - stream_seconds - hash_seconds
        if count != len(trace):
            raise ReplayMismatch("Python ended before the recorded endpoint", decision=count)
        endpoint = native_record()
        if next(stream, None) is not None:
            raise ReplayMismatch("Rust returned records after the endpoint")
        worker.close()
    except (AssertionError, RuntimeError, StopIteration) as error:
        worker.abort()
        if "Unsupported(" in str(error):
            raise ReplayUnsupported(str(error)) from error
        raise ReplayMismatch(str(error)) from error
    except BaseException:
        worker.abort()
        raise

    native_state = endpoint.pop("state")
    endpoint["rust_state_sha256"] = digest(native_state)
    if endpoint != report["endpoint"]:
        raise ReplayMismatch("Rust replay endpoint differs from the report",
                             expected=report["endpoint"], actual=endpoint)
    for key, value in (
        ("round", final.round), ("turn", final.turn), ("phase", final.phase.value),
        ("winner_team", final.winner.value if final.winner else None),
        ("winner_hero", final.individual_winner_id),
    ):
        if endpoint[key] != value:
            raise ReplayMismatch(f"Python endpoint differs at {key}", rust=endpoint[key], python=value)

    expected = normalized_state(final.model_dump(mode="json"))
    actual = normalized_state(native_state)
    if expected != actual:
        # Ignore process-local IDs for typed, generated effect steps at the cutoff.
        final_envelope, final_objects = export_state_with_shared_objects(final)
        bindings = final_envelope.get("object_bindings", [])
        normalizer = ObjectTraceNormalizer()
        normalizer.bind_originals(envelope.get("object_bindings", []), original_objects)
        expected = normalizer.record(
            dict(semantic_state=expected, object_bindings=bindings, request=None), final_objects,
        )["semantic_state"]
        actual = ObjectTraceNormalizer().record(
            dict(semantic_state=actual, object_bindings=bindings, request=None),
        )["semantic_state"]
    if expected != actual:
        raise ReplayMismatch(describe_state_difference(expected, actual))
    return {"decisions": count, "phase": endpoint["phase"], "progress": progress.result(final),
            "timing_seconds": {"restore_python": restore_seconds, "python_replay": python_seconds,
                               "project_and_hash": hash_seconds, "rust_stream_wait": stream_seconds}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--store", type=Path, required=True)
    parser.add_argument("--report-id", required=True)
    parser.add_argument("--referee", type=Path, required=True)
    args = parser.parse_args()

    try:
        verified_oracle()
    except ReplayUnsupported as error:
        parser.error(str(error))

    # Validate the identity before writing a verification result at its path.
    result = {"verifier_version": VERIFIER_VERSION, "report_id": args.report_id}
    try:
        report, envelope = load_report(args.store, args.report_id)
        result.update(status="pass", **verify_report(report, envelope, args.referee))
    except ReplayUnsupported as error:
        result.update(status="unsupported", reason=str(error))
    except ReplayMismatch as error:
        result.update(status="mismatch", reason=str(error), **error.details)
    except (ValueError, OSError) as error:
        result.update(status="invalid_report", reason=str(error))
    try:
        save_verification(args.store, args.report_id, result)
    except ValueError as error:
        parser.error(str(error))
    print(json.dumps(result, separators=(",", ":")))
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
