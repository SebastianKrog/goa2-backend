"""Frozen rock-ultimate gates, ordered piece targets and complete discard flows."""

from __future__ import annotations


def scenes():
    from combat_effect_traces import effect
    from live_passive_traces import build
    from object_execution_traces import live_choice_operations

    from goa2.domain.hex import Hex
    from goa2.domain.models import CardState, GamePhase
    from goa2.domain.models.effect import DurationType, EffectType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.steps import OfferRockUltimateStep, SetContextFlagStep

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def retain(state, steps, mode="first"):
        tail = SetContextFlagStep(step_id="fixture/rock-ult/tail", key="tail", value=True)
        state.execution_stack = list(reversed([*steps, tail]))
        state.execution_context.update(
            ultimate_steps=steps, old_affected=["old"], locations_alias=state.entity_locations
        )
        state.execution_context["_rock_ult_affected"] = state.execution_context["old_affected"]
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    variants = (
        "plain",
        "level7",
        "level9",
        "no-ultimate",
        "facedown",
        "no-actor",
        "unknown-actor",
        "token-actor",
        "actor-team-none",
        "actor-team-blue",
        "used",
        "used-false",
        "custom-used",
        "empty-used",
        "no-rocks",
        "missing-rock",
        "null-rock",
        "dict-rock",
        "list",
        "list-dicts",
        "duplicate-rocks",
        "far",
        "offboard",
        "two-enemies",
        "friendly-only",
        "empty-hand",
        "attack-shield",
        "action-shield",
        "inactive-shield",
        "old-shield",
        "split",
        "isolation-rock",
        "isolation-enemy",
        "literal-step-id",
        "skip",
        "active",
        "repeat",
    )
    for mode in ("first", "last", "invalid"):
        for variant in variants:
            state, hero = build("Mrak")
            hero.level = 8
            victim = state.get_hero("hero_brogan")
            state.place_entity(victim.id, h(2))
            state.execution_context.update(rock=h(1), rocks=[h(1)], skip=False)
            step = OfferRockUltimateStep(step_id="fixture/rock-ult/offer", rock_hex_keys=["rock"])
            if variant == "level7":
                hero.level = 7
            if variant == "level9":
                hero.level = 9
            if variant == "no-ultimate":
                hero.ultimate_card = None
            if variant == "facedown":
                hero.ultimate_card.is_facedown = True
            if variant == "no-actor":
                state.current_actor_id = None
            if variant == "unknown-actor":
                state.current_actor_id = "unknown"
            if variant == "token-actor":
                state.current_actor_id = next(iter(state.misc_entities))
            if variant == "actor-team-none":
                hero.team = None
            if variant == "actor-team-blue":
                from goa2.domain.models import TeamColor

                hero.team = TeamColor.BLUE
            if variant == "used":
                state.execution_context["rock_ultimate_used"] = [1]
            if variant == "used-false":
                state.execution_context["rock_ultimate_used"] = False
            if variant == "custom-used":
                step.used_flag_key = "custom_used"
            if variant == "empty-used":
                step.used_flag_key = ""
            if variant == "no-rocks":
                step.rock_hex_keys = []
            if variant == "missing-rock":
                state.execution_context.pop("rock")
            if variant == "null-rock":
                state.execution_context["rock"] = None
            if variant == "dict-rock":
                state.execution_context["rock"] = h(1).model_dump()
            if variant in {"list", "list-dicts"}:
                step.rock_hex_keys = []
                step.rock_hexes_key = "rocks"
                if variant == "list-dicts":
                    state.execution_context["rocks"] = [h(1).model_dump()]
            if variant == "duplicate-rocks":
                step.rock_hex_keys = ["rock", "rock"]
                step.rock_hexes_key = "rocks"
            if variant == "far":
                state.execution_context["rock"] = h(-3)
            if variant == "offboard":
                state.remove_entity(victim.id)
            if variant == "two-enemies":
                state.place_entity("hero_hanu", h(2, -1))
            if variant == "friendly-only":
                state.remove_entity(victim.id)
                state.place_entity("hero_arien", h(2))
            if variant == "empty-hand":
                victim.hand = []
            if variant in {"attack-shield", "action-shield", "inactive-shield", "old-shield"}:
                shield = effect(
                    state,
                    "rock-shield",
                    (
                        EffectType.ATTACK_IMMUNITY
                        if variant == "attack-shield"
                        else EffectType.IMMUNITY_ENEMY_ACTIONS
                    ),
                    source=str(victim.id),
                )
                if variant == "inactive-shield":
                    shield.is_active = False
                if variant == "old-shield":
                    shield.duration = DurationType.THIS_ROUND
                    shield.created_at_round = state.round - 1
                state.active_effects = [shield]
                state.execution_context["current_action_type"] = "ATTACK"
            if variant == "split":
                state.place_entity(victim.id, h(1, 1))
                state.active_effects = [
                    effect(
                        state,
                        "rock-split",
                        EffectType.TOPOLOGY_SPLIT,
                        source=str(victim.id),
                        split_axis="r",
                        split_value=1,
                    )
                ]
            if variant in {"isolation-rock", "isolation-enemy"}:
                state.active_effects = [
                    effect(
                        state,
                        "rock-isolation",
                        EffectType.TOPOLOGY_ISOLATION,
                        source="isolation-anchor",
                        isolated_hex=h(1) if variant == "isolation-rock" else h(2),
                        split_axis="q",
                        split_value=0,
                    )
                ]
            if variant == "literal-step-id":
                step.step_id = "native-step-999999"
            if variant == "skip":
                step.skip_if_key = "skip"
            if variant == "active":
                step.active_if_key = "missing"
            steps = [step, step] if variant == "repeat" else [step]
            yield f"rock-ultimate-{variant}-{mode}", state, retain(state, steps, mode)

    from aura_traces import _board
    from model_traces import setup

    for actor_piece in (False, True):
        for mode in ("first", "last", "invalid"):
            state = setup(
                ["Razzle", "Arien"] if actor_piece else ["Mrak", "Arien"],
                ["Brogan", "Hanu"] if actor_piece else ["Razzle", "Brogan"],
            )
            _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_razzle" if actor_piece else "hero_mrak")
            hero.level = 8
            state.current_actor_id = hero.id
            state.execution_context["rock"] = h(1)
            if actor_piece:
                ids = state.get_hero("hero_razzle").piece_supply
                points = [h(0), *[point for point in state.board.tiles if point != h(0)]]
                for i, point in zip(range(1, ids + 1), points[:ids], strict=True):
                    state.place_entity(f"hero_razzle_piece_{i}", point)
                state.current_actor_id = "hero_razzle_piece_1"
                state.acting_piece_id = "hero_razzle_piece_2"
                state.place_entity("hero_brogan", h(2))
            else:
                state.place_entity(hero.id, h(0))
                for i, point in enumerate([h(2), h(1, 1), h(-2)], 1):
                    state.place_entity(f"hero_razzle_piece_{i}", point)
                state.place_entity("hero_brogan", h(2, -1))
            step = OfferRockUltimateStep(step_id="fixture/rock-ult/pieces", rock_hex_keys=["rock"])
            yield f"rock-ultimate-pieces-{actor_piece}-{mode}", state, retain(state, [step], mode)

    for unlocked in (False, True):
        for full in (False, True):
            for mode in ("first", "last", "invalid"):
                state, hero = build("Mrak")
                hero.level = 8 if unlocked else 7
                victim = state.get_hero("hero_brogan")
                state.place_entity(victim.id, h(2))
                card = next(c for c in [*hero.hand, *hero.deck] if c.effect_id == "stone_grip")
                card.state = CardState.UNRESOLVED
                card.is_facedown = False
                hero.current_turn_card = card
                if full:
                    free = [tile.hex for tile in state.board.tiles.values() if not tile.is_occupied]
                    for token, point in zip(
                        state.token_pool[
                            next(kind for kind in state.token_pool if kind.value == "rock")
                        ],
                        free,
                        strict=False,
                    ):
                        state.place_entity(token.id, point)
                body = CardEffectRegistry.get("stone_grip").get_steps(state, hero, card)
                for i, step in enumerate(body):
                    step.step_id = f"fixture/rock-ult/stone-grip/{i}"
                yield f"rock-ultimate-stone-grip-{unlocked}-{full}-{mode}", state, retain(
                    state, body, mode
                )


def export(path):
    from object_execution_traces import reference
    from oracle import verify_sources
    from shared_objects import export_state_with_shared_objects
    from stack_traces import write_corpus_stream

    manifest = verify_sources()

    def cases():
        for name, state, operations in scenes():
            envelope, _ = export_state_with_shared_objects(state)
            yield {
                "name": "object-execution-" + name,
                "envelope": envelope,
                "operations": operations,
                "expected": reference(state, operations),
            }

    return write_corpus_stream(
        path,
        {
            "schema_version": 1,
            "oracle_revision": manifest["oracle_revision"],
            "source_sha256": manifest["source_sha256"],
        },
        cases(),
    )
