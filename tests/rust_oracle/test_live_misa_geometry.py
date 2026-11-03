"""Source contracts for geometric versus directional topological predicates."""

from live_misa_geometry_traces import (
    adjacency_scenes,
    between_scenes,
    distance_scenes,
    relative_scenes,
)
from object_execution_traces import reference


def test_relative_distance_preserves_infinity_comparisons_and_origin_precedence():
    for name, state, ops in relative_scenes():
        reference(state, ops)
        if "-UNIT-both-infinite-" in name:
            operator = name.rsplit("-", 1)[-1]
            assert state.execution_context["relative_result"] is (
                True if operator in ("==", ">=", "<=") else None
            )
        if name.endswith(
            ("unknown-key", "numeric-key", "missing-reference", "missing-origin", "no-actor")
        ):
            assert state.execution_context["relative_result"] is None
        if name in {
            "misa-geometry-relative-" + variant for variant in ("literal", "key", "false-key")
        }:
            assert state.execution_context["relative_result"] is True


def test_check_distance_compares_disconnected_distance_as_positive_infinity():
    for name, state, ops in distance_scenes():
        reference(state, ops)
        if name.startswith("misa-geometry-distance-True-"):
            operator = name.removeprefix("misa-geometry-distance-True-").rsplit("-", 1)[0]
            assert state.execution_context["distance_result"] is (
                True if operator in (">", ">=", "!=") else None
            )
        if name.endswith(("missing-key", "false-key", "unknown", "offboard")):
            assert state.execution_context["distance_result"] is None
        if name.endswith("skip"):
            assert state.execution_context["distance_result"] == "retained"


def test_between_uses_geometric_exclusive_endpoints_and_tolerant_hex_validation():
    for name, state, ops in between_scenes():
        reference(state, ops)
        if "-UNIT-" in name:
            variant = name.split("-UNIT-", 1)[1]
            assert state.execution_context["between_result"] is (
                True
                if variant in ("models", "dicts", "numeric-strings", "off-map", "topology-split")
                else None
            )


def test_adjacency_checks_map_presence_and_directional_isolation():
    for name, state, ops in adjacency_scenes():
        reference(state, ops)
        if "-UNIT-" in name:
            variant = name.split("-UNIT-", 1)[1]
            assert state.execution_context["adjacent_result"] is (
                True if variant in ("ordinary", "isolation-source") else None
            )
