#!/usr/bin/env python3
"""Export a manifest-verified source artifact for use without original Git history."""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/rust_oracle"))


def main():
    from pinned_source import verify_archive

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--git-repo",
        type=Path,
        required=True,
        help="Original source repository holding the frozen revision",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads((ROOT / "rust/oracle-manifest.json").read_text())
    data = subprocess.check_output(
        ["git", "archive", "--format=tar", manifest["oracle_revision"], "src/goa2", "src/automata"],
        cwd=args.git_repo,
    )
    verify_archive(data, manifest)
    with args.output.open("xb") as out:
        out.write(data)
    print(
        json.dumps(
            {
                "oracle_revision": manifest["oracle_revision"],
                "sha256": hashlib.sha256(data).hexdigest(),
                "bytes": len(data),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
