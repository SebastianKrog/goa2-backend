"""Tunable policy fields that preserve the arena's common work and rules."""

import json


def integer(minimum, maximum):
    return lambda value: type(value) is int and minimum <= value <= maximum


def choice(*values):
    return lambda value: value in values


def boolean(value):
    return type(value) is bool


COMMON = {
    "max_depth": integer(0, 8),
    "rollout_turns": integer(1, 4),
    "sample_worlds": integer(1, 8),
    "evaluation": choice("material_position", "lab_attack_range_v1",
                         *(f"lane_pressure_v{version}" for version in range(1, 6)),
                         "lane_pressure_v3_accumulated_gold"),
}

FIELDS = {
    "planning": dict(
        COMMON, cap=integer(1, 32), reply_cap=integer(1, 32), reply_probe_cap=integer(1, 32),
        forecast_node_share=choice("zero", "eighth", "quarter"), greedy_card_order=boolean,
        legacy_fixed_followup=boolean, fixed_future_planning=boolean, branch_future_planning=boolean,
    ),
    "resolution": dict(
        COMMON, opponent_policy=choice("adversarial", "adversarial_attack_targets",
                                      "adversarial_movement_targets", "first_legal"),
    ),
}


def validate_policy(policy):
    if not isinstance(policy, dict) or policy.keys() - FIELDS.keys():
        raise ValueError("arena policy accepts only planning and resolution settings")
    normalized = {}
    for phase, fields in policy.items():
        validators = FIELDS[phase]
        if not isinstance(fields, dict) or fields.keys() - validators.keys():
            raise ValueError(f"unknown or reserved arena {phase} policy setting")
        for key, value in fields.items():
            if not validators[key](value):
                raise ValueError(f"invalid arena {phase} policy setting {key}")
        if fields.get("fixed_future_planning") and fields.get("branch_future_planning"):
            raise ValueError("choose at most one future planning mode")
        if fields:
            normalized[phase] = dict(sorted(fields.items()))
    return dict(sorted(normalized.items()))


def load_policy(path):
    return validate_policy(json.loads(path.read_text(encoding="utf-8")))


def apply_policy(request, phase, fields):
    request.update((key, value) for key, value in fields.items() if key != "sample_worlds")
    if "sample_worlds" in fields:
        worlds = fields["sample_worlds"]
        if phase == "planning":
            request["bot_team"] = dict(request["bot_team"], worlds=worlds)
        else:
            request["resolution_sample_worlds"] = worlds
