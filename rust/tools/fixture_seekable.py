#!/usr/bin/env python3
"""Build the private seekable fixture pack from a checksum-pinned legacy tar."""

import argparse
import fcntl
import gzip
import hashlib
import io
import json
import os
import re
import struct
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

import fixture_archive
import fixtures
from fixture_pack import FRAME_BYTES, MAX_PAYLOAD_BYTES


class Frames:
    def __init__(self, output, zstd):
        self.output = output
        self.compress = zstd.compress
        parameter = zstd.CompressionParameter
        self.options = {
            parameter.compression_level: 3,
            parameter.window_log: 22,
            parameter.enable_long_distance_matching: 1,
            parameter.checksum_flag: 1,
        }
        self.buffer = bytearray()
        self.frames = []

    def write(self, data):
        rest = memoryview(data)
        while rest:
            size = min(FRAME_BYTES - len(self.buffer), len(rest))
            self.buffer.extend(rest[:size])
            rest = rest[size:]
            if len(self.buffer) == FRAME_BYTES:
                self.flush_frame()
        return len(data)

    def flush_frame(self):
        if self.buffer:
            compressed = self.compress(self.buffer, options=self.options)
            self.output.write(compressed)
            self.frames.append((len(compressed), len(self.buffer)))
            self.buffer.clear()

    def finish(self):
        self.flush_frame()
        size = 8 * len(self.frames) + 9
        self.output.write(struct.pack("<II", 0x184D2A5E, size))
        for frame in self.frames:
            self.output.write(struct.pack("<II", *frame))
        self.output.write(struct.pack("<IBI", len(self.frames), 0, 0x8F92EAB1))


def indexed_order():
    names = []
    for index_name in (
        "object-execution.json",
        "stack-traces.json",
        "complete-games.json",
        "complete-games-hanu.json",
    ):
        index = json.loads(fixtures.raw(fixtures.FIXTURES / index_name))
        names.extend(row["path"] for row in index["cases"])
    return names


def build(source, sha256, destination, zstd, *, deduplicate=False):
    destination = destination.absolute()
    destination.parent.mkdir(parents=True, exist_ok=True)
    lock = destination.with_name(destination.name + ".install.lock")
    with lock.open("a") as guard:
        fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(f"Refusing to replace fixture pack: {destination}")
        with tempfile.TemporaryDirectory(prefix=".seekable-", dir=destination.parent) as temporary:
            stage = Path(temporary)
            pack = stage / "pack"
            pack.mkdir()
            with source.open("rb") as raw:
                if fixture_archive.archive_digest(raw) != sha256:
                    raise ValueError("Reference archive checksum mismatch")
                raw.seek(0)
                with tarfile.open(fileobj=raw, mode="r:") as original:
                    members = {}
                    for member in original:
                        name = member.name.removesuffix(".gz")
                        fixtures.checked_path(pack, name)
                        if not member.isfile() or member.issparse() or name in members:
                            raise ValueError(f"Unexpected or duplicate reference member: {name}")
                        members[name] = member

                    def payload(name):
                        with original.extractfile(members[name]) as encoded:
                            if members[name].name.endswith(".gz"):
                                with gzip.GzipFile(fileobj=encoded) as decoded:
                                    data = decoded.read(MAX_PAYLOAD_BYTES + 1)
                            else:
                                data = encoded.read(MAX_PAYLOAD_BYTES + 1)
                        if len(data) > MAX_PAYLOAD_BYTES:
                            raise ValueError(f"Fixture exceeds expanded size limit: {name}")
                        return data

                    _, required = fixture_archive.admitted_paths()
                    order = []
                    for name in indexed_order():
                        order.append(name)
                        if name.startswith(("complete-games/", "complete-games-hanu/")):
                            chunks = json.loads(payload(name))["expected_delta_chunks"]
                            for chunk in chunks:
                                required.add(chunk["path"])
                                order.append(chunk["path"])
                    if set(members) != required:
                        raise ValueError("Archive membership differs from admitted fixture paths")
                    order = list(dict.fromkeys(order + sorted(required)))
                    index = {}
                    aliases = {}
                    shared = {}
                    case_name = re.compile(rb'\A\{"name":"(?:[^"\\]|\\.)*",')
                    archive_path = pack / "fixtures-4MiB.tar.zst"
                    with archive_path.open("xb") as output:
                        frames = Frames(output, zstd)
                        with tarfile.open(
                            fileobj=frames, mode="w|", format=tarfile.PAX_FORMAT
                        ) as tar:
                            for name in order:
                                data = payload(name)
                                checksum = hashlib.sha256(data).hexdigest()
                                match = case_name.match(data) if deduplicate else None
                                if match:
                                    prefix = match.group().decode("utf-8")
                                    key = hashlib.sha256(memoryview(data)[match.end() :]).digest()
                                    if key in shared:
                                        representative, original_prefix = shared[key]
                                        start, length, _ = index[representative]
                                        index[name] = [start, length, checksum]
                                        aliases[name] = [representative, original_prefix, prefix]
                                        continue
                                    shared[key] = (name, prefix)
                                entry = tarfile.TarInfo(name)
                                entry.size = len(data)
                                entry.mode = 0o600
                                offset = tar.offset + len(entry.tobuf(format=tarfile.PAX_FORMAT))
                                index[name] = [offset, len(data), checksum]
                                tar.addfile(entry, io.BytesIO(data))
                        frames.finish()
                raw.seek(0)
                if fixture_archive.archive_digest(raw) != sha256:
                    raise ValueError("Reference archive changed during packaging")

            index_path = pack / "fixtures.index.json"
            index_data = {"schema": 2 if deduplicate else 1, "fixtures": index}
            if deduplicate:
                index_data["aliases"] = aliases
            index_path.write_text(json.dumps(index_data, separators=(",", ":")))
            subprocess.run(
                [sys.executable, str(Path(fixtures.__file__)), "verify", "--full"],
                env={**os.environ, "GOA2_FIXTURE_PACK": str(pack)},
                check=True,
            )
            oracle = json.loads((fixtures.WORKSPACE / "oracle-manifest.json").read_bytes())
            manifest = {
                "schema": 1,
                "source_archive_sha256": sha256,
                "files": len(index),
                "shared_payloads": len(aliases),
                "oracle_revision": oracle["oracle_revision"],
                "source_sha256": oracle["source_sha256"],
            }
            for path in (archive_path, index_path):
                with path.open("rb") as stream:
                    manifest[path.name] = hashlib.file_digest(stream, "sha256").hexdigest()
            (pack / "manifest.json").write_text(json.dumps(manifest, indent=2) + chr(10))
            pack.rename(destination)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--deduplicate", action="store_true")
    args = parser.parse_args()
    try:
        from compression import zstd
    except ImportError:
        parser.error("Packing requires Python 3.14 with the standard-library zstd module")
    print(
        json.dumps(
            build(args.source, args.sha256, args.output, zstd, deduplicate=args.deduplicate),
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
