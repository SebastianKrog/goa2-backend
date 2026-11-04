"""Manager-created action control and handler ownership across real decisions."""

from __future__ import annotations

import copy


def build(variant="plain"):
    from initiative_traces import build as base

    from goa2.domain.models import CardState
    from goa2.domain.models.effect import DurationType, EffectScope, EffectType, Shape
    from goa2.engine.effect_manager import EffectManager
    from goa2.engine.steps import ScheduleActionControlStep

    selected = variant.removeprefix("target-") if variant.startswith("target-state-") else variant
    if selected not in (
        "piece",
        "piece-offboard",
        "missing-card",
        "missing-target",
        "unknown-target",
        "numeric-target",
        "null-target",
        "false-target",
        "empty-target",
        "no-actor",
        "empty-actor",
        "unknown-actor",
        "self",
        "enemy",
        "custom-key",
        "shared-current",
        "independent-current",
        "facedown",
    ) and not selected.startswith("state-"):
        selected = "plain"
    state, target, card, source_step = base(selected)
    actor = state.get_hero("hero_hanu")
    actor.level = 8
    if variant.startswith("level-"):
        actor.level = int(variant.removeprefix("level-"))
    elif variant == "no-ultimate":
        actor.ultimate_card = None
    elif variant == "dormant-ultimate":
        actor.ultimate_card.is_facedown = True
        actor.ultimate_card.is_active = False
        actor.ultimate_card.state = CardState.RETIRED
    elif variant == "actor-other-hero":
        actor = state.get_hero("hero_brogan")
        actor.level = 8
        state.current_actor_id = actor.id
    elif variant == "actor-piece":
        actor = state.get_hero("hero_razzle")
        actor.level = 8
        piece = state.get_piece_ids(actor.id)[0]
        state.current_actor_id = piece
        state.acting_piece_id = piece
    elif variant == "context-card":
        state.execution_context.update(
            current_card_id=card.id,
            current_action_type="ATTACK",
            performing_card_id="not-a-card",
            performing_card_owner_id="unknown",
        )
    elif variant == "large-counter":
        state.next_entity_id = 2**53 + 1
    step = ScheduleActionControlStep(hero_key=source_step.hero_key)
    if variant == "active-gate":
        step.active_if_key = "missing"
    elif variant == "skip-gate":
        step.skip_if_key = source_step.hero_key
    if variant.startswith("existing-") or variant == "counter-collision":
        old = EffectManager.create_effect(
            state=state,
            source_id="hero_brogan",
            effect_type=EffectType.CONTROL_NEXT_ACTION,
            scope=EffectScope(shape=Shape.POINT, origin_id=target.id),
            duration=DurationType.THIS_ROUND,
            is_active=True,
            controlled_card_id=card.id,
        )
        if variant == "existing-inactive":
            old.is_active = False
        elif variant == "existing-expired":
            old.created_at_round = state.round - 1
        elif variant == "existing-other-card":
            old.controlled_card_id = "not-the-current-card"
        elif variant == "counter-collision":
            old.id = f"eff_e_{state.next_entity_id}"
        state.execution_context["old-effect"] = old
        state.execution_context["old-scope"] = old.scope
    state.execution_context["unresolved-alias"] = state.unresolved_hero_ids
    return state, target, card, step


def operations(state, *, stop_types=()):
    from goa2.domain.input import InputResponse, selection_value
    from goa2.engine.handler import process_stack, submit_input

    probe = copy.deepcopy(state)
    result = []
    for _ in range(200):
        if not probe.execution_stack or probe.execution_stack[-1].type.value in stop_types:
            return result
        left = 1

        def boundary(_state, _step):
            nonlocal left
            if not left:
                return True
            left -= 1
            return False

        output = process_stack(probe, stop_before_step=boundary)
        result.append({"kind": "advance_prefix", "steps": 1})
        if output.input_request:
            request = output.input_request
            ids = [option.id for option in request.options]
            if request.request_type.value == "CHOOSE_ACTION" and "HOLD" in ids:
                selection = "HOLD"
            elif "CONFIRM" in ids:
                selection = "CONFIRM"
            elif request.can_skip:
                selection = "SKIP"
            else:
                selection = selection_value(request.options[0])
            submit_input(probe, InputResponse(request_id=request.id, selection=selection))
            result.append({"kind": "input", "selection": selection})
    raise AssertionError("control scene did not reach its boundary")


def scene(name, state, steps, *, stop_types=()):
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import SetContextFlagStep

    steps = [*steps, SetContextFlagStep(key="after", value=True)]
    for index, step in enumerate(steps):
        step.step_id = f"fixture/action-control/{name}/{index}"
    push_steps(state, steps)
    return "action-control-" + name, state, operations(state, stop_types=stop_types)


def place_heroes(state):
    for team in state.teams.values():
        for hero in team.heroes:
            for unit in state.get_piece_ids(hero.id) or [hero.id]:
                state.remove_entity(unit)
    spaces = [point for point, tile in state.board.tiles.items() if not tile.is_obstacle]
    anchor = spaces[0]
    spaces.sort(key=lambda point: point.distance(anchor))
    units = ["hero_hanu", "hero_arien", "hero_brogan", *state.get_piece_ids("hero_razzle")]
    for unit, point in zip(units, spaces, strict=False):
        state.place_entity(unit, point)


def scenes():
    from goa2.domain.models import CardState, TargetType
    from goa2.engine.filters_hex import ObstacleFilter, RangeFilter
    from goa2.engine.filters_units import TeamFilter, UnitTypeFilter
    from goa2.engine.steps import (
        DiscardCardStep,
        EndPhaseCleanupStep,
        FindNextActorStep,
        ScheduleActionControlStep,
        SelectStep,
        SetActorStep,
        SetContextFlagStep,
    )

    variants = [
        "plain",
        "level-0",
        "level-1",
        "level-7",
        "level-8",
        "level-9",
        "no-ultimate",
        "dormant-ultimate",
        "actor-other-hero",
        "actor-piece",
        "context-card",
        "large-counter",
        "counter-collision",
        "existing-active",
        "existing-inactive",
        "existing-expired",
        "existing-other-card",
        "active-gate",
        "skip-gate",
        "piece",
        "piece-offboard",
        "missing-card",
        "missing-target",
        "unknown-target",
        "numeric-target",
        "null-target",
        "false-target",
        "empty-target",
        "no-actor",
        "empty-actor",
        "unknown-actor",
        "self",
        "enemy",
        "custom-key",
        "shared-current",
        "independent-current",
        "facedown",
        *["target-state-" + value.value for value in CardState],
    ]
    for variant in variants:
        state, _, _, step = build(variant)
        yield scene(variant, state, [step])
    state, _, _, step = build()
    yield scene(
        "repeat-without-card-dedup", state, [step, ScheduleActionControlStep(hero_key="who")]
    )

    for variant in (
        "plain",
        "actor-piece",
        "existing-active",
        "existing-inactive",
        "existing-expired",
        "existing-other-card",
        "counter-collision",
    ):
        state, target, _, step = build(variant)
        place_heroes(state)
        state.unresolved_hero_ids[:] = [target.id]
        yield scene("complete-owned-turn-" + variant, state, [step, FindNextActorStep()])

    for variant in ("owned", "forced-window", "no-owner", "discarded", "resolved-current"):
        state, target, card, step = build()
        place_heroes(state)
        state.resolution_owner_id = (
            None
            if variant == "no-owner"
            else "hero_hanu" if variant == "forced-window" else target.id
        )
        extra = []
        if variant == "discarded":
            extra.append(DiscardCardStep(hero_id=target.id, card_id=card.id, source="PLAYED"))
        # Resolve the card through its actual action menu; control remains through confirmation.
        if variant == "resolved-current":
            state.unresolved_hero_ids[:] = [target.id]
            yield scene("probe-" + variant, state, [step, FindNextActorStep()])
            continue
        extra.extend(
            [
                SetActorStep(actor_id=target.id),
                SetContextFlagStep(key="current_card_id", value="not-the-controlled-card"),
                SelectStep(
                    target_type=TargetType.NUMBER,
                    prompt="Controlled number",
                    number_options=[1, 2],
                    output_key="number",
                ),
                SelectStep(
                    target_type=TargetType.HEX,
                    prompt="Controlled hex",
                    filters=[RangeFilter(max_range=1), ObstacleFilter(is_obstacle=False)],
                    output_key="hex",
                    is_mandatory=False,
                ),
                SelectStep(
                    target_type=TargetType.UNIT,
                    prompt="Controlled target",
                    filters=[
                        UnitTypeFilter(unit_type="HERO"),
                        TeamFilter(relation="ENEMY"),
                        RangeFilter(max_range=4),
                    ],
                    output_key="unit",
                    is_mandatory=False,
                ),
                SelectStep(
                    target_type=TargetType.NUMBER,
                    prompt="Other player's number",
                    override_player_id="hero_brogan",
                    number_options=[1, 2],
                    output_key="other",
                ),
            ]
        )
        yield scene("probe-" + variant, state, [step, *extra])
    state, _, _, step = build()
    yield scene("round-cleanup", state, [step, EndPhaseCleanupStep()])
