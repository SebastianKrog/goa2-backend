"""Compare typed primary catalogs with their frozen numeric predecessors."""

import argparse
import hashlib
import json
from pathlib import Path


CATALOG_VERSIONS = {"primary-effects.json": 2, "primary-builders.json": 3}
LEGACY_OPERANDS = {
    1234567891: {"kind": "primary"},
    1234567892: {"kind": "range"},
    1234567893: {"kind": "radius"},
    1234568102: {"kind": "primary_add", "amount": 2},
    1234568103: {"kind": "radius_or_zero"},
    1234568104: {"kind": "primary_add", "amount": 3},
}


def validate_digest(catalog, name):
    body = {key: value for key, value in catalog.items() if key != "catalog_sha256"}
    encoded = json.dumps(body, separators=(",", ":"), ensure_ascii=False).encode()
    if hashlib.sha256(encoded).hexdigest() != catalog["catalog_sha256"]:
        raise ValueError(f"{name}: catalog digest mismatch")


def compare(old, new, name):
    version = CATALOG_VERSIONS[name]
    if old["schema_version"] != version or new["schema_version"] != version + 1:
        raise ValueError(f"{name}: expected schema versions {version} and {version + 1}")
    if old["source_sha256"] != new["source_sha256"]:
        raise ValueError(f"{name}: pinned source identity changed")
    validate_digest(old, f"{name} legacy")
    validate_digest(new, f"{name} candidate")
    operands = LEGACY_OPERANDS.copy()
    for kind in ("radius", "range"):
        for slot, default in old[f"{kind}_fallbacks"].items():
            slot = int(slot)
            if slot in operands or type(default) is not int:
                raise ValueError(f"{name}: invalid legacy {kind} fallback {slot}")
            operands[slot] = {"kind": f"{kind}_or_default", "default": default}

    converted = 0

    def check(before, after, location):
        nonlocal converted
        if type(before) is int and before in operands:
            before = {"$operand": operands[before]}
            converted += 1
        if type(before) is not type(after):
            raise ValueError(f"{location}: expected {before!r}, got {after!r}")
        if isinstance(before, dict):
            if before.keys() != after.keys():
                raise ValueError(f"{location}: different keys {before.keys() ^ after.keys()}")
            for key, value in before.items():
                check(value, after[key], f"{location}.{key}")
        elif isinstance(before, list):
            if len(before) != len(after):
                raise ValueError(f"{location}: expected {len(before)} entries, got {len(after)}")
            for index, (left, right) in enumerate(zip(before, after, strict=True)):
                check(left, right, f"{location}[{index}]")
        elif before != after:
            raise ValueError(f"{location}: expected {before!r}, got {after!r}")

    metadata = {"schema_version", "catalog_sha256", "source_sha256"}
    before = {key: value for key, value in old.items()
              if key not in metadata | {"radius_fallbacks", "range_fallbacks"}}
    after = {key: value for key, value in new.items() if key not in metadata}
    check(before, after, name)
    return converted


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("legacy", type=Path, help="directory containing the frozen catalogs")
    parser.add_argument("candidate", type=Path, help="directory containing the typed catalogs")
    args = parser.parse_args()
    for name in CATALOG_VERSIONS:
        try:
            count = compare(
                json.loads((args.legacy / name).read_bytes()),
                json.loads((args.candidate / name).read_bytes()),
                name,
            )
        except (KeyError, TypeError, ValueError) as error:
            parser.exit(1, f"{error}\n")
        print(f"{name}: {count} typed operand occurrences verified")


if __name__ == "__main__":
    main()
