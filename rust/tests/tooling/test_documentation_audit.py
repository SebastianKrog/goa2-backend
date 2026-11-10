"""Retained links cannot depend on excluded history or obsolete headings."""

import importlib.util
import json
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[2] / "tools"
sys.path.insert(0, str(TOOLS))
SPEC = importlib.util.spec_from_file_location(
    "documentation_audit", TOOLS / "documentation_audit.py"
)
docs = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(docs)


def test_retained_links_check_extraction_membership_and_heading_anchors(tmp_path):
    base = tmp_path / "rust"
    base.mkdir()
    (base / "extraction-manifest.json").write_text(
        json.dumps({"groups": [{"category": "development", "paths": ["rust/*.md"]}]})
    )
    (base / "guide.md").write_text("# Build and check\n\n# Build and check\n")
    (tmp_path / "excluded.md").write_text("# Excluded\n")
    (base / "README.md").write_text(
        "[ok](guide.md#build-and-check-1) [old](guide.md#old) [excluded](../excluded.md)\n```sh\n[not a link](missing.md)\n```\n"
    )
    result = docs.audit(tmp_path)
    assert len(result["links"]) == 3
    assert [row["status"] for row in result["failures"]] == [
        "missing-heading-anchor",
        "missing-or-excluded-target",
    ]


def test_retained_documentation_has_no_unresolved_local_links():
    result = docs.audit(TOOLS.parents[1])
    assert result["links"]
    assert result["failures"] == []
