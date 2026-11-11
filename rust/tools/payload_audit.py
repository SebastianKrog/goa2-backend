#!/usr/bin/env python3
"""Inspect the extraction selection without printing potential credentials."""

from __future__ import annotations

import argparse
import gzip
import json
import re
from pathlib import Path

from dependency_audit import selected_paths

ROOT = Path(__file__).resolve().parents[2]
MAX_TEXT_BYTES = 64 * 1024 * 1024
PATTERNS = {
    "personal-unix-path": re.compile(r"/(?:home|Users)/(?!user/|example/|runner/)[A-Za-z0-9_.-]+/"),
    "personal-windows-path": re.compile(
        r"[A-Za-z]:[\\/]+Users[\\/]+(?!user[\\/]|example[\\/])[A-Za-z0-9_. -]+[\\/]"
    ),
    "personal-wsl-share": re.compile(
        r"\\{2,}wsl(?:\.localhost|\$)\\+[^\\\s]+\\+home\\+"
        r"(?!(?:user|example|runner)\\)[^\\\s]+\\+"
    ),
    "github-credential": re.compile(r"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})"),
    "cloud-access-key": re.compile(r"AKIA[0-9A-Z]{16}"),
    "private-key": re.compile(r"-----BEGIN (?:RSA |OPENSSH |EC )?PRIVATE KEY-----"),
}


def audit(root=ROOT):
    selected = selected_paths(
        root, json.loads((root / "rust/extraction-manifest.json").read_text())
    )
    findings = []
    for name in sorted(selected):
        path = root / name
        opener = gzip.open if name.endswith(".gz") else open
        with opener(path, "rb") as stream:
            data = stream.read(MAX_TEXT_BYTES + 1)
        if len(data) > MAX_TEXT_BYTES:
            findings.append({"path": name, "kind": "oversized-content"})
            continue
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            findings.append({"path": name, "kind": "binary-needs-review"})
            continue
        for number, line in enumerate(text.splitlines(), 1):
            for kind, pattern in PATTERNS.items():
                if pattern.search(line):
                    findings.append({"path": name, "line": number, "kind": kind})
    return {
        "schema_version": 1,
        "files": len(selected),
        "findings": findings,
        "scope": "Current extraction selection, including historical guides and compressed text. Reports contain paths/categories/line numbers only. Pattern checks do not establish redistribution rights, baseline provenance or a complete credential/history review.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = audit()
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")
    if report["findings"]:
        raise SystemExit(f"{len(report['findings'])} extraction payload findings require review")


if __name__ == "__main__":
    main()
