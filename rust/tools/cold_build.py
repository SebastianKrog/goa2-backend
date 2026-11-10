#!/usr/bin/env python3
"""Periodic native cold acquisition/build rehearsal with empty project caches."""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from extract import extract

ROOT = Path(__file__).resolve().parents[2]


def cold_build(work):
    if work.exists():
        raise FileExistsError("Cold work directory must not already exist")
    work.mkdir(parents=True)
    snapshot = work / "snapshot"
    selected = extract(snapshot)
    cargo_home = work / "cargo"
    target = work / "target"
    assert not cargo_home.exists() and not target.exists()
    environment = {**os.environ, "CARGO_HOME": str(cargo_home), "CARGO_TARGET_DIR": str(target)}
    # The installed pinned compiler is independent of project/dependency caches.
    commands = [
        ["cargo", "fetch", "--locked", "--target", "x86_64-unknown-linux-gnu"],
        ["cargo", "build", "--locked", "--workspace"],
        [sys.executable, "tools/check.py", "quality"],
        [sys.executable, "tools/check.py", "quick"],
        ["cargo", "run", "--locked", "-p", "goa2-core", "--example", "local_game"],
    ]
    records = []
    for index, command in enumerate(commands):
        started = time.monotonic()
        result = subprocess.run(
            command, cwd=snapshot / "rust", env=environment, capture_output=True, text=True
        )
        (work / f"command-{index}.log").write_text(result.stdout + result.stderr)
        if result.returncode:
            raise RuntimeError(f"Cold command {index} failed; inspect {work}/command-{index}.log")
        record = {"argv": command, "exit_code": 0, "seconds": time.monotonic() - started}
        if "quick" in command:
            match = re.search(r'\{"tier": "native", "executed": (\d+),.*', result.stdout)
            if not match or int(match[1]) < 350:
                raise RuntimeError("Cold quick tier selected an unexpectedly reduced suite")
            record["native_tests"] = int(match[1])
        records.append(record)
    crates = list((cargo_home / "registry/cache").glob("*/*.crate"))
    if not crates:
        raise RuntimeError("Cold acquisition retrieved no registry archives")
    return {
        "schema_version": 1,
        "scope": "Native project dependency acquisition/build; full corpus/content distribution still requires its independent route.",
        "source_files": selected["files"],
        "initial_cargo_home_empty": True,
        "initial_target_empty": True,
        "original_git_history_in_snapshot": False,
        "shared_project_caches_used": False,
        "installed_compiler_reused": True,
        "registry_archives": len(crates),
        "registry_archive_bytes": sum(p.stat().st_size for p in crates),
        "commands": records,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    result = cold_build(args.work.resolve())
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
