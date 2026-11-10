#!/usr/bin/env python3
"""Inventory extraction inputs, consumers and optional reference boundaries without importing them."""

from __future__ import annotations

import argparse
import ast
import json
import re
import sys
import tomllib
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def selected_paths(root, manifest):
    """Apply the manifest's allowlist and exclusions; smoke is the explicit cache exception."""
    selected = {}
    exclusions = [pattern for row in manifest.get("excluded", []) for pattern in row["paths"]]
    for group in manifest["groups"]:
        for pattern in group["paths"]:
            matches = sorted(root.glob(pattern))
            if not matches and not any(c in pattern for c in "*?["):
                raise ValueError(f"Missing extraction input: {pattern}")
            for path in matches:
                if "__pycache__" in path.parts or not path.is_file():
                    continue
                relative = path.relative_to(root)
                if any(
                    part.is_symlink()
                    for part in [path, *path.parents]
                    if part != root and root in part.parents
                ):
                    raise ValueError(f"Symlink not admitted to extraction: {relative}")
                if not path.resolve().is_relative_to(root.resolve()):
                    raise ValueError(f"Extraction input escapes source root: {relative}")
                excluded = False
                for rule in exclusions:
                    if rule.startswith("rust/tests/fixtures/") and relative.is_relative_to(
                        "rust/tests/fixtures/smoke"
                    ):
                        continue
                    if relative.match(rule) or relative.as_posix().startswith(
                        rule.rstrip("/") + "/"
                    ):
                        excluded = True
                if not excluded:
                    selected[relative.as_posix()] = group["category"]
    if not selected:
        raise ValueError("Extraction manifest selected no files")
    return selected


def module_names(path):
    parts = Path(path).with_suffix("").parts
    if parts[:2] == ("scripts", "rust_oracle"):
        return [parts[-1], ".".join(parts[1:])]
    if parts[:1] == ("scripts",):
        module = ".".join(parts[1:])
        return [module.removesuffix(".__init__")]
    if parts[:2] == ("rust", "tools"):
        return [parts[-1]]
    return []


def audit(root=ROOT):
    manifest = json.loads((root / "rust/extraction-manifest.json").read_text())
    selected = selected_paths(root, manifest)
    modules = {
        name: path for path in selected if path.endswith(".py") for name in module_names(path)
    }
    edges, imports, dynamic, roots, references = [], [], [], [], []
    unresolved = []
    for name in sorted(selected):
        path = root / name
        if path.suffix == ".rs":
            text = path.read_text()
            for pattern, kind in [
                (r'include_(?:str|bytes)!\(\s*"([^"\n]+)"', "rust-embed"),
                (r'#\[path\s*=\s*"([^"\n]+)"\]', "rust-module"),
            ]:
                for match in re.finditer(pattern, text):
                    target = (path.parent / match[1]).resolve()
                    relative = (
                        target.relative_to(root.resolve()).as_posix()
                        if target.is_relative_to(root.resolve())
                        else None
                    )
                    status = "included" if relative in selected else "missing-or-excluded"
                    row = dict(
                        consumer=name,
                        line=text.count("\n", 0, match.start()) + 1,
                        input=relative or match[1],
                        kind=kind,
                        status=status,
                    )
                    edges.append(row)
                    if status != "included":
                        unresolved.append(row)
        elif path.suffix == ".py":
            tree = ast.parse(path.read_text(), filename=name)
            package = (
                ".".join(Path(name).with_suffix("").parts[1:-1])
                if name.startswith("scripts/")
                else ""
            )
            for node in ast.walk(tree):
                names = []
                if isinstance(node, ast.Import):
                    names = [alias.name for alias in node.names]
                elif isinstance(node, ast.ImportFrom):
                    if node.level:
                        parents = package.split(".")[: len(package.split(".")) - node.level + 1]
                        names = [".".join([*parents, node.module or ""]).rstrip(".")]
                    else:
                        names = [node.module or ""]
                for module in names:
                    top = module.split(".")[0]
                    if module in modules:
                        category, target = "included-module", modules[module]
                    elif top in {"backend", "state_model"}:
                        category, target = "optional-layout-lab", module
                    elif top in {"goa2", "automata"}:
                        category = (
                            "compatible-python-service"
                            if "verify_rust_" in name or name.endswith("export_override_schema.py")
                            else "pinned-oracle-source"
                        )
                        target = module
                    elif top in sys.stdlib_module_names or top in {"__future__", "builtins"}:
                        category, target = "python-stdlib", module
                    else:
                        category, target = "third-party-runtime", module
                    imports.append(
                        dict(consumer=name, line=node.lineno, input=target, category=category)
                    )
                if isinstance(node, ast.Call):
                    function = ast.unparse(node.func)
                    if function.endswith(
                        ("import_module", "__import__", "spec_from_file_location")
                    ):
                        dynamic.append(
                            dict(
                                consumer=name,
                                line=node.lineno,
                                expression=ast.unparse(node),
                                disposition="Resolve literal/local modules using included scripts; variable imports are caller-controlled or pinned source discovery. No ambient source checkout is admitted for native execution.",
                            )
                        )
                if isinstance(node, (ast.Assign, ast.AnnAssign)):
                    value = node.value
                    if value and "__file__" in ast.unparse(value):
                        roots.append(
                            dict(consumer=name, line=node.lineno, expression=ast.unparse(value))
                        )
                if isinstance(node, ast.Constant) and isinstance(node.value, str):
                    value = node.value
                    if (
                        not value.startswith(("rust/", "src/", "docs/", "scripts/"))
                        or "\n" in value
                    ):
                        continue
                    if value in selected:
                        disposition = "included"
                    elif value.startswith("src/"):
                        disposition = "optional pinned/reference Python source; explicitly acquired outside native snapshot"
                    elif value.startswith("rust/tests/fixtures"):
                        disposition = (
                            "registered optional parity/cache input or generator destination"
                        )
                    elif value.startswith("rust/artifacts"):
                        disposition = (
                            "generated diagnostic/cache destination; excluded from snapshot"
                        )
                    elif value.startswith("docs/"):
                        disposition = "original Python/history documentation; not an ordinary native build dependency"
                    elif (root / value).is_dir():
                        disposition = "directory resolved by included tool"
                    else:
                        disposition = "review"
                    references.append(
                        dict(consumer=name, line=node.lineno, input=value, disposition=disposition)
                    )
    cargo = []
    for name in sorted(selected):
        if not name.endswith("Cargo.toml"):
            continue
        config = tomllib.loads((root / name).read_text())
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for dependency, spec in config.get(section, {}).items():
                if isinstance(spec, dict) and "path" in spec:
                    target = (root / name).parent / spec["path"] / "Cargo.toml"
                    relative = target.resolve().relative_to(root.resolve()).as_posix()
                    row = dict(
                        consumer=name,
                        input=relative,
                        kind="cargo-path",
                        status="included" if relative in selected else "missing-or-excluded",
                    )
                    cargo.append(row)
                    if row["status"] != "included":
                        unresolved.append(row)
    reverse = defaultdict(set)
    for row in edges + imports + cargo:
        if row.get("status") == "included" or row.get("category") == "included-module":
            reverse[row["input"]].add(row["consumer"])
    return dict(
        schema_version=1,
        manifest="rust/extraction-manifest.json",
        selected_files=len(selected),
        group_counts={
            group: sum(value == group for value in selected.values())
            for group in sorted(set(selected.values()))
        },
        embedded_inputs=edges,
        cargo_path_dependencies=cargo,
        python_import_counts={
            category: sum(r["category"] == category for r in imports)
            for category in sorted({r["category"] for r in imports})
        },
        python_imports=[r for r in imports if r["category"] != "python-stdlib"],
        dynamic_imports=dynamic,
        python_root_resolution=roots,
        logical_paths=references,
        reverse_consumers={key: sorted(value) for key, value in sorted(reverse.items())},
        unresolved_embeds=unresolved,
        external_boundaries=manifest["external_dependencies"],
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = audit()
    encoded = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")
    if result["unresolved_embeds"]:
        raise SystemExit("Unresolved compile-time extraction dependencies")


if __name__ == "__main__":
    main()
