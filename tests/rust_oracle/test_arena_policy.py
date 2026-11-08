import pytest

from arena_policy import apply_policy, validate_policy


@pytest.mark.parametrize("key", ["max_nodes", "work_budget", "time_ms", "workers",
                                  "seed", "player_id", "bot_team"])
def test_policy_rejects_work_and_rules_overrides(key):
    with pytest.raises(ValueError, match="unknown or reserved"):
        validate_policy({"planning": {key: 1}})


def test_policy_can_tune_world_count_without_replacing_knowledge_mode_or_roster():
    profile = validate_policy({
        "planning": {"sample_worlds": 4, "forecast_node_share": "eighth"},
        "resolution": {"sample_worlds": 3, "evaluation": "lane_pressure_v4"},
    })
    planning = {"max_nodes": 16, "bot_team": {"mode": "coordinated", "bot_ids": ["x", "y"], "worlds": 2}}
    resolution = {"max_nodes": 32, "resolution_sample_worlds": 2}
    apply_policy(planning, "planning", profile["planning"])
    apply_policy(resolution, "resolution", profile["resolution"])
    assert planning == {"max_nodes": 16, "bot_team": {"mode": "coordinated", "bot_ids": ["x", "y"], "worlds": 4},
                        "forecast_node_share": "eighth"}
    assert resolution == {"max_nodes": 32, "resolution_sample_worlds": 3, "evaluation": "lane_pressure_v4"}
