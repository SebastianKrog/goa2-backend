"""Source turret creation/reuse, effect-preserving removal and salvage bodies."""

from __future__ import annotations


def scenes():
    from combat_effect_traces import effect
    from live_passive_traces import build
    from object_execution_traces import live_choice_operations

    from goa2.domain.hex import Hex
    from goa2.domain.models import CardState, TargetType, Turret
    from goa2.domain.models.effect import EffectType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.filters import ObstacleFilter
    from goa2.engine.steps import (
        CollectUnitsStep,
        PlaceTurretStep,
        RemoveTurretStep,
        SelectStep,
        SetContextFlagStep,
    )

    tid = "trinkets_turret"

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def base():
        state, hero = build("Trinkets")
        state.remove_entity(tid)
        state.misc_entities.pop(tid, None)
        state.execution_context.update(target_hex=h(1), who="hero_brogan", skip=False)
        return state, hero

    def existing(state):
        turret = Turret(id=tid, name="Custom Turret", owner_id="hero_brogan")
        state.register_entity(turret, "board_entity")
        state.execution_context["turret_alias"] = turret
        return turret

    def retain(state, steps, mode="first"):
        tail = SetContextFlagStep(step_id="fixture/turret/tail", key="tail", value=True)
        state.execution_stack = list(reversed([*steps, tail]))
        state.execution_context.update(
            turret_steps=steps,
            old_effects=state.active_effects,
            locations_alias=state.entity_locations,
        )
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    for variant in (
        "new",
        "dict",
        "terrain",
        "virtual",
        "owner-literal",
        "owner-key",
        "owner-missing",
        "owner-null",
        "owner-empty",
        "empty-owner-key",
        "empty-output",
        "no-actor",
        "missing",
        "null",
        "false",
        "occupied",
        "optional-missing",
        "optional-occupied",
        "skip",
        "active",
        "reuse",
        "same-position",
        "offboard-reuse",
        "reuse-obstacle-false",
        "reuse-no-actor",
        "snapshot",
    ):
        state, hero = base()
        step = PlaceTurretStep(step_id="fixture/turret/place", output_key="placed")
        if variant == "dict":
            state.execution_context["target_hex"] = h(1).model_dump()
        if variant == "terrain":
            state.board.get_tile(h(1)).is_terrain = True
        if variant == "virtual":
            state.execution_context["target_hex"] = h(20)
        if variant == "owner-literal":
            step.owner_id = "hero_hanu"
        if variant in {"owner-key", "owner-missing", "owner-null", "empty-owner-key"}:
            step.owner_id = "hero_hanu"
            step.owner_id_key = "who"
        if variant == "owner-missing":
            state.execution_context.pop("who")
        if variant == "owner-null":
            state.execution_context["who"] = None
        if variant == "owner-empty":
            step.owner_id = ""
        if variant == "empty-owner-key":
            step.owner_id_key = ""
        if variant == "empty-output":
            step.output_key = ""
        if variant == "no-actor":
            state.current_actor_id = None
        if variant in {"missing", "optional-missing"}:
            state.execution_context.pop("target_hex")
        if variant == "null":
            state.execution_context["target_hex"] = None
        if variant == "false":
            state.execution_context["target_hex"] = False
        if variant in {"occupied", "optional-occupied"}:
            state.execution_context["target_hex"] = state.get_position(hero.id)
        if variant.startswith("optional-"):
            step.is_mandatory = False
        if variant == "skip":
            step.skip_if_key = "skip"
        if variant == "active":
            step.active_if_key = "absent"
        if variant in {
            "reuse",
            "same-position",
            "offboard-reuse",
            "reuse-obstacle-false",
            "reuse-no-actor",
            "snapshot",
        }:
            turret = existing(state)
            if variant != "offboard-reuse":
                state.place_entity(tid, h(2))
            if variant == "same-position":
                state.execution_context["target_hex"] = h(2)
            if variant == "reuse-obstacle-false":
                turret.is_obstacle = False
            if variant == "reuse-no-actor":
                state.current_actor_id = None
        if variant == "snapshot":
            state.last_turn_positions = state.entity_locations
        yield "turret-place-" + variant, state, retain(state, [step])

    for variant in (
        "placed",
        "virtual",
        "offboard",
        "missing",
        "optional-offboard",
        "optional-missing",
        "placeholder",
        "optional-placeholder",
        "no-actor",
        "empty-output",
        "snapshot",
        "skip",
        "active",
    ):
        state, _ = base()
        if "missing" not in variant and "placeholder" not in variant:
            existing(state)
            if "offboard" not in variant:
                state.place_entity(tid, h(20) if variant == "virtual" else h(1))
        if "placeholder" in variant:
            from goa2.domain.models.base import Placeholder

            state.register_entity(Placeholder(id=tid, name="Other"), "board_entity")
            state.place_entity(tid, h(1))
        step = RemoveTurretStep(
            step_id="fixture/turret/remove",
            output_key="removed",
            is_mandatory=not variant.startswith("optional-"),
        )
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "empty-output":
            step.output_key = ""
        if variant == "snapshot":
            state.last_turn_positions = state.entity_locations
        if variant == "skip":
            step.skip_if_key = "skip"
        if variant == "active":
            step.active_if_key = "absent"
        yield "turret-remove-" + variant, state, retain(state, [step])

    for variant in ("remove-effects", "move-effects", "reuse-after-remove", "queries"):
        state, hero = base()
        turret = existing(state)
        state.place_entity(tid, h(2))
        state.active_effects = [
            effect(state, "turret-link", EffectType.DELAYED_TRIGGER, source=tid, token_id=tid)
        ]
        steps = []
        if variant in {"remove-effects", "reuse-after-remove", "queries"}:
            steps.append(
                RemoveTurretStep(step_id="fixture/turret/chain/remove", output_key="removed")
            )
        if variant != "remove-effects":
            steps.append(
                PlaceTurretStep(
                    step_id="fixture/turret/chain/place", output_key="placed", owner_id=str(hero.id)
                )
            )
        if variant == "queries":
            steps.extend(
                [
                    CollectUnitsStep(
                        step_id="fixture/turret/query",
                        target_type=TargetType.UNIT_OR_TOKEN,
                        output_key="units",
                        skip_self_filter=True,
                        skip_immunity_filter=True,
                    ),
                    SelectStep(
                        step_id="fixture/turret/empty",
                        target_type=TargetType.HEX,
                        prompt="Empty",
                        output_key="empty",
                        filters=[ObstacleFilter()],
                    ),
                ]
            )
        yield "turret-" + variant, state, retain(state, steps)

    for choice in (1, 2, 3):
        for placed in (False, True):
            for mode in ("first", "last", "invalid"):
                state, hero = base()
                if placed:
                    existing(state)
                    state.place_entity(tid, h(2))
                card = next(
                    card for card in [*hero.hand, *hero.deck] if card.effect_id == "salvage_parts"
                )
                hero.current_turn_card = card
                card.state = CardState.UNRESOLVED
                card.is_facedown = False
                discarded = hero.hand.pop(0)
                discarded.state = CardState.DISCARD
                discarded.is_facedown = False
                hero.discard_pile = [discarded]
                body = CardEffectRegistry.get("salvage_parts").get_steps(state, hero, card)
                body[0].pending_input = {"selection": choice}
                for i, step in enumerate(body):
                    step.step_id = f"fixture/turret/salvage/{i}"
                yield f"turret-salvage-{choice}-{placed}-{mode}", state, retain(state, body, mode)


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
