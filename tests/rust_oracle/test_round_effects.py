"""Source contracts for immediate cleanup and ordered token-effect events."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))


def test_cleanup_does_not_execute_round_finishing_or_expire_turn_effects():
    from round_effect_traces import states

    from goa2.engine.steps import EndPhaseCleanupStep

    expected = {"round-finishing-ignored": 0, "turn-finishing-retained": 1, "after-cards-round": 0}
    checked = set()
    for name, state in states():
        if name not in expected:
            continue
        EndPhaseCleanupStep().resolve(state, state.execution_context)
        assert len(state.active_effects) == expected[name], name
        assert "should_not_finish" not in state.execution_context
        checked.add(name)
    assert checked == set(expected)


def test_cleanup_token_events_count_only_effects_removed_at_each_ordered_token():
    from round_effect_traces import states

    from goa2.engine.steps import EndPhaseCleanupStep

    checked = set()
    for name, state in states():
        if name not in ["two-tokens-ordered", "empty-token-binding"]:
            continue
        result = EndPhaseCleanupStep().resolve(state, state.execution_context)
        events = [
            (e.target_id, e.metadata["effects_removed"])
            for e in result.events
            if e.event_type.value == "TOKEN_REMOVED"
        ]
        if name == "two-tokens-ordered":
            assert events == [("cleanup-token", 1), ("cleanup-token-2", 1)]
            assert not state.active_effects
        else:
            assert events == [("cleanup-token", 0)]
            assert len(state.active_effects) == 1 and state.active_effects[0].token_id == ""
        checked.add(name)
    assert checked == {"two-tokens-ordered", "empty-token-binding"}
