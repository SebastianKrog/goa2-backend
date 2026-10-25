"""Real-handler effect-aware unit selectors, queries, pre-actions and projectile attacks."""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

from combat_effect_traces import effect
from mine_traces import drive
from oracle import ROOT, digest, verify_sources
from reaction_effect_traces import illusion_state
from targeting_rules import states


def cases(advance=None, answer=None):
    from goa2.domain.hex import Hex
    from goa2.domain.models import TargetType, Token, TokenType
    from goa2.domain.models.effect import DurationType, EffectScope, EffectType, Shape
    from goa2.engine.filters_units import UnitTypeFilter
    from goa2.engine.steps import (
        CollectUnitsStep,
        CountStep,
        MultiSelectStep,
        ResolvePreActionDiscardStep,
        ResolvePreActionMovementStep,
        SelectStep,
        SetContextFlagStep,
    )

    result = []
    for variant, base in states():
        for mode in ["unit", "tokens", "override", "multi", "queries"]:
            state = copy.deepcopy(base)
            if mode == "queries":
                steps = [
                    CountStep(filters=[UnitTypeFilter(unit_type="MINION")], output_key="units"),
                    CollectUnitsStep(
                        filters=[UnitTypeFilter(unit_type="MINION")], output_key="unit-list"
                    ),
                    CountStep(
                        target_type=TargetType.UNIT_OR_TOKEN,
                        filters=[UnitTypeFilter(unit_type="MINION")],
                        output_key="all",
                    ),
                    CollectUnitsStep(
                        target_type=TargetType.UNIT_OR_TOKEN,
                        filters=[UnitTypeFilter(unit_type="MINION")],
                        output_key="all-list",
                    ),
                ]
            elif mode == "multi":
                steps = [
                    MultiSelectStep(
                        target_type=TargetType.UNIT,
                        prompt="Choose units",
                        output_key="chosen",
                        max_selections=3,
                        min_selections=0,
                    )
                ]
            else:
                steps = [
                    SelectStep(
                        target_type=(
                            TargetType.UNIT_OR_TOKEN if mode == "tokens" else TargetType.UNIT
                        ),
                        prompt="Choose a unit",
                        output_key="chosen",
                        is_mandatory=False,
                        override_player_id="hero_mrak" if mode == "override" else None,
                    )
                ]
            case, _ = drive(f"targeting-effect-{variant}-{mode}", state, steps)
            result.append(case)

    def h(q, r):
        return Hex(q=q, r=r, s=-q - r)

    for variant in [
        "plain",
        "zero",
        "none",
        "negative",
        "two",
        "three",
        "inactive",
        "expired",
        "wrong-source",
        "key",
        "literal-precedence",
        "gate",
        "offboard",
        "piece",
        "owner-unbound",
        "owner-bound",
        "mine-dud",
        "mine-blast",
    ]:
        for mode in ["first", "skip", "invalid"]:
            state, _ = illusion_state("plain")
            hero = "hero_arien"
            if variant.startswith("owner") or variant == "piece":
                from aura_traces import _pieces

                pieces = _pieces(state, "hero_razzle")
                hero = pieces[0] if variant == "piece" else "hero_razzle"
                state.current_actor_id = hero
                if variant == "owner-bound":
                    state.acting_piece_id = pieces[0]
            move = effect(
                state,
                "pre-move",
                EffectType.PRE_ACTION_MOVEMENT,
                source=hero,
                max_value=(
                    2
                    if variant in ["two", "mine-dud", "mine-blast"]
                    else (
                        3
                        if variant == "three"
                        else (
                            0
                            if variant == "zero"
                            else None if variant == "none" else -1 if variant == "negative" else 1
                        )
                    )
                ),
            )
            state.active_effects.append(move)
            if variant == "inactive":
                move.is_active = False
            if variant == "expired":
                move.duration = DurationType.THIS_ROUND
                move.created_at_round = state.round - 1
            if variant == "wrong-source":
                move.source_id = "hero_hanu"
            if variant == "offboard":
                state.remove_entity(hero)
            state.execution_context = {"who": hero, "ignored": []}
            step = (
                ResolvePreActionMovementStep(hero_key="who")
                if variant == "key"
                else ResolvePreActionMovementStep(
                    hero_id=hero, hero_key="ignored" if variant == "literal-precedence" else None
                )
            )
            if variant == "gate":
                step.active_if_key = "missing"
            if variant.startswith("mine-"):
                kind = TokenType.MINE_DUD if variant == "mine-dud" else TokenType.MINE_BLAST
                token = Token(
                    id="pre-mine",
                    name="Mine",
                    owner_id="hero_hanu",
                    token_type=kind,
                    is_passable=True,
                )
                state.misc_entities[token.id] = token
                state.token_pool.setdefault(kind, []).append(token)
                state.place_entity(token.id, h(-1, 3))
            count = 0

            def choose(req, mode):
                nonlocal count
                from goa2.domain.input import selection_value

                if mode == "skip":
                    return "SKIP"
                if mode == "invalid" and count == 0:
                    count += 1
                    return "not-a-hex"
                return (
                    req.options[0].id
                    if req.prompt == "Choose which mines to move through"
                    else selection_value(req.options[0])
                )

            case, _ = drive(
                f"targeting-pre-move-{variant}-{mode}",
                state,
                [step, SetContextFlagStep(key="after", value=True)],
                mode,
                select=choose,
            )
            result.append(case)
    for variant in [
        "plain",
        "multiple",
        "duplicate",
        "passive-dormant",
        "card-dormant",
        "expired",
        "off-scope",
        "immune",
        "origin-attack",
        "empty-safe",
        "empty-defeat",
        "one-card",
        "offboard-abort",
        "offboard-continue",
        "key",
        "literal-precedence",
        "processed",
        "linked-other",
        "finishing",
        "piece",
        "owner-unbound",
        "owner-bound",
        "defender",
    ]:
        for mode in ["first", "last"]:
            state, _ = illusion_state("plain")
            hero = "hero_mrak"
            if variant in ["piece", "owner-unbound", "owner-bound"]:
                from aura_traces import _pieces

                pieces = _pieces(state, "hero_razzle")
                hero = pieces[0] if variant == "piece" else "hero_razzle"
                if variant == "owner-bound":
                    state.acting_piece_id = pieces[0]
            victim = state.get_hero(hero)
            creator = state.get_hero("hero_hanu")
            carrier = creator.hand[0]
            carrier.is_active = True
            disrupt = effect(
                state,
                "disrupt",
                EffectType.PRE_ACTION_DISCARD,
                source="hero_hanu",
                source_card_id=carrier.id,
                discard_or_defeat=variant in ["empty-defeat", "one-card"],
            )
            state.active_effects.append(disrupt)
            if variant in ["multiple", "duplicate", "one-card"]:
                other = disrupt.model_copy(deep=True)
                other.id = "disrupt" if variant == "duplicate" else "disrupt-two"
                state.active_effects.append(other)
            if variant in ["passive-dormant", "card-dormant"]:
                disrupt.is_active = False
                if variant == "card-dormant":
                    disrupt.duration = DurationType.THIS_ROUND
            if variant == "expired":
                disrupt.duration = DurationType.THIS_ROUND
                disrupt.created_at_round = state.round - 1
            if variant == "off-scope":
                disrupt.scope = EffectScope(shape=Shape.POINT, origin_id="hero_hanu")
            if variant == "immune":
                state.active_effects.append(
                    effect(
                        state,
                        "immune",
                        EffectType.IMMUNITY_ENEMY_ACTIONS,
                        source=hero,
                        blocks_friendly_actors=True,
                    )
                )
            if variant == "origin-attack":
                disrupt.origin_attack_is_basic = True
                state.active_effects.append(
                    effect(
                        state,
                        "immune",
                        EffectType.ATTACK_IMMUNITY,
                        source=hero,
                        basic_attacks_only=True,
                    )
                )
            if variant.startswith("empty"):
                victim.hand = []
            if variant == "one-card":
                victim.hand = victim.hand[:1]
            if variant.startswith("offboard"):
                for piece in list(state.get_piece_ids(hero)):
                    state.remove_entity(piece)
                state.remove_entity(hero)
            if variant == "linked-other":
                state.active_effects.append(
                    disrupt.model_copy(
                        update={
                            "id": "other",
                            "effect_type": EffectType.MOVEMENT_ZONE,
                            "max_value": 8,
                        }
                    )
                )
            if variant == "finishing":
                disrupt.finishing_steps = [
                    SetContextFlagStep(
                        step_id="fixture/pre-discard/no-finish", key="should_not_finish", value=True
                    )
                ]
            state.execution_context = {"who": hero, "ignored": []}
            state.current_actor_id = hero if variant != "defender" else "hero_arien"
            step = (
                ResolvePreActionDiscardStep(hero_key="who")
                if variant == "key"
                else ResolvePreActionDiscardStep(
                    hero_id=hero,
                    hero_key="ignored" if variant == "literal-precedence" else None,
                    abort_on_defeat=variant not in ["offboard-continue", "defender"],
                )
            )
            if variant == "processed":
                step.processed_effect_ids = ["disrupt"]
            case, final = drive(
                f"targeting-pre-discard-{variant}-{mode}",
                state,
                [step, SetContextFlagStep(key="after", value=True)],
                mode,
            )
            assert "should_not_finish" not in final.execution_context
            result.append(case)
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase
    from goa2.engine.steps import AttackSequenceStep

    for variant in [
        "move",
        "discard",
        "defeat",
        "both",
        "leave-radius",
        "multiple",
        "linked-defense",
        "mine-disrupt",
    ]:
        for ranged in [False, True]:
            for mode in ["first", "skip"]:
                state = setup(["Arien", "Trinkets"], ["Wasp", "Misa"])
                board = _board(state)
                state.phase = GamePhase.RESOLUTION
                state.round, state.turn = 1, 2
                state.current_actor_id = "hero_arien"
                state.resolution_owner_id = "hero_arien"
                for uid, point in [
                    ("hero_arien", board(0, 0)),
                    ("hero_trinkets", board(2, -1)),
                    ("hero_wasp", board(1, 0)),
                    ("hero_misa", board(-3, 1)),
                ]:
                    state.place_entity(uid, point)
                defender = state.get_hero("hero_wasp")
                defense = next(c for c in defender.deck if c.effect_id == "stop_projectiles")
                if defense not in defender.hand:
                    defender.hand.append(defense)
                defense.state = CardState.UNRESOLVED
                defense.is_facedown = False
                if variant in ["defeat", "mine-disrupt"]:
                    defender.hand = [defense]
                state.active_effects = [
                    effect(
                        state,
                        "unrelated",
                        EffectType.MOVEMENT_ZONE,
                        source="hero_arien",
                        max_value=8,
                    )
                ]
                if variant in ["move", "both", "leave-radius", "linked-defense", "mine-disrupt"]:
                    provider = effect(
                        state,
                        "pre-move",
                        EffectType.PRE_ACTION_MOVEMENT,
                        source="hero_wasp",
                        max_value=2,
                    )
                    provider.duration = DurationType.NEXT_TURN
                    provider.created_at_turn = 1
                    if variant == "linked-defense":
                        provider.source_card_id = defense.id
                        defense.is_active = True
                    state.active_effects.append(provider)
                if variant not in ["move", "linked-defense"]:
                    provider = effect(
                        state,
                        "disrupt",
                        EffectType.PRE_ACTION_DISCARD,
                        source="hero_trinkets",
                        discard_or_defeat=True,
                    )
                    if variant == "leave-radius":
                        from goa2.domain.models.effect import AffectsFilter

                        provider.scope = EffectScope(
                            shape=Shape.RADIUS,
                            range=1,
                            origin_id="hero_trinkets",
                            affects=AffectsFilter.ENEMY_HEROES,
                        )
                    state.active_effects.append(provider)
                    if variant == "multiple":
                        state.active_effects.append(
                            provider.model_copy(update={"id": "second-disrupt"})
                        )
                if variant == "mine-disrupt":
                    token = Token(
                        id="defense-mine",
                        name="Mine",
                        token_type=TokenType.MINE_BLAST,
                        owner_id="hero_trinkets",
                        is_passable=True,
                    )
                    state.misc_entities[token.id] = token
                    state.token_pool.setdefault(token.token_type, []).append(token)
                    state.place_entity(token.id, board(0, 1))
                state.execution_context = {"current_action_type": "ATTACK"}

                def choose(req, mode, card_id=str(defense.id)):
                    from goa2.domain.input import selection_value

                    if req.request_type.value == "SELECT_CARD_OR_PASS":
                        return card_id
                    if "hero_wasp" in [o.id for o in req.options]:
                        return "hero_wasp"
                    if req.prompt.startswith("Pre-action movement") and mode == "skip":
                        return "SKIP"
                    if req.prompt == "Choose which mines to move through":
                        return req.options[0].id
                    return selection_value(req.options[0])

                case, final = drive(
                    f"targeting-attack-pre-{variant}-{ranged}-{mode}",
                    state,
                    [AttackSequenceStep(damage=7, range_val=3, is_ranged=ranged)],
                    mode,
                    select=choose,
                )
                if variant == "defeat":
                    assert not final.has_board_presence("hero_wasp")
                result.append(case)
    from card_selection_traces import cases as card_cases

    advance = {"kind": "advance"}

    def answer(value, request_id="current"):
        return {"kind": "input", "request_id": request_id, "selection": value}

    for case in card_cases(advance, answer, active_projectiles=True):
        if case["name"].startswith("card-select-projectile-"):
            case["name"] = "targeting-effect-" + case["name"]
            result.append(case)
    return result


def export(path):
    from stack_traces import write_corpus

    m = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": m["oracle_revision"],
        "source_sha256": m["source_sha256"],
        "cases": cases(),
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/artifacts/targeting-effects.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        from stack_traces import verify

        print(
            json.dumps(
                verify(args.corpus, args.worker[1:] if args.worker[:1] == ["--"] else args.worker),
                indent=2,
            )
        )
