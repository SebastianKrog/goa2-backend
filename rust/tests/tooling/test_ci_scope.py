"""Reference changes and releases must not silently skip restore/full acceptance."""

import importlib.util
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "ci_scope", Path(__file__).resolve().parents[2] / "tools/ci_scope.py"
)
ci_scope = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci_scope)


@pytest.mark.parametrize(
    "path",
    [
        "rust/crates/goa2-core/src/interpreter.rs",
        "rust/crates/goa2-core/src/codec.rs",
        "rust/crates/goa2-core/src/arena.rs",
        "rust/crates/goa2-search-cli/src/main.rs",
        "rust/crates/goa2-game-server/src/store.rs",
        "rust/tests/fixtures/object-execution.json",
        "rust/content/catalogs/actions.json",
        "scripts/rust_oracle/fixture_cache.py",
        ".github/workflows/rust.yml",
    ],
)
def test_reference_sensitive_changes_require_both_gates(path):
    assert ci_scope.select("pull_request", {}, [path], diff_known=True) == {
        "full": True,
        "restore": True,
        "release": False,
    }


@pytest.mark.parametrize(
    ("event_name", "event"),
    [
        ("push", {"ref": "refs/tags/v0.1.0"}),
        ("release", {}),
        ("workflow_dispatch", {"inputs": {"release_candidate": True}}),
    ],
)
def test_release_candidate_requires_matrix_even_with_only_docs(event_name, event):
    assert ci_scope.select(event_name, event, ["rust/README.md"], diff_known=True) == {
        "full": True,
        "restore": True,
        "release": True,
    }


def test_docs_only_can_skip_but_unknown_history_and_periodic_runs_fail_closed():
    assert not ci_scope.select("push", {}, ["rust/README.md"], diff_known=True)["full"]
    for name, known in [("push", False), ("schedule", True), ("workflow_dispatch", True)]:
        selected = ci_scope.select(name, {}, [], diff_known=known)
        assert selected["full"] and selected["restore"]


def test_ci_entrypoint_writes_release_outputs(tmp_path):
    import json
    import os
    import subprocess
    import sys

    event = tmp_path / "event.json"
    event.write_text(json.dumps({"ref": "refs/tags/v0.0.0-test"}))
    output = tmp_path / "outputs"
    result = subprocess.run(
        [sys.executable, ci_scope.__file__],
        env={
            **os.environ,
            "GITHUB_EVENT_PATH": str(event),
            "GITHUB_OUTPUT": str(output),
            "GITHUB_EVENT_NAME": "push",
        },
        check=True,
        capture_output=True,
        text=True,
    )
    assert json.loads(result.stdout) == {"full": True, "restore": True, "release": True}
    assert output.read_text().splitlines() == ["full=true", "restore=true", "release=true"]
