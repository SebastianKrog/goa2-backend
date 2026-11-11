"""The release-tree scan identifies locations without echoing sensitive values."""

import gzip
import importlib.util
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]


@pytest.fixture
def scan(tmp_path, monkeypatch):
    monkeypatch.syspath_prepend(str(ROOT / "rust/tools"))
    spec = importlib.util.spec_from_file_location(
        "payload_audit", ROOT / "rust/tools/payload_audit.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    (tmp_path / "rust").mkdir()
    (tmp_path / "docs").mkdir()
    (tmp_path / "rust/extraction-manifest.json").write_text(
        json.dumps({"groups": [{"category": "development", "paths": ["docs/**/*"]}]})
    )
    return module, tmp_path


@pytest.mark.parametrize("compressed", [False, True])
def test_payload_scan_covers_compressed_history_without_echoing_values(scan, compressed):
    module, root = scan
    credential = "ghp_" + "X" * 36
    lines = [
        "/home/" + "private-owner/code",
        "C:/Users/" + "private-owner/code",
        chr(92).join(["", "", "wsl.localhost", "Ubuntu", "home", "private-owner", "code"]),
        credential,
        "AKIA" + "A" * 16,
        "-----BEGIN " + "PRIVATE KEY-----",
    ]
    data = "\n".join(lines).encode()
    name = "history.md.gz" if compressed else "history.md"
    (root / "docs" / name).write_bytes(gzip.compress(data, mtime=0) if compressed else data)
    report = module.audit(root)
    assert {row["kind"] for row in report["findings"]} == set(module.PATTERNS)
    assert all(row["path"] == "docs/" + name for row in report["findings"])
    assert {row["line"] for row in report["findings"]} == set(range(1, 7))
    assert credential not in json.dumps(report)
    assert "private-owner" not in json.dumps(report)


def test_payload_scan_allows_generic_examples(scan):
    module, root = scan
    (root / "docs/examples.md").write_text(
        "/home/user/code\n/home/runner/work/project\nC:/Users/example/project\n"
        + chr(92).join(["", "", "wsl.localhost", "Ubuntu", "home", "user", "code"])
    )
    assert module.audit(root)["findings"] == []


def test_payload_scan_requires_review_for_binary_and_oversized_text(scan, monkeypatch):
    module, root = scan
    monkeypatch.setattr(module, "MAX_TEXT_BYTES", 8)
    (root / "docs/binary").write_bytes(b"\xff")
    (root / "docs/large.gz").write_bytes(gzip.compress(b"a" * 9, mtime=0))
    findings = module.audit(root)["findings"]
    assert {row["kind"] for row in findings} == {"binary-needs-review", "oversized-content"}
