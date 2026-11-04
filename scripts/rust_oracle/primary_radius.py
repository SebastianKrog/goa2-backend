"""Audit reviewed radius/range operands and freeze typed fallback slots."""

from __future__ import annotations

RADIUS_FALLBACK_BASE = 1234567900


def bind_radius_fallbacks(positive, absent, zero, radius_operand, fallbacks):
    if type(positive) is int and positive == radius_operand:
        if absent is None and type(zero) is int and zero == 0:
            return positive
        assert type(absent) is int and absent == zero, (absent, zero)
        for key, default in fallbacks.items():
            if default == absent:
                return int(key)
        slot = RADIUS_FALLBACK_BASE + len(fallbacks)
        fallbacks[str(slot)] = absent
        return slot
    if isinstance(positive, dict):
        assert positive.keys() == absent.keys() == zero.keys()
        return {
            key: bind_radius_fallbacks(value, absent[key], zero[key], radius_operand, fallbacks)
            for key, value in positive.items()
        }
    if isinstance(positive, list):
        assert len(positive) == len(absent) == len(zero)
        return [
            bind_radius_fallbacks(a, b, c, radius_operand, fallbacks)
            for a, b, c in zip(positive, absent, zero, strict=True)
        ]
    assert type(positive) is type(absent) is type(zero) and positive == absent == zero, (
        positive,
        absent,
        zero,
    )
    return positive


def bind_range_fallbacks(positive, zero, range_operand, fallbacks):
    """Audit source `range or default` without changing negative ranges."""
    if type(positive) is int and positive == range_operand:
        if type(zero) is int and zero == 0:
            return positive
        assert type(zero) is int, zero
        for key, default in fallbacks.items():
            if default == zero:
                return int(key)
        slot = 1234568000 + len(fallbacks)
        fallbacks[str(slot)] = zero
        return slot
    if isinstance(positive, dict):
        assert positive.keys() == zero.keys()
        return {
            key: bind_range_fallbacks(value, zero[key], range_operand, fallbacks)
            for key, value in positive.items()
        }
    if isinstance(positive, list):
        assert len(positive) == len(zero)
        return [
            bind_range_fallbacks(a, b, range_operand, fallbacks)
            for a, b in zip(positive, zero, strict=True)
        ]
    assert type(positive) is type(zero) and positive == zero, (positive, zero)
    return positive


def merge_operand_bindings(original, first, second, second_operand):
    """Merge independent audited substitutions without comparing reserved slots."""
    if isinstance(original, dict):
        return {
            key: merge_operand_bindings(value, first[key], second[key], second_operand)
            for key, value in original.items()
        }
    if isinstance(original, list):
        return [
            merge_operand_bindings(value, a, b, second_operand)
            for value, a, b in zip(original, first, second, strict=True)
        ]
    return second if type(original) is int and original == second_operand else first
