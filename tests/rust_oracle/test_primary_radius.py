"""Audited radius fallbacks retain nullable fields and reject shape drift."""

import pytest
from primary_radius import bind_radius_fallbacks
from primary_templates import RADIUS


def test_nullable_and_truthy_radius_fields_get_distinct_typed_operands():
    fallbacks = {}
    positive = {"nullable": RADIUS, "default": RADIUS, "zero": RADIUS}
    actual = bind_radius_fallbacks(
        positive,
        {"nullable": None, "default": 2, "zero": 0},
        {"nullable": 0, "default": 2, "zero": 0},
        RADIUS,
        fallbacks,
    )
    assert actual["nullable"] == RADIUS
    assert fallbacks[str(actual["default"])] == 2
    assert fallbacks[str(actual["zero"])] == 0
    repeated = bind_radius_fallbacks(RADIUS, 2, 2, RADIUS, fallbacks)
    assert repeated == actual["default"] and len(fallbacks) == 2


def test_radius_audit_rejects_unreviewed_branches_and_type_changes():
    for positive, absent, zero in (
        ([RADIUS], [], [0]),
        (RADIUS, 2, 3),
        (True, 1, 1),
        (RADIUS, None, False),
    ):
        with pytest.raises(AssertionError):
            bind_radius_fallbacks(positive, absent, zero, RADIUS, {})


def test_both_catalogs_share_exact_radius_fallback_operands():
    from primary_builders import catalog
    from primary_templates import build_catalog

    assert catalog()["radius_fallbacks"] == build_catalog()["radius_fallbacks"]
