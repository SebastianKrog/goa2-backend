#!/usr/bin/env python3
"""Select fail-closed Rust reference gates for changed files and release candidates."""

import json
import os
import subprocess
from pathlib import Path

# Restore is required for the complete engine/worker surface, including runtime
# metadata and acquisition code: an indirect caller can change saved semantics.
REFERENCE_PREFIXES = (
    "rust/crates/",
    "rust/test-support/",
    "rust/tests/fixtures/",
    "rust/tests/baselines/",
    "rust/content/",
    "rust/oracle-manifest.json",
    "rust/Cargo.",
    "rust/rust-toolchain.toml",
    "rust/tools/",
    "rust/test-tiers.json",
    "rust/verification-gates.json",
    "scripts/rust_oracle/",
    "scripts/rust_verification/",
    "scripts/verify_rust",
    ".github/workflows/rust.yml",
)


def select(event_name, event, paths, *, diff_known):
    release = (
        event.get("ref", "").startswith("refs/tags/")
        or event_name == "release"
        or str(event.get("inputs", {}).get("release_candidate", "false")).lower() == "true"
    )
    required = (
        release
        or event_name in {"schedule", "workflow_dispatch"}
        or not diff_known
        or any(path.startswith(REFERENCE_PREFIXES) for path in paths)
    )
    return {"full": required, "restore": required, "release": release}


def main():
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    base = event.get("pull_request", {}).get("base", {}).get("sha") or event.get("before")
    result = None
    if base and set(base) != {"0"}:
        result = subprocess.run(
            ["git", "diff", "--name-only", base, "HEAD"], text=True, capture_output=True
        )
    scope = select(
        os.environ["GITHUB_EVENT_NAME"],
        event,
        result.stdout.splitlines() if result else [],
        diff_known=result is not None and result.returncode == 0,
    )
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        for key, value in scope.items():
            output.write(f"{key}={str(value).lower()}\n")
    print(json.dumps(scope, sort_keys=True))


if __name__ == "__main__":
    main()
