#!/usr/bin/env python3
"""Retain-only audit of repository-owned validation/diagnostic material; never deletes or loads corpus caches."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def checksum(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def inventory(root=ROOT):
    tracked = subprocess.check_output(
        ["git", "ls-files", "rust/artifacts"], cwd=root, text=True
    ).splitlines()
    readers = []
    for name in subprocess.check_output(["git", "ls-files"], cwd=root, text=True).splitlines():
        path = root / name
        if (
            path.suffix in {".py", ".rs", ".md", ".toml", ".sh", ".yml"}
            and not name.startswith("rust/docs/audits/")
            and path.is_file()
        ):
            readers.append((name, path.read_text()))
    milestones = []
    for name in tracked:
        path = root / name
        if path.suffix != ".json":
            continue
        data = json.loads(path.read_text())
        literals, payload_paths = {}, []

        def inspect(value, prefix="$"):
            if isinstance(value, dict):
                for key, item in value.items():
                    location = prefix + "." + key
                    if any(
                        word in key.lower()
                        for word in (
                            "toolchain",
                            "compiler",
                            "verification_scope",
                            "scope",
                            "commit",
                        )
                    ) and isinstance(item, (str, int)):
                        literals[location] = item
                    if (
                        key
                        in {
                            "envelope",
                            "initial_state",
                            "operations",
                            "expected",
                            "queries",
                            "scenario",
                        }
                        and isinstance(item, (dict, list))
                        and item
                    ):
                        payload_paths.append(location)
                    inspect(item, location)
            elif isinstance(value, list):
                for i, item in enumerate(value):
                    inspect(item, f"{prefix}[{i}]")

        inspect(data)
        provenance = subprocess.check_output(
            ["git", "log", "-1", "--format=%H%n%cI", "--", name], cwd=root, text=True
        ).splitlines()
        milestones.append(
            dict(
                path=name,
                sha256=checksum(path),
                bytes=path.stat().st_size,
                last_change_commit=provenance[0],
                last_change_date=provenance[1],
                recorded_context=literals,
                compiler=(
                    "recorded_context"
                    if any("compiler" in k or "toolchain" in k for k in literals)
                    else "not_recorded; do not infer from the current compiler"
                ),
                embedded_regression_candidate_paths=payload_paths,
                references=[
                    filename for filename, text in readers if name in text or path.name in text
                ],
                decision="retain original bytes in original repository; exclude from standalone runtime/public payload until rights/provenance review; metadata inventory remains in snapshot",
                retirement_authorized=False,
            )
        )
    reports = []
    if (root / "rust/artifacts").exists():
        for path in sorted((root / "rust/artifacts").rglob("*")):
            if not path.is_file() or path.is_symlink():
                continue
            name = path.relative_to(root).as_posix()
            reports.append(
                dict(
                    path=name,
                    tracked=name in tracked,
                    bytes=path.stat().st_size,
                    sha256=checksum(path),
                    decision="retain; no deletion or deduplication authorized",
                    inspection="metadata only; ignored cache/private snapshot bodies are not decoded",
                )
            )
    hashes = defaultdict(list)
    for row in reports:
        hashes[row["sha256"]].append(row["path"])
    return dict(
        schema_version=1,
        scope="repository-owned rust/artifacts tree; external /workspace/work and user data are outside deletion/inventory authority",
        milestones=milestones,
        reports=reports,
        byte_duplicate_groups=[names for names in hashes.values() if len(names) > 1],
        deletions=[],
        uncommitted_work_modified=False,
        retention_policy="All material retained. Different bytes or unknown provenance are not grounds for deletion. Unique regression candidates require promotion and coverage review before any future retirement.",
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = inventory()
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                "milestones": len(result["milestones"]),
                "reports": len(result["reports"]),
                "duplicate_groups": len(result["byte_duplicate_groups"]),
                "deletions": 0,
            }
        )
    )


if __name__ == "__main__":
    main()
