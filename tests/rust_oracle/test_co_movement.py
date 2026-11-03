"""Direct contracts for mirrored movement's source-specific obstacle exceptions."""
import pytest

from co_movement_traces import build


@pytest.mark.parametrize("variant,through,token,expected", [
    ("plain", False, False, True),
    ("parallel", False, False, True),
    ("behind", False, False, True),
    ("path-terrain", False, False, False),
    ("path-terrain", True, False, True),
    ("path-gap", False, False, False),
    ("path-gap", True, False, True),
    ("vacating-terrain", False, False, True),
    ("landing-terrain", True, False, False),
    ("partner-landing-terrain", True, False, False),
    ("landing-occupied", True, False, False),
    ("partner-landing-occupied", True, False, False),
    ("same-location", False, False, False),
    ("stationary", False, False, False),
    ("nonstraight", False, False, False),
    ("mine-dud", False, False, True),
    ("mine-dud", False, True, False),
    ("mine-dud", True, True, True),
])
def test_mirrored_destination_contract(variant, through, token, expected):
    from goa2.domain.hex import Hex
    from goa2.engine.filters_geometry import CoMoverValidHexFilter

    state, _ = build(variant, token=token)
    predicate = CoMoverValidHexFilter(anchor_key="anchor", partner_key="partner", ignore_path_obstacles=through)
    destination = Hex(**state.execution_context["anchor_dest"])
    assert predicate.apply(destination, state, state.execution_context) is expected
    assert not predicate.apply(destination.model_dump(), state, state.execution_context)
    assert not predicate.apply("hero_arien", state, state.execution_context)


def test_vacating_mover_does_not_exempt_terrain_at_a_landing():
    from goa2.engine.filters_geometry import CoMoverValidHexFilter

    state, h = build("vacating-terrain")
    predicate = CoMoverValidHexFilter(anchor_key="anchor", partner_key="partner")
    assert not predicate.apply(h(-1, 0), state, state.execution_context)
