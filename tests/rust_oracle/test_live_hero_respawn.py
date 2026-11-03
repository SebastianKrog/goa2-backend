"""Frozen-source contracts for hero respawn identity and selection handling."""

from __future__ import annotations

import copy

import pytest
from live_hero_respawn_traces import inputs
from state_codec import export_state


@pytest.fixture(scope="module")
def cases():
    return {name: (state, step) for name, state, step in inputs()}


def run(cases, name):
    state, step = copy.deepcopy(cases[name])
    return state, step, step.resolve(state, state.execution_context)


def test_pass_precedes_missing_or_blocked_spawn_points_and_retains_pending(cases):
    for name in ("pending-pass-no-spawns", "pending-pass-no-space"):
        state, step = copy.deepcopy(cases[name])
        pending = step.pending_input
        out = step.resolve(state, state.execution_context)
        assert out.is_finished and not out.input_request
        assert state.execution_context["skipped_respawn"] is True
        assert step.pending_input is pending


def test_present_or_unknown_hero_finishes_without_touching_locations(cases):
    for name in ("present", "piece-present", "unknown"):
        state, step = copy.deepcopy(cases[name])
        before = export_state(state)
        out = step.resolve(state, state.execution_context)
        assert out.is_finished and not out.events and not out.input_request
        assert export_state(state) == before


def test_invalid_nonempty_dict_requests_hex_but_empty_dict_or_hex_model_requests_yes_no(cases):
    for name, kind in (
        ("pending-invalid", "CHOOSE_RESPAWN_HEX"),
        ("pending-empty", "CHOOSE_RESPAWN"),
        ("pending-hex-model", "CHOOSE_RESPAWN"),
    ):
        state, step, out = run(cases, name)
        assert out.input_request.request_type.value == kind
        assert not state.has_board_presence("hero_arien")
        assert step.pending_input is state.execution_context["pending_input_alias"]


def test_selected_dict_allocates_new_hex_and_preserves_location_and_tile_maps(cases):
    state, step = copy.deepcopy(cases["pending-valid"])
    locations = state.entity_locations
    tiles = state.board.tiles
    selected = step.pending_input["selection"]
    spawn = state.board.spawn_points[0].location
    out = step.resolve(state, state.execution_context)
    target = state.entity_locations["hero_arien"]
    assert out.is_finished and not out.events
    assert state.entity_locations is locations and state.board.tiles is tiles
    assert target == spawn and target is not spawn and target is not selected
    assert state.board.get_tile(target).occupant_id == "hero_arien"
    assert step.pending_input["selection"] is selected


def test_token_respawn_removes_source_token_and_origin_links_with_one_event(cases):
    state, step = copy.deepcopy(cases["token-effects"])
    old = state.active_effects
    point = state.board.spawn_points[0].location
    step.pending_input = {"selection": point.model_dump()}
    out = step.resolve(state, state.execution_context)
    assert len(out.events) == 1 and out.events[0].event_type.value == "TOKEN_REMOVED"
    assert out.events[0].actor_id == "hero_arien" and out.events[0].metadata == {
        "reason": "respawn"
    }
    assert state.active_effects is not old and len(old) == 4
    assert [e.id for e in state.active_effects] == ["unrelated"]
    assert "respawn_token" not in state.entity_locations
    assert state.get_position("hero_arien") == point


def test_token_on_terrain_is_not_cleared_or_offered(cases):
    state, step = copy.deepcopy(cases["terrain-token"])
    point = state.board.spawn_points[0].location
    step.pending_input = {"selection": point.model_dump()}
    out = step.resolve(state, state.execution_context)
    assert out.input_request.request_type.value == "CHOOSE_RESPAWN_HEX"
    assert point not in out.input_request.context["valid_hexes"]
    assert state.get_position("respawn_token") == point


def test_multipiece_respawn_uses_first_supplied_id_even_with_reversed_registry(cases):
    state, step = copy.deepcopy(cases["piece-reversed-registry"])
    point = state.board.spawn_points[0].location
    step.pending_input = {"selection": point.model_dump()}
    step.resolve(state, state.execution_context)
    assert state.get_position("hero_razzle_piece_1") == point
    assert "hero_razzle" not in state.entity_locations
    assert len(state.get_piece_ids("hero_razzle")) == 1


def test_numeric_coordinate_coercion_and_ignored_extra_fields_match_hex_constructor(cases):
    for name in ("pending-numeric", "pending-extra"):
        state, step, out = run(cases, name)
        assert out.is_finished and state.has_board_presence("hero_arien")
        assert step.pending_input is state.execution_context["pending_input_alias"]


def test_actor_denied_spawn_can_fall_back_using_generic_nearest_empty_hexes(cases):
    state, _step, out = run(cases, "denied")
    marked = [sp.location for sp in state.board.spawn_points]
    assert all(state.validator.is_obstacle_for_actor(state, h, "hero_arien") for h in marked)
    assert out.input_request.request_type.value == "CHOOSE_RESPAWN"
    offered = out.input_request.context["valid_hexes"]
    assert offered and any(
        state.validator.is_obstacle_for_actor(state, h, "hero_arien") for h in offered
    )


def test_token_overrides_actor_denial_only_on_marked_spawn(cases):
    state, step, out = run(cases, "token-denied")
    point = state.board.spawn_points[0].location
    assert out.input_request.context["valid_hexes"] == [point]
    step.pending_input = {"selection": point.model_dump()}
    out = step.resolve(state, state.execution_context)
    assert state.get_position("hero_arien") == point and len(out.events) == 1
