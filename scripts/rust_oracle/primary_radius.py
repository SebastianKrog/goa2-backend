"""Audit reviewed radius operands and freeze typed fallback slots."""

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
