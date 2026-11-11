#!/usr/bin/env python3
"""Install a checksum-pinned reference archive into an independent fixture cache."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import tarfile
import tempfile
from pathlib import Path
from urllib.request import urlopen

import fixtures

MAX_ARCHIVE_BYTES = 8 * 1024**3
MAX_MEMBER_BYTES = fixtures.MAX_REFERENCE_BYTES


def admitted_paths():
    records = json.loads((fixtures.FIXTURES / "registry.json").read_text())["families"]
    families = []
    paths = set()
    for record in records:
        if record["category"] == "external-parity":
            families.append(record["family"])
            paths.add(record["path"])
        elif record["category"] == "parity-index":
            families.append(record["family"])
            index = json.loads(fixtures.raw(fixtures.FIXTURES / record["path"]))
            paths.update(row["path"] for row in index["cases"])
    if not families or not paths:
        raise ValueError("No admitted reference payloads")
    return families, paths


def complete_game_paths(payload, paths):
    required = set(paths)
    for name in paths:
        if name.startswith(("complete-games/", "complete-games-hanu/")):
            case = json.loads(fixtures.read_fixture(name, None, payload_root=payload))
            required.update(chunk["path"] for chunk in case["expected_delta_chunks"])
    return required


def archive_digest(incoming, outgoing=None):
    digest = hashlib.sha256()
    count = 0
    while chunk := incoming.read(1024 * 1024):
        count += len(chunk)
        if count > MAX_ARCHIVE_BYTES:
            raise ValueError("Reference archive exceeds the download limit")
        digest.update(chunk)
        if outgoing is not None:
            outgoing.write(chunk)
    return digest.hexdigest()


def copy_archive(source, expected, output):
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("Archive SHA256 must be 64 lowercase hex characters")
    remote = source.startswith("https://")
    if "://" in source and not remote:
        raise ValueError("Remote reference archives must use HTTPS")
    linked = False
    if not remote:
        try:
            os.link(source, output)
            linked = True
        except OSError:
            pass
    if linked:
        with output.open("rb") as incoming:
            actual = archive_digest(incoming)
    else:
        with (
            urlopen(source, timeout=60) if remote else open(source, "rb") as incoming,
            output.open("xb") as outgoing,
        ):
            actual = archive_digest(incoming, outgoing)
    if actual != expected:
        raise ValueError("Reference archive checksum mismatch")
    return linked


def unpack(archive_path, payload, allowed):
    seen = set()
    total = 0
    with tarfile.open(archive_path, mode="r|*") as archive:
        for member in archive:
            name = member.name.removesuffix(".gz")
            fixtures.checked_path(payload, name)
            chunk = re.fullmatch(r"complete-games(?:-hanu)?/chunks-v3/[0-9a-f]{64}\.json", name)
            if (
                not member.isfile()
                or member.issparse()
                or name in seen
                or (name not in allowed and not chunk)
            ):
                raise ValueError(f"Unadmitted or duplicate archive member: {member.name}")
            if member.size < 0 or member.size > MAX_MEMBER_BYTES:
                raise ValueError(f"Reference member exceeds its size limit: {member.name}")
            total += member.size
            if total > MAX_ARCHIVE_BYTES:
                raise ValueError("Expanded reference archive exceeds the size limit")
            seen.add(name)
            target = fixtures.checked_path(payload, member.name)
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as incoming, target.open("xb") as output:
                shutil.copyfileobj(incoming, output, length=1024 * 1024)
    return seen


def install(source, expected, destination):
    destination = destination.absolute()
    destination.parent.mkdir(parents=True, exist_ok=True)
    lock = destination.with_name(destination.name + ".install.lock")
    with lock.open("a") as guard:
        fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(f"Refusing to replace fixture cache: {destination}")
        with tempfile.TemporaryDirectory(prefix=".reference-", dir=destination.parent) as staging:
            staging = Path(staging)
            archive = staging / "reference.tar"
            linked = copy_archive(source, expected, archive)
            payload = staging / "payload"
            payload.mkdir()
            families, allowed = admitted_paths()
            seen = unpack(archive, payload, allowed)
            reports = fixtures.full(families, payload_root=payload)
            if any(row["missing"] or row["failed"] for row in reports):
                raise ValueError("Incomplete or changed reference archive: " + json.dumps(reports))
            if seen != complete_game_paths(payload, allowed):
                raise ValueError("Reference archive contains unreferenced payloads")
            if linked:
                with archive.open("rb") as incoming:
                    if archive_digest(incoming) != expected:
                        raise ValueError("Reference archive changed during installation")
            if destination.exists() or destination.is_symlink():
                raise FileExistsError(f"Refusing to replace fixture cache: {destination}")
            payload.rename(destination)
    return {
        "sha256": expected,
        "files": len(seen),
        "reports": reports,
        "source_hardlinked": linked,
    }


def pack(source, destination):
    if destination.exists() or destination.is_symlink():
        raise FileExistsError(f"Refusing to replace reference archive: {destination}")
    families, allowed = admitted_paths()
    reports = fixtures.full(families, payload_root=source)
    if any(row["missing"] or row["failed"] for row in reports):
        raise ValueError("Cannot package incomplete or changed references: " + json.dumps(reports))
    names = complete_game_paths(source, allowed)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".reference-pack-", dir=destination.parent) as staging:
        stage = Path(staging) / "reference.tar"
        with tarfile.open(stage, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for name in sorted(names):
                path = fixtures.checked_path(source, name)
                if not path.is_file():
                    name += ".gz"
                    path = fixtures.checked_path(source, name)
                with path.open("rb") as incoming:
                    info = tarfile.TarInfo(name)
                    info.size = os.fstat(incoming.fileno()).st_size
                    info.mode = 0o600
                    archive.addfile(info, incoming)
        with stage.open("rb") as stream:
            identity = hashlib.file_digest(stream, "sha256").hexdigest()
        os.link(stage, destination)
    return {"sha256": identity, "files": len(names), "reports": reports}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    acquire = modes.add_parser("install", help="Verify and atomically install a complete archive")
    acquire.add_argument("source", help="Local tar archive or HTTPS URL")
    acquire.add_argument("--sha256", required=True, help="Independently trusted archive checksum")
    acquire.add_argument("--output", type=Path, required=True, help="New immutable cache directory")
    create = modes.add_parser(
        "pack", help="Package every admitted payload with deterministic metadata"
    )
    create.add_argument(
        "--source", type=Path, required=True, help="Complete source payload directory"
    )
    create.add_argument("--output", type=Path, required=True, help="New tar archive")
    args = parser.parse_args()
    result = (
        install(args.source, args.sha256, args.output)
        if args.mode == "install"
        else pack(args.source, args.output)
    )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, tarfile.TarError) as error:
        raise SystemExit(str(error)) from None
