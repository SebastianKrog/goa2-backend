#!/usr/bin/env python3
"""Named test-tier entry point. Missing data and zero-test results are failures."""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parents[1]


def run(command, *, tests=False, full=False, minimum=350):
    print("+ " + " ".join(command), flush=True)
    result = subprocess.run(
        command, cwd=WORKSPACE, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    )
    print(result.stdout, end="", flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
    if tests:
        rows = re.findall(
            r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; .*? (\d+) filtered out",
            result.stdout,
        )
        executed = sum(int(row[0]) for row in rows)
        ignored = sum(int(row[2]) for row in rows)
        if executed < minimum:
            raise SystemExit(f"Unexpected reduced suite: {executed} passed (minimum {minimum})")
        print(
            json.dumps(
                {
                    "tier": "native",
                    "executed": executed,
                    "failed": 0,
                    "ignored": ignored,
                    "full_parity": full,
                }
            )
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tier", choices=["quick", "full", "restore", "quality", "service"])
    parser.add_argument("--profile", default="dev", choices=["dev", "iterate", "release"])
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    cargo = ["cargo"]
    common = ["--locked", "--profile", args.profile]
    if args.tier == "quality":
        run(cargo + ["fmt", "--all", "--", "--check"])
        run(
            cargo
            + [
                "clippy",
                *common,
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ]
        )
        env = dict(os.environ, RUSTDOCFLAGS="-D warnings")
        subprocess.run(
            cargo + ["doc", *common, "--workspace", "--no-deps"], cwd=WORKSPACE, env=env, check=True
        )
    elif args.tier in {"quick", "full", "restore"}:
        run(
            [
                sys.executable,
                "tools/fixtures.py",
                "verify",
                *(["--full"] if args.tier in {"full", "restore"} else []),
            ]
        )
        if args.tier == "restore":
            run(
                cargo
                + [
                    "test",
                    *common,
                    "-p",
                    "goa2-core",
                    "--features",
                    "full-parity",
                    "--test",
                    "object_execution",
                    "every_shared_execution_boundary_restores_values_aliases_and_request_generations",
                    "--",
                    "--ignored",
                    "--exact",
                ],
                tests=True,
                minimum=1,
            )
        else:
            run(
                cargo
                + [
                    "test",
                    *common,
                    "--workspace",
                    *(["--features", "full-parity"] if args.tier in {"full", "restore"} else []),
                ],
                tests=True,
                full=args.tier == "full",
            )
            if args.tier == "full":
                run(
                    cargo
                    + [
                        "test",
                        *common,
                        "-p",
                        "goa2-search-cli",
                        "--bin",
                        "goa2-search-cli",
                        "trace_tests::",
                        "--",
                        "--ignored",
                    ],
                    tests=True,
                    full=True,
                    minimum=4,
                )
                print(json.dumps({"tier": "full-differential", "all_selected_gates_passed": True}))

    else:
        if not args.binary:
            parser.error("service requires --binary")
        run(
            [
                sys.executable,
                "../scripts/verify_rust_game_server.py",
                str(args.binary.resolve()),
                str(WORKSPACE / "content/smoke-game.json"),
            ]
        )


if __name__ == "__main__":
    main()
