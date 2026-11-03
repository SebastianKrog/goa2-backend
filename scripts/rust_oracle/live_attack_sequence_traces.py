"""Attack macro boundaries, shared filter models and optional shallow copies.

Raw fixtures isolate expansion before the separate defense/reaction consumers.
Expected values and aliases come exclusively from the frozen Python handler.
"""

from __future__ import annotations


def scenes():
    from combat_effect_traces import state_for

    from goa2.domain.models import StatType
    from goa2.engine.filters import ImmunityFilter, RangeFilter
    from goa2.engine.steps import AttackSequenceStep, SetContextFlagStep

    variants = [
        "default",
        "ranged",
        "initiative",
        "existing",
        "existing-output",
        "fallback-output",
        "missing-mandatory",
        "missing-optional",
        "empty-keys",
        "no-actor",
        "empty-actor",
        "unknown-actor",
        "negative",
        "zero",
        "negative-range",
        "bonus-int",
        "bonus-bool",
        "bonus-float",
        "bonus-string",
        "bonus-missing",
        "stats",
        "custom-filters",
        "marks-existing",
        "marks-key-alias",
        "tuple-log",
        "dict-log",
        "string-log",
        "shared-tracking",
        "marks-absent",
        "target-aggregate",
        "target-zero",
        "target-false",
        "target-empty",
        "target-flag",
        "skip",
        "active",
        "optional-first",
        "optional-skip",
        "optional-shared",
        "optional-output",
        "basic-current",
        "colored-current",
        "performing-card",
    ]
    for variant in variants:
        state, _, minion = state_for()
        context = state.execution_context
        step = AttackSequenceStep(step_id="fixture/attack", damage=4, range_val=2)
        context.update(
            attack_is_ranged="old",
            attack_damage="old",
            attack_is_basic="old",
            defense_uses_initiative="old",
            attacker_id="old",
        )
        marks, log = {"previous": 0}, [["previous_victim", "previous_killer"]]
        context.update(defeat_marks=marks, marks_alias=marks, defeat_log=log, log_alias=log)
        if variant == "ranged":
            step.is_ranged = True
        if variant == "initiative":
            step.defense_uses_initiative = True
        if variant in {
            "existing",
            "existing-output",
            "missing-mandatory",
            "missing-optional",
            "fallback-output",
            "target-aggregate",
            "target-zero",
            "target-false",
            "target-empty",
            "target-flag",
        }:
            step.target_id_key = "chosen"
        if variant in {"existing", "existing-output"}:
            context["chosen"] = "hero_brogan"
        if variant in {"existing-output", "fallback-output", "optional-output"}:
            step.target_output_key = "new_target"
        if variant == "missing-optional":
            step.is_mandatory = False
        if variant == "empty-keys":
            step.target_id_key = step.target_output_key = ""
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "empty-actor":
            state.current_actor_id = ""
        if variant == "unknown-actor":
            state.current_actor_id = "unknown_actor"
        if variant == "negative":
            step.damage = -16
        if variant == "zero":
            step.damage = 0
        if variant == "negative-range":
            step.range_val = -3
        if variant.startswith("bonus-"):
            step.damage_bonus_key, step.range_bonus_key = "damage_bonus", "range_bonus"
            if variant != "bonus-missing":
                context.update(
                    damage_bonus={
                        "bonus-int": -7,
                        "bonus-bool": True,
                        "bonus-float": -2.9,
                        "bonus-string": " +1_0 ",
                    }[variant],
                    range_bonus="2",
                )
        if variant == "stats":
            step.damage_stat_type, step.range_stat_type = StatType.ATTACK, StatType.RANGE
        if variant in {"custom-filters", "optional-shared"}:
            repeated = RangeFilter(max_range=3)
            step.target_filters = [repeated, ImmunityFilter(), repeated]
            context.update(filter_alias=repeated, filters_alias=step.target_filters)
        if variant == "marks-existing":
            marks["victim_id"] = -10
        if variant == "marks-key-alias":
            step.target_id_key = "defeat_marks"
        if variant == "tuple-log":
            context["defeat_log"] = tuple(log)
        if variant == "dict-log":
            context["defeat_log"] = {"a": 1, "b": 2}
        if variant == "string-log":
            context["defeat_log"] = "a😀ö"
        if variant == "shared-tracking":
            context["defeat_log"] = marks
        if variant == "marks-absent":
            context.pop("defeat_marks")
        if variant == "target-aggregate":
            context["chosen"] = ["truthy"]
        if variant == "target-zero":
            context["chosen"] = 0
        if variant == "target-false":
            context["chosen"] = False
        if variant == "target-empty":
            context["chosen"] = ""
        if variant == "target-flag":
            step.target_id_key = "attack_damage"
        if variant == "skip":
            step.skip_if_key = "gate"
            context["gate"] = False
        if variant == "active":
            step.active_if_key = "absent"
        if variant.startswith("optional-"):
            step.is_mandatory = False
        if variant in {"basic-current", "colored-current", "performing-card"}:
            hero = state.get_hero("hero_swift")
            basic = next(card for card in hero.deck if card.color.value in {"GOLD", "SILVER"})
            colored = next(card for card in hero.deck if card.color.value not in {"GOLD", "SILVER"})
            hero.current_turn_card = basic if variant == "basic-current" else colored
            if variant == "performing-card":
                context.update(performing_card_id=basic.id, performing_card_owner_id=str(hero.id))
        context["attack_alias"] = step
        tail = SetContextFlagStep(step_id="fixture/attack/tail", key="tail", value=True)
        state.execution_stack = [tail, step]
        ops = [{"kind": "advance_prefix", "steps": 1}]
        if variant.startswith("optional-"):
            ops.extend(
                [
                    {"kind": "advance_prefix", "steps": 1},
                    {
                        "kind": "input",
                        "request_id": "current",
                        "selection": "SKIP" if variant == "optional-skip" else str(minion.id),
                    },
                    {"kind": "advance_prefix", "steps": 1},
                    {"kind": "advance_prefix", "steps": 1},
                ]
            )
        yield "attack-macro-" + variant, state, ops

    from conditional_aura_traces import states

    from goa2.domain.models import GamePhase

    for name, state in states():
        state.phase = GamePhase.RESOLUTION
        step = AttackSequenceStep(
            step_id="fixture/attack/aura",
            damage=-2,
            range_val=1,
            damage_stat_type=StatType.ATTACK,
            range_stat_type=StatType.RANGE,
        )
        state.execution_context["attack_alias"] = step
        state.execution_stack = [step]
        yield "attack-macro-" + name, state, [{"kind": "advance_prefix", "steps": 1}]

    from aura_traces import _board, _pieces
    from model_traces import setup

    for variant in ("owner", "bound-0", "bound-1", "piece"):
        state = setup(["Razzle", "Arien"], ["Swift", "Brogan"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        pieces = _pieces(state, "hero_razzle")
        for i, piece in enumerate(pieces):
            state.place_entity(piece, h(i, 0))
        state.current_actor_id = pieces[1] if variant == "piece" else "hero_razzle"
        if variant.startswith("bound-"):
            state.acting_piece_id = pieces[int(variant[-1])]
        state.get_hero("hero_razzle").items = {StatType.ATTACK: 2, StatType.RANGE: 1}
        step = AttackSequenceStep(
            step_id="fixture/attack/piece",
            damage=-2,
            range_val=1,
            damage_stat_type=StatType.ATTACK,
            range_stat_type=StatType.RANGE,
        )
        state.execution_context["attack_alias"] = step
        state.execution_stack = [step]
        yield "attack-macro-piece-" + variant, state, [{"kind": "advance_prefix", "steps": 1}]
