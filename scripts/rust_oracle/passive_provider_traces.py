"""Ordinary and effect-granted movement passive execution and provider usage."""

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
    from goa2.domain.models import GamePhase, Token, TokenType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape

    state = setup(["Silverarrow", "Min"], ["Mrak", "Hanu"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    owner = "hero_mrak" if not variant.startswith("ordinary") else "hero_silverarrow"
    state.current_actor_id = owner
    state.resolution_owner_id = owner
    state.place_entity(owner, h(0, 0))
    carrier = state.get_hero("hero_silverarrow").hand[0]
    carrier.passive_uses_this_turn = 7
    provider = ActiveEffect(
        id="grant",
        source_id=owner,
        source_card_id=carrier.id,
        effect_type=EffectType.MOVEMENT_ZONE,
        scope=EffectScope(shape=Shape.GLOBAL),
        duration=DurationType.PASSIVE,
        max_value=8,
        is_active=True,
        granted_passive_effect_id="wild_hunt",
        created_at_turn=state.turn,
        created_at_round=state.round,
    )
    state.active_effects.append(provider)
    if variant.startswith("ordinary"):
        hero = state.get_hero(owner)
        carrier = hero.ultimate_card
        hero.level = 8
        carrier.is_facedown = False
        provider.source_card_id = carrier.id
        if variant == "ordinary-low-level":
            hero.level = 7
        if variant == "ordinary-facedown":
            carrier.is_facedown = True
        if variant != "ordinary-dedup":
            state.active_effects = []
    if variant == "inactive":
        provider.is_active = False
    if variant == "wrong-source":
        provider.source_id = "hero_min"
    if variant == "missing-card":
        provider.source_card_id = "missing-card"
    if variant == "unknown-grant":
        provider.granted_passive_effect_id = "missing-effect"
    if variant == "empty-grant":
        provider.granted_passive_effect_id = ""
    if variant == "no-grant":
        provider.granted_passive_effect_id = None
    if variant == "used":
        provider.passive_uses_this_turn = 5
    if variant == "negative-uses":
        provider.passive_uses_this_turn = -2
    if variant == "facedown-carrier":
        carrier.is_facedown = True
    if variant == "duplicate-provider":
        state.active_effects.append(provider.model_copy(update={"id": "grant-two"}))
    if variant == "offboard":
        # An off-board owner has no origin: do not invent a relocation.
        state.remove_entity(owner)
    if variant == "scope-ignored":
        provider.scope = EffectScope(shape=Shape.POINT, origin_id="hero_min")
    if variant == "old-active":
        provider.created_at_round = state.round - 1
    if variant == "empty-provider-id":
        provider.id = ""
    if variant == "no-actor":
        state.current_actor_id = None
    if variant in ["mine", "mine-linked-provider"]:
        state.board = Board(
            tiles={p: t for p, t in state.board.tiles.items() if p.r == 0 and -2 <= p.q <= 0}
        )
        token = Token(
            id="wild-mine",
            name="Mine",
            owner_id="hero_min",
            token_type=TokenType.MINE_DUD,
            is_passable=True,
        )
        state.misc_entities[token.id] = token
        state.token_pool.setdefault(token.token_type, []).append(token)
        state.place_entity(token.id, h(-1, 0))
        if variant == "mine-linked-provider":
            provider.token_id = token.id
    if variant == "actions-only-cap":
        provider.max_value = 1
        provider.limit_actions_only = True
    if variant == "movement-cap":
        provider.max_value = 1
    return state, owner, carrier, provider


def cases(advance=None, answer=None):
    from goa2.domain.input import selection_value
    from goa2.engine.steps import (
        CheckPassiveAbilitiesStep,
        MarkPassiveUsedStep,
        OfferPassiveStep,
        SetContextFlagStep,
    )

    result = []
    variants = [
        "plain",
        "offboard",
        "inactive",
        "wrong-source",
        "missing-card",
        "unknown-grant",
        "empty-grant",
        "no-grant",
        "used",
        "negative-uses",
        "facedown-carrier",
        "duplicate-provider",
        "scope-ignored",
        "old-active",
        "empty-provider-id",
        "no-actor",
        "mine",
        "mine-linked-provider",
        "actions-only-cap",
        "movement-cap",
        "ordinary",
        "ordinary-low-level",
        "ordinary-facedown",
        "ordinary-dedup",
    ]
    for variant in variants:
        for direct in [False, True]:
            for mode in ["yes", "no", "skip", "badhex"]:
                state, owner, carrier, provider = state_for(variant)
                step = (
                    OfferPassiveStep(
                        card_id=carrier.id,
                        trigger="before_action",
                        hero_id=owner,
                        active_effect_id=None if variant.startswith("ordinary") else provider.id,
                        prompt="Direct passive",
                    )
                    if direct
                    else CheckPassiveAbilitiesStep(hero_id=owner, trigger="before_action")
                )
                invalid_sent = False

                def choose(req, mode):
                    nonlocal invalid_sent
                    if req.request_type.value == "CONFIRM_PASSIVE":
                        return "NO" if mode == "no" else "YES"
                    if mode == "skip":
                        return "SKIP"
                    if mode == "badhex" and not invalid_sent:
                        invalid_sent = True
                        return {"q": 90, "r": 0, "s": -90}
                    return selection_value(req.options[0])

                case, final = drive(
                    f"provider-{variant}-{'direct' if direct else 'scan'}-{mode}",
                    state,
                    [step, SetContextFlagStep(key="after", value=True)],
                    mode,
                    select=choose,
                )
                if variant == "movement-cap" and mode in ["yes", "badhex"]:
                    assert "after" not in final.execution_context
                else:
                    assert final.execution_context.get("after") is True, (
                        variant,
                        direct,
                        mode,
                        final.execution_context,
                    )
                if variant == "mine" and mode in ["yes", "badhex"]:
                    assert "wild-mine" not in final.entity_locations
                    assert (
                        next(
                            e for e in final.active_effects if e.id == "grant"
                        ).passive_uses_this_turn
                        == 1
                    )
                if variant == "mine-linked-provider" and mode in ["yes", "badhex"]:
                    assert all(e.id != "grant" for e in final.active_effects)
                if variant == "plain" and mode in ["yes", "skip", "badhex"]:
                    assert (
                        next(
                            e for e in final.active_effects if e.id == "grant"
                        ).passive_uses_this_turn
                        == 1
                    )
                    assert final.get_card_by_id(carrier.id).passive_uses_this_turn == 7
                result.append(case)
    for variant in [
        "plain",
        "inactive",
        "wrong-source",
        "missing-card",
        "empty-provider-id",
        "no-actor",
        "used",
        "negative-uses",
        "ordinary",
        "ordinary-facedown",
    ]:
        state, owner, carrier, provider = state_for(variant)
        initial = provider.passive_uses_this_turn
        step = MarkPassiveUsedStep(
            card_id=provider.source_card_id if variant == "missing-card" else carrier.id,
            active_effect_id=None if variant.startswith("ordinary") else provider.id,
        )
        case, final = drive(f"provider-mark-{variant}", state, [step])
        if not variant.startswith("ordinary"):
            assert (
                next(e for e in final.active_effects if e.id == provider.id).passive_uses_this_turn
                == initial + 1
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
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/artifacts/passive-providers.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        from stack_traces import verify

        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        print(json.dumps(verify(args.corpus, command), indent=2))
