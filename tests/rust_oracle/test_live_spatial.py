"""Source contracts for board-key identity and live movement fixture coverage."""

from live_spatial_traces import fast_travel_scenes, scenes
from object_execution_traces import reference
from shared_objects import export_state_with_shared_objects


def test_hex_key_identity_is_not_inferred_from_equal_tile_coordinates():
    checked = set()
    for name, state, operations in scenes(tail_only=True):
        if not name.startswith("hex-identity-"):
            break
        envelope, _ = export_state_with_shared_objects(state)
        key = next(iter(state.board.tiles), None)
        if name == "hex-identity-private-key":
            assert key == state.board.tiles[key].hex
            assert key is not state.board.tiles[key].hex
            assert envelope["board_keys"][0]["path"] is None
        if name == "hex-identity-visible-key":
            assert key is state.execution_context["key_alias"]
            assert key is not state.board.tiles[key].hex
            assert envelope["board_keys"][0]["path"] == ["execution_context", "key_alias"]
        records = reference(state, operations)
        if name in {
            "hex-identity-tile-key",
            "hex-identity-private-key",
            "hex-identity-visible-key",
            "hex-identity-auto-dirty",
            "hex-identity-pending-frozen",
        }:
            assert state.execution_context["chosen"] is key
        if name == "hex-identity-pending-dict":
            assert state.execution_context["chosen"] == key
            assert state.execution_context["chosen"] is not key
        assert len(records) == len(operations)
        checked.add(name)
    assert len(checked) == 11


def test_fast_travel_card_exception_precedence_and_facedown_color():
    checked = 0
    for name, state, operations in fast_travel_scenes():
        result = reference(state, operations)[0]
        allows = name in {
            "hex-travel-source-gold",
            "hex-travel-current-gold",
            "hex-travel-card-gold",
        }
        assert bool(result["request"]) == allows, name
        if allows:
            assert len(result["request"]["options"]) == len(state.board.tiles) - 1
        checked += 1
    assert checked == 8
