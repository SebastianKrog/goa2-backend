import pytest

from arena_match import play_match
from arena_policy import validate_policy
from arena_progress import StallClock


def position(turn, *, lane="Mid", blue_lives=6, minion=True, finished=False):
    state = {
        "battle_zones": {"lane_1": lane},
        "wave_counters": {"lane_1": 5},
        "entity_locations": {"minion_1": {"q": 0, "r": 0, "s": 0}} if minion else {},
        "teams": {
            "RED": {"heroes": [{"id": "red"}], "minions": [{"id": "minion_1"}], "life_counters": 6},
            "BLUE": {"heroes": [{"id": "blue"}], "minions": [], "life_counters": blue_lives},
        },
    }
    return {"round": turn // 4 + 1, "turn": turn % 4 + 1, "revision": turn,
            "kind": "finished" if finished else "planning", "state": state,
            "envelope": {"state": state}, "upgrade_checkpoints": {},
            "slots": [{"player_id": "red", "team": "RED"}, {"player_id": "blue", "team": "BLUE"}],
            "legal_choices": {"kind": "planning", "players": []}}


def test_quiet_turns_reset_by_kind_and_exclude_turn_with_progress():
    clock = StallClock(position(0))
    for turn in range(1, 5):
        clock.observe(position(turn))
    assert clock.age("lane") == clock.age("combat") == 4
    clock.observe(position(4, minion=False))
    assert clock.age("combat") == 0 and clock.age("lane") == 4
    clock.observe(position(5, minion=False))
    assert clock.age("combat") == 0 and clock.age("lane") == 5
    clock.observe(position(6, minion=False, lane="BlueBeach"))
    assert clock.age("combat") == clock.age("lane") == 0
    clock.observe(position(7, minion=False, lane="BlueBeach", finished=True))
    assert clock.result() == {"longest_quiet_turns": {"combat": 4, "lane": 5},
                              "first_lane_progress_turn": 6}


def test_v2_stall_override_is_scoped_and_stops_after_progress():
    gold = {"evaluation": "lane_pressure_v3_accumulated_gold"}
    policy = dict(planning=gold, resolution=gold,
                  stall={"turns": 4, "progress": "lane", "evaluation": "stall_wave_clear"})
    positions = [position(turn, lane="Mid" if turn < 5 else "BlueBeach", finished=turn == 7)
                 for turn in range(8)]

    class Referee:
        def request(self, op, **request):
            if op == "arena_start":
                return positions[0]
            return {"position": positions[request["revision"] + 1], "decision": {"kind": "planning"}}

    class PolicyWorker:
        def request(self, op, **request):
            return {"nodes": 2, "score": 3, "commitments": [{"player_id": request["player_id"]}]}

    seen = {}
    result = play_match(
        Referee(), {side: PolicyWorker() for side in ("RED", "BLUE")}, positions[0]["envelope"],
        turns=7, seed=19, planning_nodes=10, resolution_nodes=10,
        policy_profiles={"RED": policy, "BLUE": {"planning": gold}},
        observe_request=lambda _, __, req, pos: seen.setdefault(
            (pos["revision"], req["player_id"]), req["evaluation"]),
    )
    assert [key for key, value in seen.items() if value == "stall_wave_clear"] == [(4, "red")]
    assert result["work"]["RED"]["planning_stall_activations"] == 1
    assert result["stall"]["longest_quiet_turns"]["lane"] == 4


@pytest.mark.parametrize("bad", [{"turns": 6}, {"turns": 3, "progress": "lane",
                                                  "evaluation": "stall_wave_clear"}])
def test_stall_policy_requires_a_known_rule_within_the_allowed_window(bad):
    with pytest.raises(ValueError, match="stall"):
        validate_policy({"stall": bad})
