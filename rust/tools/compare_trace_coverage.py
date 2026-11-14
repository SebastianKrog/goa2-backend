#!/usr/bin/env python3
"""Compare baseline and shared native coverage logs for one differential gate."""

import argparse
import json
from pathlib import Path

PREFIX = "trace-coverage "


def load(path):
    rows = {}
    passed = False
    with path.open() as stream:
        for line in stream:
            if line.startswith("test result: ok. 1 passed; 0 failed;"):
                passed = True
            _, marker, encoded = line.partition(PREFIX)
            if not marker:
                continue
            row = json.loads(encoded)
            key = (row["index"], row["case"])
            if key in rows:
                raise ValueError(f"Duplicate coverage record in {path}: {key}")
            rows[key] = row
    if not passed or not rows:
        raise ValueError(f"Expected one passing gate with coverage records: {path}")
    return rows


def compare(baseline_path, shared_path, expected=None):
    baseline, shared = load(baseline_path), load(shared_path)
    if baseline.keys() != shared.keys():
        raise ValueError("The runners checked different named cases")
    if expected is not None:
        missing = expected.keys() - baseline.keys()
        unexpected = baseline.keys() - expected.keys()
        if missing or unexpected:
            raise ValueError(
                f"Coverage differs from fixture index: {len(missing)} missing, "
                f"{len(unexpected)} unexpected"
            )
        for key, digest in expected.items():
            if baseline[key]["fixture_sha256"] != digest:
                raise ValueError(f"Coverage payload differs from fixture index: {key}")
    replayed = set()
    for key, old in baseline.items():
        new = shared[key]
        if old["replayed_as"] != old["case"] or new != {**old, "replayed_as": new["replayed_as"]}:
            raise ValueError(f"The runners disagree on original case coverage: {key}")
        proof_key = (new["index"], new["replayed_as"])
        proof = shared.get(proof_key)
        if proof is None or proof["case"] != proof["replayed_as"]:
            raise ValueError(f"Case has no directly verified representative: {key}")
        if proof_key != key and (
            new["replay_key"] is None
            or new["replay_key"] != proof["replay_key"]
            or new["operations"] != proof["operations"]
        ):
            raise ValueError(f"Case differs from its replay representative: {key}")
        replayed.add(proof_key)
    return {
        "named_cases": len(baseline),
        "named_operation_checks": sum(row["operations"] for row in baseline.values()),
        "shared_replays": len(replayed),
        "shared_operation_checks": sum(shared[key]["operations"] for key in replayed),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("shared", type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(args.baseline, args.shared), indent=2))
