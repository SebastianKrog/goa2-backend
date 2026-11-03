"""Source contracts for query pools, handler gates and fresh collection results."""

from live_query_traces import scenes
from object_execution_traces import reference


def test_unit_query_pool_excludes_select_only_illusion_candidates():
    wanted = {"query-world-plain-UNIT": 2, "query-world-plain-UNIT_OR_TOKEN": 5}
    for name, state, operations in scenes():
        if name not in wanted:
            continue
        reference(state, operations)
        assert state.execution_context["count"] == wanted.pop(name)
        assert len(state.execution_context["units"]) == state.execution_context["count"]
        assert state.execution_context["units"] is not state.execution_context["old_units"]
        assert state.execution_context["old_units"] == []
        if not wanted:
            break
    assert not wanted


def test_handler_gates_preserve_old_query_outputs_and_aliases():
    checked = 0
    for name, state, operations in scenes():
        if not name.startswith("query-gated-"):
            continue
        old = state.execution_context["result"]
        reference(state, operations)
        if name.endswith(("active", "skip-false")):
            assert state.execution_context["result"] is old
            assert state.execution_context["old_result"] is old
        else:
            assert state.execution_context["result"] is not old
        checked += 1
    assert checked == 6


def test_repeated_collect_allocates_new_list_and_feeds_for_each():
    checked = 0
    for name, state, operations in scenes():
        if not name.startswith("query-live-"):
            continue
        reference(state, operations)
        context = state.execution_context
        if name.endswith("count-history"):
            assert context["snapshots"] == [1]
            assert context["count"] == 2
        else:
            previous = context["snapshots"][0]
            assert context["units"] is not previous
            assert "hero_arien" not in previous and "hero_arien" in context["units"]
            assert "hero_mrak" in previous and "hero_mrak" not in context["units"]
            if name.endswith("collect-loop"):
                assert context["seen"] == context["units"]
        checked += 1
    assert checked == 3


def test_real_card_query_prefixes_preserve_threshold_conditions():
    checked = 0
    for name, state, operations in scenes():
        if not name.startswith("query-card-"):
            continue
        count, condition = state.execution_context["query_steps"]
        population = int(name.rsplit("-", 1)[1])
        reference(state, operations)
        assert state.execution_context[count.output_key] == population
        threshold = 2 if "steady_advance" in name else 1
        assert state.execution_context[condition.output_key] is (
            True if population >= threshold else None
        )
        checked += 1
    assert checked == 9
