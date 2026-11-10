#!/usr/bin/env python3
"""Run the pinned maintained scanner; persist package/DB identity and unsuppressed results."""

import argparse
import hashlib
import json
import os
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def review(scanner):
    version = subprocess.check_output([str(scanner), "--version"], text=True).strip()
    if version != "cargo-deny 0.20.2":
        raise ValueError("Use the reviewed cargo-deny 0.20.2 tool")
    result = subprocess.run(
        [
            str(scanner),
            "--manifest-path",
            str(ROOT / "rust/Cargo.toml"),
            "--locked",
            "--format",
            "json",
            "check",
            "advisories",
            "licenses",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    records = []
    for line in (result.stdout + "\n" + result.stderr).splitlines():
        if line.strip():
            records.append(json.loads(line))
    summaries = [row["fields"] for row in records if row.get("type") == "summary"]
    if not summaries:
        raise RuntimeError("Scanner produced no completed scan summary")
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    databases = []
    for path in (cargo_home / "advisory-dbs").glob("advisory-db-*"):
        commit = subprocess.run(
            ["git", "-C", str(path), "rev-parse", "HEAD"], capture_output=True, text=True
        )
        if commit.returncode == 0:
            databases.append(
                {
                    "repository": "https://github.com/rustsec/advisory-db",
                    "commit": commit.stdout.strip(),
                }
            )
    if not databases:
        raise RuntimeError("Advisory database identity missing")
    metadata = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--locked",
                "--manifest-path",
                str(ROOT / "rust/Cargo.toml"),
                "--format-version",
                "1",
            ],
            cwd=ROOT,
            text=True,
        )
    )
    policy = tomllib.loads((ROOT / "rust/deny.toml").read_text())
    report = {
        "schema_version": 1,
        "scanner": version,
        "scanner_sha256": hashlib.sha256(scanner.read_bytes()).hexdigest(),
        "cargo_lock_sha256": hashlib.sha256((ROOT / "rust/Cargo.lock").read_bytes()).hexdigest(),
        "policy_sha256": hashlib.sha256((ROOT / "rust/deny.toml").read_bytes()).hexdigest(),
        "advisory_databases": databases,
        "scope": "Locked all-feature/all-target Cargo graph; license text/policy and RustSec advisories. Not workspace/content rights approval.",
        "package_count": len(metadata["packages"]),
        "summaries": summaries,
        "exit_code": result.returncode,
        "advisory_ignores": policy.get("advisories", {}).get("ignore", []),
        "license_exceptions": policy.get("licenses", {}).get("exceptions", []),
    }
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scanner", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError("Do not replace an admitted scan report")
    result = review(args.scanner.resolve())
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    if result["exit_code"]:
        raise SystemExit(result["exit_code"])


if __name__ == "__main__":
    main()
