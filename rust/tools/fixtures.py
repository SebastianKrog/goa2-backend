#!/usr/bin/env python3
"""Verify runtime/smoke data, or every selected full-parity case, one at a time."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import sys
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parents[1]
FIXTURES = WORKSPACE / "tests/fixtures"


def canonical(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def checked_path(base, relative):
    path = Path(relative)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"Unsafe fixture path: {relative}")
    return base / path


def raw(path):
    try:
        return path.read_bytes()
    except FileNotFoundError:
        with gzip.open(str(path) + ".gz", "rb") as stream:
            return stream.read()


def reference_identity(value, oracle, description):
    for key in ("oracle_revision", "source_sha256"):
        if value.get(key) != oracle.get(key):
            raise ValueError(f"{description} {key} mismatch")
    if "rules_version" in value and value["rules_version"] != oracle.get("rules_version"):
        raise ValueError(f"{description} rules_version mismatch")


def quick(workspace=WORKSPACE):
    count = 0
    content = workspace / "content"
    manifest = json.loads((content / "manifest.json").read_text())
    oracle = json.loads((workspace / "oracle-manifest.json").read_text())
    actual = {path.name for path in (content / "catalogs").glob("*.json")}
    expected = {Path(row["path"]).name for row in manifest["catalogs"]}
    if actual != expected:
        raise ValueError("Runtime catalog manifest coverage differs from the directory")
    for row in manifest["catalogs"]:
        data = checked_path(content, row["path"]).read_bytes()
        if hashlib.sha256(data).hexdigest() != row["sha256"]:
            raise ValueError(f"Catalog bytes changed: {row['path']}")
        value = json.loads(data)
        if value.get("source_sha256") != oracle["source_sha256"]:
            raise ValueError(f"Catalog oracle identity changed: {row['path']}")
        if value.get("schema_version") != row["schema_version"]:
            raise ValueError(f"Catalog schema changed: {row['path']}")
        count += 1
    sample = json.loads((content / "smoke-manifest.json").read_text())
    reference_identity(sample, oracle, "Smoke content manifest")
    if hashlib.sha256((content / sample["path"]).read_bytes()).hexdigest() != sample["sha256"]:
        raise ValueError("Smoke content checksum changed")
    count += 1
    smoke = workspace / "tests/fixtures/smoke"
    manifest = json.loads((smoke / "manifest.json").read_text())
    if manifest.get("schema_version") != 1:
        raise ValueError("Unsupported smoke manifest schema")
    reference_identity(manifest, oracle, "Smoke manifest")
    expected = {row["path"] + ".gz" for row in manifest["files"]}
    actual = {p.relative_to(smoke).as_posix() for p in smoke.rglob("*.gz")}
    if actual != expected or not expected:
        raise ValueError("Smoke registry is empty or differs from bundled files")
    for row in manifest["files"]:
        data = raw(checked_path(smoke, row["path"]))
        if hashlib.sha256(data).hexdigest() != row["sha256"]:
            raise ValueError(f"Smoke bytes changed: {row['path']}")
        if row.get("case_sha256") and digest(json.loads(data)) != row["case_sha256"]:
            raise ValueError(f"Smoke semantic identity changed: {row['path']}")
        count += 1
    return {"tier": "quick-data", "expected": count, "executed": count, "missing": 0, "failed": 0}


def full(families, cache=None):
    """Do not use smoke fallback: selected parity means every admitted case."""
    result = []
    oracle = json.loads((WORKSPACE / "oracle-manifest.json").read_text())
    for family in families:
        if family not in {
            row["family"]
            for row in json.loads((FIXTURES / "registry.json").read_text())["families"]
        }:
            raise ValueError(f"Unregistered fixture family: {family}")
        record = next(
            row
            for row in json.loads((FIXTURES / "registry.json").read_text())["families"]
            if row["family"] == family
        )
        if record.get("category") == "external-parity":
            candidates = [FIXTURES / record["path"]]
            if cache:
                candidates.append(checked_path(cache, record["path"]))
            data = None
            for candidate in candidates:
                try:
                    data = raw(candidate)
                    break
                except FileNotFoundError:
                    continue
            valid = data is not None and hashlib.sha256(data).hexdigest() == record["raw_sha256"]
            result.append(
                dict(
                    family=family,
                    unit="payload",
                    expected=1,
                    executed=int(valid),
                    missing=int(data is None),
                    failed=int(data is not None and not valid),
                    skipped=0,
                )
            )
            continue
        index_path = FIXTURES / (family + ".json")
        encoded_index = raw(index_path)
        if record.get("sha256") and hashlib.sha256(encoded_index).hexdigest() != record["sha256"]:
            raise ValueError(f"Registry index byte identity mismatch: {family}")
        index = json.loads(encoded_index)
        reference_identity(index, oracle, f"Index {family}")
        identity = index.pop("corpus_sha256")
        if index.get("schema_version") != 2 or digest(index) != identity:
            raise ValueError(f"Index identity mismatch: {family}")
        if index.get("source_sha256") != oracle["source_sha256"]:
            raise ValueError(f"Index oracle mismatch: {family}")
        expected = len(index["cases"])
        if not expected:
            raise ValueError(f"Zero selected cases: {family}")
        if record.get("case_count") is not None and expected != record["case_count"]:
            raise ValueError(f"Registry case count mismatch: {family}")
        seen = set()
        executed = missing = failed = 0
        for row in index["cases"]:
            if row["name"] in seen:
                raise ValueError(f"Duplicate case: {row['name']}")
            seen.add(row["name"])
            if row["path"] != f"{family}/{row['case_sha256']}.json":
                raise ValueError(f"Case path mismatch: {row['path']}")
            candidates = [checked_path(FIXTURES, row["path"])]
            if cache:
                candidates.append(checked_path(cache, row["path"]))
            data = None
            for candidate in candidates:
                try:
                    data = raw(candidate)
                    break
                except FileNotFoundError:
                    continue
            if data is None:
                missing += 1
                continue
            case = json.loads(data)
            if case.get("name") != row["name"] or digest(case) != row["case_sha256"]:
                failed += 1
            else:
                executed += 1
        result.append(
            dict(
                family=family,
                expected=expected,
                executed=executed,
                missing=missing,
                failed=failed,
                skipped=0,
            )
        )
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["verify"])
    parser.add_argument("--full", action="store_true")
    parser.add_argument("--family", action="append")
    parser.add_argument("--cache", type=Path, default=os.environ.get("GOA2_FIXTURE_CACHE"))
    args = parser.parse_args()
    if args.family and not args.full:
        parser.error("--family requires --full")
    reports = (
        full(
            args.family
            or [
                row["family"]
                for row in json.loads((FIXTURES / "registry.json").read_text())["families"]
                if row.get("category") in {"parity-index", "external-parity"}
            ],
            args.cache,
        )
        if args.full
        else [quick()]
    )
    print(json.dumps(reports, indent=2))
    return int(any(report["missing"] or report["failed"] for report in reports))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError) as error:
        sys.exit(f"Fixture verification failed: {error}")
