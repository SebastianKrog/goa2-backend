"""Resolve native perf self samples, checking build identity and ELF file offsets.

Requires Linux perf, GNU readelf/nm and an unstripped matching worker. Generic
functions can share addresses after LTO; retain all nm aliases rather than
claiming a unique generic instantiation. Library samples remain grouped by DSO.
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import os
import re
import subprocess
from collections import Counter
from pathlib import Path


def executable_segments(headers):
    result = []
    for line in headers.splitlines():
        fields = line.split()
        if fields and fields[0] == "LOAD" and "E" in fields[6:-1]:
            result.append((int(fields[1], 16), int(fields[2], 16), int(fields[4], 16)))
    if not result:
        raise ValueError("ELF has no executable LOAD segments")
    return result


def symbol_ranges(listing):
    result = {}
    for line in listing.splitlines():
        match = re.match(r"^([0-9a-fA-F]+)\s+([0-9a-fA-F]+)\s+[tTwW]\s+(.+)$", line)
        if match:
            address, size, name = match.groups()
            entry = result.setdefault(int(address, 16), [0, set()])
            entry[0] = max(entry[0], int(size, 16))
            entry[1].add(name)
    if not result:
        raise ValueError("no sized executable symbols; use an unstripped binary")
    return result


def require_build_id(notes, identities, binary):
    match = re.search(r"Build ID:\s*([0-9a-fA-F]+)", notes)
    if not match:
        raise ValueError("binary has no build ID")
    build_id = match[1].lower()
    recorded = []
    for line in identities.splitlines():
        fields = line.split(maxsplit=1)
        if len(fields) == 2 and fields[1] == str(binary):
            recorded.append(fields[0].lower())
    if recorded != [build_id]:
        raise ValueError("profile build ID does not match binary at its recorded path")
    return build_id


def resolve_samples(report, segments, symbols, binary_name):
    starts = sorted(symbols)
    counts = Counter()
    for line in report.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = [field.strip() for field in line.split(";", 3)]
        if len(fields) != 4:
            raise ValueError(f"unexpected perf row: {line}")
        _, count, dso, symbol = fields
        key = (dso,)
        if dso == binary_name:
            symbol = re.sub(r"^\[.\]\s*", "", symbol)
            if symbol.startswith("0x"):
                offset = int(symbol, 16)
                address = next(
                    (
                        virtual + offset - file
                        for file, virtual, size in segments
                        if file <= offset < file + size
                    ),
                    None,
                )
                if address is None:
                    raise ValueError(f"sample outside executable file segments: {symbol}")
                index = bisect.bisect_right(starts, address) - 1
                if index >= 0:
                    start = starts[index]
                    size, names = symbols[start]
                    if address < start + size:
                        key = tuple(sorted(names))
                    else:
                        key = (f"unresolved executable offset {symbol}",)
                else:
                    key = (f"unresolved executable offset {symbol}",)
            else:
                key = (symbol,)
        counts[key] += int(count)
    total = counts.total()
    if not total:
        raise ValueError("profile contains no samples")
    return {
        "total_samples": total,
        "functions": [
            {"samples": count, "percent": 100 * count / total, "symbols": list(names)}
            for names, count in counts.most_common()
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--perf", default="perf")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()

    def run(*command):
        return subprocess.check_output(command, text=True, env={**os.environ, "LC_ALL": "C"})

    headers = run("readelf", "-lW", str(binary))
    listing = run("nm", "-n", "-S", "--defined-only", "--demangle", str(binary))
    identity = require_build_id(
        run("readelf", "-n", str(binary)),
        run(args.perf, "buildid-list", "-i", str(args.data)),
        binary,
    )
    report = run(
        args.perf,
        "report",
        "--stdio",
        "--no-children",
        "--show-nr-samples",
        "--field-separator=;",
        "--fields",
        "overhead,sample,dso,symbol",
        "-i",
        str(args.data),
    )
    segments = executable_segments(headers)
    result = resolve_samples(report, segments, symbol_ranges(listing), binary.name)
    result.update(
        {
            "binary": str(binary),
            "build_id": identity,
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "data_sha256": hashlib.sha256(args.data.read_bytes()).hexdigest(),
            "executable_segments": segments,
            "scope": "Self samples; shared LTO symbol aliases retained, library DSOs grouped",
        }
    )
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({**result, "functions": result["functions"][:12]}, indent=2))


if __name__ == "__main__":
    main()
