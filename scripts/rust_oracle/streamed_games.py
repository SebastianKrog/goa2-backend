"""Verify complete source/native games with one live snapshot per side.

Freeze lossless snapshot deltas rather than retaining every expanded game state.
The native worker remains in one session for the entire game, including across
planning and round boundaries. Never starts a server or imports neural models.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import resource
import subprocess
from collections import Counter
from contextlib import suppress
from pathlib import Path
from time import perf_counter

from fixture_cache import read_bytes, write_bytes, write_text
from object_execution_traces import ObjectTraceNormalizer, iter_reference
from oracle import ORACLE_REVISION, ROOT, canonical, verify_sources
from shared_objects import export_state_with_shared_objects
from stack_traces import write_corpus_stream


def snapshot_delta(before, after, path=()):
    if isinstance(before, dict) and isinstance(after, dict):
        for key in before:
            if key not in after:
                yield {"op": "delete", "path": [*path, key]}
        for key, value in after.items():
            if key not in before:
                yield {"op": "set", "path": [*path, key], "value": value}
            else:
                yield from snapshot_delta(before[key], value, (*path, key))
    elif isinstance(before, list) and isinstance(after, list) and len(before) == len(after):
        for index, (left, right) in enumerate(zip(before, after, strict=True)):
            yield from snapshot_delta(left, right, (*path, index))
    elif isinstance(before, list) and isinstance(after, list):
        prefix = 0
        while prefix < min(len(before), len(after)) and before[prefix] == after[prefix]:
            prefix += 1
        suffix = 0
        while (
            suffix < min(len(before), len(after)) - prefix
            and before[-1 - suffix] == after[-1 - suffix]
        ):
            suffix += 1
        yield {
            "op": "splice",
            "path": list(path),
            "index": prefix,
            "delete": len(before) - prefix - suffix,
            "values": after[prefix : len(after) - suffix if suffix else len(after)],
        }
    elif before != after:
        yield {"op": "set", "path": list(path), "value": after}


def apply_snapshot_edits(record, edits):
    """Apply trusted lossless test deltas in place, retaining only one snapshot."""
    for edit in edits:
        target = record
        if edit["op"] == "splice":
            for part in edit["path"]:
                target = target[part]
            index = edit["index"]
            end = index + edit["delete"]
            if not 0 <= index <= end <= len(target):
                raise ValueError("invalid snapshot splice range")
            target[index:end] = edit["values"]
            continue
        if not edit["path"]:
            if edit["op"] != "set":
                raise ValueError("cannot delete snapshot root")
            record = edit["value"]
            continue
        for part in edit["path"][:-1]:
            target = target[part]
        last = edit["path"][-1]
        if edit["op"] == "delete":
            del target[last]
        else:
            target[last] = edit["value"]
    return record


class StreamWorker:
    def __init__(self, command):
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )

    def records(self, envelope, operations):
        return self.operation_records("stack_trace_stream", len(operations),
                                      envelope=envelope, operations=operations)

    def operation_records(self, operation, expected_records, **fields):
        self.process.stdin.write(
            canonical(dict(schema_version=1, id=1, operation=operation, **fields)) + "\n"
        )
        self.process.stdin.flush()
        count = 0
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError(f"stream worker exited early ({self.process.poll()})")
            packet = json.loads(line)
            if packet.get("id") != 1 or packet.get("schema_version") != 1:
                raise ValueError("stream packet identity mismatch")
            if not packet.get("ok"):
                raise RuntimeError(
                    f"native stream failed after {count} operations: {packet.get('error')}"
                )
            result = packet["result"]
            if result.get("complete") is True:
                if result.get("records") != count or count != expected_records:
                    raise ValueError("incomplete replay stream")
                return
            if result.get("sequence") != count or "record" not in result:
                raise ValueError("invalid stream sequence")
            count += 1
            yield result["record"]

    def _close_pipes(self):
        for pipe in (self.process.stdin, self.process.stdout):
            if pipe is not None:
                with suppress(OSError):
                    pipe.close()

    def close(self):
        try:
            if self.process.stdin:
                self.process.stdin.close()
            code = self.process.wait(timeout=10)
        finally:
            self._close_pipes()
        if code:
            raise RuntimeError(f"stream worker exited {code}")

    def abort(self):
        try:
            if self.process.poll() is None:
                self.process.kill()
            self.process.wait()
        finally:
            self._close_pipes()


def verify_case(name, state, operations, command, directory):
    envelope, _ = export_state_with_shared_objects(state)
    normalizer = ObjectTraceNormalizer()
    expected = iter_reference(state, operations, inplace_records=True)
    worker = StreamWorker(command)
    previous = None
    steps_seen, events_seen = Counter(), Counter()
    initial_state = envelope["state"]
    chunks = []
    pending = []
    if directory is not None:
        directory.mkdir(parents=True, exist_ok=True)

    def flush():
        if directory is None or not pending:
            return
        encoded = canonical(pending).encode()
        identity = hashlib.sha256(encoded).hexdigest()
        path = directory / (identity + ".json")
        write_bytes(path, encoded + b"\n")
        chunks.append(
            dict(
                path=str(path.relative_to(directory.parent.parent)),
                records=len(pending),
                sha256=identity,
            )
        )
        pending.clear()

    hasher = hashlib.sha256()
    start = perf_counter()
    try:
        actual = worker.records(envelope, operations)
        for index, (want, raw) in enumerate(zip(expected, actual, strict=True)):
            before = initial_state if previous is None else previous["semantic_state"]
            if operations[index]["kind"] == "advance_prefix" and before["execution_stack"]:
                steps_seen[before["execution_stack"][-1]["type"]] += 1
            for event in want["events"]:
                events_seen[event["event_type"]] += 1
            got = normalizer.record(raw, inplace=True)
            if want != got:
                path = ROOT / "rust/artifacts/complete-game-stream-mismatch.json"
                path.parent.mkdir(parents=True, exist_ok=True)
                write_text(
                    path,
                    json.dumps(
                        dict(
                            name=name,
                            index=index,
                            operation=operations[index],
                            expected=want,
                            actual=got,
                        ),
                        indent=2,
                    )
                    + "\n",
                )
                raise AssertionError(f"{name} operation {index} differs: {path}")
            if directory is not None:
                pending.append(list(snapshot_delta(previous, want)))
                if len(pending) == 64:
                    flush()
            hasher.update(
                json.dumps(want, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
            )
            hasher.update(b"\n")
            previous = want
            if (index + 1) % 1024 == 0:
                print(
                    json.dumps(
                        dict(
                            name=name,
                            verified_operations=index + 1,
                            round=want["semantic_state"]["round"],
                            turn=want["semantic_state"]["turn"],
                        )
                    ),
                    flush=True,
                )
        worker.close()
    except BaseException:
        worker.abort()
        raise
    flush()
    assert previous["semantic_state"]["phase"] == "GAME_OVER"
    facts = dict(
        name=name,
        operations=len(operations),
        round=previous["semantic_state"]["round"],
        turn=previous["semantic_state"]["turn"],
        phase="GAME_OVER",
        step_types_encountered=dict(steps_seen),
        events_emitted=dict(events_seen),
        record_sha256=hasher.hexdigest(),
        seconds=round(perf_counter() - start, 2),
        source_peak_rss_kib=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
    )
    case = dict(name=name, envelope=envelope, operations=operations, expected_delta_chunks=chunks)
    return case, facts


def export(path, command, *, roster="baseline"):
    from live_game_traces import hanu_scenes, scenes

    provider, policy = {
        "baseline": (scenes, "progress-v1"),
        "hanu": (hanu_scenes, "hanu-progress-v1"),
    }[roster]

    manifest = verify_sources()
    facts = []

    def cases():
        for name, state, operations in provider():
            case, result = verify_case(
                name, state, operations, command, path.parent / path.stem / "chunks-v3"
            )
            facts.append(result)
            print(json.dumps(result), flush=True)
            yield case

    index = write_corpus_stream(
        path,
        dict(
            schema_version=1,
            oracle_revision=ORACLE_REVISION,
            source_sha256=manifest["source_sha256"],
            contract="complete-games-snapshot-deltas-v3",
            policy=policy,
            python_hash_seed="0",
        ),
        cases(),
    )
    return index, facts


def verify_compact_corpus(path, command):
    """Replay all stored full-game commands against a native worker, without rerunning Python rules."""
    from stack_traces import read_case, read_index

    manifest = verify_sources()
    index, identity = read_index(path)
    assert index["source_sha256"] == manifest["source_sha256"]
    assert index["contract"] == "complete-games-snapshot-deltas-v3"
    results = []
    start = perf_counter()
    for entry in index["cases"]:
        case = read_case(path, entry)
        worker = StreamWorker(command)
        normalizer = ObjectTraceNormalizer()
        record = None
        count = 0
        hasher = hashlib.sha256()

        def gold(case=case):
            nonlocal record
            for chunk in case["expected_delta_chunks"]:
                raw = read_bytes(path.parent / chunk["path"])
                assert hashlib.sha256(raw.removesuffix(b"\n")).hexdigest() == chunk["sha256"]
                rows = json.loads(raw)
                assert len(rows) == chunk["records"]
                for edits in rows:
                    record = apply_snapshot_edits(record, edits)
                    yield record

        try:
            for count, (want, raw) in enumerate(
                zip(gold(), worker.records(case["envelope"], case["operations"]), strict=True), 1
            ):
                actual = normalizer.record(raw, inplace=True)
                if actual != want:
                    raise AssertionError(
                        f"{case['name']} operation {count-1} differs from compact gold"
                    )
                hasher.update(
                    json.dumps(
                        want, sort_keys=True, separators=(",", ":"), ensure_ascii=False
                    ).encode()
                )
                hasher.update(b"\n")
            worker.close()
        except BaseException:
            worker.abort()
            raise
        assert count == len(case["operations"]) and record["semantic_state"]["phase"] == "GAME_OVER"
        results.append(dict(name=case["name"], operations=count, record_sha256=hasher.hexdigest()))
    return dict(
        cases=len(results),
        operations=sum(r["operations"] for r in results),
        games=results,
        corpus_sha256=identity,
        state_request_event_and_identity_parity=True,
        seconds=round(perf_counter() - start, 2),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--roster", choices=("baseline", "hanu", "mixed"), default="baseline")
    parser.add_argument(
        "--check-only", action="store_true", help="Compare live games without exporting a corpus"
    )
    parser.add_argument(
        "--output", type=Path, help="Defaults to the compact corpus matching --roster"
    )
    args = parser.parse_args()
    if args.check_only and args.output is not None:
        parser.error("--output cannot be used with --check-only")
    if args.roster == "mixed" and not args.check_only:
        parser.error("--roster mixed requires --check-only")
    if args.output is None and not args.check_only:
        name = "complete-games-hanu.json" if args.roster == "hanu" else "complete-games.json"
        args.output = ROOT / "rust/tests/fixtures" / name
    if os.environ.get("PYTHONHASHSEED") != "0":
        parser.error("run with PYTHONHASHSEED=0 for reproducible source option order")
    start = perf_counter()
    if args.check_only:
        from live_game_traces import hanu_scenes, scenes

        verify_sources()
        games = hanu_scenes() if args.roster == "hanu" else scenes(mixed=args.roster == "mixed")
        facts = [
            verify_case(name, state, ops, [args.worker], None)[1]
            for name, state, ops in games
        ]
    else:
        index, facts = export(args.output, [args.worker], roster=args.roster)
    result = dict(
        cases=len(facts),
        operations=sum(c["operations"] for c in facts),
        games=facts,
        state_request_event_and_identity_parity=True,
        seconds=round(perf_counter() - start, 2),
    )
    if not args.check_only:
        result["corpus_sha256"] = index["corpus_sha256"]
        write_text(
            ROOT / f"rust/artifacts/{args.output.stem}-stream-validation.json",
            json.dumps(result, indent=2) + "\n",
        )
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
