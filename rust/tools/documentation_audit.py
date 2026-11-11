#!/usr/bin/env python3
"""Audit retained Markdown links and separately inventory excluded Python-era documents."""

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path
from urllib.parse import unquote

from dependency_audit import selected_paths

ROOT = Path(__file__).resolve().parents[2]


def prose(text):
    lines = []
    fence = None
    for line in text.splitlines(keepends=True):
        match = re.match(r" {0,3}(`{3,}|~{3,})(.*)$", line.rstrip("\r\n"))
        if fence is None:
            if match and (match[1][0] != "`" or "`" not in match[2]):
                fence = match[1]
            else:
                lines.append(line)
        elif (
            match
            and match[1][0] == fence[0]
            and len(match[1]) >= len(fence)
            and not match[2].strip()
        ):
            fence = None
    return "".join(lines)


def links(text):
    text = prose(text)
    return re.findall(r"\[[^\]\n]+\]\(([^\s)]+)\)", text)


def headings(text):
    seen, result = {}, set()
    for title in re.findall(r"^#{1,6} (.+)$", prose(text), re.M):
        title = re.sub(r"[^\w\- ]", "", title.lower()).replace(" ", "-")
        count = seen.get(title, 0)
        seen[title] = count + 1
        result.add(title + (f"-{count}" if count else ""))
    return result


def audit(root=ROOT):
    selected = selected_paths(
        root, json.loads((root / "rust/extraction-manifest.json").read_text())
    )
    rows, failures = [], []
    for name in sorted(selected):
        if not name.endswith(".md"):
            continue
        path = root / name
        text = path.read_text()
        status = "historical-not-onboarding" if "/archive/" in name else "maintained"
        if status.startswith("historical") and not text.startswith("> Historical record"):
            failures.append({"consumer": name, "reason": "Missing historical provenance banner"})
        for link in links(text):
            if ":" in link or link.startswith("//"):
                continue
            target, _, anchor = unquote(link).partition("#")
            resolved = (path.parent / target).resolve() if target else path
            relative = (
                resolved.relative_to(root).as_posix() if resolved.is_relative_to(root) else None
            )
            reason = None
            if relative not in selected:
                reason = "missing-or-excluded-target"
            elif (
                anchor and resolved.suffix == ".md" and anchor not in headings(resolved.read_text())
            ):
                reason = "missing-heading-anchor"
            row = {
                "consumer": name,
                "link": link,
                "target": relative,
                "status": reason or "included",
            }
            rows.append(row)
            if reason:
                failures.append(row)
    return {
        "schema_version": 1,
        "scope": "Inline Markdown links outside fenced code in the extraction selection; local file and Markdown heading targets. Remote URLs are not fetched. Code-path examples are audited separately by maintained command guides.",
        "links": rows,
        "failures": failures,
        "historical_policy": "Archive remains dated provenance, never current acceptance/onboarding. Excluded raw reports are referenced through the retained material inventory; original bytes remain in the source repository.",
    }


def excluded_history(root=ROOT):
    if not (root / ".git").exists():
        return []
    result = []
    names = subprocess.check_output(["git", "ls-files", "docs"], cwd=root, text=True).splitlines()
    for name in names:
        path = root / name
        if path.suffix != ".md":
            continue
        commit = subprocess.check_output(
            ["git", "log", "-1", "--format=%H", "--", name], cwd=root, text=True
        ).strip()
        result.append(
            {
                "path": name,
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "last_change_commit": commit,
                "disposition": "Retain source-repository plans/status, including original Rust and Python/training/hero records, for provenance; exclude from standalone onboarding. The cleanup checklist and current native guides supply maintained status/instructions.",
            }
        )
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--inventory-excluded", action="store_true")
    args = parser.parse_args()
    result = audit()
    if args.inventory_excluded:
        result["excluded_original_documents"] = excluded_history()
        if (ROOT / ".git").exists():
            result["archive_origins"] = []
            for name, source in [
                ("development-history.md", "rust/README.md"),
                ("fixture-history.md", "rust/tests/fixtures/README.md"),
            ]:
                original = subprocess.check_output(["git", "show", "f0fb93e1:" + source], cwd=ROOT)
                provenance = subprocess.check_output(
                    ["git", "log", "-1", "--format=%H %cI", "f0fb93e1", "--", source],
                    cwd=ROOT,
                    text=True,
                ).strip()
                result["archive_origins"].append(
                    {
                        "archive": "rust/docs/archive/" + name,
                        "original_path": source,
                        "baseline_commit": "f0fb93e117f358266ef522e6d5f23d3452e83e97",
                        "original_sha256": hashlib.sha256(original).hexdigest(),
                        "original_last_change_commit_and_date": provenance,
                        "edit_scope": "Historical body retained; stale links redirect to retained inventory or current build instructions. Personal machine paths in development-history.md use generic user placeholders. Original source and its byte hash remain available in Git.",
                    }
                )
    encoded = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")
    if result["failures"]:
        raise SystemExit(f"{len(result['failures'])} retained documentation link failures")


if __name__ == "__main__":
    main()
