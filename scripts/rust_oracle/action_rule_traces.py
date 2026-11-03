"""Frozen reference action restrictions, movement caps, repeats and card hooks.

Ordinary negative attacks are legal. Witching Hour is a specific card rule;
its restriction must never become a general negative-value legality test.
"""

from __future__ import annotations

from fixture_cache import read_text, write_text

import argparse
import copy
import json
from pathlib import Path

from oracle import ROOT, Worker, digest, verify_sources
from state_codec import export_state_with_instances


def restriction_states():
    from displacement_effect_traces import states

    from goa2.domain.models import ActionType, CardColor
    from goa2.domain.models.effect import EffectType

    for name, state, *_ in states():
        restriction = state.active_effects[-1]
        if name != "plain":
            restriction.restrictions = [ActionType.ATTACK, ActionType.MOVEMENT]
        if not name.startswith("type-"):
            restriction.effect_type = EffectType.REPEAT_PREVENTION
        cap = restriction.model_copy(
            deep=True,
            update={
                "id": "movement-cap",
                "effect_type": EffectType.MOVEMENT_ZONE,
                "restrictions": [],
                "max_value": 1,
            },
        )
        state.active_effects.append(cap)
        if name == "plain":
            state.active_effects.clear()
        yield name, state

    original = next(state for name, state in restriction_states_base() if name == "blocked")
    for variant in [
        "gold-exception",
        "all-color-exceptions",
        "facedown-exception",
        "cap-none",
        "cap-negative",
        "cap-zero",
        "cap-tie",
        "cap-lower",
        "actions-only",
        "no-flags-cap",
        "prerequisite-order",
        "all-actions",
        "bound-owner",
        "one-piece-free",
        "both-pieces-blocked",
        "no-pieces",
    ]:
        state = copy.deepcopy(original)
        e = state.active_effects[-1]
        e.effect_type = EffectType.REPEAT_PREVENTION
        e.restrictions = [ActionType.ATTACK, ActionType.MOVEMENT]
        e.scope.range = 4
        if "exception" in variant:
            e.except_card_colors = (
                list(CardColor) if variant == "all-color-exceptions" else [CardColor.GOLD]
            )
        if variant == "facedown-exception":
            for card in state.get_hero("hero_arien").deck:
                card.is_facedown = True
        if variant == "all-actions":
            e.restrictions = list(ActionType)
        if variant == "prerequisite-order":
            first = e.model_copy(
                deep=True,
                update={
                    "id": "actual-action-first",
                    "restrictions": [ActionType.FAST_TRAVEL, ActionType.CLEAR],
                },
            )
            state.active_effects.insert(0, first)
        if variant in ("bound-owner", "one-piece-free", "both-pieces-blocked", "no-pieces"):
            from aura_traces import _pieces

            pieces = _pieces(state, "hero_razzle")
            e.blocks_friendly_actors = True
            if variant == "bound-owner":
                state.acting_piece_id = pieces[0]
            if variant == "one-piece-free":
                e.scope.range = 1
            if variant == "no-pieces":
                for piece in pieces:
                    state.remove_entity(piece)
        cap = e.model_copy(
            deep=True,
            update={
                "id": "cap-first",
                "effect_type": EffectType.MOVEMENT_ZONE,
                "restrictions": [],
                "max_value": 1,
            },
        )
        if variant == "cap-none":
            cap.max_value = None
        if variant == "cap-negative":
            cap.max_value = -1
        if variant == "cap-zero":
            cap.max_value = 0
        if variant == "actions-only":
            cap.limit_actions_only = True
        if variant == "no-flags-cap":
            cap.blocks_enemy_actors = cap.blocks_friendly_actors = cap.blocks_self = False
            e.restrictions = []
        state.active_effects.append(cap)
        if variant in ("cap-tie", "cap-lower"):
            state.active_effects.append(
                cap.model_copy(
                    deep=True,
                    update={"id": "cap-second", "max_value": 1 if variant == "cap-tie" else 0},
                )
            )
        yield variant, state


def restriction_states_base():
    from displacement_effect_traces import states

    for name, state, *_ in states():
        yield name, state


def witch_states():
    from aura_traces import _board, _pieces
    from model_traces import setup

    from goa2.domain.models import CardState, StatType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape

    for variant in [
        "ordinary-negative",
        "ordinary-zero",
        "active-negative",
        "active-zero",
        "active-positive",
        "level-seven",
        "facedown",
        "inactive-flag",
        "resolved",
        "unresolved",
        "source-offboard",
        "actor-offboard",
        "radius-zero",
        "radius-extended",
        "attack-shield",
        "actor-immunity",
        "friendly-immunity",
        "effect-priority",
        "topology-split",
        "bound-owner",
        "unbound-owner",
        "performing-context",
        "facedown-action",
    ]:
        state = setup(["Arien", "Razzle"], ["Cordelia", "Brogan"])
        h = _board(state)
        source = state.get_hero("hero_cordelia")
        actor = state.get_hero("hero_arien")
        source.level = 8
        source.ultimate_card.is_facedown = False
        actor.items[StatType.ATTACK] = -20
        pieces = _pieces(state, "hero_razzle")
        for uid, point in [
            (source.id, h(0, 0)),
            (actor.id, h(1, 0)),
            ("hero_brogan", h(-1, 0)),
            (pieces[0], h(0, 2)),
            (pieces[1], h(4, 0)),
        ]:
            state.place_entity(uid, point)
        state.get_hero("hero_razzle").items[StatType.ATTACK] = -20
        if variant.startswith("ordinary") or variant == "level-seven":
            source.level = 7
        if variant.endswith("zero"):
            for card in actor.deck:
                if card.primary_action.value == "ATTACK":
                    card.primary_action_value = 0
                if any(a.value == "ATTACK" for a in card.secondary_actions):
                    from goa2.domain.models import ActionType

                    card.secondary_actions[ActionType.ATTACK] = 0
            actor.items[StatType.ATTACK] = 0
        if variant == "active-positive":
            actor.items[StatType.ATTACK] = 2
        if variant == "facedown":
            source.ultimate_card.is_facedown = True
        if variant == "facedown-action":
            for card in actor.deck:
                card.is_facedown = True
        if variant == "inactive-flag":
            source.ultimate_card.is_active = False
        if variant in ("resolved", "unresolved"):
            source.level = 1
            source.played_cards = [source.ultimate_card]
            source.ultimate_card.state = (
                CardState.RESOLVED if variant == "resolved" else CardState.UNRESOLVED
            )
        if variant == "source-offboard":
            state.remove_entity(source.id)
        if variant == "actor-offboard":
            state.remove_entity(actor.id)
        if variant == "radius-zero":
            source.items[StatType.RADIUS] = -4
        if variant == "radius-extended":
            source.items[StatType.RADIUS] = 2
        if variant in ("bound-owner", "performing-context"):
            state.acting_piece_id = pieces[0]
        if variant == "performing-context":
            state.execution_context = {"current_card_id": str(actor.hand[0].id)}
        if variant in (
            "attack-shield",
            "actor-immunity",
            "friendly-immunity",
            "effect-priority",
            "topology-split",
        ):
            kind = {
                "attack-shield": EffectType.ATTACK_IMMUNITY,
                "actor-immunity": EffectType.IMMUNITY_ENEMY_ACTIONS,
                "friendly-immunity": EffectType.IMMUNITY_ENEMY_ACTIONS,
                "effect-priority": EffectType.REPEAT_PREVENTION,
                "topology-split": EffectType.TOPOLOGY_SPLIT,
            }[variant]
            e = ActiveEffect(
                id="witch-companion",
                source_id=str(actor.id),
                effect_type=kind,
                scope=EffectScope(shape=Shape.GLOBAL),
                duration=DurationType.PASSIVE,
                is_active=True,
                created_at_turn=state.turn,
                created_at_round=state.round,
                split_axis="q",
                split_value=0,
            )
            if variant == "friendly-immunity":
                e.blocks_friendly_actors = True
            if variant == "effect-priority":
                from goa2.domain.models import ActionType

                e.source_id = str(source.id)
                e.restrictions = [ActionType.CLEAR]
            state.active_effects.append(e)
        yield "witch-" + variant, state


def validation_case(name, state):
    from goa2.domain.models import ActionType, Card

    envelope, objects = export_state_with_instances(state)
    units = [
        "hero_arien",
        "hero_razzle",
        "hero_brogan",
        "hero_hanu",
        "hero_cordelia",
        "restriction-token",
        "unknown",
    ]
    units += [str(uid) for uid in state.get_piece_ids("hero_razzle")]
    units += [str(team.minions[0].id) for team in state.teams.values()]
    units = list(dict.fromkeys(units))
    hero = state.get_hero("hero_arien")
    cards = [None, *[c for c in hero.hand if isinstance(c, Card)]]
    other = state.get_hero("hero_brogan")
    cards.append(other.hand[0])
    if name == "witch-ordinary-negative":
        from goa2.domain.models import StatType
        from goa2.engine.stats import get_computed_stat

        attack_cards = [
            card for card in hero.hand if card.current_primary_action == ActionType.ATTACK
        ]
        assert attack_cards
        assert all(
            get_computed_stat(
                state,
                hero.id,
                StatType.ATTACK,
                card.current_primary_action_value or 0,
                performing_card=card,
            )
            < 0
            for card in attack_cards
        ), "Negative-attack reference scenario must actually compute negative values"
    indexes = {id(obj): i for i, obj in enumerate(objects)}
    queries, expected = [], []
    for unit in units:
        for card in cards:
            context = dict(state.execution_context)
            if card is not None:
                context["card"] = card
            index = indexes[id(card)] if card is not None else None
            for action in ActionType:
                q = {"kind": "action", "unit": unit, "action": action.value, "card": index}
                verdict = state.validator.can_perform_action(state, unit, action, context)
                if name == "witch-ordinary-negative" and action == ActionType.ATTACK:
                    assert (
                        verdict.allowed
                    ), "Reference illegally blocked an ordinary negative attack"
                queries.append(q)
                expected.append(verdict.model_dump(mode="json"))
            for distance in [-2, -1, 0, 1, 2, 5]:
                for is_action in [False, True]:
                    queries.append(
                        {
                            "kind": "move",
                            "unit": unit,
                            "card": index,
                            "distance": distance,
                            "is_action": is_action,
                        }
                    )
                    expected.append(
                        state.validator.can_move(
                            state, unit, distance, context, is_movement_action=is_action
                        ).model_dump(mode="json")
                    )
        queries.append({"kind": "repeat", "unit": unit, "card": None})
        expected.append(state.validator.can_repeat_action(state, unit).model_dump(mode="json"))
    return {"name": name, "envelope": envelope, "queries": queries, "expected": expected}


def menu_case(name, state):
    from action_traces import normalize

    from goa2.engine.steps import ResolveCardStep

    envelope, objects = export_state_with_instances(state)
    hero = state.get_hero("hero_arien")
    operations, expected = [], []

    def resolve(card, selection=None, pending=False):
        hero.current_turn_card = card
        op = {"hero": str(hero.id), "card": next(i for i, obj in enumerate(objects) if obj is card)}
        step = ResolveCardStep(hero_id=str(hero.id))
        if pending:
            op["selection"] = selection
            step.pending_input = {"selection": selection}
        before = state.model_dump(mode="json")
        before.pop("execution_context")
        result = step.resolve(state, state.execution_context)
        after = state.model_dump(mode="json")
        after.pop("execution_context")
        assert before == after
        operations.append(op)
        expected.append(
            normalize(
                {
                    "context": state.execution_context,
                    "request": (
                        result.input_request.model_dump(mode="json")
                        if result.input_request
                        else None
                    ),
                    "steps": [s.model_dump(mode="json") for s in result.new_steps or []],
                },
                result.new_steps or [],
            )
        )
        return result

    for card in hero.hand:
        result = resolve(card)
        if result.input_request:
            for choice in dict.fromkeys(o.id for o in result.input_request.options):
                resolve(card, choice, True)
        for blocked in ["ATTACK", "CLEAR", "FAST_TRAVEL", "invalid"]:
            resolve(card, blocked, True)
    return {"name": name, "envelope": envelope, "operations": operations, "expected": expected}


def cases(advance, answer):
    """Actual handler execution, with every instruction available to restore."""
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        PlaceUnitStep,
        SetActorStep,
        SetContextFlagStep,
        ValidateRepeatStep,
    )

    result = []
    for name, state in restriction_states():
        state.phase = GamePhase.RESOLUTION
        state.execution_context = {
            "previous_true": True,
            "previous_false": False,
            "previous_int": 7,
            "previous_text": "truthy",
            "previous_null": None,
        }
        actors = [
            "hero_arien",
            "hero_brogan",
            "hero_razzle",
            *map(str, state.get_piece_ids("hero_razzle")),
            "restriction-token",
            "unknown",
        ]
        steps = []
        for i, actor in enumerate(actors):
            steps.append(
                ValidateRepeatStep(
                    actor_id=actor, output_key=f"repeat_{i}", step_id=f"fixture/action-rule/{i}"
                )
            )
        for key in [
            "previous_true",
            "previous_false",
            "previous_int",
            "previous_text",
            "previous_null",
            "missing",
        ]:
            steps.append(
                ValidateRepeatStep(
                    actor_id="hero_arien",
                    and_with_key=key,
                    output_key="and_" + key,
                    step_id="fixture/action-rule/and/" + key,
                )
            )
        steps.extend(
            [
                SetActorStep(actor_id="hero_brogan", step_id="fixture/action-rule/actor"),
                ValidateRepeatStep(output_key="live_actor", step_id="fixture/action-rule/live"),
                PlaceUnitStep(
                    unit_id="hero_arien",
                    target_hex_arg=Hex(q=-3, r=0, s=3),
                    step_id="fixture/action-rule/move-source",
                ),
                ValidateRepeatStep(
                    actor_id="hero_brogan",
                    output_key="moved_source",
                    step_id="fixture/action-rule/moved",
                ),
                SetActorStep(actor_id=None, step_id="fixture/action-rule/no-actor"),
                ValidateRepeatStep(output_key="absent_actor", step_id="fixture/action-rule/absent"),
                SetContextFlagStep(key="after", value=True, step_id="fixture/action-rule/after"),
            ]
        )
        push_steps(state, steps)
        envelope = export_state(state)
        ops = [{"kind": "advance_prefix", "steps": 1}, advance]
        result.append(
            {
                "name": "action-rule-repeat-" + name,
                "envelope": envelope,
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )
    return result


def export_transitions(path):
    from stack_traces import write_corpus

    manifest = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "cases": cases({"kind": "advance"}, None),
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


def export(path):
    manifest = verify_sources()
    cases, menus = [], []
    menu_names = {
        "plain",
        "blocked",
        "gold-exception",
        "all-color-exceptions",
        "facedown-exception",
        "all-actions",
        "prerequisite-order",
        "offboard",
    }
    for name, state in [*restriction_states(), *witch_states()]:
        cases.append(validation_case(name, state))
        if name in menu_names or name.startswith("witch-"):
            menus.append(menu_case(name, state))
    data = {
        "schema_version": 1,
        "source_sha256": manifest["source_sha256"],
        "cases": cases,
        "menus": menus,
    }
    data["corpus_sha256"] = digest(data)
    write_text(path, json.dumps(data, separators=(",", ":")) + "\n")
    return data


def verify(path, command):
    from action_traces import normalize

    data = json.loads(read_text(path))
    identity = data.pop("corpus_sha256")
    if (
        data["schema_version"] != 1
        or digest(data) != identity
        or data["source_sha256"] != verify_sources()["source_sha256"]
    ):
        raise ValueError("action-validation corpus source/identity mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request(
                "action_legality", envelope=case["envelope"], queries=case["queries"]
            )["results"]
            if len(actual) != len(case["expected"]):
                raise AssertionError("action legality response shape mismatch")
            for i, (got, want) in enumerate(zip(actual, case["expected"], strict=True)):
                if got != want:
                    raise AssertionError((case["name"], case["queries"][i], got, want))
        for case in data["menus"]:
            actual = worker.request(
                "action_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            for i, (got, want) in enumerate(zip(actual, case["expected"], strict=True)):
                if normalize(got) != want:
                    failure = ROOT / "rust/artifacts/action-validation-mismatch.json"
                    failure.parent.mkdir(parents=True, exist_ok=True)
                    write_text(failure, json.dumps(
                            {
                                "case": case["name"],
                                "operation": case["operations"][i],
                                "got": normalize(got),
                                "want": want,
                            },
                            indent=2,
                        ))
                    raise AssertionError(f"{case['name']} menu operation {i}: saved {failure}")
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "queries": sum(len(c["queries"]) for c in data["cases"]),
        "menus": len(data["menus"]),
        "menu_operations": sum(len(c["operations"]) for c in data["menus"]),
        "corpus_sha256": identity,
        "full_verdict_and_menu_parity": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify", "export-transitions"])
    parser.add_argument("--corpus", type=Path, default=None)
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    args.corpus = args.corpus or ROOT / (
        "rust/artifacts/action-rule-repeats.json"
        if args.mode == "export-transitions"
        else "rust/tests/fixtures/action-validation.json"
    )
    if args.mode == "export-transitions":
        data = export_transitions(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    elif args.mode == "export":
        data = export(args.corpus)
        print(
            json.dumps(
                {
                    "cases": len(data["cases"]),
                    "queries": sum(len(c["queries"]) for c in data["cases"]),
                    "menus": len(data["menus"]),
                    "menu_operations": sum(len(c["operations"]) for c in data["menus"]),
                    "corpus_sha256": data["corpus_sha256"],
                }
            )
        )
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker executable required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))
