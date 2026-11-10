#!/usr/bin/env python3
"""Run the bundled synthetic manual/bot/clock/upgrade/restart/replay baseline set."""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))


def main():
    from rust_verification.baselines import run

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads((ROOT / "rust/tests/baselines/manifest.json").read_text())
    result = run(args.binary.resolve(), ROOT / "rust/content/smoke-game.json", manifest)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                "cases": len(result["cases"]),
                "passed": sum(r["status"] == "passed" for r in result["cases"]),
                "observations": sum(len(r["observations"]) for r in result["cases"]),
                "output": str(args.output),
            }
        )
    )


if __name__ == "__main__":
    main()
