"""Frozen source contracts for ordered UNIT choices and live selector aliases."""

from live_unit_traces import scenes
from object_execution_traces import reference


def test_unit_and_token_pools_keep_source_order_and_prompt_ownership():
    wanted = {
        "unit-world-plain-unit",
        "unit-world-plain-tokens",
        "unit-world-empty-actor-unit",
        "unit-world-no-actor-unit",
        "unit-input-piece-owner",
        "unit-input-owner-key",
    }
    checked = set()
    for name, state, operations in scenes():
        if name not in wanted:
            continue
        checked.add(name)
        first = reference(state, operations)[0]
        request = first["request"]
        if name == "unit-world-plain-unit":
            ids = [option["id"] for option in request["options"]]
            illusions = [value for value in ids if value.startswith("illusion-")]
            assert illusions == ["illusion-0", "illusion-1", "illusion-2"]
            assert ids[-3:] == illusions
        if name == "unit-world-plain-tokens":
            assert request["request_type"] == "SELECT_UNIT_OR_TOKEN"
        if name == "unit-world-empty-actor-unit":
            assert request["player_id"] == ""
        if name == "unit-world-no-actor-unit":
            assert request["player_id"] == "None"
        if name == "unit-input-piece-owner":
            assert request["player_id"] == "hero_razzle"
        if name == "unit-input-owner-key":
            assert request["player_id"] == "hero_mrak"
        if checked == wanted:
            break
    assert checked == wanted


def test_automatic_selection_clears_step_pending_but_retains_detached_alias():
    for name, state, operations in scenes():
        if name != "unit-input-dirty-auto":
            continue
        pending = state.execution_context["pending_alias"]
        reference(state, operations)
        assert state.execution_context["chosen"] == "hero_mrak"
        assert state.execution_context["selector"].pending_input is None
        assert state.execution_context["selector"].pending_request_id is None
        assert state.execution_context["pending_alias"] is pending
        assert pending == {"selection": "invalid"}
        return
    raise AssertionError("missing automatic unit fixture")


def test_repeated_shared_selection_reads_new_history_and_new_actor():
    checked = 0
    for name, state, operations in scenes():
        if not name.startswith("unit-live-"):
            continue
        original = state.execution_context["history_alias"]
        records = reference(state, operations)
        requests = [r["request"] for r in records if r["request"]]
        assert len(requests) == 2
        assert requests[0]["id"] != requests[1]["id"]
        first = original[0]
        later = [o["id"] for o in requests[1]["options"]]
        if name != "unit-live-replace":
            assert first not in later
        if name == "unit-live-actor":
            assert state.current_actor_id == "hero_mrak"
        if name == "unit-live-replace":
            assert state.execution_context["history"] is not original
        assert state.execution_context["selector"].pending_input is None
        checked += 1
    assert checked == 3
