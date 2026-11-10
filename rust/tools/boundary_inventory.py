#!/usr/bin/env python3
"""Inventory native public consumers, worker dispatch and panic assumptions; never retire inputs."""

from __future__ import annotations

import argparse
import ast
import json
import re
from pathlib import Path

from dependency_audit import selected_paths

ROOT = Path(__file__).resolve().parents[2]


def operation_variants(text):
    block = text.split("enum Operation {", 1)[1].split("\n}\ntype UpgradeCheckpoints", 1)[0]
    variants = []
    for match in re.finditer(r"^    (\w+)(?: \{(.*?)^    \}|,)", block, re.M | re.S):
        variant, body = match.group(1), match.group(2) or ""
        fields = []
        for field in re.finditer(r"^        (\w+): ([^\n]+),$", body, re.M):
            prefix = body[: field.start()].rstrip().splitlines()
            optional = field[2].startswith("Option<") or (prefix and "serde(default" in prefix[-1])
            fields.append({"name": field[1], "type": field[2], "optional": bool(optional)})
        name = re.sub(r"(?<!^)(?=[A-Z])", "_", variant).lower()
        variants.append({"variant": variant, "operation": name, "fields": fields})
    return variants


def inventory(root=ROOT):
    paths = selected_paths(root, json.loads((root / "rust/extraction-manifest.json").read_text()))
    sources = {
        name: (root / name).read_text() for name in sorted(paths) if name.endswith((".rs", ".py"))
    }
    worker = "rust/crates/goa2-search-cli/src/main.rs"
    variants = operation_variants(sources[worker])
    operations = {row["operation"]: [] for row in variants}
    dynamic = []
    for name, text in sources.items():
        if not name.endswith(".py"):
            continue
        tree = ast.parse(text)
        aliases = {
            target.id
            for node in ast.walk(tree)
            if isinstance(node, ast.Assign)
            and isinstance(node.value, ast.Attribute)
            and node.value.attr in {"request", "operation_records"}
            for target in node.targets
            if isinstance(target, ast.Name)
        }
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call) or not node.args:
                continue
            request = (
                isinstance(node.func, ast.Attribute)
                and node.func.attr in {"request", "operation_records"}
                or isinstance(node.func, ast.Name)
                and node.func.id in aliases
            )
            if not request:
                continue
            operation = node.args[0]
            row = {"consumer": name, "line": node.lineno, "expression": ast.unparse(node.func)}
            if isinstance(operation, ast.Constant) and isinstance(operation.value, str):
                if operation.value in operations:
                    operations[operation.value].append(row)
            else:
                row["transport"] = (
                    "HTTP client, not worker dispatch"
                    if ast.unparse(node.func) == "self.http.request"
                    else "worker/dynamic adapter"
                )
                row["operation_expression"] = ast.unparse(operation)
                row["review"] = (
                    "Retain dynamic wrapper; operation is supplied by its caller. Cross-check literal callers and protocol inventory; HTTP client request methods are not worker operations."
                )
                dynamic.append(row)
    modules = re.findall(r"^pub mod (\w+);", sources["rust/crates/goa2-core/src/lib.rs"], re.M)
    exports = []
    for module in modules:
        defining = f"rust/crates/goa2-core/src/{module}.rs"
        text = sources[defining]
        items = sorted(
            set(re.findall(r"^pub (?:struct|enum|trait|type|fn|const) (\w+)", text, re.M))
        )
        uses = []
        for name, consumer in sources.items():
            if not name.endswith(".rs") or name == defining:
                continue
            for match in re.finditer(r"(?m)^(?:pub )?use [\s\S]*?;", consumer):
                if re.search(r"\b" + re.escape(module) + r"\b", match[0]) and any(
                    key in match[0] for key in ["crate::", "goa2_core::", "super::"]
                ):
                    uses.append(
                        {
                            "consumer": name,
                            "line": consumer.count("\n", 0, match.start()) + 1,
                            "kind": "use-tree",
                        }
                    )
            for match in re.finditer(
                r"(?:crate|goa2_core)::" + re.escape(module) + r"::", consumer
            ):
                uses.append(
                    {
                        "consumer": name,
                        "line": consumer.count("\n", 0, match.start()) + 1,
                        "kind": "qualified-use",
                    }
                )
        exports.append(
            {
                "module": module,
                "source": defining,
                "declared_items": items,
                "reexports": re.findall(r"^pub use [\s\S]*?;", text, re.M),
                "consumers": uses,
                "status": (
                    "intended facade"
                    if module == "api"
                    else (
                        "current identity metadata"
                        if module == "versions"
                        else (
                            "internal diagnostic codes"
                            if module == "diagnostic"
                            else "experimental retained; no visibility removal"
                        )
                    )
                ),
            }
        )
    # Enumerate all occurrences, including tests, without pretending a textual
    # match is a proof of reachability. The companion review owns assumptions.
    panics = []
    test_counts = {}
    for name, text in sources.items():
        if not name.endswith(".rs"):
            continue
        test_module = re.search(r"#\[cfg\(test\)\]\s*\nmod \w+\s*\{", text)
        test_offset = test_module.start() if test_module else -1
        for match in re.finditer(
            r"\.unwrap\s*\(|\.expect\s*\(|\b(?:panic|unreachable|assert|assert_eq|assert_ne)!", text
        ):
            test_only = (
                "/tests/" in name
                or "_tests.rs" in name
                or test_offset >= 0
                and match.start() > test_offset
            )
            if test_only:
                test_counts[name] = test_counts.get(name, 0) + 1
                continue
            elif "/goa2-game-server/" in name:
                category = "service-boundary-review"
            elif name.endswith(("/codec.rs", "/geometry.rs", "/setup.rs")):
                category = "serialized-or-trusted-content-boundary-review"
            elif "/goa2-search-cli/" in name:
                category = "worker-boundary-review"
            else:
                category = "validated-native-invariant-review"
            line = text.count("\n", 0, match.start()) + 1
            panics.append(
                {
                    "source": name,
                    "line": line,
                    "expression": match[0],
                    "category": category,
                    "context": text.splitlines()[line - 1].strip(),
                }
            )
    native = []
    for name, text in sources.items():
        if not name.endswith(".rs"):
            continue
        calls = sorted(
            set(
                re.findall(
                    r"\b(?:NativeSession|LiveGame)::(?:compile|import|restore|create|import_with_rollback)|\b(?:objects|round_transition_bound|export_envelope|materialize|advance_to_decision|prepare_submission|search_resolution|search_plans_on_sample)\b",
                    text,
                )
            )
        )
        if calls:
            native.append(
                {
                    "consumer": name,
                    "entrypoints_or_representation_terms": calls,
                    "decision": "Retain dense and object-aware implementations; selection is determined by admitted state/bindings, not a caller filename. Full unique regression migration required before retirement.",
                }
            )
    return {
        "schema_version": 1,
        "method": "Conservative static Rust use-tree/qualified-path inventory and Python AST request calls. Not a compiler-resolved whole-program call graph; aliases, dispatch and runtime representation decisions are explicitly retained. No absent textual caller proves an export removable.",
        "core_exports": exports,
        "worker_variants": variants,
        "worker_callers": operations,
        "dynamic_request_sites": dynamic,
        "dense_object_consumers": native,
        "panic_assumptions": panics,
        "test_panic_site_counts": test_counts,
        "deletions": 0,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = inventory()
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                key: len(result[key])
                for key in [
                    "core_exports",
                    "worker_variants",
                    "dynamic_request_sites",
                    "dense_object_consumers",
                    "panic_assumptions",
                ]
            }
        )
    )


if __name__ == "__main__":
    main()
