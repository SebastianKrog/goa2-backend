from itertools import product
from math import prod

from arena_acceptance import BETTING, confirmation_pair_counts, held_out


def test_large_advantage_can_pass_with_fewer_pairs_than_a_small_noninferiority_margin():
    assert held_out([1] * 12, {"track": "stronger", "effect": 0, "alpha": 0.05})["status"] == "passed"
    margin = {"track": "non_inferior", "effect": 0.1, "alpha": 0.05}
    assert held_out([0] * 12, margin)["status"] == "inconclusive"
    assert held_out([0] * 1000, margin)["status"] == "passed"


def test_missing_pairs_widen_held_out_bounds_without_entering_the_estimate():
    config = {"track": "stronger", "effect": 0, "alpha": 0.05}
    complete = held_out([1] * 12, config)
    missing = held_out([1] * 12 + [None] * 12, config)
    assert missing["estimate"] == complete["estimate"] == 1
    assert missing["lower"] < complete["lower"]
    assert missing["status"] == "inconclusive"


def test_equivalence_requires_both_bounds_inside_the_declared_margin():
    config = {"track": "equivalent", "effect": 0.25, "alpha": 0.05}
    assert held_out([0] * 1000, config)["status"] == "passed"
    assert held_out([1] * 1000, config)["status"] == "rejected"


def test_confirmation_sizing_depends_on_effect_and_desired_power():
    stronger = confirmation_pair_counts({"track": "stronger", "effect": 0, "alpha": 0.05}, 0.4, 0.8)
    tied = confirmation_pair_counts({"track": "non_inferior", "effect": 0.1, "alpha": 0.05}, 0, 0.8)
    assert stronger["minimum_at_estimated_advantage"] == 38
    assert tied["minimum_at_estimated_advantage"] == 600
    assert tied["sufficient_for_power"] > stronger["sufficient_for_power"] > 38


def test_betting_mixture_rewards_tight_pair_outcomes_and_penalizes_missing_pairs():
    config = {"track": "non_inferior", "effect": 0.1, "alpha": 0.05, "method": BETTING}
    plan = confirmation_pair_counts(config, 0, 0.8)
    count = plan["optimistic_pairs_at_estimated_advantage"]
    assert count < 100
    assert held_out([0] * (count - 1), config)["status"] == "inconclusive"
    passed = held_out([0] * count, config)
    assert passed["status"] == "passed" and passed["method"] == BETTING
    spread = [1, -1] * (count // 2) + [0] * (count % 2)
    assert held_out(spread, config)["status"] == "inconclusive"
    assert held_out([0] * (count - 1) + [None], config)["lower"] < passed["lower"]


def test_betting_mixture_controls_false_promotion_for_varying_pair_means():
    config = {"track": "stronger", "effect": 0, "alpha": 0.05, "method": BETTING}
    probabilities = [0.8] * 4 + [0.2] * 4
    false_positive = sum(
        prod(p if value == 1 else 1 - p for value, p in zip(values, probabilities, strict=True))
        for values in product((-1, 1), repeat=len(probabilities))
        if held_out(values, config)["status"] == "passed"
    )
    assert 0 < false_positive <= config["alpha"]


def test_betting_equivalence_requires_both_sides_of_the_declared_margin():
    config = {"track": "equivalent", "effect": 0.2, "alpha": 0.05, "method": BETTING}
    count = confirmation_pair_counts(config, 0, 0.8)["optimistic_pairs_at_estimated_advantage"]
    result = held_out([0] * count, config)
    assert result["status"] == "passed"
    assert -0.2 < result["lower"] < result["upper"] < 0.2
    assert held_out([1] * 100, config)["status"] == "rejected"
