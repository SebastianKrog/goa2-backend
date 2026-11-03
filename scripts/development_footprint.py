"""Report the bot development footprint; never enforce limits or delete files."""

import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def report():
    seen = set()
    groups = {}
    # Count shared hard links once. The lab cache belongs to a separate tool.
    for label, base in [
        ("checkout (including Git and fixture cache)", ROOT),
        ("Rust builds and oracle environment", Path.home() / ".cache/goa2-rust"),
    ]:
        size = 0
        for directory, _, files in os.walk(base, followlinks=False):
            for name in files:
                path = Path(directory) / name
                try:
                    stat = path.lstat()
                except FileNotFoundError:
                    continue
                key = (stat.st_dev, stat.st_ino)
                if key not in seen:
                    seen.add(key)
                    size += stat.st_size
        groups[label] = size
    total = sum(groups.values())
    result = {
        "bytes": groups,
        "total_gib": round(total / 2**30, 2),
        "review_at_gib": 10,
        "development_budget_gib": 12,
        "guidance": "Consider cleanup or review the budget"
        if total >= 10 * 2**30
        else "Within development budget",
    }
    print(json.dumps(result, indent=2))
    return result


if __name__ == "__main__":
    report()
