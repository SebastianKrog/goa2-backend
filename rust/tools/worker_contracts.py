#!/usr/bin/env python3
"""Check persistent JSON-lines rejection/correlation with a caller-selected native worker."""

import argparse
import json
import subprocess
from pathlib import Path


def verify(binary):
    commands = [
        "{",
        "null",
        "[]",
        json.dumps({"schema_version": 1, "id": "unsupported", "operation": "does_not_exist"}),
        json.dumps({"schema_version": 99, "id": "future", "operation": "capabilities"}),
    ]
    for index in range(64):
        commands.append(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": index,
                    "operation": "load_board",
                    "tiles": [],
                    "unknown": index,
                }
            )
        )
    commands.extend(
        [
            json.dumps(
                {"schema_version": 1, "id": 2**64 - 1, "operation": "load_board", "tiles": []}
            ),
            json.dumps(
                {"schema_version": 1, "id": "after-rejections", "operation": "capabilities"}
            ),
        ]
    )
    result = subprocess.run(
        [str(binary)],
        input="\n".join(commands) + "\n",
        text=True,
        capture_output=True,
        check=True,
        timeout=30,
    )
    packets = [json.loads(line) for line in result.stdout.splitlines()]
    assert len(packets) == len(commands)
    for packet in packets[:-2]:
        assert packet["schema_version"] == 1 and packet["ok"] is False
        assert "error" in packet and "result" not in packet
    assert packets[-2]["id"] == 2**64 - 1 and packets[-2]["ok"] is True
    assert packets[-1]["id"] == "after-rejections" and packets[-1]["ok"] is True
    assert packets[-1]["result"]["state_codec"] is True
    return {
        "expected": len(commands),
        "executed": len(packets),
        "rejected": len(commands) - 2,
        "successful_after_rejections": 2,
        "failed": 0,
        "scope": "Transport/schema only; unavailable native workloads are not executed.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.binary.resolve()), indent=2))


if __name__ == "__main__":
    main()
