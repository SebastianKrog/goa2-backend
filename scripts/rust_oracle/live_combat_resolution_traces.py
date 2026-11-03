"""Live combat values, adjacency snapshots and attack-local defeat tracking."""

from __future__ import annotations


def scenes():
    import copy
    from unittest.mock import patch

    import combat_effect_traces
    from aura_traces import _board, _pieces
    from combat_effect_traces import state_for
    from live_defeat_traces import retain
    from model_traces import setup
    from targeting_rules import states

    from goa2.domain.models import GamePhase
    from goa2.engine.steps import (
        CheckTargetDefeatedStep,
        ResolveCombatStep,
        SnapshotAdjacentHeroesStep,
    )

    emitted = []

    def capture(name, state, steps, mode="first", unordered_keys=(), select=None):
        ops, final = retain(state, steps, mode, select)
        if name.startswith("combat-effect-negative-"):
            emitted.append(("live-" + name, state, ops))
        return {"name": name}, final

    with patch.object(combat_effect_traces, "drive", capture):
        combat_effect_traces.cases()
    yield from emitted

    for damage in (-16, -1, 0, 4):
        for defense in (None, -17, -16, -1, 0, 3, False, True, 2.5, 4.5):
            for modifier in (0, 0.5):
                state, _, _ = state_for()
                state.execution_context.update(
                    victim_id="hero_brogan",
                    defense_value=defense,
                    minion_defense_modifier=modifier,
                    last_combat_target="old",
                    block_succeeded="old",
                )
                step = ResolveCombatStep(damage=damage)
                yield f"combat-live-values-{damage}-{defense}-{modifier}", state, retain(
                    state, [step]
                )[0]

    for variant in (
        "invalid",
        "auto",
        "both",
        "ignore",
        "initiative",
        "bonus",
        "bonus-bool",
        "bonus-string",
        "cached-none",
        "cached-false",
        "uncached",
        "missing-target",
        "offboard",
        "no-actor",
        "empty-actor",
        "custom-key",
        "alias-block",
        "unused-defense",
        "skip",
        "active",
    ):
        state, _, _ = state_for()
        state.execution_context.update(
            victim_id="hero_brogan",
            defense_value=2,
            minion_defense_modifier=0,
            defense_bonus=3,
            last_combat_target="old",
            block_succeeded="old",
        )
        step = ResolveCombatStep(damage=4)
        if variant in {"invalid", "both"}:
            state.execution_context["defense_invalid"] = {"yes": True}
        if variant in {"auto", "both", "unused-defense"}:
            state.execution_context["auto_block"] = [True]
        if variant == "ignore":
            state.execution_context.update(ignore_minion_defense=True, minion_defense_modifier=100)
        if variant == "initiative":
            state.execution_context.update(defense_uses_initiative=True)
        if variant == "bonus":
            state.execution_context["defense_bonus"] = -2.9
        if variant == "bonus-bool":
            state.execution_context["defense_bonus"] = True
        if variant == "bonus-string":
            state.execution_context["defense_bonus"] = " +1_0 "
        if variant == "cached-none":
            state.execution_context["minion_defense_modifier"] = None
        if variant == "cached-false":
            state.execution_context["minion_defense_modifier"] = False
        if variant == "uncached":
            state.execution_context.pop("minion_defense_modifier")
        if variant == "missing-target":
            state.execution_context.pop("victim_id")
        if variant == "offboard":
            state.remove_entity("hero_brogan")
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "empty-actor":
            state.current_actor_id = ""
        if variant in {"custom-key", "alias-block"}:
            step.target_key = "custom" if variant == "custom-key" else "block_succeeded"
            state.execution_context[step.target_key] = "hero_brogan"
        if variant == "unused-defense":
            state.execution_context.update(
                defense_value={"bad": "unused"}, minion_defense_modifier=["unused"]
            )
        if variant == "skip":
            step.skip_if_key = "gate"
            state.execution_context["gate"] = False
        if variant == "active":
            step.active_if_key = "absent"
        yield "combat-live-flags-" + variant, state, retain(state, [step])[0]

    for index, (damage, defense, bonus, modifier) in enumerate(
        (
            (9007199254740993, 9007199254740992.0, 0, 0),
            (9007199254740991, 9007199254740992.0, 0, 0),
            (-9007199254740993, -9007199254740992.0, 0, 0),
            (9223372036854775807, 9223372036854775808.0, 0, 0),
            (-9223372036854775808, -9223372036854775808.0, 0, 0),
            (9223372036854775807, 9223372036854775807, 1, 0),
            (9223372036854775807, 9223372036854775807, 1, -1024.0),
            (0, 1e308, 0, 1e308),
            (0, -1e308, 0, -1e308),
        )
    ):
        state, _, _ = state_for()
        state.execution_context.update(
            victim_id="hero_brogan",
            defense_value=defense,
            defense_bonus=bonus,
            minion_defense_modifier=modifier,
        )
        yield f"combat-live-precision-{index}", state, retain(
            state, [ResolveCombatStep(damage=damage)]
        )[0]

    for name, state in states():
        for relation in ("ENEMY", "FRIENDLY", "ANY"):
            current = copy.deepcopy(state)
            current.execution_context.update(target="hero_mrak", neighbors=["old"])
            current.execution_context["old_neighbors"] = current.execution_context["neighbors"]
            step = SnapshotAdjacentHeroesStep(
                target_key="target", output_key="neighbors", relation=relation
            )
            yield f"combat-snapshot-{name}-{relation}", current, retain(current, [step])[0]

    for mark in (None, -5, -1, 0, 1, 2, 100, False, True):
        for by_actor in (False, True):
            for history in (
                [],
                [["hero_brogan", "hero_swift"]],
                [["hero_brogan", "hero_hanu"], ["hero_brogan", "hero_swift"]],
                [["hero_brogan", None], ["hero_arien", "hero_swift"]],
            ):
                state, _, _ = state_for()
                state.execution_context.update(
                    attack_target="hero_brogan",
                    defeat_marks={"attack_target": mark},
                    defeat_log=copy.deepcopy(history),
                    chosen="old",
                )
                state.execution_context["marks_alias"] = state.execution_context["defeat_marks"]
                state.execution_context["log_alias"] = state.execution_context["defeat_log"]
                step = CheckTargetDefeatedStep(
                    target_key="attack_target", output_key="chosen", by_actor=by_actor
                )
                yield f"combat-track-{mark}-{by_actor}-{len(history)}-{history}", state, retain(
                    state, [step]
                )[0]

    for variant in (
        "piece-killer",
        "owner-killer",
        "wrong-victim",
        "no-actor",
        "unknown-actor",
        "missing-target",
        "missing-mark",
        "missing-log",
        "empty-log-aggregate-target",
        "tuple-log",
        "numeric-target",
        "output-marks",
        "skip",
        "active",
    ):
        state = setup(["Swift", "Razzle"], ["Brogan", "Arien"])
        _board(state)
        pieces = _pieces(state, "hero_razzle")
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = pieces[0]
        state.execution_context.update(
            attack_target="hero_brogan",
            defeat_marks={"attack_target": 0},
            defeat_log=[["hero_brogan", pieces[1]]],
        )
        step = CheckTargetDefeatedStep(
            target_key="attack_target", output_key="chosen", by_actor=True
        )
        if variant == "owner-killer":
            state.execution_context["defeat_log"] = [["hero_brogan", "hero_razzle"]]
        if variant == "wrong-victim":
            state.execution_context.update(
                attack_target=pieces[0], defeat_log=[["hero_razzle", pieces[1]]]
            )
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "unknown-actor":
            state.current_actor_id = "unknown"
            state.execution_context["defeat_log"] = [["hero_brogan", "unknown"]]
        if variant == "missing-target":
            state.execution_context.pop("attack_target")
        if variant == "missing-mark":
            state.execution_context["defeat_marks"] = {}
        if variant == "missing-log":
            state.execution_context.pop("defeat_log")
        if variant == "empty-log-aggregate-target":
            state.execution_context.update(attack_target={"unused": True}, defeat_log=[])
        if variant == "tuple-log":
            state.execution_context["defeat_log"] = tuple(
                (a, b) for a, b in state.execution_context["defeat_log"]
            )
        if variant == "numeric-target":
            state.execution_context.update(attack_target=42, defeat_log=[["42", "hero_razzle"]])
        if variant == "output-marks":
            step.output_key = "defeat_marks"
        if variant == "skip":
            step.skip_if_key = "gate"
            state.execution_context["gate"] = False
        if variant == "active":
            step.active_if_key = "absent"
        yield "combat-track-special-" + variant, state, retain(state, [step])[0]

    yield from snapshot_special_scenes()


def export(path):
    from object_execution_traces import reference
    from oracle import verify_sources
    from shared_objects import export_state_with_shared_objects
    from stack_traces import write_corpus_stream

    m = verify_sources()

    def cases():
        for name, state, ops in scenes():
            envelope, _ = export_state_with_shared_objects(state)
            yield {
                "name": "object-execution-" + name,
                "envelope": envelope,
                "operations": ops,
                "expected": reference(state, ops),
            }

    return write_corpus_stream(
        path,
        {
            "schema_version": 1,
            "oracle_revision": m["oracle_revision"],
            "source_sha256": m["source_sha256"],
        },
        cases(),
    )


def snapshot_special_scenes():
    import copy

    from live_defeat_traces import retain
    from targeting_rules import states

    from goa2.domain.hex import Hex
    from goa2.domain.models import Token
    from goa2.domain.models.base import Placeholder
    from goa2.engine.steps import SnapshotAdjacentHeroesStep

    bases = {name: state for name, state in states() if name in {"plain", "los-plain"}}
    state = copy.deepcopy(bases["los-plain"])
    candidate = "hero_mrak"
    assert not state.validator.can_be_targeted(
        state, str(state.current_actor_id), candidate, state.execution_context
    ).allowed
    point = state.get_position(candidate)
    offsets = ((1, 0, -1), (1, -1, 0), (0, -1, 1), (-1, 0, 1), (-1, 1, 0), (0, 1, -1))
    adjacent = [Hex(q=point.q + q, r=point.r + r, s=point.s + s) for q, r, s in offsets]
    anchor_hex = next(
        h for h in adjacent if h in state.board.tiles and h not in state.entity_locations.values()
    )
    anchor = Placeholder(id="snapshot_anchor", name="Anchor")
    state.register_entity(anchor, "board_entity")
    state.place_entity(anchor.id, anchor_hex)
    state.execution_context.update(target=anchor.id, denied_candidate=candidate)
    step = SnapshotAdjacentHeroesStep(target_key="target", output_key="neighbors", relation="ANY")
    yield "combat-snapshot-targeting-independent", state, retain(state, [step])[0]
    state = copy.deepcopy(bases["plain"])
    token = next(id for id in state.entity_locations if isinstance(state.get_entity(id), Token))
    state.current_actor_id = token
    state.execution_context["target"] = "hero_mrak"
    step = SnapshotAdjacentHeroesStep(target_key="target", output_key="neighbors", relation="ANY")
    yield "combat-snapshot-token-actor", state, retain(state, [step])[0]
