"""Offline gold-standard export and differential runner; never starts a server.

Use the checkout's Python environment with PYTHONPATH=src. No model dependency.
The Rust worker is a persistent subprocess, including when launched through WSL.
"""

from __future__ import annotations

from fixture_cache import read_bytes, read_text, write_text

import argparse
import hashlib
import io
import json
import platform
import random
import subprocess
import sys
import tarfile
import time
from concurrent.futures import ThreadPoolExecutor, wait
from importlib.metadata import version
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ORACLE_REVISION = "00becf4a68615afa410f8eabe0c841809af0427b"
MANIFEST = ROOT / "rust/oracle-manifest.json"
SOURCE_ROOTS = ("src/goa2", "src/automata")


def canonical(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def git(*args):
    return subprocess.check_output(["git", "-c", "core.bare=false", *args], cwd=ROOT)


def reference_runtime():
    return {
        "implementation": platform.python_implementation(),
        "python": platform.python_version(),
        "packages": {name: version(name) for name in ("pydantic", "pydantic_core", "jsonpatch")},
    }


def source_manifest():
    hashes = {}
    archive = git("archive", "--format=tar", ORACLE_REVISION, *SOURCE_ROOTS)
    with tarfile.open(fileobj=io.BytesIO(archive)) as source:
        for member in source.getmembers():
            if member.isfile():
                data = source.extractfile(member).read()
                hashes[member.name] = hashlib.sha256(data.replace(b"\r\n", b"\n")).hexdigest()
    hashes = dict(sorted(hashes.items()))
    runtime = reference_runtime()
    return {
        "schema_version": 1,
        "oracle_revision": ORACLE_REVISION,
        "rules_version": 9,
        "source_sha256": digest({"files": hashes, "runtime": runtime}),
        "files": hashes,
        "runtime": runtime,
    }


def verify_sources():
    manifest = json.loads(read_text(MANIFEST))
    if manifest["oracle_revision"] != ORACLE_REVISION or manifest["source_sha256"] != digest(
        {"files": manifest["files"], "runtime": manifest["runtime"]}
    ):
        raise ValueError("oracle manifest identity mismatch")
    if manifest["runtime"] != reference_runtime():
        raise ValueError("oracle runtime drift: use the frozen Python/Pydantic/jsonpatch versions")
    for name, expected in manifest["files"].items():
        actual = hashlib.sha256(read_bytes(ROOT / name).replace(b"\r\n", b"\n")).hexdigest()
        if actual != expected:
            raise ValueError(f"oracle source drift: {name}; explicitly re-pin before exporting")
    # New Python modules can change registries/import discovery as well.
    tracked = set(manifest["files"])
    for source in SOURCE_ROOTS:
        base = ROOT / source
        paths = [base] if base.is_file() else base.rglob("*")
        for path in paths:
            if (
                path.is_file()
                and path.suffix in (".py", ".json")
                and path.relative_to(ROOT).as_posix() not in tracked
            ):
                raise ValueError(f"untracked oracle source: {path}")
    return manifest


def movement_corpus():
    from automata.runtime.clone import clone_state
    from goa2.domain.board import Board
    from goa2.domain.hex import Hex
    from goa2.domain.models import Team, TeamColor
    from goa2.domain.state import GameState
    from goa2.domain.tile import Tile
    from goa2.engine.map_loader import load_map
    from goa2.engine.rules import _validate_movement_path_general, validate_movement_paths

    manifest = verify_sources()
    boards = []
    for map_path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        boards.append((map_path.stem, load_map(str(map_path))))
    for seed in (1, 7, 29):
        rng = random.Random(seed)
        tiles = {}
        for q in range(-4, 5):
            for r in range(-4, 5):
                if abs(q + r) <= 4 and rng.random() >= 0.08:
                    h = Hex(q=q, r=r, s=-q - r)
                    tiles[h] = Tile(hex=h, is_terrain=rng.random() < 0.2)
        boards.append((f"generated-{seed}", Board(tiles=tiles)))
    cases = []
    for index, (name, board) in enumerate(boards):
        rng = random.Random(100 + index)
        for i, tile in enumerate(board.tiles.values()):
            if rng.random() < 0.14:
                tile.occupant_id = f"fixture_obstacle_{i}"
        state = GameState(
            board=board,
            teams={color: Team(color=color, heroes=[], minions=[]) for color in TeamColor},
        )
        optimized = clone_state(state)
        ordered = sorted(board.tiles, key=lambda h: (h.q, h.r, h.s))
        outside = Hex(q=80, r=-80, s=0)
        starts = [ordered[0], ordered[len(ordered) // 2], outside]
        ends = [*ordered, outside, Hex(q=81, r=-81, s=0)]
        queries, expected, stateful_expected = [], [], []
        for start in starts:
            for steps in (0, 1, 4):
                for ignore, through in ((False, False), (True, False), (False, True), (True, True)):
                    kwargs = dict(
                        start=start,
                        max_steps=steps,
                        ignore_obstacles=ignore,
                        pass_through_obstacles=through,
                    )
                    reference = [
                        _validate_movement_path_general(board, end=end, **kwargs) for end in ends
                    ]
                    actual = validate_movement_paths(board, ends=ends, **kwargs)
                    if actual != reference:
                        raise AssertionError(
                            f"Python bare-board batch/BFS disagreement: {name}: {kwargs}"
                        )
                    # Stateful topology has a different off-map contract. Verify
                    # that optimization against its own independent reference.
                    state_reference = [
                        _validate_movement_path_general(board, end=end, state=state, **kwargs)
                        for end in ends
                    ]
                    optimized_actual = validate_movement_paths(
                        optimized.board, ends=ends, state=optimized, **kwargs
                    )
                    if optimized_actual != state_reference:
                        raise AssertionError(
                            f"Python stateful batch/BFS disagreement: {name}: {kwargs}"
                        )
                    queries.append(
                        {
                            **kwargs,
                            "start": start.model_dump(),
                            "ends": [h.model_dump() for h in ends],
                        }
                    )
                    expected.append(reference)
                    stateful_expected.append(state_reference)
        cases.append(
            {
                "name": name,
                "tiles": [
                    {
                        "hex": h.model_dump(),
                        "terrain": board.tiles[h].is_terrain,
                        "occupied": board.tiles[h].occupant_id is not None,
                    }
                    for h in ordered
                ],
                "queries": queries,
                "expected": expected,
                "stateful_expected": stateful_expected,
            }
        )
    payload = {
        "schema_version": 1,
        "oracle_revision": ORACLE_REVISION,
        "source_sha256": manifest["source_sha256"],
        "contract": "plain-board-movement",
        "cases": cases,
    }
    return {**payload, "corpus_sha256": digest(payload)}


class WorkerUnavailable(RuntimeError):
    """Transport or protocol failure. The worker cannot accept another request."""


class Worker:
    def __init__(self, command):
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.counter = 0
        self._retired = False

    def request(self, operation, *, timeout=None, **fields):
        """Send one request. Transport failures retire the worker."""
        if self._retired:
            raise WorkerUnavailable("worker was retired")
        if timeout is not None and not 0 < timeout < float("inf"):
            raise ValueError("worker timeout must be positive and finite")
        self.counter += 1
        request_id = self.counter
        request = {"schema_version": 1, "id": request_id, "operation": operation, **fields}
        packet = canonical(request) + "\n"
        try:
            if timeout is None:
                return self._exchange(request_id, packet)
            with ThreadPoolExecutor(max_workers=1) as executor:
                exchange = executor.submit(self._exchange, request_id, packet)
                if not wait((exchange,), timeout=timeout).done:
                    self._retire()
                    raise TimeoutError(f"worker timed out during {operation}")
                return exchange.result()
        except WorkerUnavailable:
            self._retire()
            raise

    def _retire(self):
        self._retired = True
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait()

    def _exchange(self, request_id, packet):
        try:
            self.process.stdin.write(packet)
            self.process.stdin.flush()
            line = self.process.stdout.readline()
            if not line:
                raise WorkerUnavailable("worker exited without a response")
            response = json.loads(line)
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise WorkerUnavailable("worker response transport failed") from error
        if (
            not isinstance(response, dict)
            or response.get("id") != request_id
            or response.get("schema_version") != 1
            or type(response.get("ok")) is not bool
            or ("result" if response["ok"] else "error") not in response
        ):
            raise WorkerUnavailable("worker response does not match the request protocol")
        if not response["ok"]:
            raise RuntimeError(f"worker rejected request: {response['error']}")
        return response["result"]

    def close(self):
        try:
            self.process.stdin.close()
        except OSError:
            if not self._retired:
                raise
        try:
            code = self.process.wait(timeout=10)
            if code and not self._retired:
                raise RuntimeError(f"worker failed: exit {code}")
        except subprocess.TimeoutExpired:
            self._retire()
            raise
        finally:
            self.process.stdout.close()


def verify_corpus(path, command):
    manifest = verify_sources()
    corpus = json.loads(read_text(path))
    identity = corpus.pop("corpus_sha256")
    if digest(corpus) != identity or corpus["oracle_revision"] != ORACLE_REVISION:
        raise ValueError("corpus identity mismatch")
    if corpus["source_sha256"] != manifest["source_sha256"]:
        raise ValueError("corpus source mismatch")
    worker = Worker(command)
    checked = 0
    started = time.perf_counter()
    try:
        capabilities = worker.request("capabilities")
        assert capabilities["plain_board_movement"] and not capabilities["game_transitions"]
        for case in corpus["cases"]:
            worker.request("load_board", tiles=case["tiles"])
            actual = worker.request("movement", queries=case["queries"])["results"]
            if actual != case["expected"]:
                for index, (got, expected) in enumerate(zip(actual, case["expected"], strict=True)):
                    if got != expected:
                        failure = {
                            "case": case["name"],
                            "tiles": case["tiles"],
                            "query": case["queries"][index],
                            "actual": got,
                            "expected": expected,
                        }
                        failure_path = ROOT / "rust/artifacts/movement-mismatch.json"
                        failure_path.parent.mkdir(parents=True, exist_ok=True)
                        write_text(failure_path, json.dumps(failure, indent=2))
                        raise AssertionError(f"native mismatch saved to {failure_path}")
                raise AssertionError("native result shape mismatch")
            stateful = worker.request("movement", queries=case["queries"], stateful=True)["results"]
            if stateful != case["stateful_expected"]:
                failure_path = ROOT / "rust/artifacts/stateful-movement-mismatch.json"
                failure_path.parent.mkdir(parents=True, exist_ok=True)
                write_text(failure_path, json.dumps({"case": case, "actual": stateful}, indent=2))
                raise AssertionError(f"stateful movement mismatch saved to {failure_path}")
            checked += sum(len(result) for result in actual)
    finally:
        worker.close()
    return {
        "cases": len(corpus["cases"]),
        "destination_checks": checked,
        "stateful_destination_checks": checked,
        "elapsed_seconds_including_transport": time.perf_counter() - started,
        "corpus_sha256": identity,
    }


def main():
    parser = argparse.ArgumentParser(__doc__)
    subs = parser.add_subparsers(dest="mode", required=True)
    subs.add_parser("pin")
    export = subs.add_parser("export-movement")
    export.add_argument("--output", type=Path, default=ROOT / "rust/tests/fixtures/movement.json")
    verify = subs.add_parser("verify-movement")
    verify.add_argument("--corpus", type=Path, default=ROOT / "rust/tests/fixtures/movement.json")
    verify.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "pin":
        write_text(MANIFEST, json.dumps(source_manifest(), indent=2) + "\n")
        verify_sources()
        print(f"Pinned {ORACLE_REVISION}")
    elif args.mode == "export-movement":
        corpus = movement_corpus()
        args.output.parent.mkdir(parents=True, exist_ok=True)
        write_text(args.output, canonical(corpus) + "\n")
        print(json.dumps({"cases": len(corpus["cases"]), "corpus_sha256": corpus["corpus_sha256"]}))
    else:
        command = args.worker
        if command and command[0] == "--":
            command = command[1:]
        if not command:
            parser.error("provide a worker command after --")
        print(json.dumps(verify_corpus(args.corpus, command), indent=2))


if __name__ == "__main__":
    sys.path.insert(0, str(ROOT / "src"))
    main()
