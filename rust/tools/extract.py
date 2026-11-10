#!/usr/bin/env python3
"""Rehearse a standalone snapshot without original Git history or sibling repositories."""

import argparse
import hashlib
import json
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "rust/extraction-manifest.json"


def extract(destination):
    manifest = json.loads(MANIFEST.read_text())
    if destination.exists():
        raise FileExistsError(f"Extraction destination already exists: {destination}")
    paths = {}
    for group in manifest["groups"]:
        for pattern in group["paths"]:
            for path in sorted(ROOT.glob(pattern)):
                if not path.is_file() or "__pycache__" in path.parts:
                    continue
                if path.is_symlink():
                    raise ValueError(f"Symlink not admitted to extraction: {path}")
                paths[path.relative_to(ROOT).as_posix()] = group["category"]
    if not paths:
        raise ValueError("Extraction manifest selected no files")
    destination.mkdir(parents=True)
    rows = []
    for relative, category in sorted(paths.items()):
        source = ROOT / relative
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        rows.append(
            {
                "path": relative,
                "origin": relative,
                "category": category,
                "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
            }
        )
    (destination / "README.md").write_text(
        "# GoA2 native workspace\n\nStart with [rust/README.md](rust/README.md). Build from `rust/`.\nThis is a local extraction rehearsal, not a public release. See `source-map.json` for provenance and `rust/docs/readiness.md` for release blockers.\n"
    )
    shutil.copyfile(ROOT / "rust/AGENTS.md", destination / "AGENTS.md")
    (destination / "source-map.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "baseline_backend_commit": manifest["baseline_backend_commit"],
                "plan_commit": manifest["plan_commit"],
                "history": "clean snapshot; original repository not rewritten",
                "files": rows,
            },
            indent=2,
        )
        + "\n"
    )
    return {
        "files": len(rows),
        "destination": str(destination),
        "private_databases": 0,
        "original_git_history_required_for_native_build": False,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    print(json.dumps(extract(args.destination.resolve()), indent=2))


if __name__ == "__main__":
    main()
