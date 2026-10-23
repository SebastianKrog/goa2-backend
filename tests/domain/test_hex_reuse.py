import copy

import pytest
from pydantic import Field, ValidationError

from goa2.domain.hex import Hex

OFFSETS = ((1, 0, -1), (1, -1, 0), (0, -1, 1), (-1, 0, 1), (-1, 1, 0), (0, 1, -1))


def test_neighbors_preserve_values_and_order_over_negative_and_positive_coordinates():
    for q in range(-6, 7):
        for r in range(-6, 7):
            center = Hex(q=q, r=r, s=-q - r)
            expected = [Hex(q=q + dq, r=r + dr, s=-q - r + ds) for dq, dr, ds in OFFSETS]
            assert center.neighbors() == expected
            for index in range(-12, 13):
                assert center.neighbor(index) == expected[index % 6]
            for index, neighbor in enumerate(expected):
                assert center.direction_to(neighbor) == index
                assert center.direction_to(center + (neighbor - center).scale(4)) == index


def test_neighbor_lists_are_private_and_coordinate_values_are_frozen():
    center = Hex(q=0, r=0, s=0)
    first, second = center.neighbors(), center.neighbors()
    assert first is not second
    first.pop()
    assert len(second) == len(center.neighbors()) == 6
    with pytest.raises(ValidationError):
        second[0].q = 99


def test_deepcopy_shares_only_exact_immutable_hex_values():
    point = Hex(q=1, r=0, s=-1)
    assert copy.deepcopy(point) is point

    class RichHex(Hex):
        values: list[int] = Field(default_factory=list)

    rich = RichHex(q=1, r=0, s=-1, values=[1])
    copied = copy.deepcopy(rich)
    copied.values.append(2)
    assert rich.values == [1]


def test_deep_model_copy_with_updates_cannot_change_original_or_cached_neighbors():
    point = Hex(q=1, r=0, s=-1)
    neighbor = point.neighbor(0)
    changed = neighbor.model_copy(deep=True, update={"q": 9, "s": -9})
    assert changed is not neighbor
    assert neighbor.q == 2 and neighbor.s == -2
    assert point.neighbor(0) == Hex(q=2, r=0, s=-2)
