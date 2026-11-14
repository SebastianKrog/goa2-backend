#!/usr/bin/env python3
"""Regenerate registered fixture families and verify their admitted identities."""

import argparse
import gzip
import hashlib
import json
import os
import subprocess
from pathlib import Path

from fixtures import FIXTURES, WORKSPACE, full


def regenerate(row, output, python, worker, identity_source, *, generated_identities=False):
    repo = WORKSPACE.parent
    family = row["family"]
    source = repo / row["generator"]
    if hashlib.sha256(source.read_bytes()).hexdigest() != row["generator_sha256"]:
        raise ValueError(f"Generator source hash mismatch for {family}")
    destination = output / family
    destination.mkdir()
    target = destination / row["path"].removesuffix(".gz")
    replacements = {"OUTPUT": str(target), "WORKER": str(worker)}
    argv = [str(python), *(replacements.get(arg, arg) for arg in row["generator_argv"][1:])]
    if family == "cache-identities" and identity_source:
        argv.extend(["--source", str(identity_source)])
    if family == "cache-identities" and generated_identities:
        argv.extend(["--source", str(output), "--family-directories"])
    env = os.environ.copy()
    env.pop("GOA2_FIXTURE_PACK", None)
    env.pop("GOA2_FIXTURE_CACHE", None)
    log_path = destination / "generator.log"
    with log_path.open("x") as log:
        process = subprocess.run(argv, cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT)
    if process.returncode:
        raise ValueError(f"{family} generator exited {process.returncode}. See {log_path}")
    if target.is_file():
        with target.open("rb") as payload:
            actual = hashlib.file_digest(payload, "sha256").hexdigest()
    else:
        with gzip.open(target.with_suffix(target.suffix + ".gz"), "rb") as payload:
            actual = hashlib.file_digest(payload, "sha256").hexdigest()
    expected = row.get("raw_sha256") or row["sha256"]
    if actual != expected:
        raise ValueError(f"Regenerated {family} hash differs: {actual}; expected {expected}")
    if row["category"] == "parity-index":
        checks = full([family], payload_root=destination)
        if any(check["executed"] != check["expected"] for check in checks):
            raise ValueError(f"Regenerated {family} is missing verified cases or chunks: {checks}")
    return {"family": family, "sha256": actual}


def main():
    rows = json.loads((FIXTURES / "registry.json").read_bytes())["families"]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python", type=Path, required=True, help="Pinned oracle Python executable")
    parser.add_argument("--worker", type=Path, help="Native worker for complete-game generators")
    parser.add_argument("--identity-source", type=Path, help="Directory with top-level fixtures")
    parser.add_argument("--family", action="append", choices=[row["family"] for row in rows])
    parser.add_argument("--output", type=Path, required=True, help="New directory for outputs")
    args = parser.parse_args()
    selected = [row for row in rows if not args.family or row["family"] in args.family]
    if not args.worker and any("WORKER" in row["generator_argv"] for row in selected):
        parser.error("--worker is required for complete-game fixture families")
    output = args.output.resolve()
    output.mkdir()
    identity_source = args.identity_source.resolve() if args.identity_source else None
    worker = args.worker.resolve() if args.worker else None
    identities = next((row for row in selected if row["family"] == "cache-identities"), None)
    prerequisites = {row["family"] for row in rows if row["category"] == "external-parity"}
    generated_identities = (
        identities is not None
        and identity_source is None
        and prerequisites <= {row["family"] for row in selected}
    )
    if generated_identities:
        selected.remove(identities)
        selected.append(identities)
    for row in selected:
        print(f"Regenerating {row['family']}", flush=True)
        result = regenerate(
            row, output, args.python.absolute(), worker, identity_source,
            generated_identities=generated_identities,
        )
        print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
