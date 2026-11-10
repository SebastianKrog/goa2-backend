"""Maintained operation inventory and examples cannot omit a dispatch variant."""

import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "rust/tools"))
SPEC = importlib.util.spec_from_file_location(
    "boundary_inventory", ROOT / "rust/tools/boundary_inventory.py"
)
boundaries = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(boundaries)


def test_protocol_examples_and_statuses_cover_every_worker_variant():
    variants = boundaries.operation_variants(
        (ROOT / "rust/crates/goa2-search-cli/src/main.rs").read_text()
    )
    examples = json.loads((ROOT / "rust/docs/worker-protocol-examples.json").read_text())[
        "examples"
    ]
    inventory = json.loads((ROOT / "rust/docs/worker-operations.json").read_text())["operations"]
    names = {row["operation"] for row in variants}
    assert len(names) == len(variants) == 39
    assert (
        names == {row["operation"] for row in examples} == {row["operation"] for row in inventory}
    )
    for row in inventory:
        assert row["owner"] and row["status"] and row["retirement"]
        assert row["protocol_examples"] and row["schema_tests"] and row["error_tests"]
        for path in row["callers"]:
            assert (ROOT / path).is_file(), path


def test_worker_field_inventory_distinguishes_required_default_and_optional():
    variants = boundaries.operation_variants(
        "enum Operation {\n    Example {\n        seed: u64,\n        turns: Option<u8>,\n        #[serde(default)]\n        trace: bool,\n    },\n}\ntype UpgradeCheckpoints = ();"
    )
    assert [(row["name"], row["optional"]) for row in variants[0]["fields"]] == [
        ("seed", False),
        ("turns", True),
        ("trace", True),
    ]


def test_test_only_import_does_not_hide_production_panic_sites(tmp_path):
    base = tmp_path / "rust"
    (base / "crates/goa2-core/src").mkdir(parents=True)
    (base / "crates/goa2-search-cli/src").mkdir(parents=True)
    (base / "extraction-manifest.json").write_text(
        json.dumps({"groups": [{"category": "runtime", "paths": ["rust/crates/*/src/*.rs"]}]})
    )
    (base / "crates/goa2-core/src/lib.rs").write_text("pub mod api;\n")
    (base / "crates/goa2-core/src/api.rs").write_text(
        "#[cfg(test)]\nuse std::fmt;\nfn boundary() { input.unwrap(); }\n#[cfg(test)]\nmod tests { fn test() { expected.unwrap(); } }\n"
    )
    (base / "crates/goa2-search-cli/src/main.rs").write_text(
        "enum Operation {\n    Capabilities,\n}\ntype UpgradeCheckpoints=();"
    )
    result = boundaries.inventory(tmp_path)
    assert len(result["panic_assumptions"]) == 1
    assert "input.unwrap" in result["panic_assumptions"][0]["context"]
    assert sum(result["test_panic_site_counts"].values()) == 1
