from types import SimpleNamespace

import pytest
import rollout_parity


@pytest.mark.parametrize("include_completed_pair", [False, True])
def test_strength_estimate_excludes_unfinished_pairs(
    monkeypatch, capsys, include_completed_pair,
):
    root = SimpleNamespace(teams={
        side: SimpleNamespace(heroes=[]) for side in ("RED", "BLUE")
    })

    def starts(_maps, _selected_map, *, held_out, midgame, game_type):
        assert held_out and not midgame and game_type == "QUICK"
        yield "capped", 1, root
        if include_completed_pair:
            yield "terminal", 2, root

    stats = {"fallback": 0, "planning": 1, "resolution": 0, "forced": 0}

    def match(_worker, map_id, _root, turns, **kwargs):
        assert turns == 64
        assert kwargs["decision_ms"] == 5000
        red, blue = kwargs["policy_pairs"]
        assert red == blue[::-1]
        results = [
            {"phase": "GAME_OVER", "winner_team": side, "trace": [], "policies": [stats, stats]}
            for side in ("RED", "BLUE")
        ]
        if map_id == "capped":
            results[1].update(phase="PLANNING", winner_team=None)
        return results

    monkeypatch.setattr(rollout_parity, "strength_start_positions", starts)
    monkeypatch.setattr(rollout_parity, "verify_rust_match_replay", match)
    rollout_parity.verify_strength_baseline(
        None, None, 5000, trial="lane_timing", held_out=True,
    )

    output = capsys.readouterr().out
    assert "'unfinished': 1" in output
    if include_completed_pair:
        assert "Completed 1/2 side-swapped pairs" in output
        assert "Candidate game points from completed pairs: 2/2" in output
        assert "0.5000 (1 starts)" in output
    else:
        assert "Completed 0/1 side-swapped pairs" in output
        assert "Paired strength estimate unavailable" in output
    assert "Held-out lane timing threshold missed" in output
