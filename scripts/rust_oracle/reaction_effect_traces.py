"""Actor-bound illusion defense and effect-aware real defense/discard windows.

Raw steps isolate reaction/combat primitives. Physical cards come from the real
registered decks; Python's handler supplies every expected state/request/event.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from combat_effect_traces import effect
from mine_traces import drive
from model_traces import setup
from oracle import ROOT, digest, verify_sources


def illusion_state(variant):
    from aura_traces import _board, _pieces

    from goa2.domain.models import GamePhase, MinionType, TeamColor, Token, TokenType
    from goa2.domain.models.effect import DurationType, EffectScope, EffectType, Shape

    state = setup(["Arien", "Razzle"], ["Mrak", "Hanu"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_arien"
    state.resolution_owner_id = "hero_arien"
    state.place_entity("hero_arien", h(0, 3))
    state.place_entity("hero_mrak", h(0, 0))
    state.place_entity("hero_hanu", h(3, -1))
    pieces = _pieces(state, "hero_razzle")
    state.place_entity(pieces[0], h(-2, 0))
    state.place_entity(pieces[1], h(-2, 1))
    for i, pos in enumerate([h(1, 0), h(0, 1), h(0, 2)]):
        token = Token(
            id=f"illusion-{i}",
            name="Illusion",
            token_type=TokenType.ILLUSION,
            owner_id="hero_hanu",
            is_passable=False,
        )
        state.misc_entities[token.id] = token
        state.token_pool.setdefault(token.token_type, []).append(token)
        state.place_entity(token.id, pos)
    friendly = next(m for m in state.teams[TeamColor.BLUE].minions if m.type == MinionType.MELEE)
    enemy = next(m for m in state.teams[TeamColor.RED].minions if m.type == MinionType.RANGED)
    state.place_entity(friendly.id, h(-1, 0))
    state.place_entity(enemy.id, h(2, 0))
    provider = effect(
        state, "equivalence", EffectType.ILLUSION_MINION_EQUIVALENCE, source="hero_arien"
    )
    state.active_effects = [provider]
    target = "hero_mrak"
    if variant == "inactive":
        provider.is_active = False
    if variant == "other-actor":
        state.current_actor_id = "hero_hanu"
    if variant == "no-actor":
        state.current_actor_id = None
    if variant == "empty-actor":
        state.current_actor_id = ""
    if variant == "unknown-actor":
        state.current_actor_id = "unknown"
    if variant == "unknown-source":
        provider.source_id = "unknown"
    if variant == "token-source":
        provider.source_id = "illusion-0"
        state.current_actor_id = "illusion-0"
    if variant == "blue-source":
        provider.source_id = "hero_hanu"
        state.current_actor_id = "hero_hanu"
    if variant in ["actor-piece", "source-piece", "both-pieces"]:
        provider.source_id = pieces[0] if variant != "actor-piece" else "hero_razzle"
        state.current_actor_id = "hero_razzle" if variant == "source-piece" else pieces[1]
    if variant == "old-active":
        provider.duration = DurationType.THIS_ROUND
        provider.created_at_round = state.round - 1
    if variant == "off-scope":
        provider.scope = EffectScope(shape=Shape.POINT, origin_id="hero_arien")
    if variant == "wrong-subject":
        provider.subject_id = "hero_hanu"
    if variant == "source-offboard":
        state.remove_entity("hero_arien")
    if variant == "duplicate":
        state.active_effects.append(provider.model_copy(deep=True))
    if variant == "inactive-first":
        inactive = provider.model_copy(update={"id": "inactive", "is_active": False})
        state.active_effects.insert(0, inactive)
    if variant == "wrong-first":
        state.active_effects.insert(
            0, provider.model_copy(update={"id": "other", "source_id": "hero_hanu"})
        )
    if variant == "non-illusion":
        state.misc_entities["illusion-0"].token_type = TokenType.TOTEM
    if variant == "offboard-token":
        state.remove_entity("illusion-0")
    if variant == "target-piece":
        state.remove_entity("hero_mrak")
        state.place_entity(pieces[0], h(0, 0))
        target = pieces[0]
    if variant == "target-owner":
        target = "hero_razzle"
    if variant == "target-minion":
        target = str(friendly.id)
    if variant == "target-offboard":
        state.remove_entity(target)
    if variant in ["split", "isolation"]:
        top = effect(
            state,
            "topology",
            EffectType.TOPOLOGY_SPLIT if variant == "split" else EffectType.TOPOLOGY_ISOLATION,
        )
        top.split_axis = "q"
        top.split_value = 0
        top.isolated_hex = h(0, 0)
        state.active_effects.append(top)
    return state, target


def cases(advance=None, answer=None):
    from aura_traces import _board
    from discard_effect_traces import state_for as shield_state

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState, GamePhase, StatType, TeamColor
    from goa2.domain.models.effect import EffectType
    from goa2.engine.stats import calculate_minion_defense_modifier
    from goa2.engine.steps import (
        ReactionWindowStep,
        ResolveCombatStep,
        ResolveDefenseTextStep,
        ResolveOnBlockEffectStep,
        RestoreActionTypeStep,
    )

    result = []
    for variant in [
        "plain",
        "inactive",
        "other-actor",
        "no-actor",
        "empty-actor",
        "unknown-actor",
        "unknown-source",
        "token-source",
        "blue-source",
        "actor-piece",
        "source-piece",
        "both-pieces",
        "old-active",
        "off-scope",
        "wrong-subject",
        "source-offboard",
        "duplicate",
        "inactive-first",
        "wrong-first",
        "non-illusion",
        "offboard-token",
        "target-piece",
        "target-owner",
        "target-minion",
        "target-offboard",
        "split",
        "isolation",
    ]:
        for window in [False, True]:
            state, target = illusion_state(variant)
            modifier = calculate_minion_defense_modifier(state, target)
            if variant in [
                "plain",
                "old-active",
                "off-scope",
                "wrong-subject",
                "source-offboard",
                "duplicate",
            ]:
                assert modifier == -2, (variant, modifier)
            state.execution_context = {
                "target_id": target,
                "defense_value": 0,
                "attack_damage": -1,
                "current_action_type": "ATTACK",
            }
            steps = (
                [ReactionWindowStep()]
                if window
                else [ResolveCombatStep(damage=-1, target_key="target_id")]
            )
            case, _ = drive(
                f"reaction-effect-illusion-{variant}-{window}",
                state,
                steps,
                select=lambda req, mode: "PASS",
            )
            result.append(case)
    for variant in [
        "plain",
        "current",
        "facedown",
        "current-facedown",
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
        "irrelevant-provider",
    ]:
        for mode in ["shield", "hand", "pass", "invalid"]:
            state, hero, shield = shield_state(variant)
            state.execution_context = {
                "target_id": str(hero.id),
                "attack_damage": 3,
                "current_action_type": "ATTACK",
                "defense_bonus": 99,
                "auto_block": True,
            }
            count = 0

            def choose(req, mode, shield_id=str(shield.id)):
                nonlocal count
                count += 1
                ids = [o.id for o in req.options]
                if mode == "invalid" and count == 1:
                    return "not-a-card"
                if mode == "pass":
                    return "PASS"
                if mode in ["shield", "invalid"] and shield_id in ids:
                    return shield_id
                return selection_value(req.options[0])

            steps = [
                ReactionWindowStep(),
                ResolveDefenseTextStep(),
                ResolveCombatStep(damage=3, target_key="target_id"),
                ResolveOnBlockEffectStep(),
                RestoreActionTypeStep(),
            ]
            case, final = drive(
                f"reaction-effect-shield-{variant}-{mode}", state, steps, mode, select=choose
            )
            assert "should_not_finish" not in final.execution_context
            result.append(case)
    # Each real deck is offered with active effects. Choosing a reaction routes
    # through the real discard and BEFORE_DEFENSE trigger, without inventing text.
    setup()
    for name in sorted(HeroRegistry._heroes):
        for initiative in [False, True]:
            others = [n for n in ["Arien", "Hanu", "Brogan", "Tigerclaw"] if n != name]
            state = setup([name, others[0]], others[1:3])
            h = _board(state)
            hero = state.teams[TeamColor.RED].heroes[0]
            state.place_entity(hero.id, h(0, 0))
            state.current_actor_id = state.teams[hero.team].heroes[1].id
            state.phase = GamePhase.RESOLUTION
            state.execution_context = {
                "target_id": str(hero.id),
                "attack_damage": -2,
                "current_action_type": "ATTACK",
                "defense_uses_initiative": initiative,
            }
            state.active_effects = [
                effect(
                    state,
                    "basic",
                    EffectType.BASIC_ACTION_STAT_BONUS,
                    source=str(hero.id),
                    stat_type=StatType.INITIATIVE if initiative else StatType.DEFENSE,
                    stat_value=3,
                )
            ]
            case, _ = drive(
                f"reaction-effect-deck-{name}-{initiative}", state, [ReactionWindowStep()]
            )
            result.append(case)
    from goa2.domain.models.marker import MarkerType

    for name, effect_id in [
        ("Arien", "aspiring_duelist"),
        ("Bain", "perfect_getaway"),
        ("Bain", "close_call"),
        ("Bain", "narrow_escape"),
        ("Wasp", "stop_projectiles"),
    ]:
        for marked in [False, True]:
            for ranged in [False, True]:
                others = [n for n in ["Arien", "Hanu", "Brogan", "Tigerclaw"] if n != name]
                state = setup([name, others[0]], others[1:3])
                h = _board(state)
                state.phase = GamePhase.RESOLUTION
                hero = state.teams[TeamColor.RED].heroes[0]
                attacker = state.teams[TeamColor.BLUE].heroes[0]
                state.current_actor_id = attacker.id
                state.resolution_owner_id = attacker.id
                state.place_entity(attacker.id, h(2, 0))
                state.place_entity(hero.id, h(0, 0))
                card = next(c for c in hero.deck if c.effect_id == effect_id)
                if card not in hero.hand:
                    hero.hand.append(card)
                card.state = CardState.UNRESOLVED
                card.is_facedown = False
                if marked:
                    state.place_marker(MarkerType.BOUNTY, attacker.id, 1, hero.id)
                state.active_effects = [
                    effect(
                        state,
                        "stat",
                        EffectType.AREA_STAT_MODIFIER,
                        source=str(hero.id),
                        stat_type=StatType.DEFENSE,
                        stat_value=-2,
                    )
                ]
                state.execution_context = {
                    "target_id": str(hero.id),
                    "attacker_id": str(attacker.id),
                    "attack_damage": 3,
                    "attack_is_ranged": ranged,
                    "current_action_type": "ATTACK",
                }
                case, _ = drive(
                    f"reaction-effect-body-{effect_id}-{marked}-{ranged}",
                    state,
                    [
                        ReactionWindowStep(),
                        ResolveDefenseTextStep(),
                        ResolveCombatStep(damage=3, target_key="target_id"),
                        ResolveOnBlockEffectStep(),
                        RestoreActionTypeStep(),
                    ],
                    select=lambda req, mode, card_id=str(card.id): card_id,
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
        "--corpus", type=Path, default=ROOT / "rust/artifacts/reaction-effects.json"
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
