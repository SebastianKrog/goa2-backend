"""Frozen real-handler forced discard, shield selection and linked-effect expiry."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from mine_traces import drive
from model_traces import setup
from oracle import ROOT, digest, verify_sources


def state_for(variant):
    from aura_traces import _board

    from goa2.domain.board import Board
    from goa2.domain.models import CardState, GamePhase, Token, TokenType
    from goa2.domain.models.effect import (
        ActiveEffect,
        AffectsFilter,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )
    from goa2.engine.steps import SetContextFlagStep

    state = setup(["Min", "Hanu"], ["Mrak", "Arien"])
    h = _board(state)
    state.board = Board(
        tiles={p: t for p, t in state.board.tiles.items() if p.r == 0 and 0 <= p.q <= 2}
    )
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_min"
    state.resolution_owner_id = "hero_min"
    state.place_entity("hero_mrak", h(0, 0))
    state.place_entity("hero_min", h(2, 1))
    hero = state.get_hero("hero_mrak")
    # Registered physical deck cards, with their existing aliases preserved.
    shield = next(c for c in hero.deck if c.id == "stone_carapace")
    hero.hand = [c for c in hero.hand if c.id != shield.id]
    shield.state = CardState.RESOLVED
    shield.is_facedown = False
    shield.is_active = True
    hero.played_cards = [shield]
    if variant in ["current", "current-facedown"]:
        hero.current_turn_card = shield
        hero.played_cards = []
        shield.state = CardState.UNRESOLVED
    if variant in ["facedown", "current-facedown"]:
        shield.is_facedown = True
    if variant == "duplicate":
        hero.played_cards.append(shield)
        hero.current_turn_card = shield
    if variant == "missing-card":
        hero.played_cards = []
    scope = EffectScope(shape=Shape.GLOBAL, affects=AffectsFilter.SELF)
    effect = ActiveEffect(
        id="shield",
        source_id="hero_mrak",
        source_card_id=shield.id,
        effect_type=EffectType.DISCARD_SHIELD,
        scope=scope,
        duration=DurationType.THIS_ROUND,
        created_at_turn=state.turn,
        created_at_round=state.round,
        is_active=True,
        finishing_steps=[SetContextFlagStep(key="should_not_finish", value=True)],
    )
    state.active_effects.append(effect)
    state.active_effects.append(
        ActiveEffect(
            id="unrelated",
            source_id="hero_min",
            effect_type=EffectType.MOVEMENT_ZONE,
            scope=EffectScope(shape=Shape.GLOBAL),
            max_value=8,
            duration=DurationType.PASSIVE,
            created_at_turn=state.turn,
            created_at_round=state.round,
            is_active=True,
        )
    )
    if variant == "inactive":
        effect.is_active = False
    if variant == "old-active":
        effect.created_at_round = state.round - 1
    if variant == "wrong-source":
        effect.source_id = "hero_arien"
    if variant == "empty-card":
        effect.source_card_id = ""
    if variant == "off-scope":
        effect.scope = EffectScope(
            shape=Shape.POINT, origin_id="hero_min", affects=AffectsFilter.ENEMY_HEROES
        )
    if variant == "multiple":
        other = next(c for c in hero.deck if c.id == "rock_solid")
        hero.hand = [c for c in hero.hand if c.id != other.id]
        other.state = CardState.RESOLVED
        other.is_facedown = False
        hero.played_cards.append(other)
        state.active_effects.append(
            effect.model_copy(update={"id": "shield-two", "source_card_id": other.id})
        )
    if variant == "linked-expiry":
        state.active_effects.append(
            effect.model_copy(
                update={
                    "id": "linked-other",
                    "effect_type": EffectType.MOVEMENT_ZONE,
                    "max_value": 2,
                }
            )
        )
    if variant == "empty-hand":
        hero.hand = []
    if variant == "no-basic":
        hero.hand = [c for c in hero.hand if not c.is_basic]
    if variant in ["immune", "friendly-immunity", "no-immunity-source"]:
        state.active_effects.append(
            ActiveEffect(
                id="immunity",
                source_id="hero_mrak",
                effect_type=EffectType.IMMUNITY_ENEMY_ACTIONS,
                scope=EffectScope(shape=Shape.GLOBAL, affects=AffectsFilter.SELF),
                duration=DurationType.PASSIVE,
                created_at_turn=state.turn,
                created_at_round=state.round,
                is_active=True,
            )
        )
    if variant == "irrelevant-provider":
        # The grant has no AFTER_CARD_DISCARD config; it must not block a discard.
        state.active_effects.append(
            effect.model_copy(
                update={
                    "id": "grant",
                    "source_card_id": hero.hand[0].id,
                    "granted_passive_effect_id": "big_sodding_gun",
                }
            )
        )
    token = Token(
        id="blast",
        name="Blast",
        owner_id="hero_min",
        token_type=TokenType.MINE_BLAST,
        is_passable=True,
    )
    state.misc_entities["blast"] = token
    state.token_pool.setdefault(token.token_type, []).append(token)
    state.place_entity("blast", h(1, 0))
    state.execution_context = {"target_hex": h(2, 0).model_dump(), "victim": "hero_mrak"}
    return state, hero, shield


def cases(advance=None, answer=None):
    from goa2.engine.steps import (
        DiscardCardStep,
        ForceDiscardStep,
        MoveUnitStep,
        SetContextFlagStep,
        TriggerMineStep,
    )

    result = []
    for variant in [
        "plain",
        "current",
        "current-facedown",
        "facedown",
        "duplicate",
        "missing-card",
        "inactive",
        "old-active",
        "wrong-source",
        "empty-card",
        "off-scope",
        "multiple",
        "linked-expiry",
        "empty-hand",
        "no-basic",
        "basic",
        "non-basic",
        "immune",
        "friendly-immunity",
        "no-immunity-source",
        "irrelevant-provider",
    ]:
        for mode in ["first", "last", "invalid"]:
            state, _hero, shield = state_for(variant)
            options = {"victim_id": "hero_mrak", "immunity_source_id": "hero_min"}
            if variant in ["basic", "no-basic"]:
                options["card_is_basic"] = True
            if variant == "non-basic":
                options["card_is_basic"] = False
            if variant == "friendly-immunity":
                options["immunity_source_id"] = "hero_arien"
            if variant == "no-immunity-source":
                options["immunity_source_id"] = None
            case, final = drive(
                f"discard-effect-{variant}-{mode}",
                state,
                [ForceDiscardStep(**options), SetContextFlagStep(key="after", value=True)],
                mode,
            )
            assert (
                final.execution_context["after"]
                and "should_not_finish" not in final.execution_context
            )
            if variant == "immune":
                assert not final.get_hero("hero_mrak").discard_pile
            if variant == "plain" and mode == "last":
                assert any(c.id == shield.id for c in final.get_hero("hero_mrak").discard_pile)
                assert all(e.source_card_id != shield.id for e in final.active_effects)
            result.append(case)
    for variant in ["plain", "current", "linked-expiry", "inactive"]:
        for played in [False, True]:
            state, _hero, shield = state_for(variant)
            step = DiscardCardStep(
                hero_id="hero_mrak", card_id=shield.id, source="PLAYED" if played else "HAND"
            )
            case, final = drive(f"discard-effect-direct-{variant}-{played}", state, [step])
            assert all(e.source_card_id != shield.id for e in final.active_effects)
            assert "should_not_finish" not in final.execution_context
            result.append(case)
    for variant in [
        "plain",
        "current",
        "basic",
        "immune",
        "inactive",
        "empty-hand",
        "linked-expiry",
        "irrelevant-provider",
    ]:
        for movement in [False, True]:
            for mode in ["first", "last"]:
                state, _hero, _shield = state_for(variant)
                step = (
                    MoveUnitStep(unit_id="hero_mrak", range_val=2)
                    if movement
                    else TriggerMineStep(mine_ids=["blast"], victim_id="hero_mrak")
                )
                case, final = drive(
                    f"discard-effect-blast-{variant}-{'move' if movement else 'trigger'}-{mode}",
                    state,
                    [step, SetContextFlagStep(key="after", value=True)],
                    mode,
                )
                assert (
                    "blast" not in final.entity_locations
                    and "should_not_finish" not in final.execution_context
                )
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
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/artifacts/discard-effects.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        from stack_traces import verify

        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        print(json.dumps(verify(args.corpus, command), indent=2))
