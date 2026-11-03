"""Frozen-source turret lifecycle and imported Salvage Parts contracts.

Raw boundary fixtures deliberately exercise aliases and stack prefixes rather
than effect-author helpers: these are cross-engine oracle contracts.
"""

import copy

import pytest
from live_turret_traces import scenes
from object_execution_traces import reference

TURRET = "trinkets_turret"


@pytest.fixture(scope="module")
def turret_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(turret_scenes, name):
    state, ops = copy.deepcopy(turret_scenes["turret-" + name])
    return state, reference(state, ops)


def test_creation_owner_precedence_and_output(turret_scenes):
    for variant, owner in (
        ("new", "hero_trinkets"),
        ("owner-literal", "hero_hanu"),
        ("owner-key", "hero_brogan"),
        ("owner-missing", "hero_hanu"),
        ("owner-null", "hero_trinkets"),
        ("owner-empty", ""),
        ("empty-owner-key", "hero_hanu"),
        ("no-actor", None),
    ):
        state, records = run(turret_scenes, "place-" + variant)
        turret = state.misc_entities[TURRET]
        assert turret.owner_id == owner
        assert turret.name == "Turret"
        assert state.execution_context["placed"] == TURRET
        events = [e for row in records for e in row["events"]]
        assert len(events) == 1
        assert events[0]["event_type"] == "BOARD_ENTITY_PLACED"
        assert events[0]["actor_id"] == (owner or None)


def test_reuse_preserves_model_identity_name_and_obstacle_flag(turret_scenes):
    for variant in (
        "reuse",
        "same-position",
        "offboard-reuse",
        "reuse-obstacle-false",
        "reuse-no-actor",
    ):
        state, records = run(turret_scenes, "place-" + variant)
        turret = state.misc_entities[TURRET]
        assert turret is state.execution_context["turret_alias"]
        assert turret.name == "Custom Turret"
        assert turret.is_obstacle == (variant != "reuse-obstacle-false")
        assert turret.owner_id == (None if variant == "reuse-no-actor" else "hero_trinkets")
        assert sum(len(row["events"]) for row in records) == 1


def test_failures_abort_only_mandatory_actions(turret_scenes):
    for verb, variants in (
        (
            "place",
            ("missing", "null", "false", "occupied", "optional-missing", "optional-occupied"),
        ),
        (
            "remove",
            (
                "offboard",
                "missing",
                "placeholder",
                "optional-offboard",
                "optional-missing",
                "optional-placeholder",
            ),
        ),
    ):
        for variant in variants:
            state, records = run(turret_scenes, verb + "-" + variant)
            assert state.execution_context.get("tail", False) == variant.startswith("optional-")
            assert not [e for row in records for e in row["events"]]
            if verb == "place":
                assert TURRET not in state.misc_entities


def test_removal_and_replacement_keep_linked_effects_and_model(turret_scenes):
    for variant in ("remove-effects", "move-effects", "reuse-after-remove"):
        state, records = run(turret_scenes, variant)
        assert state.misc_entities[TURRET] is state.execution_context["turret_alias"]
        assert state.active_effects is state.execution_context["old_effects"]
        assert len(state.active_effects) == 1
        assert state.active_effects[0].id == "turret-link"
        assert (state.get_position(TURRET) is None) == (variant == "remove-effects")
        events = [e["event_type"] for row in records for e in row["events"]]
        assert events == (
            {
                "remove-effects": ["BOARD_ENTITY_REMOVED"],
                "move-effects": ["BOARD_ENTITY_PLACED"],
                "reuse-after-remove": ["BOARD_ENTITY_REMOVED", "BOARD_ENTITY_PLACED"],
            }[variant]
        )


def test_spatial_aliases_virtual_tiles_and_same_position_events(turret_scenes):
    for verb in ("place", "remove"):
        state, _ = run(turret_scenes, verb + "-snapshot")
        assert state.last_turn_positions is state.entity_locations
        assert state.execution_context["locations_alias"] is state.entity_locations
        state, _ = run(turret_scenes, verb + "-virtual")
        assert (state.get_position(TURRET) is None) == (verb == "remove")
        if verb == "place":
            assert state.get_position(TURRET).q == 20
    state, records = run(turret_scenes, "place-same-position")
    event = next(e for row in records for e in row["events"])
    assert event["from_hex"] == event["to_hex"]


def test_turret_is_an_obstacle_but_not_a_unit_or_token(turret_scenes):
    state, records = run(turret_scenes, "queries")
    assert TURRET not in state.execution_context["units"]
    request = next(row["request"] for row in records if row["request"])
    occupied = state.get_position(TURRET).model_dump(mode="json")
    assert occupied not in request["options"]


def test_imported_salvage_parts_branches_finish_or_abort_at_missing_turret(turret_scenes):
    for choice in (1, 2, 3):
        for placed in (False, True):
            for mode in ("first", "last", "invalid"):
                state, records = run(turret_scenes, f"salvage-{choice}-{placed}-{mode}")
                assert state.execution_stack == []
                if choice > 1 and not placed:
                    assert "tail" not in state.execution_context
                    assert not [e for row in records for e in row["events"]]
                else:
                    assert state.execution_context["tail"] is True
                    assert (state.get_position(TURRET) is not None) == (choice == 1)
