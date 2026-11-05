"""Alias-aware offline boundary for mutable aggregates and stored GameSteps.

Card/token identity uses the existing physical instance table. Object bindings
retain mutable and frozen model/container kinds and physical object identity
that JSON values alone cannot preserve. Capture is opt-in until dense
world/register/frame consumers bind their fields to the native object arena.
"""

from __future__ import annotations


def capture(state, compact):
    from enum import Enum

    from pydantic import BaseModel

    from goa2.domain.hex import Hex
    from goa2.domain.models import Card, Token
    from goa2.engine.steps.base import GameStep

    seen = {}
    physical = []

    def walk(value, raw, path):
        if isinstance(value, (Card, Token)):
            physical.append(value)
            return
        kind = None
        if isinstance(value, GameStep):
            kind = "step"
        elif isinstance(value, BaseModel):
            kind = "frozen_model" if type(value).model_config.get("frozen") else "model"
        elif isinstance(value, dict) and isinstance(raw, dict):
            kind = "dict"
        elif isinstance(value, list) and isinstance(raw, list):
            kind = "list"
        if kind:
            entry = seen.setdefault(id(value), [value, kind, []])
            entry[2].append(path)
        if isinstance(value, BaseModel) and isinstance(raw, dict):
            for name, field in type(value).model_fields.items():
                key = field.serialization_alias or name
                if key in raw:
                    walk(getattr(value, name), raw[key], [*path, key])
        elif isinstance(value, dict) and isinstance(raw, dict):
            for key, item in value.items():
                text = str(key.value) if isinstance(key, Enum) else str(key)
                if text in raw:
                    walk(item, raw[text], [*path, text])
        elif isinstance(value, (list, tuple)) and isinstance(raw, list):
            for i, (item, child) in enumerate(zip(value, raw, strict=True)):
                walk(item, child, [*path, i])

    walk(state, compact, [])
    # Physical instances are atomic in this envelope. Their mutable descendants
    # cannot yet share a generic arena node with context/steps/world data. Reject
    # that capture rather than return a boundary that silently loses the alias.
    visited = set()
    queue = [
        child
        for instance in physical
        for name in type(instance).model_fields
        for child in [getattr(instance, name)]
    ]
    while queue:
        value = queue.pop()
        if not isinstance(value, (BaseModel, dict, list, tuple)):
            continue
        if id(value) in seen:
            raise ValueError("physical instance child aliases require arena slot bindings")
        if id(value) in visited:
            continue
        visited.add(id(value))
        if isinstance(value, BaseModel):
            queue.extend(getattr(value, name) for name in type(value).model_fields)
        elif isinstance(value, dict):
            queue.extend(value.values())
        else:
            queue.extend(value)
    # Keep singleton kind bindings too: a shallow snapshot can make an initially
    # unique list/model shared later. Tuples remain immutable value containers.
    entries = list(seen.values())
    return (
        [
            {
                "kind": kind,
                "paths": paths,
                **({"deepcopy_shared": True} if type(value) is Hex else {}),
            }
            for value, kind, paths in entries
        ],
        tuple(value for value, _, _ in entries),
    )


def export_state_with_shared_objects(state, *, instance_exporter=None):
    if instance_exporter is None:
        from state_codec import export_state_with_instances

        instance_exporter = export_state_with_instances
    envelope, _ = instance_exporter(state)
    aliases, objects = capture(state, envelope["state"])
    if aliases:
        envelope["object_bindings"] = aliases
    by_identity = {
        id(value): binding["paths"][0] for value, binding in zip(objects, aliases, strict=True)
    }
    envelope["board_keys"] = [
        {"hex": key.model_dump(mode="json"), "path": by_identity.get(id(key))}
        for key in state.board.tiles
    ]
    return envelope, objects


def instance_bindings(compact):
    """Physical identity paths, independent of first-import table indices."""
    groups = {}

    def walk(value, path):
        if isinstance(value, dict):
            if "$goa2_instance" in value:
                groups.setdefault(value["$goa2_instance"], []).append(path)
            else:
                for key, child in value.items():
                    walk(child, [*path, key])
        elif isinstance(value, list):
            for index, child in enumerate(value):
                walk(child, [*path, index])

    walk(compact, [])
    return list(groups.values())
