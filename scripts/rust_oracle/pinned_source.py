"""Acquire the frozen source without requiring the original Git object database."""

from __future__ import annotations

import hashlib
import io
import json
import os
import subprocess
import tarfile
from pathlib import Path


def archive_bytes(
    root: Path, manifest: Path, revision: str, source_roots: tuple[str, ...]
) -> bytes:
    source = os.environ.get("GOA2_ORACLE_SOURCE")
    if source:
        data = Path(source).read_bytes()
    else:
        try:
            data = subprocess.check_output(
                ["git", "archive", "--format=tar", revision, *source_roots],
                cwd=root,
                stderr=subprocess.PIPE,
            )
        except subprocess.CalledProcessError as error:
            raise ValueError(
                "Frozen oracle source is unavailable. Set GOA2_ORACLE_SOURCE to a verified "
                "source tar archive for the committed oracle-manifest.json; see rust/docs/content.md. "
                "Do not repin to the current checkout."
            ) from error
    verify_archive(data, json.loads(manifest.read_text()))
    return data


def verify_archive(data: bytes, manifest: dict) -> None:
    """Check every file and reject extras, duplicate members, links and special files."""
    expected = manifest["files"]
    seen = set()
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:*") as archive:
        for member in archive:
            if member.isdir():
                continue
            if not member.isfile() or member.name not in expected or member.name in seen:
                raise ValueError(f"Unexpected oracle archive member: {member.name}")
            # Names are from the admitted manifest, nevertheless never extract outside root.
            if Path(member.name).is_absolute() or ".." in Path(member.name).parts:
                raise ValueError("Unsafe oracle archive path")
            total += member.size
            if total > 64 * 1024 * 1024:
                raise ValueError("Oracle source archive exceeds 64 MiB")
            raw = archive.extractfile(member).read()
            if hashlib.sha256(raw.replace(b"\r\n", b"\n")).hexdigest() != expected[member.name]:
                raise ValueError(f"Oracle source identity mismatch: {member.name}")
            seen.add(member.name)
    if seen != set(expected):
        raise ValueError(f"Oracle source archive is missing {len(set(expected) - seen)} files")


def extract_verified(data: bytes, destination: Path, manifest: dict) -> None:
    verify_archive(data, manifest)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:*") as archive:
        for member in archive:
            if member.isfile():
                path = destination / member.name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(archive.extractfile(member).read())
