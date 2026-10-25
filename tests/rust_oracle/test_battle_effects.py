"""Source contracts for battle exclusions and their distinct activation rules."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))


def test_battle_exclusions_sum_origins_and_reserve_overlapping_minions_in_effect_order():
    from battle_effect_traces import exclusion_states

    from goa2.domain.models import TeamColor
    from goa2.engine.steps import MinionBattleStep

    expected = {
        "ordinary": 2,
        "same-origin": 4,
        "opposite-caps": 1,
        "overlap": 2,
        "overlap-reversed": 3,
    }
    checked = set()
    for name, state in exclusion_states():
        if name in expected:
            counts = MinionBattleStep()._battle_exclusion_reductions(
                state, state.board.zones["battle"]
            )
            assert counts == {TeamColor.RED: 0, TeamColor.BLUE: expected[name]}, name
            checked.add(name)
    assert checked == set(expected)


def test_battle_exclusions_use_raw_active_flag_and_origin_team_not_duration_or_token_owner():
    from battle_effect_traces import exclusion_states

    from goa2.domain.models import TeamColor
    from goa2.engine.steps import MinionBattleStep

    expected = {"ordinary": 2, "expired": 2, "inactive": 0, "token-source": 0, "no-team": 0}
    checked = set()
    for name, state in exclusion_states():
        if name in expected:
            counts = MinionBattleStep()._battle_exclusion_reductions(
                state, state.board.zones["battle"]
            )
            assert counts[TeamColor.BLUE] == expected[name], name
            checked.add(name)
    assert checked == set(expected)
