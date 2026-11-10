#!/usr/bin/env python3
"""Install a game-content pack with an independently supplied SHA256 identity."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from pathlib import Path
from urllib.request import urlopen

WORKSPACE = Path(__file__).resolve().parents[1]


def install(source: str, expected: str, destination: Path):
    if len(expected) != 64 or any(c not in "0123456789abcdef" for c in expected):
        raise ValueError("Expected SHA256 must be 64 lowercase hex characters")
    if destination.exists():
        raise FileExistsError(f"Refusing to replace {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    if "://" in source and not source.startswith("https://"):
        raise ValueError("Remote content must use HTTPS")
    fd, stage_name = tempfile.mkstemp(prefix=".content-", dir=destination.parent)
    stage = Path(stage_name)
    try:
        with (
            os.fdopen(fd, "wb") as out,
            (
                urlopen(source, timeout=30) if source.startswith("https://") else open(source, "rb")
            ) as inp,
        ):
            digest = hashlib.sha256()
            count = 0
            while chunk := inp.read(1024 * 1024):
                count += len(chunk)
                if count > 64 * 1024 * 1024:
                    raise ValueError("Content pack exceeds 64 MiB")
                digest.update(chunk)
                out.write(chunk)
        if digest.hexdigest() != expected:
            raise ValueError("Content checksum mismatch; destination left untouched")
        data = json.loads(stage.read_bytes())
        oracle = json.loads((WORKSPACE / "oracle-manifest.json").read_text())
        if data.get("version") != 1 or any(
            data.get(k) != oracle[k] for k in ("oracle_revision", "source_sha256")
        ):
            raise ValueError("Unsupported game-content version or oracle identity")
        os.link(
            stage, destination
        )  # Atomic publication without overwriting a concurrent installer.
        return {"sha256": expected, "bytes": count, "path": str(destination)}
    finally:
        stage.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "source", help="Local file or HTTPS URL; identity must come from a trusted release"
    )
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(install(args.source, args.sha256, args.output), indent=2))


if __name__ == "__main__":
    main()
