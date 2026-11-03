"""Oracle contracts for sequential selections and persistent list aliases.

Raw fixtures intentionally test execution boundaries and model identity, outside
normal effect-author helpers. Python src remains the frozen gold standard.
"""

import copy

import pytest
from live_multi_selection_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def multi_scenes():
    return {name: (state, ops) for name, state, ops in scenes()}


def run(multi_scenes, name):
    state, ops = copy.deepcopy(multi_scenes["multi-" + name])
    return state, reference(state, ops)


def test_unit_pool_excludes_illusions_but_unit_or_token_includes_them(multi_scenes):
    for tokens in (False, True):
        _, records = run(multi_scenes, f"world-plain-{tokens}")
        request = records[0]["request"]
        assert request["request_type"] == "SELECT_UNIT"
        ids = [option["id"] for option in request["options"]]
        assert any(value.startswith("illusion-") for value in ids) == tokens


def test_actorless_output_aliases_the_persistent_selection_list(multi_scenes):
    for variant in ("no-actor", "empty-actor", "no-actor-preselected"):
        state, records = run(multi_scenes, variant + "-first")
        selector = state.execution_context["selector"]
        assert state.execution_context["chosen"] is selector.selections
        assert state.execution_context["selected_alias"] is selector.selections
        assert all(row["request"] is None for row in records)
        assert len(selector.selections) == int(variant == "no-actor-preselected")


def test_done_retains_pending_mapping_and_copies_output(multi_scenes):
    for variant in ("done", "skip"):
        state, records = run(multi_scenes, variant + "-first")
        selector = state.execution_context["selector"]
        assert selector.pending_input is state.execution_context["pending_alias"]
        assert state.execution_context["chosen"] == selector.selections
        assert state.execution_context["chosen"] is not selector.selections
        assert not [row for row in records if row["request"]]
    for variant in ("done-under", "skip-under"):
        state, records = run(multi_scenes, variant + "-first")
        assert records[0]["request"]["can_skip"] is False
        assert len(state.execution_context["chosen"]) == 2


def test_bounds_are_checked_after_acceptance_and_empty_candidates_enforce_minimum(multi_scenes):
    for variant, count in (
        ("max-zero", 1),
        ("max-negative", 1),
        ("preselected-max", 3),
        ("min-above-max", 1),
    ):
        state, _ = run(multi_scenes, variant + "-first")
        assert len(state.execution_context["chosen"]) == count
        assert state.execution_context["tail"] is True
    for variant, finishes in (("empty", False), ("optional-empty", True)):
        state, _ = run(multi_scenes, variant + "-first")
        assert state.execution_context["chosen"] == []
        assert state.execution_context.get("tail", False) == finishes


def test_accepted_selection_changes_the_output_filter_before_the_next_scan(multi_scenes):
    state, _ = run(multi_scenes, "output-filter-first")
    ctx = state.execution_context
    assert len(ctx["chosen"]) == 1
    assert len(ctx["old_output"]) == 3
    assert ctx["chosen"] is not ctx["selected_alias"]
    assert ctx["chosen"] == ctx["selected_alias"]
    assert "tail" not in ctx
    state, _ = run(multi_scenes, "shared-output-first")
    ctx = state.execution_context
    assert ctx["old_output"] is ctx["selected_alias"]
    assert ctx["chosen"] == ctx["selected_alias"]
    assert ctx["chosen"] is not ctx["selected_alias"]


def test_handler_gates_keep_old_output_and_selection_list(multi_scenes):
    for variant in ("skip-gate", "active-gate"):
        state, records = run(multi_scenes, variant + "-first")
        assert state.execution_context["chosen"] is state.execution_context["old_output"]
        assert state.execution_context["selected_alias"] == []
        assert not [row for row in records if row["request"]]
        assert state.execution_context["tail"] is True


def test_hex_multiselect_prompts_with_string_coordinates_as_unit_options(multi_scenes):
    state, records = run(multi_scenes, "hex-plain-range-invalid")
    request = records[0]["request"]
    assert request["request_type"] == "SELECT_UNIT"
    assert all(
        option["id"].startswith("(") and option["metadata"] == {} for option in request["options"]
    )
    assert len(set(state.execution_context["chosen"])) == 2
    assert state.execution_context["chosen"] == state.execution_context["selected_alias"]


def test_injected_requests_keep_the_existing_token_when_pending_input_is_cleared(multi_scenes):
    for value in (None, 0.0, -0.0, 1.5, 1e-5, 1e-4, 1e15, 1e16, 1e20, -1e-20):
        state, ops = copy.deepcopy(multi_scenes[f"multi-injected-request-{value}"])
        reference(state, ops[:1])
        selector = state.execution_context["selector"]
        assert selector.pending_input is None
        assert selector.pending_request_id == "fixture/multi/request"
        assert state.execution_context["pending_alias"] == {"selection": value}


def test_source_built_turret_discard_bodies_finish_and_remove_turret(multi_scenes):
    for effect in ("self_destruct", "emergency_protocol"):
        for placed in (False, True):
            state, ops = copy.deepcopy(multi_scenes[f"multi-card-{effect}-{placed}-first"])
            before = sum(len(hero.hand) for team in state.teams.values() for hero in team.heroes)
            records = reference(state, ops)
            victims = state.execution_context["self_destruct_victims"]
            assert len(victims) == (2 if placed else 0)
            after = sum(len(hero.hand) for team in state.teams.values() for hero in team.heroes)
            assert before - after == len(victims)
            assert state.get_position("trinkets_turret") is None
            assert state.execution_stack == []
            assert state.execution_context.get("tail", False) == placed
            events = [e["event_type"] for row in records for e in row["events"]]
            assert events.count("BOARD_ENTITY_REMOVED") == int(placed)
