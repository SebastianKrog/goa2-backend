"""Frozen immediate round cleanup, persistence, upgrades and token-effect events.

EndPhaseCleanup intentionally ignores the finishing payload returned by its second
round-duration sweep. Scheduled end-phase/turn finishing is a separate consumer.
Raw cleanup steps isolate this behavior; cards remain real registered objects.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from combat_effect_traces import effect
from mine_traces import drive
from oracle import ROOT, digest, verify_sources


def states():
    from model_traces import setup

    from goa2.domain.models import GamePhase, Token, TokenType
    from goa2.domain.models.effect import DurationType, EffectScope, EffectType, Shape
    from goa2.engine.steps import SetContextFlagStep

    variants = [
        (f"{duration.value}-{active}-{link}", duration, active, link)
        for duration in DurationType
        for active in [False, True]
        for link in [None, "", "played", "current", "discard", "hand", "deck", "ultimate"]
    ]
    variants += [
        (v, DurationType.PASSIVE, True, None)
        for v in [
            "temporary-token",
            "persistent-token",
            "unpooled-token",
            "unplaced-token",
            "duplicate-token-pool",
            "empty-token-binding",
            "token-played-card",
            "scope-token",
            "source-token",
            "duplicate-id",
            "round-finishing-ignored",
            "turn-finishing-retained",
            "after-cards-round",
            "two-tokens-ordered",
        ]
    ]
    for name, duration, active, link in variants:
        state = setup()
        state.phase, state.round, state.turn = GamePhase.CLEANUP, 3, 4
        state.current_actor_id = "hero_arien"
        hero = state.get_hero("hero_arien")
        played = hero.hand[0]
        hero.play_card(played)
        hero.resolve_current_card(turn_number=1)
        current = hero.hand[0]
        hero.play_card(current)
        discarded = hero.hand[0]
        hero.discard_card(discarded)
        links = {
            "played": played,
            "current": current,
            "discard": discarded,
            "hand": hero.hand[0],
            "deck": next(
                c
                for c in hero.deck
                if c not in hero.hand
                and c is not played
                and c is not current
                and c is not discarded
            ),
            "ultimate": hero.ultimate_card,
        }
        for card in [played, current, discarded, *hero.hand]:
            card.is_active = True
        printed = links[link].id if link else link
        e = effect(
            state,
            "cleanup-effect",
            EffectType.MOVEMENT_ZONE,
            source="hero_arien",
            max_value=9,
            source_card_id=printed,
        )
        e.duration, e.is_active = duration, active
        state.active_effects = [e]
        if "token" in name:
            token = Token(
                id="cleanup-token",
                name="Cleanup token",
                token_type=TokenType.TOTEM,
                owner_id="hero_arien",
                persists_end_of_round=name in ["persistent-token", "token-played-card"],
            )
            state.misc_entities[token.id] = token
            if name != "unpooled-token":
                state.token_pool.setdefault(token.token_type, []).append(token)
            if name != "unplaced-token":
                target = next(
                    h for h in state.board.tiles if not state.board.get_tile(h).is_obstacle
                )
                state.place_entity(token.id, target)
            e.token_id = "" if name == "empty-token-binding" else token.id
            if name == "empty-token-binding":
                e.duration = DurationType.THIS_ROUND
            if name == "duplicate-token-pool":
                state.token_pool[token.token_type].append(token)
            if name == "scope-token":
                e.token_id = None
                e.scope = EffectScope(shape=Shape.GLOBAL, origin_id=token.id)
            if name == "source-token":
                e.token_id = None
                e.source_id = token.id
            if name == "token-played-card":
                e.source_card_id = played.id
            if name == "two-tokens-ordered":
                second = token.model_copy(deep=True, update={"id": "cleanup-token-2"})
                state.misc_entities[second.id] = second
                state.token_pool[token.token_type].append(second)
                target = next(
                    h for h in state.board.tiles if not state.board.get_tile(h).is_obstacle
                )
                state.place_entity(second.id, target)
                state.active_effects.append(
                    e.model_copy(deep=True, update={"id": "second-effect", "token_id": second.id})
                )
        if name == "duplicate-id":
            e.duration = DurationType.THIS_ROUND
            state.active_effects.append(
                e.model_copy(
                    deep=True,
                    update={"duration": DurationType.PASSIVE, "source_card_id": hero.hand[0].id},
                )
            )
        if name in ["round-finishing-ignored", "turn-finishing-retained", "after-cards-round"]:
            e.duration = (
                DurationType.THIS_TURN
                if name == "turn-finishing-retained"
                else DurationType.THIS_ROUND
            )
            e.finishing_steps = [SetContextFlagStep(key="should_not_finish", value=True)]
            if name == "after-cards-round":
                e.effect_type = EffectType.AFTER_CARDS_PLAYED_TRIGGER
        yield name, state


def cases():
    from round_traces import cases as round_cases

    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import EndPhaseCleanupStep, SetContextFlagStep

    def add_effect(state, name):
        state.active_effects = [
            effect(
                state,
                "persistent-round-effect",
                EffectType.MOVEMENT_ZONE,
                source=str(state.current_actor_id or "hero_arien"),
                max_value=99,
            )
        ]
        if name.startswith("token-"):
            state.active_effects.append(
                effect(
                    state,
                    "round-token-effect",
                    EffectType.MOVEMENT_ZONE,
                    source="hero_arien",
                    max_value=99,
                    token_id="round_token",
                )
            )

    advance = {"kind": "advance"}

    def answer(value, request_id="current"):
        return {"kind": "input", "request_id": request_id, "selection": value}

    result = round_cases(advance, answer, effect_factory=add_effect, cleanup_only=True)
    for case in result:
        case["name"] = "round-effect-regression-" + case["name"]
    for name, state in states():
        case, final = drive(
            "round-effect-cleanup-" + name,
            state,
            [EndPhaseCleanupStep(), SetContextFlagStep(key="cleanup_done", value=True)],
        )
        assert final.execution_context["cleanup_done"]
        assert "should_not_finish" not in final.execution_context
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
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/artifacts/round-effects.json")
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
