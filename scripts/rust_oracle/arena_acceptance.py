"""Predeclared side-paired arena stages and held-out acceptance."""

from collections import Counter
from math import exp, floor, isfinite, log, log1p, sqrt
from statistics import mean


HOEFFDING = "hoeffding_paired_v1"
BETTING = "betting_paired_v1"
BET_FRACTIONS = (0.0625, 0.125, 0.25, 0.5, 0.75, 0.9)


def _number(value, minimum, maximum):
    return type(value) in (int, float) and minimum <= value <= maximum and isfinite(value)


def validate_acceptance(config, starts):
    expected = {
        "preflight_pairs", "screen_pairs", "extend_batches", "confirm_pairs",
        "advance_at", "stop_below", "max_unfinished_rate", "max_fallback_rate",
        "track", "effect", "alpha",
    }
    if (not isinstance(config, dict) or config.keys() - {"method"} != expected
            or not isinstance(config["extend_batches"], list)):
        raise ValueError("invalid arena acceptance fields")
    counts = [config["preflight_pairs"], config["screen_pairs"],
              *config["extend_batches"], config["confirm_pairs"]]
    if any(type(value) is not int or value < 1 for value in counts) or sum(counts) != len(starts):
        raise ValueError("acceptance counts must account for every distinct start")
    for key, lower, upper in (
        ("advance_at", -1, 1), ("stop_below", -1, 1),
        ("max_unfinished_rate", 0, 1), ("max_fallback_rate", 0, 1),
        ("effect", 0, 1), ("alpha", 0, 0.5),
    ):
        if not _number(config[key], lower, upper):
            raise ValueError(f"invalid acceptance value for {key}")
    if config.get("method", HOEFFDING) not in (HOEFFDING, BETTING):
        raise ValueError("unknown arena confidence method")
    track = config["track"]
    if track not in ("stronger", "non_inferior", "equivalent"):
        raise ValueError("unknown acceptance track")
    if (config["stop_below"] > config["advance_at"] or not 0 < config["alpha"] < 0.5
            or (track != "stronger" and not 0 < config["effect"] < 1)
            or (track == "stronger" and config["effect"] >= 1)):
        raise ValueError("invalid acceptance thresholds")


def stages(profile):
    """Each stage covers a contiguous range in the normalized starts list."""
    config = profile.get("acceptance")
    if config is None:
        return [("diagnostic", 0, len(profile["starts"]))]
    sizes = [
        ("preflight", 2 * config["preflight_pairs"]), ("screen", config["screen_pairs"]),
        *(("extend", count) for count in config["extend_batches"]),
        ("confirm", config["confirm_pairs"]),
    ]
    offset = 0
    result = []
    for name, count in sizes:
        result.append((name, offset, offset + count))
        offset += count
    return result


def log_mixture(log_products):
    peak = max(log_products)
    return peak + log(sum(exp(value - peak) for value in log_products) / len(log_products))


def log_bet(value, null, fraction):
    return log1p(fraction * (value - null) / (1 + null))


def betting_lower_bound(values, missing, log_threshold):
    counts = Counter(values)
    counts[-1] += missing
    low, high = -1.0, 1.0
    for _ in range(52):
        null = (low + high) / 2
        log_products = [sum(count * log_bet(value, null, fraction) for value, count in counts.items())
                        for fraction in BET_FRACTIONS]
        if log_mixture(log_products) > log_threshold:
            low = null
        else:
            high = null
    return low


def held_out(values, config):
    """Fixed-sample confidence bounds for independent paired outcomes in [-1, 1]."""
    n = len(values)
    track, effect, alpha = (config[key] for key in ("track", "effect", "alpha"))
    log_threshold = -log(alpha) + (log(2) if track == "equivalent" else 0)
    threshold = effect if track == "stronger" else -effect
    observed = [value for value in values if value is not None]
    estimate = mean(observed) if observed else None
    missing = n - len(observed)
    method = config.get("method", HOEFFDING)
    if not n:
        lower, upper = -1, 1
    elif method == BETTING:
        lower = betting_lower_bound(observed, missing, log_threshold)
        upper = -betting_lower_bound([-value for value in observed], missing, log_threshold)
    else:
        radius = sqrt(2 * log_threshold / n)
        total = sum(observed)
        lower = max(-1, (total - missing) / n - radius)
        upper = min(1, (total + missing) / n + radius)
    if lower > threshold and (track != "equivalent" or upper < effect):
        verdict = "passed"
    elif upper <= threshold or (track == "equivalent" and lower >= effect):
        verdict = "rejected"
    else:
        verdict = "inconclusive"
    return {"status": verdict, "track": track, "effect": effect, "alpha": alpha,
            "method": method, "pairs": n, "scored_pairs": len(observed),
            "estimate": estimate,
            "lower": lower, "upper": upper}


def optimistic_betting_count(advantage, threshold, log_threshold):
    if advantage <= threshold:
        raise ValueError("expected advantage is outside the acceptance region")
    growth = [log_bet(advantage, threshold, fraction) for fraction in BET_FRACTIONS]
    low, high = 0, floor((log_threshold + log(len(growth))) / max(growth)) + 1
    while low + 1 < high:
        count = (low + high) // 2
        if log_mixture([count * term for term in growth]) > log_threshold:
            high = count
        else:
            low = count
    return high


def confirmation_pair_counts(config, advantage, power):
    """Scenario sizing for the pinned method, with a power guarantee for Hoeffding."""
    if not _number(advantage, -1, 1) or not _number(power, 0, 1) or not 0 < power < 1:
        raise ValueError("advantage must be in [-1, 1] and power must be in (0, 1)")
    track, effect, alpha = (config[key] for key in ("track", "effect", "alpha"))
    threshold = effect if track == "stronger" else -effect
    if config.get("method", HOEFFDING) == BETTING:
        log_threshold = -log(alpha) + (log(2) if track == "equivalent" else 0)
        count = optimistic_betting_count(advantage, threshold, log_threshold)
        if track == "equivalent":
            count = max(count, optimistic_betting_count(-advantage, threshold, log_threshold))
        return {"method": BETTING, "optimistic_pairs_at_estimated_advantage": count,
                "assumed_advantage": advantage}
    separation = advantage - threshold
    beta = 1 - power
    if track == "equivalent":
        separation = min(separation, effect - advantage)
        alpha /= 2
        beta /= 2
    if separation <= 0:
        raise ValueError("expected advantage is outside the acceptance region")
    alpha_term, beta_term = sqrt(log(1 / alpha)), sqrt(log(1 / beta))
    return {
        "minimum_at_estimated_advantage": floor(2 * alpha_term**2 / separation**2) + 1,
        "sufficient_for_power": floor(2 * (alpha_term + beta_term)**2 / separation**2) + 1,
        "assumed_advantage": advantage, "target_power": power,
    }


def guardrail(summary, config):
    outcomes = summary["outcomes"]
    games = sum(count for name, count in outcomes.items() if name != "verified")
    if not games:
        return "no completed arena games"
    if (outcomes.get("unfinished", 0) + outcomes.get("individual_winner", 0)) / games > config["max_unfinished_rate"]:
        return "unfinished-game guardrail exceeded"
    for role, work in summary["work"].items():
        requests = sum(value for key, value in work.items() if key.endswith("_requests"))
        fallbacks = sum(value for key, value in work.items() if key.endswith(("_unsupported", "_unscored")))
        if requests and fallbacks / requests > config["max_fallback_rate"]:
            return f"{role} fallback guardrail exceeded"
    return None
