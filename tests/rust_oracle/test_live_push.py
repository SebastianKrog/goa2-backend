"""Independent source contracts for full push path and event behavior."""

import copy

import pytest
from live_push_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def push_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(push_scenes, name):
    state, ops = copy.deepcopy(push_scenes["arena-push-" + name])
    return state, reference(state, ops)


def events(records, kind):
    return [event for row in records for event in row["events"] if event["event_type"] == kind]


def test_signed_zero_distances_still_emit_push_and_queue_passives(push_scenes):
    for variant in ("zero", "negative", "boolean"):
        state, records = run(push_scenes, variant)
        push = events(records, "UNIT_PUSHED")
        assert len(push) == 1 and push[0]["metadata"] == {"distance": 0, "collision": False}
        assert push[0]["from_hex"] == push[0]["to_hex"]
        assert state.execution_context["push_victim_id"] == "hero_brogan"
        assert (
            records[0]["semantic_state"]["execution_stack"][-1]["type"] == "check_passive_abilities"
        )


def test_tokens_do_not_schedule_after_push_passives(push_scenes):
    state, records = run(push_scenes, "token-zero")
    assert len(events(records, "TOKEN_PUSHED")) == 1
    assert records[0]["semantic_state"]["execution_stack"][-1]["type"] == "set_context_flag"
    assert state.execution_context["push_victim_id"] == "push_token"


def test_actor_source_precedence_and_dynamic_distance_coercion(push_scenes):
    for variant, distance in (
        ("ordinary", 3),
        ("integer-string", 2),
        ("float", 2),
        ("literal-wins", 3),
        ("source-literal-wins", 3),
        ("null-distance", 3),
    ):
        _, records = run(push_scenes, variant)
        assert events(records, "UNIT_PUSHED")[0]["metadata"]["distance"] == distance
    state, records = run(push_scenes, "no-actor")
    assert not events(records, "UNIT_PUSHED")
    assert state.execution_context["collision"] == "old"
    _, records = run(push_scenes, "no-actor-explicit")
    assert events(records, "UNIT_PUSHED")[0]["actor_id"] is None


def test_obstacle_ignore_trims_landing_and_edges_report_collision(push_scenes):
    for variant, distance, collision in (
        ("terrain", 0, True),
        ("unit-wall", 0, True),
        ("ignore-wall", 3, False),
        ("trim-wall", 0, True),
        ("edge", 5, True),
    ):
        _, records = run(push_scenes, variant)
        assert events(records, "UNIT_PUSHED")[0]["metadata"] == {
            "distance": distance,
            "collision": collision,
        }


def test_mine_path_snapshots_and_order_survive_literal_queueing(push_scenes):
    state, records = run(push_scenes, "mines-ordered-mines")
    first = records[0]["semantic_state"]
    assert first["execution_context"]["triggered_mine_ids"] == ["push-mine-1", "push-mine-2"]
    trigger = first["execution_stack"][-1]
    assert trigger["type"] == "trigger_mine" and trigger["mine_ids"] == [
        "push-mine-1",
        "push-mine-2",
    ]
    assert [event["target_id"] for event in events(records, "MINE_TRIGGERED")] == [
        "push-mine-1",
        "push-mine-2",
    ]
    assert state.execution_context["old_position"] != state.entity_locations["hero_arien"]


def test_trimmed_and_unowned_mines_do_not_trigger(push_scenes):
    for variant in (
        "trim-last",
        "trim-ignore-wall",
        "friendly",
        "unowned",
        "unknown-owner",
        "topology-anchor",
    ):
        _, records = run(push_scenes, "mines-" + variant)
        assert not events(records, "MINE_TRIGGERED")
    for variant in ("trim-last", "trim-ignore-wall", "topology-anchor"):
        _, records = run(push_scenes, "mines-" + variant)
        assert events(records, "UNIT_PUSHED")[0]["metadata"] == {"distance": 0, "collision": True}


def test_mandatory_denial_aborts_and_optional_unknown_target_continues(push_scenes):
    state, records = run(push_scenes, "unknown")
    assert "push_done" not in state.execution_context
    assert not events(records, "UNIT_PUSHED")
    state, _ = run(push_scenes, "unknown-optional")
    assert state.execution_context["push_done"]


def test_piece_uses_raw_target_identity_and_enemy_mines(push_scenes):
    _, records = run(push_scenes, "mines-piece")
    assert events(records, "UNIT_PUSHED")[0]["target_id"] == "hero_razzle_piece_1"
    assert len(events(records, "MINE_TRIGGERED")) == 2


def test_primary_push_cards_produce_real_displacement(push_scenes):
    for effect in (
        "rogue_wave",
        "tidal_blast",
        "brutal_jab",
        "mighty_punch",
        "savage_kick",
        "point_blank_shot",
    ):
        _, records = run(push_scenes, "card-" + effect + "-positive")
        assert any(
            event["metadata"]["distance"] > 0 for event in events(records, "UNIT_PUSHED")
        ), effect
