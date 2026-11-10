#!/usr/bin/env python3
"""Generate a new admitted synthetic historical archive or verify existing readers/rollback."""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from rust_verification.history import generate, verify  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["generate", "verify"])
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--content", required=True, type=Path)
    parser.add_argument("--fixtures", type=Path, default=ROOT / "rust/tests/compatibility")
    parser.add_argument("--producer-binary", type=Path)
    args = parser.parse_args()
    if args.action == "generate":
        result = generate(args.binary.resolve(), args.content.resolve(), args.fixtures.resolve())
    else:
        result = verify(
            args.binary.resolve(),
            args.content.resolve(),
            args.fixtures.resolve(),
            args.producer_binary.resolve() if args.producer_binary else None,
        )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
