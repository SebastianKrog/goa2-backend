"""Frozen combat with minion protection, bounty counters and source expiry.

Raw steps deliberately isolate engine primitives; all cards are registered deck
objects. Expected snapshots/events are produced only by the frozen Python engine.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from mine_traces import drive
from model_traces import setup
from oracle import ROOT, digest, verify_sources


def state_for():
    from aura_traces import _board

    from goa2.domain.models import GamePhase, TeamColor

    state = setup(["Swift", "Hanu"], ["Brogan", "Arien"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_swift"
    state.resolution_owner_id = "hero_swift"
    for uid, point in [
        ("hero_swift", h(0, 0)),
        ("hero_hanu", h(-2, 0)),
        ("hero_brogan", h(2, 0)),
        ("hero_arien", h(2, -2)),
    ]:
        state.place_entity(uid, point)
    minion = state.teams[TeamColor.BLUE].minions[0]
    state.place_entity(minion.id, h(1, 0))
    return state, h, minion


def effect(state, eid, kind, source="hero_brogan", **kwargs):
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, Shape

    return ActiveEffect(
        id=eid,
        source_id=source,
        effect_type=kind,
        scope=EffectScope(shape=Shape.GLOBAL),
        duration=DurationType.PASSIVE,
        created_at_turn=state.turn,
        created_at_round=state.round,
        is_active=True,
        **kwargs,
    )


def cases(advance=None, answer=None):
    from goa2.domain.input import selection_value
    from goa2.domain.models import MinionType, Token, TokenType
    from goa2.domain.models.effect import (
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )
    from goa2.engine.steps import (
        CheckMinionProtectionStep,
        CheckPassiveAbilitiesStep,
        DefeatUnitStep,
        ForceDiscardOrDefeatStep,
        ResolveCombatStep,
        SetContextFlagStep,
    )

    result = []
    variants = [
        "card",
        "skip",
        "invalid",
        "legacy",
        "nested",
        "facedown",
        "empty-hand",
        "wrong-colors",
        "inactive",
        "expired",
        "off-scope",
        "wrong-type",
        "tried",
        "two",
        "two-skip",
        "two-invalid",
        "duplicate-id",
        "missing-source",
        "totem",
        "totem-priority",
        "totem-missing",
        "totem-offboard",
        "totem-wrong-kind",
        "token-bound",
        "card-linked",
        "immune",
        "no-protection",
        "missing-minion",
        "fallback-direct",
        "ranged",
        "heavy",
        "matching-type",
        "bounty-subject",
        "all-decline",
    ]
    for variant in variants:
        for direct in [False, True]:
            if variant == "missing-minion" and not direct:
                continue  # Defeat rejects an unknown unit; the checker is a no-op.
            state, h, minion = state_for()
            if variant in ["ranged", "heavy"]:
                minion = next(
                    m for m in state.teams[minion.team].minions if m.type.value == variant.upper()
                )
                state.place_entity(minion.id, h(1, -1))
            protector = state.get_hero("hero_brogan")
            colors = list(dict.fromkeys(c.color for c in protector.hand if c.color is not None))
            prot = effect(
                state, "protect", EffectType.MINION_PROTECTION, allowed_discard_colors=colors
            )
            bounty = effect(
                state,
                "bounty",
                EffectType.MINION_DEFEAT_BOUNTY,
                source="hero_swift",
                subject_id="hero_swift",
                max_value=2,
            )
            bounty.scope.range = 3
            state.active_effects = [prot, bounty]
            if variant == "facedown":
                for c in protector.hand:
                    c.is_facedown = True
            if variant == "empty-hand":
                protector.hand = []
            if variant == "wrong-colors":
                prot.allowed_discard_colors = []
            if variant == "inactive":
                prot.is_active = False
            if variant == "expired":
                prot.duration = DurationType.THIS_ROUND
                prot.created_at_round = state.round - 1
                prot.is_active = False
            if variant == "off-scope":
                prot.scope = EffectScope(shape=Shape.POINT, origin_id="hero_brogan")
            if variant == "wrong-type":
                prot.protected_minion_types = [t for t in MinionType if t != minion.type]
            if variant == "matching-type":
                prot.protected_minion_types = [minion.type]
            if variant == "bounty-subject":
                bounty.subject_id = "hero_hanu"
            if variant == "missing-source":
                prot.source_id = "missing-hero"
            if variant in ["two", "two-skip", "two-invalid", "duplicate-id", "all-decline"]:
                state.active_effects.insert(1, prot.model_copy(deep=True))
                state.active_effects[1].id = (
                    "protect" if variant == "duplicate-id" else "protect-two"
                )
            if variant.startswith("totem") or variant == "token-bound":
                token = Token(
                    id="protect-totem",
                    name="Totem",
                    token_type=TokenType.TOTEM,
                    owner_id="hero_brogan",
                    is_passable=True,
                )
                state.misc_entities[token.id] = token
                state.token_pool.setdefault(token.token_type, []).append(token)
                state.place_entity(token.id, h(1, -1))
                sacr = prot.model_copy(deep=True)
                sacr.id = "sacrifice"
                sacr.sacrifice_origin_token = True
                sacr.scope.origin_id = token.id
                sacr.token_id = token.id
                state.active_effects.append(sacr)
                if variant != "totem-priority":
                    state.active_effects.remove(prot)
                if variant == "totem-missing":
                    state.misc_entities.pop(token.id)
                if variant == "totem-offboard":
                    state.remove_entity(token.id)
                if variant == "totem-wrong-kind":
                    sacr.scope.origin_id = "hero_brogan"
            if variant == "card-linked":
                prot.source_card_id = protector.hand[0].id
                prot.finishing_steps = [SetContextFlagStep(key="should_not_finish", value=True)]
            if variant == "immune":
                state.active_effects.append(
                    effect(state, "immune", EffectType.IMMUNITY_ENEMY_ACTIONS)
                )
            if variant in ["no-protection", "fallback-direct"]:
                state.active_effects = [bounty]
            target = "missing-minion" if variant == "missing-minion" else str(minion.id)
            step = (
                CheckMinionProtectionStep(
                    minion_id=target,
                    killer_id="hero_swift",
                    tried_effect_ids=["protect"] if variant == "tried" else [],
                )
                if direct
                else DefeatUnitStep(victim_id=target, killer_id="hero_swift")
            )
            answers = 0

            def choose(req, mode, variant=variant):
                nonlocal answers
                answers += 1
                if variant in ["skip", "duplicate-id", "all-decline"] or (
                    variant == "two-skip" and answers == 1
                ):
                    return "SKIP"
                if variant == "invalid" or (variant == "two-invalid" and answers == 1):
                    return "not-a-card"
                selected = selection_value(req.options[0])
                if variant == "legacy":
                    return {"selected_card_id": selected}
                if variant == "nested":
                    return {"selected_card_id": "", "selection": selected}
                return selected

            case, final = drive(
                f"combat-effect-protection-{variant}-{direct}",
                state,
                [step, SetContextFlagStep(key="after", value=True)],
                select=choose,
            )
            assert final.execution_context["after"]
            assert "should_not_finish" not in final.execution_context
            if variant in ["card", "facedown", "card-linked"]:
                assert target in final.entity_locations, (variant, direct)
                assert final.get_hero("hero_swift").gold == minion.value
                assert next(e for e in final.active_effects if e.id == "bounty").max_value == 2
            if variant in ["totem", "totem-priority", "token-bound"]:
                assert (
                    target in final.entity_locations
                    and "protect-totem" not in final.entity_locations
                )
                assert final.get_hero("hero_swift").gold == 0
            result.append(case)
    # Imported pending_input can use the old direct key; normal client submissions
    # are wrapped by submit_input and deliberately do not unwrap nested payloads.
    for pending in ["raw", "legacy", "selection", "null-selection", "invalid", "skip"]:
        state, _, minion = state_for()
        hero = state.get_hero("hero_brogan")
        prot = effect(
            state,
            "protect",
            EffectType.MINION_PROTECTION,
            allowed_discard_colors=[hero.hand[0].color],
        )
        state.active_effects = [prot]
        card = str(hero.hand[0].id)
        value = {
            "raw": card,
            "legacy": {"selected_card_id": card},
            "selection": {"selected_card_id": "", "selection": card},
            "null-selection": {"selection": None},
            "invalid": 42,
            "skip": "SKIP",
        }[pending]
        case, _ = drive(
            f"combat-effect-protection-pending-{pending}",
            state,
            [
                CheckMinionProtectionStep(
                    minion_id=str(minion.id), killer_id="hero_swift", pending_input=value
                )
            ],
        )
        result.append(case)
    for variant in [
        "plain",
        "zero",
        "none",
        "negative",
        "inactive",
        "expired",
        "friendly",
        "no-beneficiary",
        "offboard",
        "range",
        "empty-subject",
        "shape-ignored",
        "twice",
        "multiple",
        "last-coin",
        "split",
        "isolated",
        "immune",
        "source-fallback",
    ]:
        state, h, minion = state_for()
        bounty = effect(
            state,
            "bounty",
            EffectType.MINION_DEFEAT_BOUNTY,
            source="hero_hanu",
            subject_id="hero_swift",
            max_value=2,
        )
        bounty.scope.range = 2
        state.active_effects = [bounty]
        if variant == "zero":
            bounty.max_value = 0
        if variant == "none":
            bounty.max_value = None
        if variant == "negative":
            bounty.max_value = -1
        if variant == "inactive":
            bounty.is_active = False
        if variant == "expired":
            bounty.duration = DurationType.THIS_ROUND
            bounty.created_at_round = state.round - 1
            bounty.is_active = False
        if variant == "friendly":
            bounty.subject_id = "hero_brogan"
        if variant == "no-beneficiary":
            bounty.subject_id = "missing"
        if variant == "offboard":
            state.remove_entity("hero_swift")
        if variant == "range":
            bounty.scope.range = 0
        if variant == "empty-subject":
            bounty.subject_id = ""
        if variant == "shape-ignored":
            bounty.scope.shape = Shape.POINT
            bounty.scope.affects = AffectsFilter.SELF
        if variant == "source-fallback":
            bounty.subject_id = None
        if variant == "multiple":
            state.active_effects.append(
                bounty.model_copy(update={"id": "second", "subject_id": "hero_hanu"})
            )
            state.active_effects[1].scope = bounty.scope.model_copy(update={"range": 5})
        if variant == "last-coin":
            bounty.max_value = 1
        if variant in ["split", "isolated"]:
            topology = effect(
                state,
                "topology",
                EffectType.TOPOLOGY_SPLIT if variant == "split" else EffectType.TOPOLOGY_ISOLATION,
            )
            topology.split_axis = "q"
            topology.split_value = 0
            topology.isolated_hex = h(0, 0)
            state.active_effects.append(topology)
        if variant == "immune":
            state.active_effects.append(effect(state, "immune", EffectType.IMMUNITY_ENEMY_ACTIONS))
        steps = [DefeatUnitStep(victim_id=str(minion.id), killer_id="hero_hanu")]
        if variant in ["twice", "last-coin"]:
            other = state.teams[minion.team].minions[1]
            state.place_entity(other.id, h(1, -1))
            steps.append(DefeatUnitStep(victim_id=str(other.id), killer_id="hero_hanu"))
        case, final = drive(f"combat-effect-bounty-{variant}", state, steps)
        if variant == "twice":
            assert final.active_effects[0].max_value == 0
        result.append(case)
    for variant in ["source", "foreign-card", "owned-card", "token", "empty-token", "inactive"]:
        state, h, _ = state_for()
        eff = effect(state, "expiry", EffectType.MOVEMENT_ZONE, max_value=9)
        eff.finishing_steps = [SetContextFlagStep(key="should_not_finish", value=True)]
        if variant == "foreign-card":
            eff.source_card_id = state.get_hero("hero_swift").hand[0].id
        if variant == "owned-card":
            eff.source_id = "hero_swift"
            eff.source_card_id = state.get_hero("hero_brogan").hand[0].id
        if variant == "token":
            eff.token_id = "totem"
        if variant == "empty-token":
            eff.token_id = ""
        if variant == "inactive":
            eff.is_active = False
        state.active_effects = [eff]
        case, final = drive(
            f"combat-effect-expiry-{variant}",
            state,
            [DefeatUnitStep(victim_id="hero_brogan", killer_id="hero_swift")],
        )
        assert "should_not_finish" not in final.execution_context
        result.append(case)
    for attack in [-16, -1, 0, 2]:
        for defense in [None, -17, -16, -2, 0, 3]:
            state, _, _ = state_for()
            state.active_effects = [
                effect(state, "unrelated", EffectType.MOVEMENT_ZONE, max_value=9)
            ]
            state.execution_context = {
                "victim_id": "hero_brogan",
                "defense_value": defense,
                "minion_defense_modifier": 0,
            }
            case, final = drive(
                f"combat-effect-negative-{attack}-{defense}",
                state,
                [ResolveCombatStep(damage=attack)],
            )
            assert final.execution_context["block_succeeded"] == (
                defense is not None and defense >= attack
            )
            result.append(case)
    for variant in ["plain", "empty", "immune", "shield"]:
        state, _, _ = state_for()
        victim = state.get_hero("hero_brogan")
        state.active_effects = [effect(state, "unrelated", EffectType.MOVEMENT_ZONE, max_value=9)]
        if variant == "empty":
            victim.hand = []
        if variant == "immune":
            state.active_effects.append(
                effect(state, "immune", EffectType.IMMUNITY_ENEMY_ACTIONS, subject_id="hero_brogan")
            )
        if variant == "shield":
            state.active_effects.append(
                effect(state, "shield", EffectType.DISCARD_SHIELD, source_card_id=victim.hand[0].id)
            )
        state.execution_context = {"forced_victim": "hero_brogan"}
        case, _ = drive(
            f"combat-effect-force-{variant}",
            state,
            [ForceDiscardOrDefeatStep(victim_key="forced_victim", immunity_source_id="hero_swift")],
        )
        result.append(case)
    for variant in ["hand", "empty", "victim-carrier"]:
        state, _, _ = state_for()
        victim = state.get_hero("hero_brogan")
        carrier = (
            victim.hand[0] if variant == "victim-carrier" else state.get_hero("hero_swift").hand[0]
        )
        state.active_effects = [
            effect(
                state,
                "gun",
                EffectType.MOVEMENT_ZONE,
                source="hero_swift",
                source_card_id=carrier.id,
                max_value=9,
                granted_passive_effect_id="big_sodding_gun",
            )
        ]
        if variant in ["empty", "victim-carrier"]:
            victim.played_cards = victim.hand
            victim.hand = []
        state.execution_context = {"push_victim_id": "hero_brogan"}
        case, _ = drive(
            f"combat-effect-gun-{variant}",
            state,
            [CheckPassiveAbilitiesStep(trigger="after_push", hero_id="hero_swift")],
        )
        result.append(case)
    from goa2.engine.steps import CreateEffectStep

    for kind in [EffectType.MINION_PROTECTION, EffectType.MINION_DEFEAT_BOUNTY]:
        state, h, minion = state_for()
        hero = state.get_hero("hero_swift")
        create = CreateEffectStep(
            effect_type=kind,
            scope=EffectScope(shape=Shape.GLOBAL, range=4),
            duration=DurationType.PASSIVE,
            is_active=True,
            source_card_id=hero.hand[0].id,
            use_context_card=False,
            max_value=2,
            allowed_discard_colors=[hero.hand[0].color],
        )
        steps = [create, DefeatUnitStep(victim_id=str(minion.id), killer_id=hero.id)]
        if kind == EffectType.MINION_DEFEAT_BOUNTY:
            # Recreating the same card-bound effect must not refill spent coins.
            other = state.teams[minion.team].minions[1]
            state.place_entity(other.id, h(1, -1))
            steps += [
                create.model_copy(deep=True),
                DefeatUnitStep(victim_id=str(other.id), killer_id=hero.id),
            ]
        case, final = drive(f"combat-effect-create-{kind.value}", state, steps)
        if kind == EffectType.MINION_DEFEAT_BOUNTY:
            assert len(final.active_effects) == 1 and final.active_effects[0].max_value == 0
        result.append(case)
    state, _, _ = state_for()
    state.teams[state.get_hero("hero_brogan").team].life_counters = 1
    state.active_effects = [
        effect(state, "retained", EffectType.MOVEMENT_ZONE, source="hero_swift", max_value=9)
    ]
    case, final = drive(
        "combat-effect-terminal",
        state,
        [DefeatUnitStep(victim_id="hero_brogan", killer_id="hero_swift")],
    )
    assert final.phase.value == "GAME_OVER" and final.active_effects
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
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/artifacts/combat-effects.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        from stack_traces import verify

        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        print(json.dumps(verify(args.corpus, command), indent=2))
