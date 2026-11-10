#!/usr/bin/env python3
"""List, render or run explicit expensive gates; metadata validation never executes them."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CATALOG = ROOT / "rust/verification-gates.json"


def catalog():
    data = json.loads(CATALOG.read_text())
    oracle = json.loads((ROOT / "rust/oracle-manifest.json").read_text())
    if (
        data["source_sha256"] != oracle["source_sha256"]
        or data["oracle_revision"] != oracle["oracle_revision"]
    ):
        raise ValueError("Gate catalog oracle identity drift")
    ids = [row["id"] for row in data["gates"]]
    if not ids or len(ids) != len(set(ids)):
        raise ValueError("Gate catalog is empty or has duplicate IDs")
    for gate in data["gates"]:
        if not gate["commands"]:
            raise ValueError(f"Gate {gate['id']} has no commands")
        declared = set(gate["required_variables"]) | {"PYTHON", "ROOT"}
        for command in gate["commands"]:
            if not command["argv"] or any(not isinstance(arg, str) for arg in command["argv"]):
                raise ValueError(f"Gate {gate['id']} has invalid argv")
            if "minimum_tests" in command and command["minimum_tests"] < 1:
                raise ValueError(f"Gate {gate['id']} permits a zero-test success")
            names = {
                name for arg in command["argv"] for name in re.findall(r"\$\{([A-Z_]+)\}", arg)
            }
            if not names <= declared:
                raise ValueError(
                    f"Gate {gate['id']} uses undeclared inputs: {sorted(names - declared)}"
                )
    for name, digest in data["bridge_sha256"].items():
        if hashlib.sha256((ROOT / name).read_bytes()).hexdigest() != digest:
            raise ValueError(
                f"Gate command source changed: {name}; review its workload and refresh the catalog"
            )
    return data


def render(gate, variables):
    values = {**variables, "PYTHON": sys.executable, "ROOT": str(ROOT)}
    missing = [name for name in gate["required_variables"] if not values.get(name)]
    if missing:
        raise ValueError("Missing explicit gate inputs: " + ", ".join(missing))

    def substitute(value):
        return re.sub(r"\$\{([A-Z_]+)\}", lambda match: values[match[1]], value)

    commands = [[substitute(arg) for arg in row["argv"]] for row in gate["commands"]]
    return commands


def execute(gate, variables):
    commands = render(gate, variables)
    for name in gate.get("input_files", []):
        if not Path(variables[name]).is_file() or not Path(variables[name]).is_absolute():
            raise ValueError(f"{name} must name an existing absolute input file")
    for name in gate.get("input_directories", []):
        if not Path(variables[name]).is_dir() or not Path(variables[name]).is_absolute():
            raise ValueError(f"{name} must name an existing absolute input directory")
    for name in gate.get("output_paths", []):
        if not Path(variables[name]).is_absolute():
            raise ValueError(f"{name} must name an absolute output path")
    records = []
    for spec, command in zip(gate["commands"], commands, strict=True):
        environment = {**os.environ, **variables}
        for name, value in gate.get("environment", {}).items():
            environment[name] = re.sub(r"\$\{([A-Z_]+)\}", lambda match: variables[match[1]], value)
        result = subprocess.run(
            command,
            cwd=ROOT,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        print(result.stdout, end="", flush=True)
        if result.returncode:
            raise ValueError(f"Gate {gate['id']} command failed with exit {result.returncode}")
        minimum = spec.get("minimum_tests")
        passed = sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed;", result.stdout))
        if minimum is not None and passed < minimum:
            raise ValueError(
                f"Gate {gate['id']} selected {passed} tests; requires at least {minimum}"
            )
        records.append({"exit_code": 0, "tests_passed": passed if minimum is not None else None})
    return {"gate": gate["id"], "status": "passed", "commands": records, "scope": gate["scope"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["list", "validate", "show", "run"])
    parser.add_argument("gate", nargs="?")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    data = catalog()
    if args.action in {"list", "validate"}:
        print(
            json.dumps(
                {
                    "gate_count": len(data["gates"]),
                    "gates": [
                        {key: row[key] for key in ("id", "owner", "when", "required_variables")}
                        for row in data["gates"]
                    ],
                },
                indent=2,
            )
        )
        return
    gate = next((row for row in data["gates"] if row["id"] == args.gate), None)
    if gate is None:
        parser.error("Select a registered gate ID")
    if args.action == "show":
        print(json.dumps(gate, indent=2))
    else:
        result = execute(gate, dict(os.environ))
        if args.report:
            args.report.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        raise SystemExit(str(error))
