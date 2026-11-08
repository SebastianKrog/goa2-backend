import json

from arena_profiles import MAPS, apply_calibrated_tier, generated_starts


def test_recipe_is_repeatable_and_holdout_has_a_separate_stream():
    heroes = ["A", "B", "C", "D", "E", "F"]
    tuning = list(generated_starts("extend", 17, 12, heroes))
    held_out = list(generated_starts("confirm", 17, 12, heroes))
    assert list(generated_starts("extend", 17, 12, heroes)) == tuning
    assert {start[1] for start in tuning}.isdisjoint(start[1] for start in held_out)
    assert tuple(start[0] for start in held_out[:6]) == MAPS
    assert all(len(set(red + blue)) == 4 for _, _, red, blue in held_out)


def test_screen_map_rotation_changes_between_batch_seeds():
    heroes = ["A", "B", "C", "D", "E", "F"]
    first = list(generated_starts("screen", 17, 3, heroes, map_offset=17 % len(MAPS)))
    second = list(generated_starts("screen", 18, 3, heroes, map_offset=18 % len(MAPS)))
    assert [start[0] for start in first] == [MAPS[5], MAPS[0], MAPS[1]]
    assert [start[0] for start in second] == list(MAPS[:3])
    assert {start[1] for start in first}.isdisjoint(start[1] for start in second)


def test_profile_pins_calibrated_caps(tmp_path):
    report = tmp_path / "calibration.json"
    report.write_text(json.dumps({
        "protocol": "nodes_v2", "worker_sha256": "worker",
        "node_tiers": [{"target_ms": 500, "planning_nodes": 1024, "resolution_nodes": 2048}],
    }))
    template = {"parameters": {}}
    apply_calibrated_tier(template, report, 500)
    assert template["parameters"] == {"planning_nodes": 1024, "resolution_nodes": 2048}
    assert template["budget_calibration"]["target_ms"] == 500
    assert template["budget_calibration"]["worker_sha256"] == "worker"
