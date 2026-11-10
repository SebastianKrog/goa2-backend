"""Lossless fixture cache I/O. Logical .json paths remain stable in indexes."""

from __future__ import annotations

import gzip
import os
import re
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "rust/tests/fixtures"
# Bootstrap examples and corpus indexes stay reviewable; production catalogs live in rust/content/catalogs/.
COMMITTED = {
    "movement.json",
    "states.json",
    "effect-errors.json",
    "mine-reference-errors.json",
    "stack-traces.json",
    "object-execution.json",
    "object-arena.json",
    "complete-games.json",
    "complete-games-hanu.json",
    "cache-identities.json",
    "registry.json",
}


def cached(path):
    path = Path(path).absolute()
    if path.suffix != ".json":
        return False
    if path.is_relative_to(FIXTURES):
        return path.parent != FIXTURES or path.name not in COMMITTED
    return path.is_relative_to(ROOT / "rust/artifacts") and bool(
        re.fullmatch(r"[a-f0-9]{64}\.json", path.name)
    )


def stored_path(path):
    path = Path(path)
    return path.with_suffix(path.suffix + ".gz") if cached(path) else path


def read_bytes(path):
    path = Path(path)
    candidates = [path]
    try:
        relative = path.absolute().relative_to(FIXTURES)
    except ValueError:
        pass
    else:
        if cache := os.environ.get("GOA2_FIXTURE_CACHE"):
            candidates.append(Path(cache) / relative)
        candidates.append(FIXTURES / "smoke" / relative)
    for candidate in candidates:
        try:
            return candidate.read_bytes()
        except FileNotFoundError:
            pass
        try:
            with gzip.open(candidate.with_suffix(candidate.suffix + ".gz"), "rb") as source:
                return source.read()
        except FileNotFoundError:
            pass
    raise FileNotFoundError(
        f"Missing fixture {path}. Regenerate with scripts/rust_oracle/ using the pinned oracle, "
        "or set GOA2_FIXTURE_CACHE to a verified cache; see rust/docs/testing.md."
    )


def read_text(path, encoding="utf-8", errors=None):
    return read_bytes(path).decode(encoding or "utf-8", errors or "strict")


def write_bytes(path, data):
    path = Path(path)
    target = stored_path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    if target == path:
        temporary = path.with_suffix(path.suffix + ".tmp")
        temporary.unlink(missing_ok=True)
        try:
            temporary.write_bytes(data)
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
        return len(data)
    # Unique staging files and atomic publication preserve concurrent readers.
    fd, temporary = tempfile.mkstemp(prefix=target.name + ".", suffix=".tmp", dir=target.parent)
    try:
        with (
            os.fdopen(fd, "wb") as raw,
            gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=6) as out,
        ):
            out.write(data)
        os.replace(temporary, target)
    finally:
        Path(temporary).unlink(missing_ok=True)
    # Only replace the legacy copy after the compressed publication succeeded.
    path.unlink(missing_ok=True)
    return len(data)


def write_text(path, data, encoding="utf-8", errors=None, newline=None):
    if not cached(path):
        # Non-cache metadata/corpus files are often themselves staging files.
        # Preserve their existing call sites' publication and newline behavior.
        return Path(path).write_text(data, encoding=encoding, errors=errors, newline=newline)
    write_bytes(path, data.encode(encoding or "utf-8", errors or "strict"))
    return len(data)


def install_verified_case(source, target, encoded):
    """Reuse a caller-verified immutable gzip case across corpus snapshots."""
    import shutil

    source, target = Path(source), Path(target)
    compressed_source = source.with_suffix(source.suffix + ".gz")
    destination = stored_path(target)
    if source.exists() or not compressed_source.exists() or destination == target:
        return write_bytes(target, encoded + b"\n")
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, stage = tempfile.mkstemp(
        prefix=destination.name + ".", suffix=".tmp", dir=destination.parent
    )
    os.close(fd)
    os.unlink(stage)
    try:
        try:
            os.link(compressed_source, stage)
        except OSError:
            shutil.copyfile(compressed_source, stage)
        os.replace(stage, destination)
        target.unlink(missing_ok=True)
    finally:
        Path(stage).unlink(missing_ok=True)


def compress_existing(*, untrack=False, workers=2):
    """Offline, verified migration. Original bytes survive until gzip verifies."""
    import concurrent.futures
    import hashlib
    import json
    import shutil
    import subprocess

    roots = [FIXTURES, ROOT / "rust/artifacts"]
    paths = [p for base in roots if base.exists() for p in base.rglob("*.json") if cached(p)]
    # Hard-linked corpus snapshots share their compressed inode too. Grouping
    # avoids recompressing identical linked cases in temporary export corpora.
    groups = {}
    for path in paths:
        stat = path.stat()
        groups.setdefault((stat.st_dev, stat.st_ino), []).append(path)

    def migrate(group):
        source, *aliases = group
        target = stored_path(source)
        source_stat = source.stat()
        original = hashlib.sha256()
        fd, stage = tempfile.mkstemp(prefix=target.name + ".", suffix=".tmp", dir=target.parent)
        try:
            with (
                source.open("rb") as inp,
                os.fdopen(fd, "wb") as raw,
                gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=6) as out,
            ):
                while chunk := inp.read(1024 * 1024):
                    original.update(chunk)
                    out.write(chunk)
            verified = hashlib.sha256()
            with gzip.open(stage, "rb") as inp:
                while chunk := inp.read(1024 * 1024):
                    verified.update(chunk)
            if verified.digest() != original.digest():
                raise ValueError(f"Compressed verification failed: {source}")
            current = source.stat()
            if (current.st_ino, current.st_size, current.st_mtime_ns) != (
                source_stat.st_ino,
                source_stat.st_size,
                source_stat.st_mtime_ns,
            ):
                raise ValueError(f"Fixture changed during offline compression: {source}")
            os.replace(stage, target)
            for alias in aliases:
                alias_target = stored_path(alias)
                fd, alias_stage = tempfile.mkstemp(
                    prefix=alias_target.name + ".", suffix=".tmp", dir=alias_target.parent
                )
                os.close(fd)
                os.unlink(alias_stage)
                try:
                    os.link(target, alias_stage)
                except OSError:
                    shutil.copyfile(target, alias_stage)
                os.replace(alias_stage, alias_target)
            # All aliases have a verified replacement before releasing originals.
            for path in group:
                path.unlink()
            return (
                source_stat.st_size,
                target.stat().st_size,
                len(group),
                original.hexdigest(),
                group,
            )
        finally:
            Path(stage).unlink(missing_ok=True)

    before = after = files = 0
    identities = {}
    identity_path = FIXTURES / "cache-identities.json"
    if identity_path.exists():
        identities = json.loads(identity_path.read_text())
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        for index, (raw, packed, count, identity, group) in enumerate(
            pool.map(migrate, groups.values()), 1
        ):
            for path in group:
                if path.parent == FIXTURES:
                    identities[path.name] = {"raw_sha256": identity, "raw_bytes": raw}
            before += raw
            after += packed
            files += count
            if index % 500 == 0:
                print(
                    json.dumps(
                        dict(groups=index, files=files, raw_bytes=before, compressed_bytes=after)
                    ),
                    flush=True,
                )
    if identities:
        temporary = identity_path.with_suffix(".json.tmp")
        temporary.write_text(json.dumps(identities, indent=2) + "\n")
        temporary.replace(identity_path)
    if untrack:
        tracked = subprocess.check_output(
            ["git", "ls-files", "-z", "rust/tests/fixtures"], cwd=ROOT
        ).split(b"\0")
        payloads = [name for name in tracked if name and cached(ROOT / os.fsdecode(name))]
        # This only removes verified cached payloads from the current index.
        # No source changes, commits, refs or historical objects are rewritten.
        for name in payloads:
            if not stored_path(ROOT / os.fsdecode(name)).exists():
                raise FileNotFoundError(f"Refusing to untrack an uncached fixture: {name!r}")
        if payloads:
            subprocess.run(
                ["git", "update-index", "--force-remove", "-z", "--stdin"],
                cwd=ROOT,
                input=b"\0".join(payloads) + b"\0",
                check=True,
            )
    summary = dict(files=files, unique_raw_bytes=before, unique_compressed_bytes=after)
    print(json.dumps(summary, indent=2), flush=True)
    return summary


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["compress"])
    parser.add_argument(
        "--untrack",
        action="store_true",
        help="Remove verified payloads from the current Git index (no history rewrite)",
    )
    parser.add_argument("--workers", type=int, choices=range(1, 5), default=2)
    args = parser.parse_args()
    compress_existing(untrack=args.untrack, workers=args.workers)
