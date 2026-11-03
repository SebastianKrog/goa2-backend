"""Real source stack execution with shared steps and context object identity."""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

from oracle import ROOT, Worker, verify_sources
from shared_objects import export_state_with_shared_objects, instance_bindings
from stack_traces import TraceNormalizer, read_case, read_index, write_corpus_stream


class ObjectTraceNormalizer(TraceNormalizer):
    def __init__(self, unordered_keys=()):
        super().__init__()
        self.unordered_keys = tuple(unordered_keys)

    def bind_originals(self, bindings, objects):
        self.original_step_ids = {
            obj.step_id
            for binding, obj in zip(bindings, objects, strict=True)
            if binding["kind"] == "step"
        }

    def record(self, record, live_steps=None):
        record = copy.deepcopy(record)
        generated_rows = record.pop("generated_step_objects", None)
        generated_objects = dict(generated_rows) if generated_rows is not None else None
        if generated_objects is not None and (
            len(generated_objects) != len(generated_rows)
            or len(set(generated_objects.values())) != len(generated_objects)
            or any(
                index < 0
                or index >= len(record["object_bindings"])
                or record["object_bindings"][index]["kind"] != "step"
                for index in generated_objects
            )
        ):
            raise ValueError("invalid native generated-object identity metadata")
        if record["request"] is not None:
            record["request"]["id"] = self.request_id(record["request"]["id"])
        # Only typed step bindings carry request IDs. Do not rewrite matching
        # user strings, or collapse independently owned equal textual step IDs.
        for index, binding in enumerate(record["object_bindings"]):
            if binding["kind"] != "step":
                continue
            generated = None
            if live_steps is not None:
                live = live_steps[index]
                if live.step_id not in self.original_step_ids:
                    identity = id(live)
                    if identity not in self.python_steps:
                        self.python_steps[identity] = f"dynamic-step/{len(self.python_steps)+1}"
                        self.retained_steps.append(live)
                    generated = self.python_steps[identity]
            for path in binding["paths"]:
                value = record["semantic_state"]
                for part in path:
                    value = value[part]
                if generated is not None:
                    value["step_id"] = generated
                elif live_steps is None and (
                    index in generated_objects
                    if generated_objects is not None
                    else str(value.get("step_id", "")).startswith("native-step-")
                ):
                    step_id = (
                        generated_objects[index]
                        if generated_objects is not None
                        else value["step_id"]
                    )
                    if step_id not in self.steps:
                        self.steps[step_id] = f"dynamic-step/{len(self.steps)+1}"
                    value["step_id"] = self.steps[step_id]
                value["pending_request_id"] = self.request_id(value["pending_request_id"])
        if self.unordered_keys:
            from mine_traces import normalize_choice_record

            normalize_choice_record(record, self.unordered_keys)
        return record


def scenes():
    from combat_effect_traces import effect
    from model_traces import setup

    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase, TargetType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import SelectStep
    from goa2.engine.steps.utility import (
        AddContextValueStep,
        RecordTargetStep,
        SetContextFlagStep,
    )

    def base(red=None):
        state = setup(red=red)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_min" if red else "hero_arien"
        return state

    for independent in (False, True):
        state = base()
        left = SelectStep(
            step_id="fixture/objects/number",
            target_type=TargetType.NUMBER,
            number_options=[1, 2],
            prompt="Shared selection",
            output_key="chosen",
        )
        right = left.model_copy(deep=True) if independent else left
        first = effect(state, "first", EffectType.MOVEMENT_ZONE, source="hero_arien", max_value=1)
        second = effect(state, "second", EffectType.MOVEMENT_ZONE, source="hero_arien", max_value=2)
        first.finishing_steps, second.finishing_steps = [left], [right]
        state.active_effects = [first, second]
        state.execution_stack = [left, right]
        state.execution_context = {"left": left, "right": right}
        yield ("independent" if independent else "shared") + "-pending-step", state, [
            {"kind": "advance"},
            {"kind": "input", "request_id": "stale", "selection": 1},
            {"kind": "input", "selection": {"bad": [1]}},
            {"kind": "advance"},
            {"kind": "advance"},
            {"kind": "input", "selection": "2"},
            {"kind": "advance"},
            {"kind": "input", "selection": 1},
            {"kind": "advance"},
        ]
    for variant in ("auto", "skip", "empty-optional", "empty-mandatory", "condition"):
        state = base()
        select = SelectStep(
            step_id="fixture/objects/" + variant,
            target_type=TargetType.NUMBER,
            number_options=(
                [2] if variant == "auto" else ([] if variant.startswith("empty") else [1, 2])
            ),
            prompt="Stored selection",
            output_key="chosen",
            auto_select_if_one=variant == "auto",
            is_mandatory=variant not in ("skip", "empty-optional"),
        )
        select.pending_input = {"selection": "SKIP" if variant == "skip" else 2}
        select.pending_request_id = "fixture/previous-request"
        if variant == "condition":
            select.skip_if_key = "disabled"
        trailing = SetContextFlagStep(step_id="fixture/objects/trailing", key="after", value=True)
        state.execution_stack = [trailing, select]
        state.execution_context = {"stored": select, "chosen": 9, "disabled": False}
        yield variant + "-pending-step", state, [{"kind": "advance"}]
    for variant in ("list", "tuple", "dict", "empty", "model"):
        state = base()
        box = {"nested": [8]}
        history = (
            []
            if variant in ("list", "empty", "model")
            else ((9,) if variant == "tuple" else {"existing": 1})
        )
        target = Hex(q=0, r=0, s=0) if variant == "model" else ([] if variant == "empty" else box)
        setter = SetContextFlagStep(step_id="fixture/objects/set", key="target", value=True)
        # Share the actual post-construction payload, after GameStep isolation.
        setter.value = target
        record = RecordTargetStep(
            step_id="fixture/objects/record", input_key="target", output_list_key="history"
        )
        add = AddContextValueStep(step_id="fixture/objects/add", key="scalar", amount=2)
        state.execution_stack = [add, record, record, setter]
        state.execution_context = {
            "history": history,
            "mirror": history,
            "target": target,
            "old": {"history": history, "scalar": 1},
            "scalar": 1,
        }
        yield "context-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]
    state = base()
    setter = SetContextFlagStep(step_id="fixture/objects/live-set", key="result", value=[1])
    record = RecordTargetStep(
        step_id="fixture/objects/live-record", input_key="target", output_list_key="history"
    )
    state.execution_stack = [setter, record]
    state.execution_context = {"target": 2, "history": setter.value}
    yield "live-set-payload", state, [
        {"kind": "advance_prefix", "steps": 1},
        {"kind": "advance"},
    ]
    state = base()
    record = RecordTargetStep(
        step_id="fixture/objects/new-record", input_key="target", output_list_key="history"
    )
    state.execution_stack = [record, record]
    state.execution_context = {"target": {"nested": [1]}}
    yield "new-shared-list", state, [
        {"kind": "advance_prefix", "steps": 1},
        {"kind": "advance"},
    ]
    for value in (True, 1.5, "not a number", [4]):
        state = base()
        state.execution_stack = [
            AddContextValueStep(step_id="fixture/objects/add", key="scalar", amount=-2)
        ]
        state.execution_context = {"scalar": value}
        yield "add-" + type(value).__name__, state, [{"kind": "advance"}]

    yield from loop_scenes(base)
    yield from scope_scenes(base)
    yield from effect_lifecycle_scenes(base)
    yield from scheduled_effect_scenes(base)
    yield from turn_boundary_scenes(base)
    yield from revelation_scenes(base)
    yield from planning_command_scenes(base)
    yield from card_planning_scenes(base)
    yield from confirmation_scenes(base)
    yield from finalization_scenes(base)
    yield from piece_planning_scenes(base)
    yield from minion_return_scenes(base)
    yield from placement_scenes(base)
    yield from movement_scenes(base)
    yield from live_mine_scenes(base)
    yield from forced_discard_scenes(base)
    yield from card_selection_scenes(base)
    yield from discard_scenes(base)
    from live_passive_traces import scenes as passive_scenes

    yield from passive_scenes()
    from live_passive_body_traces import scenes as passive_body_scenes

    yield from passive_body_scenes()
    from live_rune_traces import scenes as rune_scenes

    yield from rune_scenes()
    from live_spatial_traces import scenes as spatial_scenes

    yield from spatial_scenes()
    from live_unit_traces import scenes as unit_scenes

    yield from unit_scenes()
    from live_query_traces import scenes as query_scenes

    yield from query_scenes()
    from live_retrieval_traces import scenes as retrieval_scenes

    yield from retrieval_scenes()
    from live_token_traces import scenes as token_scenes

    yield from token_scenes()
    from live_token_path_traces import scenes as token_path_scenes

    yield from token_path_scenes()
    from live_token_batch_traces import scenes as token_batch_scenes

    yield from token_batch_scenes()
    from live_rock_ultimate_traces import scenes as rock_ultimate_scenes

    yield from rock_ultimate_scenes()
    from live_turret_traces import scenes as turret_scenes

    yield from turret_scenes()
    from live_multi_selection_traces import scenes as multi_selection_scenes

    yield from multi_selection_scenes()
    from live_defeat_traces import scenes as defeat_scenes

    yield from defeat_scenes()
    from live_combat_resolution_traces import scenes as combat_resolution_scenes

    yield from combat_resolution_scenes()

    from live_attack_sequence_traces import scenes as attack_sequence_scenes

    yield from attack_sequence_scenes()

    from live_reaction_traces import scenes as reaction_scenes

    yield from reaction_scenes()

    from live_pre_action_traces import scenes as pre_action_scenes

    yield from pre_action_scenes()
    from live_swap_traces import scenes as swap_scenes

    yield from swap_scenes()
    from live_marker_traces import scenes as marker_scenes

    yield from marker_scenes()
    from live_effect_creation_traces import scenes as effect_creation_scenes

    yield from effect_creation_scenes()
    from live_defense_body_traces import scenes as defense_body_scenes

    yield from defense_body_scenes()
    from live_resource_traces import scenes as resource_scenes

    yield from resource_scenes()
    from live_primary_body_traces import scenes as primary_body_scenes

    yield from primary_body_scenes()
    from live_utility_traces import scenes as utility_scenes

    yield from utility_scenes()
    from live_push_traces import scenes as push_scenes

    yield from push_scenes()
    from live_card_action_traces import scenes as card_action_scenes

    yield from card_action_scenes()
    from live_travel_traces import scenes as travel_scenes

    yield from travel_scenes()
    from live_perform_primary_traces import scenes as perform_primary_scenes

    yield from perform_primary_scenes()
    from live_arien_primary_traces import scenes as arien_primary_scenes

    yield from arien_primary_scenes()
    from live_perform_card_traces import scenes as perform_card_scenes

    yield from perform_card_scenes()
    from live_mind_grip_traces import scenes as mind_grip_scenes

    yield from mind_grip_scenes()
    from live_tigerclaw_primary_traces import scenes as tigerclaw_primary_scenes

    yield from tigerclaw_primary_scenes()
    from live_brogan_primary_traces import (
        protection_scenes as brogan_protection_scenes,
    )
    from live_brogan_primary_traces import (
        query_scenes as brogan_query_scenes,
    )
    from live_brogan_primary_traces import (
        scenes as brogan_primary_scenes,
    )

    yield from brogan_primary_scenes()
    yield from brogan_query_scenes()
    yield from brogan_protection_scenes()
    from live_spellbook_traces import scenes as spellbook_scenes

    yield from spellbook_scenes()
    from live_spell_body_traces import (
        extra_scenes as spell_body_extra_scenes,
    )
    from live_spell_body_traces import (
        scenes as spell_body_scenes,
    )
    from live_spell_primitive_traces import scenes as spell_primitive_scenes

    yield from spell_body_scenes()
    yield from spell_body_extra_scenes()
    yield from spell_primitive_scenes()
    from live_gydion_primary_traces import scenes as gydion_primary_scenes

    yield from gydion_primary_scenes()
    from live_misa_geometry_traces import scenes as misa_geometry_scenes
    from live_misa_primary_traces import scenes as misa_primary_scenes

    yield from misa_primary_scenes()
    yield from misa_geometry_scenes()
    from live_wasp_primary_traces import (
        query_scenes as wasp_query_scenes,
    )
    from live_wasp_primary_traces import (
        scenes as wasp_primary_scenes,
    )

    yield from wasp_primary_scenes()
    yield from wasp_query_scenes()
    from live_min_primary_traces import (
        after_scenes as min_after_scenes,
    )
    from live_min_primary_traces import (
        item_scenes as min_item_scenes,
    )
    from live_min_primary_traces import (
        scenes as min_primary_scenes,
    )

    yield from min_primary_scenes()
    yield from min_item_scenes()
    yield from min_after_scenes()
    from live_dynamic_passive_traces import scenes as dynamic_passive_scenes

    yield from dynamic_passive_scenes()
    from live_battle_traces import scenes as live_battle_scenes

    yield from live_battle_scenes()
    from live_cleanup_traces import scenes as live_cleanup_scenes

    yield from live_cleanup_scenes()


def live_choice_operations(state, mode="first", stop_types=("offer_passive",), *, max_steps=80):
    """Drive real one-step boundaries up to the next unbound consumer."""
    from goa2.domain.input import InputResponse, selection_value
    from goa2.engine.handler import process_stack, submit_input

    probe = copy.deepcopy(state)
    result = []
    bad = False
    for _ in range(max_steps):
        if not probe.execution_stack or probe.execution_stack[-1].type.value in stop_types:
            return result
        remaining = 1

        def boundary(_state, _step):
            nonlocal remaining
            if not remaining:
                return True
            remaining -= 1
            return False

        output = process_stack(probe, stop_before_step=boundary)
        result.append({"kind": "advance_prefix", "steps": 1})
        if output.input_request:
            request = output.input_request
            if mode in ("invalid", "numeric") and not bad:
                selection = "invalid" if mode == "invalid" else 0
                bad = True
            else:
                option = request.options[-1] if mode == "last" else request.options[0]
                selection = (
                    option.id
                    if request.prompt == "Choose which mines to move through"
                    else selection_value(option)
                )
            submit_input(probe, InputResponse(request_id=request.id, selection=selection))
            result.append({"kind": "input", "selection": selection})
    raise AssertionError("live choice scene did not reach its boundary")


def discard_scenes(base):
    """Primitive boundaries use registered cards and the actual source discard."""
    from combat_effect_traces import effect

    from goa2.domain.models import CardContainerType, CardState
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps.cards import DiscardCardStep
    from goa2.engine.steps.utility import SetContextFlagStep

    variants = (
        "hand",
        "played",
        "current",
        "fallback-played",
        "fallback-hand",
        "deck-fallback",
        "missing-hero",
        "no-hero",
        "no-card",
        "missing-card",
        "context",
        "literal-wins",
        "equal-current",
        "revision-current",
        "metadata-current",
        "metadata-order-current",
        "equal-earlier-played",
        "shared-hand-discard",
        "cross-hero-hand",
        "shared-discard",
        "effects",
        "effects-global-first",
        "log-existing",
        "log-shared",
        "skipped",
    )
    for variant in variants:
        state = base()
        hero = state.get_hero("hero_arien")
        other = state.get_hero("hero_hanu")
        card = hero.hand[0]
        card.is_active = True
        step = DiscardCardStep(
            step_id="fixture/discard/" + variant, hero_id=str(hero.id), card_id=card.id
        )
        if variant in {
            "played",
            "fallback-played",
            "equal-current",
            "revision-current",
            "metadata-current",
            "metadata-order-current",
            "equal-earlier-played",
        }:
            hero.hand.remove(card)
            card.state = CardState.RESOLVED
            card.is_facedown = False
            hero.played_cards = [card]
            if variant != "fallback-played":
                step.source = CardContainerType.PLAYED
        if variant == "current":
            hero.hand.remove(card)
            card.state = CardState.UNRESOLVED
            card.initiative_override = 15
            hero.current_turn_card = card
            step.source = CardContainerType.PLAYED
        if variant == "fallback-hand":
            step.source = CardContainerType.PLAYED
        if variant == "deck-fallback":
            step.source = CardContainerType.DECK
        if variant == "missing-hero":
            step.hero_id = "hero_missing"
        if variant == "no-hero":
            step.hero_id = None
        if variant == "no-card":
            step.card_id = None
        if variant == "missing-card":
            step.card_id = "card_missing"
        if variant == "context":
            step.hero_id = step.card_id = None
            step.hero_key, step.card_key = "who", "which"
            state.execution_context.update(who=str(hero.id), which=card.id)
        if variant == "literal-wins":
            step.hero_key, step.card_key = "who", "which"
            state.execution_context.update(who="hero_missing", which="card_missing")
        if variant in {
            "equal-current",
            "revision-current",
            "metadata-current",
            "metadata-order-current",
        }:
            clone = card.model_copy(deep=True)
            hero.current_turn_card = clone
            if variant == "revision-current":
                clone._definition_revision += 1
            if variant == "metadata-current":
                clone.metadata = {"different": True}
            if variant == "metadata-order-current":
                card.metadata = {"a": 1, "b": [2]}
                clone.metadata = {"b": [2.0], "a": True}
            state.execution_context["current_alias"] = clone
        if variant == "equal-earlier-played":
            clone = card.model_copy(deep=True)
            hero.played_cards = [None, clone, card]
            state.execution_context["later_alias"] = card
        if variant == "shared-hand-discard":
            hero.discard_pile = hero.hand
        if variant == "cross-hero-hand":
            other.hand = hero.hand
        if variant == "shared-discard":
            other.discard_pile = hero.discard_pile
        if variant in {"effects", "effects-global-first"}:
            unrelated = hero.hand[1]
            rows = [
                effect(state, "same-id", EffectType.DELAYED_TRIGGER, source_card_id=c.id)
                for c in (card, unrelated, card)
            ]
            for row in rows:
                row.finishing_steps = [
                    SetContextFlagStep(
                        step_id="fixture/discard/not-run", key="finishing_ran", value=True
                    )
                ]
            state.active_effects = rows
            if variant == "effects-global-first":
                clone = card.model_copy(deep=True)
                hero.current_turn_card = clone
                state.execution_context["global_first_alias"] = clone
        if variant in {"log-existing", "log-shared"}:
            state.record_public_revealed_card(hero.id, card.id)
            state.turn_discard_log[hero.id] = [card.id]
            state.execution_context["log_alias"] = state.turn_discard_log[hero.id]
            if variant == "log-shared":
                state.turn_discard_log[other.id] = state.turn_discard_log[hero.id]
        if variant == "skipped":
            step.skip_if_key = "skip"
            state.execution_context["skip"] = True
        state.execution_context.update(
            card_alias=card,
            hand_alias=hero.hand,
            played_alias=hero.played_cards,
            discard_alias=hero.discard_pile,
            old_effects=state.active_effects,
            step_alias=step,
        )
        state.execution_stack = [step]
        yield "discard-" + variant, state, [{"kind": "advance_prefix", "steps": 1}]


def card_selection_scenes(base):
    """Reuse the complete registered-roster selector matrix with live aliases."""
    from card_selection_traces import cases

    collected = []

    def capture(name, state, operations):
        if not all(
            step.type.value == "set_context_flag"
            or (step.type.value == "select_step" and step.target_type.value == "CARD")
            for step in state.execution_stack
        ):
            return
        step = next(step for step in state.execution_stack if step.type.value == "select_step")
        state.execution_context["selector"] = step
        state.execution_context["pending_alias"] = step.pending_input
        state.execution_context["locations_alias"] = state.entity_locations
        collected.append(("card-selector-" + name, state, operations))

    def answer(selection, token="current"):
        return {"kind": "input", "request_id": token, "selection": selection}

    cases({"kind": "advance"}, answer, emit=capture)
    yield from collected


def forced_discard_scenes(base):
    """Source forced-discard expansion, before its card consumer executes."""
    from discard_effect_traces import state_for

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import ForceDiscardStep, SetContextFlagStep

    variants = [
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
        "key",
        "empty-literal",
        "no-victim",
        "unknown-victim",
        "repeated",
        "skipped",
        "dirty-pending",
    ]
    for variant in variants:
        state, hero, _shield = state_for(variant)
        # Original fixture steps need stable IDs: Python's default id(object())
        # can collide with a later generated step and defeat ID normalization.
        original_finishers = {}
        for active in state.active_effects:
            for finisher in active.finishing_steps:
                identity = id(finisher)
                if identity not in original_finishers:
                    original_finishers[identity] = (
                        f"fixture/live-discard/finishing/{len(original_finishers)}"
                    )
                finisher.step_id = original_finishers[identity]
        options = {"victim_id": "hero_mrak", "immunity_source_id": "hero_min"}
        if variant in ("basic", "no-basic"):
            options["card_is_basic"] = True
        if variant == "non-basic":
            options["card_is_basic"] = False
        if variant == "friendly-immunity":
            options["immunity_source_id"] = "hero_arien"
        if variant == "no-immunity-source":
            options["immunity_source_id"] = None
        if variant in ("key", "empty-literal", "no-victim"):
            options.update(
                victim_id="" if variant == "empty-literal" else None, victim_key="victim"
            )
            if variant == "no-victim":
                state.execution_context.pop("victim", None)
        if variant == "unknown-victim":
            options["victim_id"] = "unknown"
        step = ForceDiscardStep(**options, step_id="fixture/live-discard/main")
        if variant == "skipped":
            step.active_if_key = "absent"
        if variant == "dirty-pending":
            step.pending_input = {"selection": "ignored"}
            step.pending_request_id = "ignored-discard-request"
        state.execution_context.update(
            discard_step=step,
            hand_alias=hero.hand,
            played_alias=hero.played_cards,
            effects=state.active_effects,
        )
        state.execution_stack = []
        push_steps(
            state,
            [
                step,
                SetContextFlagStep(key="after", value=True, step_id="fixture/live-discard/after"),
            ],
        )
        if variant == "repeated":
            state.execution_stack = [step, step]
        yield "forced-discard-" + variant, state, live_choice_operations(state)


def live_mine_scenes(base):
    """Real mine choice/requeue/removal; blast discard is a staged consumer."""
    from mine_traces import states

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        MinePathChoiceStep,
        MoveUnitStep,
        SetContextFlagStep,
        TriggerMineStep,
    )

    for variant, original, h, _pieces, _mines in states():
        for mode in ("first", "last", "invalid", "numeric"):
            state = copy.deepcopy(original)
            step = MinePathChoiceStep(
                unit_id="hero_arien", range_val=4, step_id="fixture/live-mines/choice"
            )
            state.execution_context = {
                "target_hex": h(1, 0),
                "triggered_mine_ids": ["stale"],
                "choice_step": step,
            }
            state.execution_stack = []
            push_steps(
                state,
                [
                    step,
                    SetContextFlagStep(key="after", value=True, step_id="fixture/live-mines/after"),
                ],
            )
            yield "mines-choice-" + variant + "-" + mode, state, live_choice_operations(state, mode)

    for variant in (
        "unit-key",
        "no-target",
        "offboard",
        "custom-output",
        "pending-valid",
        "pending-invalid",
        "pending-empty",
        "pending-false",
        "skipped",
        "no-unit",
    ):
        _, state, h, _pieces, _mines = next(row for row in states() if row[0] == "one-dud")
        step = MinePathChoiceStep(
            unit_id="hero_arien", range_val=3, step_id="fixture/live-mines/alias"
        )
        state.execution_context = {
            "target_hex": h(0, 0),
            "triggered_mine_ids": ["stale"],
            "choice_step": step,
        }
        if variant == "unit-key":
            step.unit_id, step.unit_key = None, "unit"
            state.execution_context["unit"] = "hero_arien"
        if variant == "no-target":
            state.execution_context.pop("target_hex")
        if variant == "offboard":
            state.remove_entity("hero_arien")
        if variant == "custom-output":
            step.output_key = "chosen"
        if variant in ("pending-valid", "pending-invalid"):
            step.pending_input = {
                "selection": "1" if variant == "pending-valid" else 0,
                "retained": True,
            }
        if variant == "pending-empty":
            step.pending_input = {}
        if variant == "pending-false":
            step.pending_input = False
        if variant == "skipped":
            step.active_if_key = "absent"
        if variant == "no-unit":
            step.unit_id, state.current_actor_id = None, None
        state.execution_context["pending_alias"] = step.pending_input
        state.execution_stack = []
        push_steps(state, [step])
        yield "mines-choice-alias-" + variant, state, live_choice_operations(state)

    from mine_movement_traces import setup

    for blast in (False, True):
        for corridor in (False, True):
            for mode in ("first", "last", "invalid", "numeric"):
                state, h, _ = setup(blast, corridor)
                step = MoveUnitStep(
                    unit_id="hero_arien",
                    range_val=3,
                    success_output_key="success",
                    step_id="fixture/live-mines/move",
                )
                state.execution_context = {
                    "target_hex": h(0, 0),
                    "success": "old",
                    "move_step": step,
                    "locations": state.entity_locations,
                    "effects": state.active_effects,
                }
                state.execution_stack = []
                push_steps(
                    state,
                    [
                        step,
                        SetContextFlagStep(
                            key="after", value=True, step_id="fixture/live-mines/after"
                        ),
                    ],
                )
                yield f"mines-movement-{'blast' if blast else 'dud'}-{'corridor' if corridor else 'choice'}-{mode}", state, live_choice_operations(
                    state, mode
                )

    for variant in (
        "ordered",
        "duplicates",
        "missing",
        "empty-literal",
        "context",
        "custom",
        "snapshot-alias",
        "literal-alias",
        "offboard",
        "linked-effects",
        "mutate-old-effects",
    ):
        _, state, h, _pieces, _mines = next(row for row in states() if row[0] == "mixed")
        queue = ["mine_b", "mine_a", "missing", "mine_c"]
        step = TriggerMineStep(
            mine_ids=queue, victim_id="hero_arien", step_id="fixture/live-mines/trigger"
        )
        if variant == "duplicates":
            step.mine_ids = ["mine_a", "mine_a", "mine_b", "mine_b"]
        if variant == "missing":
            step.mine_ids = ["missing"]
        if variant == "empty-literal":
            step.mine_ids = []
        if variant == "context":
            step.mine_ids = None
        if variant == "custom":
            step.mine_ids_key, step.victim_key, step.victim_id, step.mine_ids = (
                "queued",
                "victim",
                None,
                None,
            )
        if variant == "snapshot-alias":
            state.last_turn_positions = state.entity_locations
        if variant == "offboard":
            state.remove_entity("mine_b")
        if variant in ("linked-effects", "mutate-old-effects"):
            from goa2.domain.models.effect import (
                ActiveEffect,
                DurationType,
                EffectScope,
                EffectType,
                Shape,
            )

            step.mine_ids = ["mine_a"]
            for name, source, token, origin in [
                ("duplicate", "hero_min", "mine_a", None),
                ("source", "mine_a", None, None),
                ("origin", "hero_min", None, "mine_a"),
                ("duplicate", "hero_min", None, None),
            ]:
                state.active_effects.append(
                    ActiveEffect(
                        id=name,
                        source_id=source,
                        token_id=token,
                        effect_type=EffectType.MOVEMENT_ZONE,
                        max_value=1,
                        scope=EffectScope(
                            shape=Shape.POINT if origin else Shape.GLOBAL, origin_id=origin
                        ),
                        duration=DurationType.PASSIVE,
                        created_at_turn=state.turn,
                        created_at_round=state.round,
                    )
                )
        state.execution_context = {
            "triggered_mine_ids": ["mine_a"],
            "mine_victim_id": "hero_arien",
            "queued": ["mine_c"],
            "victim": "hero_arien",
            "trigger_step": step,
            "queue_alias": step.mine_ids,
            "effects": state.active_effects,
            "locations": state.entity_locations,
            "old_hex": state.get_position("mine_a"),
            "tile": state.board.get_tile(h(-1, 0)),
        }
        state.execution_stack = []
        steps = [step]
        if variant == "mutate-old-effects":
            from goa2.engine.steps.utility import RecordTargetStep

            state.execution_context["detached_value"] = {"detached": True}
            steps.append(
                RecordTargetStep(
                    input_key="detached_value",
                    output_list_key="effects",
                    step_id="fixture/live-mines/detached",
                )
            )
        push_steps(state, steps)
        yield "mines-trigger-" + variant, state, live_choice_operations(state)


def movement_scenes(base):
    """Live walking against actual displacement effects and output lifecycles."""
    from displacement_effect_traces import states

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import MoveUnitStep, SetContextFlagStep

    for variant, state, h, pieces, heavy in states():
        step = MoveUnitStep(
            unit_id="hero_brogan",
            target_hex_arg=h(2, -1),
            range_val=3,
            success_output_key="moved",
            crossed_obstacles_output_key="crossed",
            is_mandatory=variant == "mandatory",
            step_id="fixture/live-walk/main",
        )
        state.execution_context["moved"] = "stale"
        state.execution_context["crossed"] = ["stale"]
        if variant == "same-place":
            step.target_hex_arg = h(1, 0)
        if variant == "keys":
            step.unit_id, step.unit_key, step.target_hex_arg = None, "unit", None
            state.execution_context.update(unit="hero_brogan", target_hex=h(2, -1).model_dump())
        if variant == "context-skip":
            step.target_hex_arg = None
            state.execution_context.pop("target_hex", None)
        if variant == "gate-off":
            step.active_if_key = "absent"
        if variant.startswith("target-"):
            step.unit_id = {
                "target-token": "restriction-token",
                "target-token-empty-actor": "restriction-token",
                "target-piece": pieces[0],
                "target-heavy": str(heavy.id),
                "target-owner": "hero_razzle",
                "target-unknown": "unknown",
            }[variant]
        state.execution_context["walk_step"] = step
        state.execution_context["locations"] = state.entity_locations
        state.execution_context["snapshot"] = state.last_turn_positions
        state.execution_stack = []
        push_steps(
            state,
            [
                step,
                SetContextFlagStep(key="after", value=True, step_id="fixture/live-walk/after"),
            ],
        )
        yield "walking-effect-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]

    for variant in (
        "success-active",
        "success-skip",
        "success-false",
        "negative-range",
        "straight",
        "through",
        "dict",
        "same-place",
        "same-place-negative",
        "snapshot-alias",
        "prechosen-empty",
    ):
        state = base()
        start = state.get_position("hero_arien")
        target = next(
            hex
            for hex in start.neighbors()
            if state.board.get_tile(hex)
            and not state.board.get_tile(hex).occupant_id
            and not state.board.get_tile(hex).is_obstacle
        )
        step = MoveUnitStep(
            unit_id="hero_arien",
            target_hex_arg=target,
            range_val=1,
            success_output_key="moved",
            crossed_obstacles_output_key="crossed",
            step_id="fixture/live-walk/main",
        )
        state.execution_context.update(moved=True, crossed=["old"])
        if variant == "success-active":
            step.active_if_key = "moved"
        if variant == "success-skip":
            step.skip_if_key = "moved"
        if variant == "success-false":
            state.execution_context["moved"] = False
        if variant in ("negative-range", "same-place-negative"):
            step.range_val = -1
        if variant == "straight":
            step.force_straight_line = True
        if variant == "through":
            step.pass_through_obstacles = True
        if variant == "dict":
            step.target_hex_arg = None
            state.execution_context["target_hex"] = target.model_dump()
        if variant.startswith("same-place"):
            step.target_hex_arg = start
        if variant == "snapshot-alias":
            state.last_turn_positions = state.entity_locations
        if variant == "prechosen-empty":
            step.mine_path_prechosen = True
            state.execution_context["triggered_mine_ids"] = []
        state.execution_context.update(
            walk_step=step,
            locations=state.entity_locations,
            old_hex=start,
            old_tile=state.board.get_tile(start),
            target_tile=state.board.get_tile(step.target_hex_arg or target),
        )
        state.execution_stack = []
        push_steps(state, [step])
        yield "walking-alias-" + variant, state, [{"kind": "advance"}]

    for name, state, operations in minion_return_scenes(base):
        if name == "minion-return-walk":
            yield "walking-minion-return-complete", state, [*operations, {"kind": "advance"}]


def placement_scenes(base):
    """Real placement rules plus retained location, tile and target identities."""
    from displacement_effect_traces import states

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import PlaceUnitStep, SetActorStep, SetContextFlagStep

    def retain(state, step, target):
        state.execution_context["placement_step"] = step
        state.execution_context["locations"] = state.entity_locations
        old = state.get_position(step.unit_id) if step.unit_id else None
        if old is not None:
            state.execution_context["old_hex"] = old
            state.execution_context["old_tile"] = state.board.get_tile(old)
        state.execution_context["target_tile"] = state.board.get_tile(target)

    for variant, state, h, pieces, heavy in states():
        step = PlaceUnitStep(
            unit_id="hero_brogan",
            target_hex_arg=h(2, -1),
            is_mandatory=variant == "mandatory",
            step_id="fixture/live-placement/main",
        )
        if variant == "same-place":
            step.target_hex_arg = h(1, 0)
        if variant == "keys":
            step.unit_id, step.unit_key, step.target_hex_arg = None, "unit", None
            state.execution_context = {
                "unit": "hero_brogan",
                "target_hex": h(2, -1).model_dump(),
            }
        if variant == "context-skip":
            step.target_hex_arg = None
            state.execution_context = {"target_hex": "SKIP"}
        if variant == "gate-off":
            step.active_if_key = "absent"
        if variant.startswith("target-"):
            step.unit_id = {
                "target-token": "restriction-token",
                "target-token-empty-actor": "restriction-token",
                "target-piece": pieces[0],
                "target-heavy": str(heavy.id),
                "target-owner": "hero_razzle",
                "target-unknown": "unknown",
            }[variant]
        retain(state, step, h(2, -1))
        steps = [
            step,
            SetContextFlagStep(key="after", value=True, step_id="fixture/live-placement/after"),
        ]
        if variant in ("live-origin", "live-piece-origin"):
            steps = [
                PlaceUnitStep(
                    unit_id="hero_arien" if variant == "live-origin" else pieces[0],
                    target_hex_arg=h(-3, 0),
                    step_id="fixture/live-placement/origin",
                ),
                SetActorStep(actor_id="hero_brogan", step_id="fixture/live-placement/actor"),
                *steps,
            ]
        state.execution_stack = []
        push_steps(state, steps)
        yield "placement-effect-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]

    variants = [
        "dict-target",
        "immutable-context",
        "board-hex",
        "snapshot-alias",
        "occupied",
        "occupied-mandatory",
        "self-mandatory",
        "no-unit",
        "fallback-unit",
        "empty-unit-key",
        "empty-key-value",
        "offboard-unit",
        "off-map",
        "off-map-collision",
        "terrain",
        "dirty-pending",
        "pending-false",
        "repeated",
        "argument-priority",
        "empty-destination-key",
        "skipped-malformed",
        "live-destination",
        "old-snapshot",
        "foreign-abort",
        "same-abort",
        "dict-coercion",
        "dict-extra-alias",
        "false-target",
        "empty-target",
    ]
    for variant in variants:
        _, state, h, _, _ = next(states())
        state.active_effects = []
        target = h(2, -1)
        step = PlaceUnitStep(
            unit_id="hero_brogan",
            target_hex_arg=target,
            is_mandatory=False,
            step_id="fixture/live-placement/" + variant,
        )
        state.execution_context = {"kept": [True]}
        before = []
        after = [
            SetContextFlagStep(key="after", value=True, step_id="fixture/live-placement/after")
        ]
        if variant in (
            "dict-target",
            "immutable-context",
            "empty-destination-key",
            "live-destination",
        ):
            step.target_hex_arg = None
            state.execution_context["target_hex"] = (
                target if variant == "immutable-context" else target.model_dump()
            )
        if variant == "board-hex":
            step.target_hex_arg = state.board.get_tile(target).hex
        if variant == "snapshot-alias":
            state.last_turn_positions = state.entity_locations
            state.execution_context["snapshot"] = state.last_turn_positions
        if variant in ("occupied", "occupied-mandatory"):
            step.target_hex_arg = h(0, 0)
            step.is_mandatory = variant.endswith("mandatory")
        if variant == "self-mandatory":
            step.target_hex_arg = state.get_position("hero_brogan")
            step.is_mandatory = True
        if variant == "no-unit":
            step.unit_id = None
            state.current_actor_id = None
        if variant == "fallback-unit":
            step.unit_id = None
        if variant in ("empty-unit-key", "empty-key-value"):
            step.unit_id = "" if variant == "empty-unit-key" else None
            step.unit_key = "unit"
            state.execution_context["unit"] = "hero_brogan" if variant == "empty-unit-key" else ""
        if variant == "offboard-unit":
            state.remove_entity("hero_brogan")
        if variant in ("off-map", "off-map-collision"):
            target = h(99, -99)
            state.board.map_id = ""
            step.target_hex_arg = target
            if variant == "off-map-collision":
                before = [
                    PlaceUnitStep(
                        unit_id="hero_arien",
                        target_hex_arg=target,
                        is_mandatory=False,
                        step_id="fixture/live-placement/virtual-first",
                    )
                ]
        if variant == "terrain":
            state.board.get_tile(target).is_terrain = True
        if variant == "dirty-pending":
            step.pending_input = {"selection": "ignored", "kept": [1]}
            step.pending_request_id = "fixture-existing-request"
            state.execution_context["pending"] = step.pending_input
        if variant == "pending-false":
            step.pending_input = False
        if variant == "repeated":
            before = [step]
        if variant == "argument-priority":
            state.execution_context["target_hex"] = h(0, -1)
        if variant == "empty-destination-key":
            step.destination_key = ""
            state.execution_context[""] = state.execution_context.pop("target_hex")
        if variant == "skipped-malformed":
            step.active_if_key = "missing"
            step.target_hex_arg = None
            state.execution_context["target_hex"] = ["unused"]
        if variant == "live-destination":
            state.execution_context["target_hex"] = h(0, 0)
            before = [
                SetContextFlagStep(
                    key="target_hex",
                    value=target,
                    step_id="fixture/live-placement/live-destination",
                )
            ]
        if variant == "old-snapshot":
            state.last_turn_positions = {"retained": state.get_position("hero_brogan")}
            state.execution_context["snapshot"] = state.last_turn_positions
        if variant in ("foreign-abort", "same-abort"):
            from goa2.engine.steps.phases import RestoreActionContextStep

            step.target_hex_arg = state.get_position("hero_brogan")
            step.is_mandatory = True
            after.insert(
                0,
                RestoreActionContextStep(
                    other_hero_action=variant == "foreign-abort",
                    step_id="fixture/live-placement/restore",
                ),
            )
        if variant in ("dict-coercion", "dict-extra-alias", "false-target", "empty-target"):
            step.target_hex_arg = None
            state.execution_context["target_hex"] = (
                {"q": "2", "r": -1.0, "s": "-1", "ignored": [42]}
                if variant == "dict-coercion"
                else (
                    {"q": 2, "r": -1, "s": -1, "ignored": state.entity_locations}
                    if variant == "dict-extra-alias"
                    else False if variant == "false-target" else {}
                )
            )
        retain(state, step, target)
        state.execution_stack = []
        push_steps(state, [*before, step, *after])
        yield "placement-alias-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]

    from goa2.bootstrap import register_all_effects
    from goa2.domain.models import GamePhase
    from goa2.engine.setup import GameSetup

    register_all_effects()
    for map_path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(map_path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_brogan"
        target = next(
            tile.hex
            for tile in state.board.tiles.values()
            if not tile.is_occupied and not tile.is_terrain
        )
        step = PlaceUnitStep(
            unit_id="hero_brogan",
            target_hex_arg=target,
            is_mandatory=False,
            step_id="fixture/live-placement/map/" + map_path.stem,
        )
        state.execution_context = {}
        retain(state, step, target)
        state.execution_stack = []
        push_steps(
            state,
            [
                step,
                SetContextFlagStep(
                    key="after", value=True, step_id="fixture/live-placement/map-after"
                ),
            ],
        )
        yield "placement-map-" + map_path.stem, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]

    for name, state, operations in minion_return_scenes(base):
        if name in (
            "minion-return-fallback",
            "minion-return-cap-zero",
            "minion-return-cap-one",
        ):
            yield "placement-" + name, state, [
                *operations,
                {"kind": "advance_prefix", "steps": 1},
            ]


def minion_return_scenes(base):
    """Raw automatic phase controller; relocation bodies are separate consumers."""
    from goa2.domain.board import Board, Zone
    from goa2.domain.hex import Hex
    from goa2.domain.input import selection_value
    from goa2.domain.models import TeamColor
    from goa2.domain.tile import Tile
    from goa2.engine.steps import ReturnMinionToZoneStep, SetContextFlagStep

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    variants = [
        "walk",
        "choice",
        "choice-last",
        "invalid-retry",
        "pending-valid",
        "pending-invalid",
        "pending-false",
        "pending-empty",
        "pending-missing",
        "fallback",
        "detour",
        "no-space",
        "disconnected",
        "inside",
        "offboard",
        "missing-zone",
        "missing-lane",
        "no-team",
        "processed",
        "two-teams",
        "blue-first",
        "two-red",
        "processed-duplicates",
        "no-actor",
        "shared-list",
        "repeated-empty",
        "skipped",
        "cap-zero",
        "cap-one",
        "cap-two",
        "cap-actions-only",
    ]
    for variant in variants:
        state = base()
        for uid in list(state.entity_locations):
            state.remove_entity(uid)
        points = [h(q, r) for q in range(-3, 4) for r in range(-3, 4) if abs(q + r) <= 3]
        state.board = Board(
            map_id="live-return-fixture",
            tiles={point: Tile(hex=point) for point in points},
            zones={"battle": Zone(id="battle", hexes={h(0), h(0, 1), h(0, -1)})},
        )
        state.battle_zones = {"lane_1": "battle"}
        state.tie_breaker_team = TeamColor.RED
        red = state.teams[TeamColor.RED].minions[0]
        blue = state.teams[TeamColor.BLUE].minions[0]
        state.place_entity(red.id, h(2))
        state.execution_context = {"kept": [True]}
        processed = []
        if variant in ("walk", "fallback", "detour", "no-space"):
            state.board.zones["battle"].hexes = {h(0)}
        if variant == "detour":
            state.board.tiles[h(1)].is_terrain = True
        if variant in ("fallback", "no-space"):
            for point in h(2).neighbors():
                if point in state.board.tiles:
                    state.board.tiles[point].is_terrain = True
        if variant == "no-space":
            state.board.tiles[h(0)].is_terrain = True
        if variant == "disconnected":
            state.board.tiles = {point: Tile(hex=point) for point in [h(0), h(2)]}
        if variant == "inside":
            state.move_unit(red.id, h(0))
        if variant in ("offboard", "repeated-empty"):
            state.remove_entity(red.id)
        if variant == "missing-zone":
            state.board.zones = {}
        if variant == "missing-lane":
            red.lane_id = "missing"
        if variant == "no-team":
            red.team = None
        if variant == "processed":
            processed = [str(red.id)]
        if variant == "processed-duplicates":
            processed = ["unknown", "unknown"]
        if variant in ("two-teams", "blue-first", "shared-list"):
            state.place_entity(blue.id, h(-2))
        if variant == "blue-first":
            state.tie_breaker_team = TeamColor.BLUE
        if variant == "two-red":
            state.place_entity(state.teams[TeamColor.RED].minions[1].id, h(-2))
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "shared-list":
            # Force an automatic choice while retaining the original processed
            # list through context and an unrelated stored step operand.
            state.board.zones["battle"].hexes = {h(0)}
        if variant.startswith("cap-"):
            state.board.zones["battle"].hexes = {h(0)}
            from combat_effect_traces import effect

            from goa2.domain.models.effect import EffectType

            state.active_effects = [
                effect(
                    state,
                    "live-return-cap",
                    EffectType.MOVEMENT_ZONE,
                    source="hero_arien",
                    max_value={
                        "cap-zero": 0,
                        "cap-one": 1,
                        "cap-two": 2,
                        "cap-actions-only": 0,
                    }[variant],
                    limit_actions_only=variant == "cap-actions-only",
                )
            ]
        step = ReturnMinionToZoneStep(
            step_id="fixture/live-return/" + variant, processed_minion_ids=processed
        )
        state.execution_stack = [step]
        state.execution_context["return_step"] = step
        state.execution_context["processed"] = step.processed_minion_ids
        operations = [{"kind": "advance_prefix", "steps": 1}]
        if variant in (
            "choice",
            "choice-last",
            "invalid-retry",
            "pending-valid",
            "pending-invalid",
        ):
            request = step.resolve(state, state.execution_context).input_request
            assert request is not None
            choice = selection_value(request.options[-1 if variant == "choice-last" else 0])
            if variant == "pending-valid":
                step.pending_input = {"selection": choice, "retained": True}
                state.execution_context["pending_alias"] = step.pending_input
            elif variant == "pending-invalid":
                step.pending_input = {"selection": "SKIP", "retained": True}
                state.execution_context["pending_alias"] = step.pending_input
            else:
                if variant == "invalid-retry":
                    operations += [
                        {"kind": "input", "selection": {"q": 0, "r": 0, "s": 1}},
                        {"kind": "advance_prefix", "steps": 1},
                    ]
                operations += [
                    {"kind": "input", "selection": choice},
                    {"kind": "advance_prefix", "steps": 1},
                ]
        if variant == "pending-false":
            step.pending_input = False
        if variant == "pending-empty":
            step.pending_input = {}
            state.execution_context["pending_alias"] = step.pending_input
        if variant == "pending-missing":
            step.pending_input = {"other": True}
        if variant == "shared-list":
            retained = SetContextFlagStep(
                step_id="fixture/live-return/retained",
                key="retained",
                value=step.processed_minion_ids,
            )
            state.execution_context["retained_step"] = retained
        if variant == "repeated-empty":
            state.execution_stack = [step, step]
            operations += [{"kind": "advance"}]
        if variant == "skipped":
            step.active_if_key = "absent"
            step.pending_input = ["malformed-but-skipped"]
        yield "minion-return-" + variant, state, operations


def loop_scenes(base):
    from combat_effect_traces import effect

    from goa2.domain.hex import Hex
    from goa2.domain.models import TargetType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import SelectStep
    from goa2.engine.steps.utility import ForEachStep, RecordTargetStep, SetContextFlagStep

    class AnnotatedHex(Hex):
        marks: list[int]

    def record(input_key="item", output="seen"):
        return RecordTargetStep(
            step_id="fixture/loop/record", input_key=input_key, output_list_key=output
        )

    def loop(templates, index=0):
        step = ForEachStep(
            step_id="fixture/loop/root", list_key="items", item_key="item", current_index=index
        )
        # Preserve intentionally shared physical templates after constructor isolation.
        step.steps_template = templates
        return step

    def prefixes(count):
        return [{"kind": "advance_prefix", "steps": 1} for _ in range(count)] + [
            {"kind": "advance"}
        ]

    state = base()
    hero = state.get_hero("hero_arien")
    box = {"nested": [8]}
    items = [box, [3], hero, hero.hand[0], Hex(q=0, r=0, s=0), box]
    step = loop([record()])
    state.execution_stack = [step]
    state.execution_context = {"items": items, "stored": step, "mirror": items}
    yield "loop-reference-items", state, prefixes(1)

    for variant in (
        "list",
        "dict",
        "tuple",
        "frozen",
        "frozen-subclass",
        "model",
        "card",
        "token",
        "world-list",
    ):
        state = base(["Min", "Hanu"] if variant == "token" else None)
        hero = state.get_hero("hero_min" if variant == "token" else "hero_arien")
        child = [4]
        value = {
            "list": child,
            "dict": {"nested": child},
            "tuple": (child, child),
            "frozen": Hex(q=0, r=0, s=0),
            "frozen-subclass": AnnotatedHex(q=0, r=0, s=0, marks=child),
            "model": hero,
            "card": hero.hand[0],
            "token": next(
                (token for tokens in state.token_pool.values() for token in tokens), None
            ),
            "world-list": hero.hand,
        }[variant]
        payload = value if variant == "world-list" else {"left": value, "right": value}
        setter = SetContextFlagStep(step_id="fixture/loop/set", key="value", value=True)
        setter.value = payload
        recorder = record("value")
        templates = [setter, recorder, setter, recorder]
        if variant == "world-list":
            mutate = record("extra", "value")
            templates = [setter, mutate, recorder, setter, mutate, recorder]
        step = loop(templates)
        state.execution_stack = [step]
        state.execution_context = {
            "items": [1, 2],
            "stored": step,
            "original": payload,
            "extra": hero.hand[-1],
        }
        yield "loop-deep-" + variant, state, prefixes(len(templates) + 1)

    for variant in (
        "missing",
        "empty",
        "tuple",
        "negative",
        "replace",
        "append",
        "append-last",
        "dirty",
        "condition",
    ):
        state = base()
        templates = [record()]
        items = [] if variant == "empty" else ([1] if variant == "append-last" else [1, 2])
        if variant == "tuple":
            items = ([1], [2])
        if variant == "replace":
            templates.append(
                SetContextFlagStep(step_id="fixture/loop/replace", key="items", value=[9])
            )
        if variant in ("append", "append-last"):
            append = record("extra", "items")
            append.active_if_key = "append"
            templates.extend(
                [
                    append,
                    SetContextFlagStep(step_id="fixture/loop/disable", key="append", value=False),
                ]
            )
        if variant == "dirty":
            select = SelectStep(
                step_id="fixture/loop/dirty",
                target_type=TargetType.NUMBER,
                number_options=[1, 2],
                output_key="chosen",
                prompt="Dirty copy",
            )
            select.pending_input = {"selection": 1}
            select.pending_request_id = "fixture/dirty-request"
            templates.append(select)
        step = loop(templates, -1 if variant == "negative" else 0)
        if variant == "condition":
            step.active_if_key = "enabled"
        state.execution_stack = [step]
        state.execution_context = {
            "items": items,
            "mirror": items,
            "stored": step,
            "extra": 3,
            "append": True,
            "enabled": False,
        }
        if variant == "missing":
            del state.execution_context["items"]
        yield "loop-" + variant, state, prefixes(2)

    state = base()
    inner = ForEachStep(
        step_id="fixture/loop/inner",
        list_key="inner_items",
        item_key="inner",
        steps_template=[record("inner")],
    )
    step = loop([inner])
    state.execution_stack = [step]
    box = {"nested": [1]}
    state.execution_context = {
        "items": [1, 2],
        "inner_items": [box, box],
        "stored": step,
        "inner_template": inner,
    }
    yield "loop-nested", state, prefixes(10)

    state = base()
    select = SelectStep(
        step_id="fixture/loop/input",
        target_type=TargetType.NUMBER,
        number_options=[1, 2],
        output_key="chosen",
        prompt="Loop copy",
    )
    step = loop([select, record("chosen")])
    state.execution_stack = [step]
    state.execution_context = {"items": [1, 2], "stored": step, "number_template": select}
    yield "loop-input", state, [
        {"kind": "advance"},
        {"kind": "input", "selection": 2},
        {"kind": "advance"},
        {"kind": "input", "selection": 1},
        {"kind": "advance"},
    ]

    for independent in (False, True):
        state = base()
        step = loop([record()])
        other = step.model_copy(deep=True) if independent else step
        stored = effect(state, "loop", EffectType.MOVEMENT_ZONE, source="hero_arien", max_value=1)
        stored.finishing_steps = [step, other]
        state.active_effects = [stored]
        state.execution_stack = [step, other]
        state.execution_context = {"items": [1, 2], "stored": step, "other": other}
        yield "loop-" + ("independent" if independent else "shared"), state, prefixes(4)

    state = base()
    step = loop([record()])
    state.execution_stack = [step]
    state.execution_context = {"items": list(range(1, 601)), "stored": step}
    yield "loop-budget", state, [{"kind": "advance_prefix", "steps": 1000}, {"kind": "advance"}]

    state = base()
    select = SelectStep(
        step_id="fixture/loop/operand",
        target_type=TargetType.NUMBER,
        number_options=[1, 2],
        output_key="chosen",
        prompt="Copied operand",
    )
    select.pending_input = {"selection": 1}
    setter = SetContextFlagStep(step_id="fixture/loop/operand-set", key="value", value=True)
    setter.value = select.number_options
    step = loop([setter, record("extra", "value"), select])
    state.execution_stack = [step]
    state.execution_context = {"items": [1, 2], "extra": 3, "stored": step}
    yield "loop-copied-operand-provenance", state, prefixes(4)


def scope_scenes(base):
    from goa2.domain.models import ActionType, TargetType
    from goa2.engine.steps import SelectStep
    from goa2.engine.steps.phases import (
        RestoreActionContextStep,
        RestoreActionTypeStep,
        push_action_context,
    )
    from goa2.engine.steps.utility import (
        ForEachStep,
        RecordTargetStep,
        SetActorStep,
        SetContextFlagStep,
    )

    def restore(other=False, key=None):
        return RestoreActionContextStep(
            step_id="fixture/scope/restore", other_hero_action=other, active_if_key=key
        )

    def select(empty=False, mandatory=True):
        return SelectStep(
            step_id="fixture/scope/number",
            target_type=TargetType.NUMBER,
            number_options=[] if empty else [1, 2],
            prompt="Actor/scope",
            output_key="chosen",
            is_mandatory=mandatory,
        )

    def flag(key):
        return SetContextFlagStep(step_id="fixture/scope/" + key, key=key, value=True)

    def outer(state):
        values = [1]
        state.execution_context = {
            "current_action_type": ActionType.ATTACK,
            "current_card_id": "outer/card",
            "performing_card_owner_id": None,
            "token_type_override": values,
            "outer_values": values,
            "skip_markers": False,
            "substitution_actor_id": None,
            "extra": 7,
            "unrelated": "before",
        }
        return values

    for count in (1, 2, 3):
        state = base()
        outer(state)
        state.execution_stack = [restore() for _ in range(count)]
        ops = [
            {
                "kind": "push_action_context",
                "action_type": "SKILL",
                "card": "nested/" + str(i),
                "owner": "hero_hanu",
            }
            for i in range(count)
        ]
        yield "scope-push-" + str(count), state, [
            *ops,
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]

    for variant in (
        "missing",
        "present-null",
        "empty-stack",
        "all-fields",
        "physical",
        "model",
    ):
        state = base()
        state.execution_context = {"unrelated": "before"}
        if variant == "present-null":
            state.execution_context.update(
                {
                    key: None
                    for key in (
                        "current_action_type",
                        "current_card_id",
                        "skip_markers",
                        "substitution_actor_id",
                    )
                }
            )
        elif variant == "empty-stack":
            stack = []
            state.execution_context.update({"action_context_stack": stack, "mirror": stack})
        elif variant == "all-fields":
            outer(state)
        elif variant in ("physical", "model"):
            hero = state.get_hero("hero_arien")
            value = hero.hand[0] if variant == "physical" else hero
            state.execution_context.update({"token_type_override": value, "original": value})
        state.execution_stack = [restore()]
        yield "scope-presence-" + variant, state, [
            {
                "kind": "push_action_context",
                "action_type": "DEFENSE",
                "card": "nested",
                "owner": "hero_hanu",
            },
            {"kind": "advance"},
        ]

    for count in (0, 1, 2, 3):
        state = base()
        outer(state)
        for i in range(count):
            push_action_context(
                state.execution_context,
                action_type=ActionType.SKILL,
                card_id="nested/" + str(i),
                card_owner_id="hero_hanu",
            )
        if count:
            state.execution_context["stack_alias"] = state.execution_context["action_context_stack"]
            state.execution_context["snapshot"] = state.execution_context["action_context_stack"][
                -1
            ]
        mutate = RecordTargetStep(
            step_id="fixture/scope/mutate", input_key="extra", output_list_key="outer_values"
        )
        state.execution_stack = [restore() for _ in range(max(1, count))] + [
            SetContextFlagStep(step_id="fixture/scope/unrelated", key="unrelated", value="after"),
            SetContextFlagStep(
                step_id="fixture/scope/replace", key="token_type_override", value=True
            ),
            mutate,
        ]
        yield "scope-restore-" + str(count), state, [
            {"kind": "advance_prefix", "steps": 3},
            {"kind": "advance"},
        ]

    for variant in (
        "own",
        "other",
        "other-boundary",
        "extra-scopes",
        "optional",
        "skipped-boundary",
        "actor-own",
        "actor-other",
    ):
        state = base()
        outer(state)
        count = 3 if variant == "extra-scopes" else 2
        for i in range(count):
            push_action_context(
                state.execution_context,
                action_type=ActionType.SKILL,
                card_id="nested/" + str(i),
                card_owner_id="hero_hanu",
            )
        state.execution_context["stack_alias"] = state.execution_context["action_context_stack"]
        other = variant in ("other", "other-boundary", "skipped-boundary", "actor-other")
        state.execution_stack = [
            flag("after_outer"),
            restore(),
            flag("after_inner"),
            restore(other, "never" if variant == "skipped-boundary" else None),
            flag("skipped"),
            select(True, variant != "optional"),
        ]
        if variant == "extra-scopes":
            state.execution_stack = [flag("skipped"), select(True)]
        if variant in ("actor-own", "actor-other"):
            state.execution_stack = [
                flag("after_outer"),
                SetActorStep(
                    step_id="fixture/scope/actor-restore",
                    actor_key="previous_actor",
                    save_key="discarded_actor",
                ),
                restore(other),
                select(True),
                SetActorStep(
                    step_id="fixture/scope/actor",
                    actor_id="hero_hanu",
                    save_key="previous_actor",
                ),
            ]
        ops = [{"kind": "advance_prefix", "steps": 1}] if variant == "other-boundary" else []
        yield "scope-abort-" + variant, state, [*ops, {"kind": "advance"}]

    for other in (False, True):
        state = base()
        outer(state)
        push_action_context(
            state.execution_context,
            action_type=ActionType.SKILL,
            card_id="nested",
            card_owner_id="hero_hanu",
        )
        step = ForEachStep(
            step_id="fixture/scope/loop",
            list_key="items",
            item_key="item",
            steps_template=[select(True), restore(other), flag("after_boundary")],
        )
        state.execution_stack = [step]
        state.execution_context.update({"items": [1, 2], "stored_loop": step})
        yield "scope-loop-abort-" + str(other), state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]

    for variant in ("missing", "empty", "single", "shared", "model"):
        state = base()
        value = {"nested": [1]} if variant == "model" else "ATTACK"
        values = [] if variant == "empty" else [value]
        state.execution_context = {"current_action_type": "DEFENSE", "original": value}
        if variant != "missing":
            state.execution_context["action_type_stack"] = values
        if variant == "shared":
            state.execution_context["mirror"] = values
        state.execution_stack = [RestoreActionTypeStep(step_id="fixture/scope/type")]
        yield "scope-action-type-" + variant, state, [{"kind": "advance"}]

    for variant in (
        "literal",
        "key",
        "empty-key",
        "missing-key",
        "false-key",
        "no-actor",
        "empty-literal",
        "same-save-key",
        "unknown",
    ):
        state = base()
        state.execution_context = {"candidate": "hero_hanu", "": "hero_brogan"}
        values = {"actor_id": "hero_hanu"}
        if variant in ("key", "missing-key", "false-key", "no-actor", "same-save-key"):
            values = {"actor_id": "hero_brogan", "actor_key": "candidate"}
        if variant == "empty-key":
            values["actor_key"] = ""
        if variant == "missing-key":
            del state.execution_context["candidate"]
        if variant == "false-key":
            state.execution_context["candidate"] = 0
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "empty-literal":
            values["actor_id"] = ""
        if variant == "same-save-key":
            values["save_key"] = "candidate"
        if variant == "unknown":
            values["actor_id"] = "hero_missing"
        state.execution_stack = [
            select(),
            SetActorStep(step_id="fixture/scope/actor", **values),
        ]
        yield "scope-actor-" + variant, state, [
            {"kind": "advance"},
            {"kind": "input", "selection": 1},
            {"kind": "advance"},
        ]

    for variant, values in (
        ("empty-literal", {"override_player_id": "", "override_player_id_key": "candidate"}),
        ("empty-key", {"override_player_id_key": ""}),
        ("empty-literal-no-key", {"override_player_id": ""}),
        (
            "literal-wins",
            {"override_player_id": "hero_hanu", "override_player_id_key": "candidate"},
        ),
    ):
        state = base()
        state.execution_context = {
            "candidate": "hero_brogan" if variant == "literal-wins" else "hero_hanu",
            "": "hero_brogan",
        }
        step = select()
        for key, value in values.items():
            setattr(step, key, value)
        state.execution_stack = [step]
        yield "scope-owner-" + variant, state, [
            {"kind": "advance"},
            {"kind": "input", "selection": 1},
            {"kind": "advance"},
        ]

    for controlled in (False, True):
        state = base()
        if controlled:
            from goa2.domain.models.effect import (
                ActiveEffect,
                DurationType,
                EffectScope,
                EffectType,
                Shape,
            )

            hero = state.get_hero("hero_arien")
            hero.current_turn_card = hero.hand[1]
            state.resolution_owner_id = "hero_arien"
            state.active_effects = [
                ActiveEffect(
                    id="scope-control",
                    source_id="hero_hanu",
                    effect_type=EffectType.CONTROL_NEXT_ACTION,
                    scope=EffectScope(shape=Shape.GLOBAL, origin_id="hero_arien"),
                    duration=DurationType.PASSIVE,
                    is_active=True,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                    controlled_card_id=hero.current_turn_card.id,
                )
            ]
        state.execution_stack = [
            select(),
            SetActorStep(
                step_id="fixture/scope/return",
                actor_key="previous_actor",
                save_key="discarded_actor",
            ),
            select(),
            SetActorStep(
                step_id="fixture/scope/swap", actor_id="hero_hanu", save_key="previous_actor"
            ),
        ]
        yield "scope-actor-swap-" + str(controlled), state, [
            {"kind": "advance"},
            {"kind": "input", "selection": 1},
            {"kind": "advance"},
            {"kind": "input", "selection": 2},
            {"kind": "advance"},
        ]


def effect_lifecycle_scenes(base):
    from combat_effect_traces import effect
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, TargetType
    from goa2.domain.models.effect import DurationType, EffectType
    from goa2.engine.steps import AskConfirmationStep, FinishedExpiringEffectStep, SelectStep
    from goa2.engine.steps.utility import ForEachStep, RecordTargetStep, SetContextFlagStep

    def flag(key):
        return SetContextFlagStep(step_id="fixture/lifecycle/" + key, key=key, value=True)

    def make(state, name, card, duration=DurationType.THIS_ROUND, **values):
        value = effect(
            state,
            name,
            EffectType.DELAYED_TRIGGER,
            source=str(state.current_actor_id),
            source_card_id=card.id,
            **values,
        )
        value.duration = duration
        return value

    for requested in DurationType:
        state = base()
        card = state.get_hero("hero_arien").hand[0]
        card.is_active = True
        effects = [
            make(state, "duration/" + duration.value, card, duration) for duration in DurationType
        ]
        for i, value in enumerate(effects):
            value.is_active = bool(i % 2)
            value.finishing_steps = [flag("not_run_" + str(i))]
        state.active_effects = effects
        state.execution_context = {"old_effects": effects, "card": card}
        yield "effects-duration-" + requested.value, state, [
            {"kind": "expire_effects", "duration": requested.value},
            {"kind": "expire_effects", "duration": requested.value},
        ]
    state = base()
    card = state.get_hero("hero_arien").hand[0]
    card.is_active = True
    state.active_effects = [
        make(state, "all/" + duration.value, card, duration) for duration in DurationType
    ]
    state.execution_context = {"old_effects": state.active_effects, "card": card}
    yield "effects-all-durations", state, [
        {"kind": "expire_effects", "duration": duration.value} for duration in DurationType
    ]

    for variant in (
        "token",
        "empty-token",
        "after-cards",
        "duplicate-id",
        "same-object",
        "unknown-payload",
    ):
        state = base()
        card = state.get_hero("hero_arien").hand[0]
        card.is_active = True
        value = make(state, "selected", card)
        value.finishing_steps = [flag("not_run")]
        state.active_effects = [value]
        if variant in ("token", "empty-token"):
            value.token_id = "token/none" if variant == "token" else ""
        elif variant == "after-cards":
            value.effect_type = EffectType.AFTER_CARDS_PLAYED_TRIGGER
        elif variant == "duplicate-id":
            other = state.get_hero("hero_hanu").hand[0]
            other.is_active = True
            state.active_effects.append(make(state, "selected", other, DurationType.PASSIVE))
            state.execution_context["other_card"] = other
        elif variant == "same-object":
            state.active_effects.append(value)
        else:
            value.finishing_steps = [
                AskConfirmationStep(step_id="fixture/lifecycle/unknown", prompt="Never run")
            ]
        state.execution_context.update(
            {"old_effects": state.active_effects, "saved_effect": value, "card": card}
        )
        yield "effects-expire-" + variant, state, [
            {"kind": "expire_effects", "duration": "THIS_ROUND"}
        ]

    for ids in ([], ["missing"], ["selected"], ["selected", "selected"]):
        state = base()
        card = state.get_hero("hero_arien").hand[0]
        card.is_active = True
        value = make(state, "selected", card)
        value.finishing_steps = [flag("never_on_removal")]
        state.active_effects = [value]
        state.execution_context = {"old_effects": state.active_effects, "card": card}
        yield "effects-remove-" + str(ids), state, [{"kind": "remove_effects", "ids": ids}]

    for priority in (False, True):
        state = base()
        hero = state.get_hero("hero_arien")
        card = hero.hand[0]
        copy_card = card.model_copy(deep=True)
        card.is_active = copy_card.is_active = True
        hero.hand.append(copy_card)
        if priority:
            hero.current_turn_card = copy_card
        state.active_effects = [make(state, "selected", card)]
        state.execution_context = {"card": card, "copy": copy_card}
        yield "effects-printed-priority-" + str(priority), state, [
            {"kind": "remove_effects", "ids": ["selected"]}
        ]

    for variant in ("spell", "enraged"):
        state = setup(
            ["Gydion" if variant == "spell" else "Ursafar", "Hanu"], ["Brogan", "Tigerclaw"]
        )
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_gydion" if variant == "spell" else "hero_ursafar"
        hero = state.get_hero(state.current_actor_id)
        card = hero.spells[0] if variant == "spell" else hero.ultimate_card
        card.is_active = True
        if variant == "enraged":
            card.state = CardState.RESOLVED
            card.enraged_active_override = True
        state.active_effects = [make(state, "selected", card)]
        state.execution_context = {"card": card}
        yield "effects-card-" + variant, state, [
            {"kind": "expire_effects", "duration": "THIS_ROUND"}
        ]

    for phase in (GamePhase.RESOLUTION, GamePhase.CLEANUP):
        state = base()
        state.phase = phase
        card = state.get_hero("hero_arien").hand[0]
        card.is_active = True
        value = make(state, "selected", card)
        number = SelectStep(
            step_id="fixture/lifecycle/number",
            target_type=TargetType.NUMBER,
            number_options=[1, 2],
            output_key="chosen",
            prompt="Live finishing",
        )
        value.finishing_steps = [number, flag("after_finishing")]
        state.active_effects = [value]
        state.execution_stack = [FinishedExpiringEffectStep(step_id="fixture/lifecycle/boundary")]
        state.execution_context = {
            "saved_effect": value,
            "old_effects": state.active_effects,
            "card": card,
        }
        queue = {
            "kind": "queue_steps",
            "path": ["execution_context", "saved_effect", "finishing_steps"],
        }
        yield "effects-live-finishing-" + phase.value, state, [
            queue,
            {"kind": "advance"},
            {"kind": "input", "selection": 1},
            {"kind": "advance"},
            {"kind": "remove_effects", "ids": ["selected"]},
            queue,
            {"kind": "advance"},
            {"kind": "input", "selection": 2},
            {"kind": "advance"},
        ]

    for same in (False, True):
        state = base()
        card = state.get_hero("hero_arien").hand[0]
        card.is_active = True
        value = make(state, "selected", card)
        value.finishing_steps = [flag("after_expiry")]
        state.active_effects = [value, value] if same else [value]
        state.execution_context = {
            "saved_effect": value,
            "old_effects": state.active_effects,
            "card": card,
        }
        yield "effects-expired-finishing-" + str(same), state, [
            {"kind": "expire_effects", "duration": "THIS_ROUND"},
            {
                "kind": "queue_steps",
                "path": ["execution_context", "saved_effect", "finishing_steps"],
            },
            {"kind": "advance"},
        ]

    state = base()
    card = state.get_hero("hero_arien").hand[0]
    card.is_active = False
    first, second = make(state, "first", card), make(state, "second", card)
    state.active_effects = [first, second]
    setter = SetContextFlagStep(step_id="fixture/lifecycle/copy", key="copy", value=card)
    # Preserve the actual physical card after normal constructor isolation.
    setter.value = card
    loop = ForEachStep(
        step_id="fixture/lifecycle/loop",
        list_key="items",
        item_key="item",
        steps_template=[
            setter,
            RecordTargetStep(
                step_id="fixture/lifecycle/record", input_key="copy", output_list_key="copies"
            ),
        ],
    )
    # Source constructor deep-copies templates; keep the live original payload.
    loop.steps_template[0].value = card
    state.execution_stack = [loop]
    state.execution_context = {
        "items": [1, 2],
        "card": card,
        "old_effects": state.active_effects,
    }
    yield "effects-physical-copy-snapshot", state, [
        {"kind": "advance_prefix", "steps": 1},
        {"kind": "remove_effects", "ids": ["first"]},
        {"kind": "advance_prefix", "steps": 3},
        {"kind": "remove_effects", "ids": ["second"]},
        {"kind": "advance"},
    ]

    state = base()
    card = state.get_hero("hero_arien").hand[0]
    value = make(state, "selected", card)
    state.active_effects = [value]
    state.execution_context = {"old_effects": state.active_effects, "extra": value}
    state.execution_stack = [
        RecordTargetStep(
            step_id="fixture/lifecycle/old-list",
            input_key="extra",
            output_list_key="old_effects",
        )
    ]
    yield "effects-detached-list", state, [
        {"kind": "remove_effects", "ids": ["selected"]},
        {"kind": "advance"},
    ]

    for variant in ("plain", "scope", "other-hero", "skipped-marker", "copied-loop"):
        from goa2.domain.models import ActionType
        from goa2.engine.steps.phases import RestoreActionContextStep, push_action_context

        state = base()
        card = state.get_hero("hero_arien").hand[0]
        value = make(state, "selected", card)
        value.finishing_steps = [flag("not_run")]
        state.active_effects = [value]
        empty = SelectStep(
            step_id="fixture/lifecycle/abort",
            target_type=TargetType.NUMBER,
            number_options=[],
            prompt="Fizzle",
            output_key="nothing",
        )
        marker = FinishedExpiringEffectStep(step_id="fixture/lifecycle/marker")
        if variant == "skipped-marker":
            marker.active_if_key = "never"
        state.execution_context = {
            "current_card_id": "outer",
            "old_effects": state.active_effects,
        }
        if variant in ("scope", "other-hero"):
            push_action_context(
                state.execution_context,
                action_type=ActionType.SKILL,
                card_id="inner",
                card_owner_id="hero_arien",
            )
        state.execution_stack = [flag("after_marker"), marker, flag("skip_this"), empty]
        if variant == "other-hero":
            state.execution_stack.insert(
                2,
                RestoreActionContextStep(step_id="fixture/lifecycle/other", other_hero_action=True),
            )
        if variant == "copied-loop":
            loop = ForEachStep(
                step_id="fixture/lifecycle/marker-loop",
                list_key="items",
                item_key="item",
                steps_template=[empty, flag("skip_this"), marker, flag("after_marker")],
            )
            state.execution_stack = [loop]
            state.execution_context["items"] = [1, 2]
            state.execution_context["loop"] = loop
        yield "effects-finishing-marker-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]


def scheduled_effect_scenes(base):
    from combat_effect_traces import effect

    from goa2.domain.models import TargetType
    from goa2.domain.models.effect import DurationType, EffectType
    from goa2.engine.steps import SelectStep
    from goa2.engine.steps.utility import AddContextValueStep, SetContextFlagStep

    def make(state, name, **kwargs):
        card = state.get_hero("hero_arien").hand[0]
        card.is_active = True
        result = effect(
            state,
            name,
            EffectType.AFTER_CARDS_PLAYED_TRIGGER,
            source="hero_arien",
            source_card_id=card.id,
        )
        result.duration = DurationType.NEXT_TURN
        result.created_at_round = state.round
        result.created_at_turn = state.turn - 1
        for key, value in kwargs.items():
            setattr(result, key, value)
        result.finishing_steps = [
            AddContextValueStep(step_id="fixture/scheduled/count", key="count", amount=1)
        ]
        return result

    for variant in (
        "due",
        "inactive",
        "stale",
        "future",
        "cross-round",
        "empty",
        "token",
        "empty-token",
        "duplicate-id",
        "same-object",
        "shared-step",
        "independent-step",
        "input",
        "abort",
        "no-triggers",
    ):
        state = base()
        state.turn = 2
        trigger = make(state, "trigger")
        if variant == "inactive":
            trigger.is_active = False
        if variant == "stale":
            trigger.created_at_turn = 0
        if variant == "future":
            trigger.created_at_turn = 2
        if variant == "cross-round":
            trigger.created_at_round = state.round + 1
        if variant == "empty":
            trigger.finishing_steps = []
        if variant == "token":
            trigger.token_id = "retained-token"
        if variant == "empty-token":
            trigger.token_id = ""
        state.active_effects = [trigger]
        if variant in ("duplicate-id", "same-object", "shared-step", "independent-step"):
            other = (
                trigger
                if variant == "same-object"
                else make(state, "other", source_id="hero_brogan")
            )
            if variant == "duplicate-id":
                other.id = trigger.id
                other.effect_type = EffectType.DELAYED_TRIGGER
            if variant == "shared-step":
                other.finishing_steps = trigger.finishing_steps
            state.active_effects.append(other)
        if variant == "no-triggers":
            trigger.effect_type = EffectType.DELAYED_TRIGGER
        if variant in ("input", "abort"):
            select = SelectStep(
                step_id="fixture/scheduled/select",
                target_type=TargetType.NUMBER,
                number_options=[] if variant == "abort" else [1, 2],
                output_key="selected",
                prompt="Scheduled choice",
            )
            trigger.finishing_steps.insert(0, select)
        state.execution_stack = [
            SetContextFlagStep(step_id="fixture/scheduled/tail", key="after", value=True)
        ]
        state.execution_context = {
            "old_effects": state.active_effects,
            "trigger": trigger,
            "card": state.get_hero("hero_arien").hand[0],
        }
        ops = [
            {"kind": "after_cards_played"},
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]
        if variant == "input":
            ops += [{"kind": "input", "selection": 2}, {"kind": "advance"}]
        ops += [{"kind": "after_cards_played"}, {"kind": "advance"}]
        yield "scheduled-after-cards-" + variant, state, ops

    for variant in (
        "this-turn",
        "inactive",
        "next-due",
        "next-future",
        "next-stale",
        "cross-round",
        "token",
        "after-cards",
        "duplicate-id",
        "same-object",
    ):
        state = base()
        state.turn = 2
        value = make(state, "turn")
        value.effect_type = EffectType.DELAYED_TRIGGER
        value.duration = DurationType.THIS_TURN
        if variant == "inactive":
            value.is_active = False
        if variant.startswith("next-") or variant == "cross-round":
            value.duration = DurationType.NEXT_TURN
        if variant == "next-future":
            value.created_at_turn = 2
        if variant == "next-stale":
            value.created_at_turn = 0
        if variant == "cross-round":
            value.created_at_round = state.round + 1
        if variant == "token":
            value.token_id = "retained-token"
        if variant == "after-cards":
            value.effect_type = EffectType.AFTER_CARDS_PLAYED_TRIGGER
        state.active_effects = [value]
        if variant == "duplicate-id":
            other = make(state, "turn", duration=DurationType.PASSIVE)
            other.effect_type = EffectType.DELAYED_TRIGGER
            state.active_effects.append(other)
        if variant == "same-object":
            state.active_effects.append(value)
        state.execution_context = {
            "old_effects": state.active_effects,
            "value": value,
            "card": state.get_hero("hero_arien").hand[0],
        }
        yield "scheduled-turn-expiry-" + variant, state, [
            {"kind": "expire_turn_effects"},
            {"kind": "expire_turn_effects"},
        ]

    state = base()
    state.turn = 2
    value = make(state, "mutable-options", duration=DurationType.THIS_TURN)
    value.effect_type = EffectType.DELAYED_TRIGGER
    select = SelectStep(
        step_id="fixture/scheduled/mutable-options",
        target_type=TargetType.NUMBER,
        number_options=[1],
        output_key="chosen",
        prompt="Detached options",
    )
    value.finishing_steps = [select]
    from goa2.engine.steps.utility import RecordTargetStep

    state.active_effects = [value]
    state.execution_context = {"saved": value, "target": 2, "history": select.number_options}
    state.execution_stack = [
        RecordTargetStep(
            step_id="fixture/scheduled/mutate-options",
            input_key="target",
            output_list_key="history",
        )
    ]
    yield "scheduled-detached-mutable-options", state, [
        {"kind": "expire_turn_effects"},
        {"kind": "advance"},
        {"kind": "queue_steps", "path": ["execution_context", "saved", "finishing_steps"]},
        {"kind": "advance"},
        {"kind": "input", "selection": 2},
        {"kind": "advance"},
    ]


def turn_boundary_scenes(base):
    from combat_effect_traces import effect

    from goa2.domain.models import GamePhase, TargetType
    from goa2.domain.models.effect import DurationType, EffectType
    from goa2.engine.steps import AdvanceTurnStep, EndPhaseStep, SelectStep
    from goa2.engine.steps.utility import (
        AddContextValueStep,
        RecordTargetStep,
        SetContextFlagStep,
    )

    def flag(key):
        return SetContextFlagStep(step_id="fixture/turn-boundary/" + key, key=key, value=True)

    def prepare(turn=2, version=9):
        state = base()
        state.turn, state.rules_version = turn, version
        state.turn_discard_log = {"hero_arien": ["noble_blade"]}
        state.execution_context = {
            "old_log": state.turn_discard_log,
            "old_snapshot": state.last_turn_positions,
            "locations": state.entity_locations,
            "old_pending": state.pending_inputs,
            "old_done": state.planning_done,
        }
        state.execution_stack = [flag("tail")]
        return state

    def add_effect(state, kind="flag"):
        hero = state.get_hero("hero_arien")
        card = hero.hand[0]
        hero.play_card(card)
        hero.resolve_current_card(turn_number=state.turn)
        card.is_active = True
        value = effect(
            state,
            "turn-boundary/effect",
            EffectType.DELAYED_TRIGGER,
            source="hero_arien",
            source_card_id=card.id,
        )
        value.duration = DurationType.THIS_TURN
        value.finishing_steps = [
            AddContextValueStep(step_id="fixture/turn-boundary/count", key="count", amount=1)
        ]
        state.active_effects = [value]
        if kind == "empty":
            value.finishing_steps = []
        if kind == "token":
            value.token_id = "some-token"
        if kind == "after-cards":
            value.effect_type = EffectType.AFTER_CARDS_PLAYED_TRIGGER
        if kind in ("next-due", "next-future"):
            value.duration = DurationType.NEXT_TURN
            value.created_at_round = state.round
            value.created_at_turn = state.turn - (1 if kind == "next-due" else 0)
        if kind == "same-object":
            state.active_effects.append(value)
        if kind == "duplicate-id":
            other = effect(state, value.id, EffectType.DELAYED_TRIGGER, source="hero_arien")
            other.duration = DurationType.PASSIVE
            state.active_effects.append(other)
        if kind in ("pending-input", "abort"):
            value.finishing_steps.insert(
                0,
                SelectStep(
                    step_id="fixture/turn-boundary/select",
                    target_type=TargetType.NUMBER,
                    number_options=[] if kind == "abort" else [1, 2],
                    prompt="Turn-boundary choice",
                    output_key="selected",
                ),
            )
        state.execution_context.update(
            {"old_effects": state.active_effects, "effect": value, "card": card}
        )
        return value

    for kind in (
        "empty",
        "flag",
        "pending-input",
        "abort",
        "token",
        "after-cards",
        "same-object",
        "duplicate-id",
        "next-due",
        "next-future",
    ):
        state = prepare()
        add_effect(state, kind)
        ops = [
            {"kind": "end_turn"},
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]
        if kind == "pending-input":
            ops += [{"kind": "input", "selection": 2}, {"kind": "advance"}]
        yield "turn-boundary-" + kind, state, ops

    for version in (9,):
        for turn in (1, 4):
            state = prepare(turn, version)
            add_effect(state)
            yield f"turn-boundary-version-{version}-turn-{turn}", state, [
                {"kind": "end_turn"},
                *[{"kind": "advance_prefix", "steps": 1} for _ in range(4 if turn == 1 else 5)],
            ]

    for turn in (1, 3, 4):
        state = prepare(turn)
        yield f"turn-boundary-no-effects-{turn}", state, [
            {"kind": "end_turn"},
            {"kind": "advance_prefix", "steps": 1},
        ]

    for kind in ("empty", "matching", "missing", "skip", "append-id"):
        state = prepare()
        value = add_effect(state)
        step = AdvanceTurnStep(
            step_id="fixture/turn-boundary/direct",
            expire_effect_ids=[value.id] if kind in ("matching", "skip") else [],
        )
        if kind == "missing":
            step.expire_effect_ids = ["not-an-effect"]
        if kind == "skip":
            step.active_if_key = "missing-active"
        state.execution_stack.append(step)
        state.execution_context["advance"] = step
        state.execution_context["ids"] = step.expire_effect_ids
        if kind == "append-id":
            state.execution_context["target"] = value.id
            state.execution_stack.append(
                RecordTargetStep(
                    step_id="fixture/turn-boundary/append",
                    input_key="target",
                    output_list_key="ids",
                )
            )
        yield "turn-boundary-direct-" + kind, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance"},
        ]

    for kind in ("one", "all", "all-current", "all-trigger"):
        state = prepare(1)
        state.execution_context["played"] = state.get_hero("hero_arien").played_cards
        if kind == "all-current":
            hero = state.get_hero("hero_arien")
            hero.play_card(hero.hand[0])
            state.execution_context["current"] = hero.current_turn_card
        if kind == "all-trigger":
            value = effect(
                state,
                "after-auto-pass",
                EffectType.AFTER_CARDS_PLAYED_TRIGGER,
                source="hero_arien",
            )
            value.duration = DurationType.NEXT_TURN
            value.created_at_round, value.created_at_turn = state.round, 1
            value.finishing_steps = [flag("after_auto_pass")]
            state.active_effects = [value]
            state.execution_context["trigger"] = value
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        for hero in heroes[:1] if kind == "one" else heroes:
            for card in list(hero.hand):
                hero.discard_card(card)
        ops = [{"kind": "end_turn"}]
        if kind == "one":
            ops += [{"kind": "advance"}]
        elif kind == "all-trigger":
            ops += [{"kind": "advance_prefix", "steps": 1} for _ in range(5)]
        else:
            ops += [{"kind": "advance_prefix", "steps": 1}]
        yield "turn-boundary-auto-pass-" + kind, state, ops

    for kind in ("flag", "token", "same-object", "after-cards"):
        state = prepare(4)
        value = add_effect(state, kind)
        value.duration = DurationType.THIS_ROUND
        state.phase = GamePhase.CLEANUP
        state.execution_stack.append(EndPhaseStep(step_id="fixture/turn-boundary/end-phase"))
        ops = [{"kind": "advance_prefix", "steps": 1}]
        if kind in ("flag", "same-object"):
            ops += [
                {"kind": "advance_prefix", "steps": 1} for _ in range(3 if kind == "flag" else 6)
            ]
        yield "turn-boundary-end-phase-" + kind, state, ops


def revelation_scenes(base):
    """Isolated phase primitives using real registered hero/card models."""
    from combat_effect_traces import effect

    from goa2.domain.models import GamePhase, TeamColor
    from goa2.domain.models.effect import DurationType, EffectType
    from goa2.engine.steps import (
        FindNextActorStep,
        ResolveCardStep,
        ResolveTieBreakerStep,
        RetrieveUnresolvedCardStep,
    )
    from goa2.engine.steps.utility import (
        FlipTieBreakerCoinStep,
        RecordTargetStep,
        SetContextFlagStep,
    )

    def roster(state):
        return [hero for team in state.teams.values() for hero in team.heroes]

    def prepared(red=None):
        state = base(red)
        state.phase = GamePhase.PLANNING
        state.current_actor_id = None
        state.resolution_owner_id = None
        state.execution_stack = []
        state.execution_context = {
            "old_pending": state.pending_inputs,
            "old_second": state.pending_second_cards,
            "old_done": state.planning_done,
            "old_unresolved": state.unresolved_hero_ids,
            "old_revealed": state.public_revealed_cards,
        }
        return state

    def commit(state, hero, number=0, second=False):
        card = hero.hand[number]
        hero.play_card(card)
        (state.pending_second_cards if second else state.pending_inputs)[hero.id] = card
        return card

    for variant in (
        "basic",
        "known-card",
        "unknown-player",
        "passed-current",
        "initiative-override",
        "no-candidates",
    ):
        state = prepared()
        heroes = roster(state)
        for hero in heroes:
            card = commit(state, hero)
            state.execution_context["card_" + hero.id] = card
        if variant == "known-card":
            state.record_public_revealed_card(heroes[0].id, heroes[0].current_turn_card.id)
            state.execution_context["old_revealed"] = state.public_revealed_cards
            state.execution_context["known"] = state.public_revealed_cards[0]
        if variant == "unknown-player":
            state.pending_inputs["not-a-player"] = heroes[0].current_turn_card
        if variant == "passed-current":
            state.pending_inputs[heroes[0].id] = None
            heroes[0].current_turn_card.initiative_override = 19
            state.execution_context["played"] = heroes[0].played_cards
        if variant == "initiative-override":
            heroes[1].current_turn_card.initiative_override = 40
        if variant == "no-candidates":
            state.pending_inputs.clear()
            state.pending_inputs["not-a-player"] = heroes[0].current_turn_card
        yield "revelation-" + variant, state, [{"kind": "start_revelation"}]

    for variant in (
        "first",
        "second",
        "invalid",
        "same-object",
        "trigger-after",
        "old-history",
    ):
        state = prepared(["Emmitt"])
        hero = state.get_hero("hero_emmitt")
        hero.level = 8
        first = commit(state, hero)
        second = commit(state, hero, second=True)
        if variant == "same-object":
            second = first
            state.pending_second_cards[hero.id] = first
        first.initiative_override = 23
        for other in roster(state):
            if other is not hero:
                commit(state, other)
        if variant == "trigger-after":
            state.turn = 2
            value = effect(
                state, "reveal-trigger", EffectType.AFTER_CARDS_PLAYED_TRIGGER, source=hero.id
            )
            value.duration = DurationType.NEXT_TURN
            value.created_at_round, value.created_at_turn = state.round, 1
            step = SetContextFlagStep(
                step_id="fixture/revelation/hand", key="returned_hand", value=[]
            )
            step.value = hero.hand
            value.finishing_steps = [step]
            state.active_effects = [value]
            state.execution_context["trigger"] = value
        if variant == "old-history":
            state.record_public_revealed_card(hero.id, first.id)
            state.execution_context["old_revealed"] = state.public_revealed_cards
            state.execution_context["known"] = state.public_revealed_cards[0]
        state.execution_context.update({"first": first, "second": second, "hand": hero.hand})
        chosen = second.id if variant == "second" else first.id
        ops = [{"kind": "start_revelation"}, {"kind": "advance_prefix", "steps": 1}]
        if variant == "invalid":
            ops += [
                {"kind": "input", "selection": "not-a-card"},
                {"kind": "advance_prefix", "steps": 1},
            ]
        ops += [{"kind": "input", "selection": chosen}, {"kind": "advance_prefix", "steps": 1}]
        if variant == "trigger-after":
            ops += [{"kind": "advance_prefix", "steps": 1} for _ in range(4)]
        else:
            ops += [{"kind": "advance_prefix", "steps": 1}]
        yield "revelation-dual-" + variant, state, ops

    for variant in (
        "normal",
        "reversed",
        "inactive",
        "stale",
        "cross-round",
        "token-passive",
        "no-card",
        "unknown",
        "duplicate-pool",
    ):
        state = prepared()
        state.phase = GamePhase.RESOLUTION
        for number, hero in enumerate(roster(state)):
            card = commit(state, hero)
            card.is_facedown = False
            card.initiative_override = 5 - number * 3
        state.pending_inputs = {}
        state.unresolved_hero_ids = [hero.id for hero in roster(state)]
        if variant in ("reversed", "inactive", "stale", "cross-round", "token-passive"):
            state.turn = 2
            value = effect(
                state, "reverse", EffectType.REVERSED_INITIATIVE, source=roster(state)[0].id
            )
            value.duration = DurationType.NEXT_TURN
            value.created_at_round, value.created_at_turn = state.round, 1
            if variant == "inactive":
                value.is_active = False
            if variant == "stale":
                value.created_at_turn = 0
            if variant == "cross-round":
                value.created_at_round = state.round + 1
            if variant == "token-passive":
                value.token_id = "a-token"
                value.duration = DurationType.PASSIVE
            state.active_effects = [value]
        if variant == "no-card":
            for hero in roster(state):
                hero.current_turn_card = None
        if variant == "unknown":
            state.unresolved_hero_ids = ["not-a-player"]
        if variant == "duplicate-pool":
            state.unresolved_hero_ids = [roster(state)[0].id] * 2
        state.execution_context["pool"] = state.unresolved_hero_ids
        state.execution_stack = [FindNextActorStep(step_id="fixture/revelation/find")]
        yield "revelation-rank-" + variant, state, [{"kind": "advance_prefix", "steps": 1}]

    for variant in (
        "cross",
        "same-team",
        "favored-choice",
        "wrong-choice",
        "pending-manual",
        "append-candidate",
        "empty",
        "piece-owner",
    ):
        state = prepared(["Arien", "Razzle"] if variant == "piece-owner" else ["Arien", "Hanu"])
        state.phase = GamePhase.RESOLUTION
        heroes = roster(state)
        for hero in heroes:
            card = commit(state, hero)
            card.is_facedown = False
            card.initiative_override = 5
        state.pending_inputs = {}
        if variant == "cross":
            tied = [heroes[0].id, heroes[-1].id]
        elif variant == "empty":
            tied = []
        elif variant == "piece-owner":
            tied = ["hero_razzle"]
        else:
            tied = [heroes[0].id, heroes[1].id]
        if variant in ("favored-choice", "wrong-choice"):
            tied.append(heroes[-1].id)
            state.tie_breaker_team = TeamColor.RED
        step = ResolveTieBreakerStep(step_id="fixture/revelation/tie", tied_hero_ids=tied)
        state.unresolved_hero_ids = list(tied)
        state.execution_stack = [step]
        state.execution_context.update(
            {"tie": step, "ties": step.tied_hero_ids, "pool": state.unresolved_hero_ids}
        )
        if variant == "pending-manual":
            step.pending_input = {"selection": heroes[0].id}
            step.pending_request_id = "fixture/retained-request"
        if variant == "append-candidate":
            state.tie_breaker_team = TeamColor.BLUE
            state.execution_context["target"] = heroes[-1].id
            state.execution_stack.append(
                RecordTargetStep(
                    step_id="fixture/revelation/append",
                    input_key="target",
                    output_list_key="ties",
                )
            )
        ops = [{"kind": "advance_prefix", "steps": 1}]
        if variant == "append-candidate":
            ops += [{"kind": "advance_prefix", "steps": 1}]
        elif variant in ("same-team", "favored-choice", "wrong-choice"):
            if variant == "wrong-choice":
                ops += [
                    {"kind": "input", "selection": heroes[-1].id},
                    {"kind": "advance_prefix", "steps": 1},
                ]
            ops += [
                {"kind": "input", "selection": heroes[0].id},
                {"kind": "advance_prefix", "steps": 1},
            ]
        yield "revelation-tie-" + variant, state, ops

    for variant in (
        "first",
        "second",
        "dirty-request",
        "invalid-pending",
        "no-extra",
        "missing-hero",
        "override",
    ):
        state = prepared()
        state.phase = GamePhase.RESOLUTION
        hero = roster(state)[0]
        first = commit(state, hero)
        second = commit(state, hero, second=True)
        hero.current_turn_card, hero.extra_turn_card = first, second
        first.is_facedown = second.is_facedown = False
        first.initiative_override = 30
        state.pending_inputs = {}
        state.pending_second_cards = {}
        step = RetrieveUnresolvedCardStep(step_id="fixture/revelation/retrieve", hero_id=hero.id)
        if variant == "no-extra":
            hero.extra_turn_card = None
        if variant == "missing-hero":
            step.hero_id = "not-a-player"
        if variant in ("dirty-request", "override"):
            step.pending_input = {"selection": first.id}
            step.pending_request_id = "fixture/dirty-retrieval"
        if variant == "invalid-pending":
            step.pending_input = {"selection": "bad"}
            step.pending_request_id = "fixture/retained-retrieval"
        state.execution_stack = [step]
        state.execution_context = {
            "step": step,
            "first": first,
            "second": second,
            "hand": hero.hand,
            "played": hero.played_cards,
        }
        if variant == "override":
            state.execution_context["card_copy"] = copy.deepcopy(first)
        ops = [{"kind": "advance_prefix", "steps": 1}]
        if variant in ("first", "second", "invalid-pending"):
            ops += [
                {"kind": "input", "selection": second.id if variant == "second" else first.id},
                {"kind": "advance_prefix", "steps": 1},
            ]
        yield "revelation-retrieve-" + variant, state, ops

    for variant in ("open", "done", "two"):
        state = prepared(["Emmitt"])
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_emmitt")
        hero.level = 8
        first = commit(state, hero)
        if variant == "done":
            state.planning_done.append(hero.id)
        if variant == "two":
            commit(state, hero, second=True)
        for other in roster(state):
            if other is not hero:
                for card in list(other.hand):
                    other.discard_card(card)
        state.execution_context["first"] = first
        ops = [{"kind": "end_turn"}]
        if variant == "two":
            ops.append({"kind": "advance_prefix", "steps": 1})
        yield "revelation-auto-pass-" + variant, state, ops

    state = prepared()
    state.phase = GamePhase.RESOLUTION
    state.execution_stack = [FlipTieBreakerCoinStep(step_id="fixture/revelation/flip")]
    yield "revelation-flip", state, [{"kind": "advance"}]
    state = prepared()
    hero = roster(state)[0]
    state.execution_stack = [
        ResolveCardStep(
            step_id="fixture/revelation/skipped-action",
            hero_id=hero.id,
            active_if_key="missing",
        )
    ]
    yield "revelation-skipped-parked-action", state, [{"kind": "advance"}]


def planning_command_scenes(base):
    """Public planning commands, including retained collection identities."""
    from goa2.domain.models import GamePhase

    for variant in (
        "pass",
        "pass-requests",
        "pass-nonempty",
        "pass-unknown",
        "pass-duplicate",
        "pass-wrong-phase",
        "finish",
        "finish-missing",
        "finish-unknown-pending",
        "finish-wrong-phase",
        "finish-last",
        "finish-dual-last",
        "pass-last",
    ):
        state = base(["Emmitt"] if variant == "finish-dual-last" else None)
        state.phase = GamePhase.PLANNING
        state.current_actor_id = None
        state.resolution_owner_id = None
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        hero = heroes[0]
        state.execution_context = {
            "pending": state.pending_inputs,
            "done": state.planning_done,
            "requests": state.starting_position_requests,
            "hand": hero.hand,
        }
        kind = "pass" if variant.startswith("pass") else "finish"
        target = str(hero.id)
        if kind == "pass" and variant != "pass-nonempty":
            for card in list(hero.hand):
                hero.discard_card(card)
        if variant == "pass-requests":
            state.starting_position_requests.update(
                {
                    "outgoing": {"from": str(hero.id), "to": str(heroes[1].id)},
                    "incoming": {"from": str(heroes[1].id), "to": str(hero.id)},
                    "keep": {"from": str(heroes[1].id), "to": str(heroes[2].id)},
                }
            )
            state.execution_context["kept_request"] = state.starting_position_requests["keep"]
        if variant == "pass-duplicate":
            state.pending_inputs[hero.id] = None
        if variant in ("pass-unknown", "finish-unknown-pending"):
            target = "not-a-player"
        if variant in ("pass-wrong-phase", "finish-wrong-phase"):
            state.phase = GamePhase.RESOLUTION
        if kind == "finish" and variant not in ("finish-missing", "finish-wrong-phase"):
            if variant == "finish-unknown-pending":
                state.pending_inputs[target] = None
            else:
                card = hero.hand[0]
                hero.play_card(card)
                state.pending_inputs[hero.id] = card
                if variant == "finish-dual-last":
                    hero.level = 8
        if variant in ("finish-last", "finish-dual-last", "pass-last"):
            for other in heroes[1:]:
                card = other.hand[0]
                other.play_card(card)
                state.pending_inputs[other.id] = card
        operations = [{"kind": kind, "hero": target}]
        if variant in ("pass", "finish", "finish-unknown-pending"):
            operations.append({"kind": kind, "hero": target})
        yield "planning-command-" + variant, state, operations


def card_planning_scenes(base):
    """Commit/take-back phase APIs with registered cards and live aliases."""
    from state_codec import export_state_with_instances

    from goa2.domain.models import GamePhase

    variants = (
        "cycle",
        "wrong-phase",
        "unknown",
        "foreign",
        "duplicate",
        "passed",
        "missing",
        "requests",
        "override-copy",
        "shared-deck",
        "shared-hero-hand",
        "last",
        "dual-cycle",
        "dual-done",
        "dual-low-level",
        "dual-first-last",
        "dual-second-last",
        "dual-last-card",
        "dual-requests",
        "duplicate-done",
        "spell-foreign",
        "uncommit-twice",
        "aliased-buffers",
    )
    for variant in variants:
        red = (
            ["Emmitt"]
            if variant.startswith("dual")
            else (["Gydion"] if variant.startswith("spell") else None)
        )
        state = base(red)
        state.phase = GamePhase.PLANNING
        state.current_actor_id = None
        state.resolution_owner_id = None
        state.execution_stack = []
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        hero = heroes[0]
        if variant.startswith("dual"):
            hero.level = 7 if variant == "dual-low-level" else 8
        first, second, third = hero.hand[:3]
        state.execution_context = {
            "hand": hero.hand,
            "pending": state.pending_inputs,
            "second": state.pending_second_cards,
            "done": state.planning_done,
            "requests": state.starting_position_requests,
            "first": first,
            "second_card": second,
        }

        # Store actual cards until all context/model references have been added;
        # physical table positions depend on the complete initial object graph.
        def op(kind, card=None, target=None, hero_id=str(hero.id)):
            value = {"kind": kind, "hero": hero_id if target is None else target}
            if card is not None:
                value["card"] = card
            return value

        operations = [op("commit", first), op("uncommit"), op("commit", second)]
        if variant == "wrong-phase":
            state.phase = GamePhase.RESOLUTION
            operations = [op("commit", first), op("uncommit")]
        elif variant == "unknown":
            operations = [
                op("commit", first, "not-a-player"),
                op("uncommit", target="not-a-player"),
            ]
        elif variant == "foreign":
            operations = [op("commit", heroes[1].hand[0])]
        elif variant == "duplicate":
            operations = [op("commit", first), op("commit", second), op("uncommit")]
        elif variant == "passed":
            state.pending_inputs[hero.id] = None
            operations = [op("commit", first), op("uncommit")]
        elif variant == "missing":
            operations = [op("uncommit")]
        elif variant in ("requests", "dual-requests"):
            if variant == "dual-requests":
                hero.play_card(first)
                state.pending_inputs[hero.id] = first
                operations = [op("commit", second), op("uncommit"), op("uncommit")]
            state.starting_position_requests.update(
                {
                    "outgoing": {"from": str(hero.id), "to": str(heroes[1].id)},
                    "incoming": {"from": str(heroes[1].id), "to": str(hero.id)},
                    "keep": {"from": str(heroes[1].id), "to": str(heroes[-1].id)},
                }
            )
            state.execution_context["kept_request"] = state.starting_position_requests["keep"]
        elif variant == "override-copy":
            first.initiative_override = 19
            state.execution_context["copy"] = copy.deepcopy(first)
            operations = [op("commit", first), op("uncommit")]
        elif variant == "shared-deck":
            hero.deck = hero.hand
            operations = [op("commit", first), op("uncommit")]
        elif variant == "shared-hero-hand":
            heroes[1].hand = hero.hand
            operations = [
                op("commit", first),
                op("uncommit"),
                op("commit", first, str(heroes[1].id)),
            ]
        elif variant in ("last", "dual-first-last", "dual-second-last", "dual-last-card"):
            for other in heroes[1:]:
                card = other.hand[0]
                other.play_card(card)
                state.pending_inputs[other.id] = card
            if variant == "dual-last-card":
                for card in list(hero.hand)[1:]:
                    hero.discard_card(card)
            operations = [op("commit", first)]
            if variant == "dual-first-last":
                operations.append(op("finish"))
            elif variant == "dual-second-last":
                operations.extend([op("commit", second), {"kind": "advance"}])
            elif variant == "last":
                operations.append(op("uncommit"))
        elif variant == "dual-cycle":
            operations = [
                op("commit", first),
                op("commit", second),
                op("commit", third),
                op("uncommit"),
                op("uncommit"),
            ]
        elif variant in ("dual-done", "dual-low-level"):
            operations = [op("commit", first)]
            if variant == "dual-done":
                operations.append(op("finish"))
            operations.extend([op("commit", second), op("uncommit")])
        elif variant == "duplicate-done":
            hero.play_card(first)
            state.pending_inputs[hero.id] = first
            state.planning_done.extend([str(hero.id), str(hero.id)])
            operations = [op("uncommit")]
        elif variant == "spell-foreign":
            operations = [op("commit", hero.spells[0])]
        elif variant == "uncommit-twice":
            operations = [op("commit", first), op("uncommit"), op("uncommit")]
        elif variant == "aliased-buffers":
            state.pending_second_cards = state.pending_inputs
            state.execution_context["second"] = state.pending_second_cards
            operations = [op("commit", first), op("uncommit")]
        _, physical = export_state_with_instances(state)
        for operation in operations:
            if "card" in operation:
                operation["card"] = next(
                    index for index, value in enumerate(physical) if value is operation["card"]
                )
        yield "card-planning-" + variant, state, operations


def confirmation_scenes(base):
    """Live confirmation, controller ownership and real abort boundaries."""
    from goa2.domain.models import GamePhase, TargetType
    from goa2.domain.models.effect import (
        ActiveEffect,
        DurationType,
        EffectScope,
        EffectType,
        Shape,
    )
    from goa2.engine.steps import (
        ConfirmResolutionStep,
        FinalizeHeroTurnStep,
        ForEachStep,
        SelectStep,
    )
    from goa2.engine.steps.utility import SetContextFlagStep

    def prepared(red=None, owner="hero_arien"):
        state = base(red)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_razzle" if red else "hero_arien"
        state.resolution_owner_id = state.current_actor_id
        step = ConfirmResolutionStep(step_id="fixture/confirmation/step", hero_id=owner)
        state.execution_stack = [step]
        state.execution_context = {"confirmation": step, "same_step": step}
        return state, step

    def answer():
        return [
            {"kind": "advance"},
            {"kind": "input", "selection": "CONFIRM"},
            {"kind": "advance"},
        ]

    for variant in (
        "basic",
        "retry",
        "stale",
        "retained-valid",
        "retained-invalid",
        "empty-mapping",
        "empty-list",
        "unknown-owner",
        "piece-owner",
        "skipped",
    ):
        state, step = prepared(
            ["Razzle"] if variant == "piece-owner" else None,
            (
                "hero_razzle_piece_1"
                if variant == "piece-owner"
                else ("not-a-player" if variant == "unknown-owner" else "hero_arien")
            ),
        )
        operations = answer()
        if variant == "retry":
            operations = [{"kind": "advance"}]
            for selection in ("NO", False, [], {"wrong": 1}, "CONFIRM"):
                operations.extend([{"kind": "input", "selection": selection}, {"kind": "advance"}])
        elif variant == "stale":
            operations.insert(
                1, {"kind": "input", "selection": "CONFIRM", "request_id": "stale-confirm"}
            )
        elif variant.startswith("retained"):
            step.pending_input = {"selection": "CONFIRM" if variant == "retained-valid" else "NO"}
            step.pending_request_id = "fixture/confirmation/retained-request"
            state.execution_context["old_pending"] = step.pending_input
            if variant == "retained-valid":
                operations = [{"kind": "advance"}]
        elif variant in ("empty-mapping", "empty-list"):
            step.pending_input = {} if variant == "empty-mapping" else []
            state.execution_context["old_pending"] = step.pending_input
        elif variant == "skipped":
            step.active_if_key = "not-set"
            step.pending_input = {"selection": "NO"}
            operations = [{"kind": "advance"}]
        yield "confirmation-" + variant, state, operations

    for flag in ("rollback_frozen", "rollback_reanchor_pending"):
        for index, value in enumerate((True, 0, {}, {"yes": 1}, [], [1])):
            state, step = prepared()
            step.pending_input = {"selection": "NO"}
            step.pending_request_id = "fixture/confirmation/frozen-request"
            state.execution_context["old_pending"] = step.pending_input
            state.execution_context[flag] = value
            yield "confirmation-" + flag + "-" + str(index), state, (
                [{"kind": "advance"}] if value else answer()
            )

    for variant in ("controlled", "inactive", "foreign-owner", "foreign-prompt"):
        state, step = prepared(owner="hero_brogan" if variant == "foreign-prompt" else "hero_arien")
        hero = state.get_hero("hero_arien")
        hero.current_turn_card = hero.hand[1]
        state.active_effects = [
            ActiveEffect(
                id="confirmation-control",
                source_id="hero_hanu",
                effect_type=EffectType.CONTROL_NEXT_ACTION,
                scope=EffectScope(shape=Shape.GLOBAL, origin_id="hero_arien"),
                duration=DurationType.PASSIVE,
                is_active=variant != "inactive",
                created_at_round=state.round,
                created_at_turn=state.turn,
                controlled_card_id=hero.current_turn_card.id,
            )
        ]
        if variant == "foreign-owner":
            state.resolution_owner_id = "hero_hanu"
        yield "confirmation-" + variant, state, answer()

    for variant in ("confirm", "finalize", "frozen-confirm", "scope-confirm"):
        state, step = prepared()
        boundary = (
            step
            if variant != "finalize"
            else FinalizeHeroTurnStep(step_id="fixture/confirmation/finalize", hero_id="hero_arien")
        )
        if variant == "frozen-confirm":
            state.execution_context["rollback_frozen"] = True
        steps = [
            boundary,
            SetContextFlagStep(
                step_id="fixture/confirmation/skipped-after-abort",
                key="should_not_run",
                value=True,
            ),
            SelectStep(
                step_id="fixture/confirmation/fail",
                target_type=TargetType.NUMBER,
                number_options=[],
                prompt="No legal selection",
            ),
        ]
        state.execution_stack = steps
        state.execution_context["boundary"] = boundary
        if variant == "scope-confirm":
            from goa2.domain.models import ActionType
            from goa2.engine.steps.phases import push_action_context

            state.execution_context["current_card_id"] = "outer"
            push_action_context(
                state.execution_context,
                action_type=ActionType.SKILL,
                card_id="inner",
                card_owner_id="hero_arien",
            )
        operations = (
            [{"kind": "advance_prefix", "steps": 1}]
            if variant == "finalize"
            else ([{"kind": "advance"}] if variant == "frozen-confirm" else answer())
        )
        yield "confirmation-abort-" + variant, state, operations

    state, step = prepared()
    state.execution_context["items"] = [1, 2]
    state.execution_stack = [
        ForEachStep(
            step_id="fixture/confirmation/loop",
            list_key="items",
            item_key="item",
            steps_template=[step],
        )
    ]
    yield "confirmation-loop", state, [
        *answer(),
        {"kind": "input", "selection": "CONFIRM"},
        {"kind": "advance"},
    ]

    state, step = prepared()
    state.execution_stack = [step, step]
    yield "confirmation-repeated-step", state, [
        *answer(),
        {"kind": "input", "selection": "CONFIRM"},
        {"kind": "advance"},
    ]


def finalization_scenes(base):
    """Exact hero-turn completion before staged minion/lane consumers."""
    from combat_effect_traces import effect

    from goa2.domain.models import CardState, GamePhase
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import FinalizeHeroTurnStep
    from goa2.engine.steps.utility import SetContextFlagStep

    variants = (
        "basic",
        "no-current",
        "existing-slot",
        "turn-four",
        "overwrite",
        "positive-uses",
        "nonpositive-uses",
        "effects",
        "duplicate-effect-ids",
        "shared-effect",
        "own-own",
        "own-foreign",
        "foreign-own",
        "foreign-foreign",
        "empty-reveals",
        "missing-owner",
        "unknown-hero",
        "piece-id",
        "piece-effects",
        "shared-played",
        "duplicate-print",
        "override",
        "facedown",
        "high-resolved-count",
        "extra-card",
        "retained-context",
        "repeated",
        "skipped",
    )
    for variant in variants:
        piece_variant = variant in ("piece-id", "piece-effects")
        state = base(
            ["Razzle"] if piece_variant else (["Emmitt"] if variant == "extra-card" else None)
        )
        state.phase = GamePhase.RESOLUTION
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        hero = heroes[0]
        state.current_actor_id = str(hero.id)
        state.resolution_owner_id = str(hero.id)
        state.acting_piece_id = "hero_razzle_piece_1" if piece_variant else None
        target = (
            "not-a-player"
            if variant == "unknown-hero"
            else ("hero_razzle_piece_1" if piece_variant else str(hero.id))
        )
        card, previous = hero.hand[:2]
        if variant in ("overwrite", "positive-uses", "nonpositive-uses", "existing-slot"):
            state.turn = 2
            hero.play_card(previous)
            previous.is_facedown = False
            hero.resolve_current_card(
                turn_number=2 if variant in ("overwrite", "existing-slot") else 1
            )
        if variant not in ("no-current", "existing-slot"):
            hero.play_card(card)
            card.is_facedown = variant == "facedown"
        if variant == "no-current":
            state.turn = 3
        if variant == "turn-four":
            state.turn = 4
        if variant == "positive-uses":
            card.passive_uses_this_turn = 3
            previous.passive_uses_this_turn = 2
            hero.ultimate_card.passive_uses_this_turn = 4
            hero.hand[0].passive_uses_this_turn = 5
        if variant == "nonpositive-uses":
            card.passive_uses_this_turn = -2
            previous.passive_uses_this_turn = 0
            hero.ultimate_card.passive_uses_this_turn = -1
        if variant == "high-resolved-count":
            hero.resolved_turn_count = 4
        if variant == "override":
            card.initiative_override = 19
        if variant == "extra-card":
            hero.level = 8
            hero.play_card(previous)
            hero.current_turn_card = card
            hero.extra_turn_card = previous
            previous.passive_uses_this_turn = 5
        if variant == "shared-played":
            heroes[1].played_cards = hero.played_cards
        if variant == "duplicate-print":
            duplicate = copy.deepcopy(card)
            duplicate.state = CardState.HAND
            duplicate.is_facedown = False
            hero.hand.append(duplicate)
        if variant in ("effects", "duplicate-effect-ids", "shared-effect", "piece-effects"):
            for index in range(4):
                item = effect(
                    state,
                    (
                        "finalize-effect"
                        if variant == "duplicate-effect-ids"
                        else "finalize-effect-" + str(index)
                    ),
                    EffectType.DELAYED_TRIGGER,
                    source=str(hero.id) if index % 2 == 0 else str(heroes[1].id),
                    source_card_id=card.id if index < 2 else previous.id,
                )
                item.is_active = False
                item.passive_uses_this_turn = index + 2
                state.active_effects.append(item)
            if variant == "shared-effect":
                state.active_effects.append(state.active_effects[0])
        if variant in ("own-own", "own-foreign", "foreign-own", "foreign-foreign"):
            guess, reveal = variant.split("-")
            state.card_guess = {
                "guesser_id": str(hero.id) if guess == "own" else str(heroes[1].id),
                "note": "keep original mapping",
            }
            state.card_reveal = {
                "revealer_id": str(hero.id) if reveal == "own" else str(heroes[1].id),
                "note": "keep original mapping",
            }
        if variant == "empty-reveals":
            state.card_guess = {}
            state.card_reveal = {}
        if variant == "missing-owner":
            state.card_guess = {"note": "no owner"}
            state.card_reveal = {"note": "no owner"}
        if variant in (
            "own-own",
            "own-foreign",
            "foreign-own",
            "foreign-foreign",
            "empty-reveals",
            "missing-owner",
        ):
            keeper = SetContextFlagStep(
                step_id="fixture/finalization/reveal-keeper", key="old_reveals", value=None
            )
            keeper.value = [state.card_guess, state.card_reveal]
            item = effect(
                state,
                "finalize-reveal-keeper",
                EffectType.DELAYED_TRIGGER,
                source=str(heroes[1].id),
            )
            item.finishing_steps = [keeper]
            state.active_effects.append(item)
        if variant == "piece-effects":
            state.card_guess = {"guesser_id": str(hero.id)}
            state.card_reveal = {"revealer_id": target}
        step = FinalizeHeroTurnStep(step_id="fixture/finalization/step", hero_id=target)
        state.execution_stack = [step]
        state.execution_context = {
            "finalizer": step,
            "played": hero.played_cards,
            "card": card,
            "previous": previous,
            "old_guess": state.card_guess,
            "old_reveal": state.card_reveal,
        }
        operations = [{"kind": "advance_prefix", "steps": 1}]
        if variant == "retained-context":
            keeper = SetContextFlagStep(
                step_id="fixture/finalization/context-keeper", key="later", value=None
            )
            keeper.value = state.execution_context
            item = effect(
                state,
                "finalize-context-keeper",
                EffectType.DELAYED_TRIGGER,
                source=str(hero.id),
            )
            item.finishing_steps = [keeper]
            state.active_effects.append(item)
        if variant == "repeated":
            item = effect(state, "finalize-repeat", EffectType.DELAYED_TRIGGER, source=str(hero.id))
            item.finishing_steps = [step]
            state.active_effects.append(item)
            step.pending_input = {"selection": "retained data"}
            step.pending_request_id = "fixture/finalization/retained-request"
            operations.extend(
                [
                    {"kind": "queue_steps", "path": ["active_effects", 0, "finishing_steps"]},
                    {"kind": "advance_prefix", "steps": 1},
                ]
            )
        if variant == "skipped":
            step.active_if_key = "not-set"
        yield "finalization-" + variant, state, operations


def piece_planning_scenes(base):
    """get_hero resolves piece IDs, while phase buffers use raw caller keys."""
    from state_codec import export_state_with_instances

    from goa2.domain.models import GamePhase

    for variant in ("cycle", "pass", "last", "buffers"):
        state = base(["Razzle"])
        state.phase = GamePhase.PLANNING
        state.current_actor_id = None
        state.resolution_owner_id = None
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        hero = heroes[0]
        piece = "hero_razzle_piece_1"
        card = hero.hand[0]
        state.execution_context = {
            "hand": hero.hand,
            "pending": state.pending_inputs,
            "done": state.planning_done,
            "card": card,
        }
        if variant == "pass":
            for item in list(hero.hand):
                hero.discard_card(item)
            operations = [
                {"kind": "pass", "hero": piece},
                {"kind": "finish", "hero": piece},
                {"kind": "uncommit", "hero": piece},
            ]
        elif variant == "last":
            for other in heroes[1:]:
                other_card = other.hand[0]
                other.play_card(other_card)
                state.pending_inputs[other.id] = other_card
            operations = [{"kind": "commit", "hero": piece, "card": card}]
        elif variant == "buffers":
            operations = [
                {"kind": "commit", "hero": piece, "card": card},
                {"kind": "finish", "hero": str(hero.id)},
                {"kind": "finish", "hero": piece},
                {"kind": "uncommit", "hero": str(hero.id)},
                {"kind": "uncommit", "hero": piece},
            ]
        else:
            operations = [
                {"kind": "commit", "hero": piece, "card": card},
                {"kind": "uncommit", "hero": piece},
                {"kind": "commit", "hero": str(hero.id), "card": card},
                {"kind": "uncommit", "hero": piece},
            ]
        _, physical = export_state_with_instances(state)
        for operation in operations:
            if "card" in operation:
                operation["card"] = next(
                    index for index, value in enumerate(physical) if value is operation["card"]
                )
        yield "piece-planning-" + variant, state, operations


def reference(state, operations, unordered_keys=()):
    from state_codec import export_state_with_instances, materialize

    from goa2.domain.input import InputResponse
    from goa2.engine.handler import process_stack, submit_input

    request = None
    records = []
    normalizer = ObjectTraceNormalizer(unordered_keys)
    initial_envelope, original_objects = export_state_with_shared_objects(state)
    normalizer.bind_originals(initial_envelope["object_bindings"], original_objects)
    physical = (
        export_state_with_instances(state)[1]
        if any(op["kind"] == "commit" for op in operations)
        else ()
    )
    for op in operations:
        current, events, error = None, [], None
        if op["kind"] in ("advance", "advance_prefix"):
            boundary = None
            if op["kind"] == "advance_prefix":
                remaining = op["steps"]

                def boundary(_state, _step):
                    nonlocal remaining
                    if remaining == 0:
                        return True
                    remaining -= 1
                    return False

            result = process_stack(state, stop_before_step=boundary)
            request = result.input_request
            current = request.model_dump(mode="json") if request else None
            events = [event.model_dump(mode="json") for event in result.events]
        elif op["kind"] in ("commit", "uncommit", "pass", "finish"):
            from goa2.engine.phases import (
                commit_card,
                finish_planning,
                pass_turn,
                uncommit_card,
            )

            try:
                if op["kind"] == "commit":
                    commit_card(state, op["hero"], physical[op["card"]])
                else:
                    {"uncommit": uncommit_card, "pass": pass_turn, "finish": finish_planning}[
                        op["kind"]
                    ](state, op["hero"])
            except ValueError:
                error = "invalid_planning_action"
        elif op["kind"] == "start_revelation":
            from goa2.engine.phases import start_revelation_phase

            start_revelation_phase(state)
        elif op["kind"] == "next_actor":
            from goa2.engine.phases import resolve_next_action

            resolve_next_action(state)
        elif op["kind"] == "end_turn":
            from goa2.engine.phases import end_turn

            end_turn(state)
        elif op["kind"] == "after_cards_played":
            from goa2.engine.handler import push_steps
            from goa2.engine.phases import _collect_after_cards_played_steps

            push_steps(state, _collect_after_cards_played_steps(state))
        elif op["kind"] == "expire_turn_effects":
            from goa2.engine.effect_manager import EffectManager

            EffectManager.expire_active_turn_effects(state)
        elif op["kind"] == "expire_effects":
            from goa2.domain.models.effect import DurationType
            from goa2.engine.effect_manager import EffectManager

            EffectManager.expire_effects(state, DurationType(op["duration"]))
        elif op["kind"] == "remove_effects":
            from goa2.engine.effect_manager import EffectManager

            EffectManager.remove_effects(state, op["ids"])
        elif op["kind"] == "queue_steps":
            from pydantic import BaseModel

            from goa2.engine.handler import push_steps

            value = state
            for part in op["path"]:
                value = getattr(value, part) if isinstance(value, BaseModel) else value[part]
            push_steps(state, value)
        elif op["kind"] == "push_action_context":
            from goa2.domain.models import ActionType
            from goa2.engine.steps.phases import push_action_context

            push_action_context(
                state.execution_context,
                action_type=ActionType(op["action_type"]),
                card_id=op["card"],
                card_owner_id=op["owner"],
            )
        else:
            token = op.get("request_id", "current")
            if token == "current":
                if request is None:
                    raise ValueError("input without request")
                token = request.id
            try:
                submit_input(state, InputResponse(request_id=token, selection=op.get("selection")))
            except ValueError:
                error = "invalid_input"
        envelope, live_objects = export_state_with_shared_objects(state)
        records.append(
            normalizer.record(
                {
                    "semantic_state": materialize(envelope),
                    "object_bindings": envelope["object_bindings"],
                    "context_keys": list(state.execution_context),
                    "instance_bindings": instance_bindings(envelope["state"]),
                    "request": current,
                    "events": events,
                    "error": error,
                },
                live_objects,
            )
        )
    return records


def _export_scene_case(item):
    name, state, operations = item
    envelope, _ = export_state_with_shared_objects(state)
    keys = ("triggered_mine_ids", "chosen") if name.startswith("mines-choice-") else ()
    case = {
        "name": "object-execution-" + name,
        "envelope": envelope,
        "operations": operations,
        "expected": reference(state, operations, keys),
    }
    if keys:
        case["unordered_mine_keys"] = list(keys)
    return case


def _export_cases(items, workers):
    if workers < 1:
        raise ValueError("source export workers must be positive")
    if workers == 1:
        yield from map(_export_scene_case, items)
        return
    import multiprocessing
    import os
    from collections import deque
    from concurrent.futures import ProcessPoolExecutor

    cpus = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else os.cpu_count() or 1
    count = min(workers, cpus, 4)
    method = "fork" if "fork" in multiprocessing.get_all_start_methods() else "spawn"
    iterator = iter(items)
    with ProcessPoolExecutor(
        max_workers=count, mp_context=multiprocessing.get_context(method)
    ) as pool:
        pending = deque()
        for _ in range(count * 2):
            item = next(iterator, None)
            if item is None:
                break
            pending.append(pool.submit(_export_scene_case, item))
        while pending:
            yield pending.popleft().result()
            item = next(iterator, None)
            if item is not None:
                pending.append(pool.submit(_export_scene_case, item))


def export(path, *, workers=1):
    """Stream ordered goldens; optional workers cap CPU and in-flight snapshots.

    A caller using process workers must use the normal __main__ guard on spawn
    platforms. The default serial path supports every existing source fixture.
    """
    manifest = verify_sources()
    return write_corpus_stream(
        path,
        {
            "schema_version": 1,
            "oracle_revision": manifest["oracle_revision"],
            "source_sha256": manifest["source_sha256"],
        },
        _export_cases(scenes(), workers),
    )


def verify(path, command):
    manifest = verify_sources()
    data, identity = read_index(path)
    if data["source_sha256"] != manifest["source_sha256"]:
        raise ValueError("object execution oracle mismatch")
    worker = Worker(command)
    count = 0
    try:
        for entry in data["cases"]:
            case = read_case(path, entry)
            raw = worker.request(
                "stack_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            normalizer = ObjectTraceNormalizer(case.get("unordered_mine_keys", ()))
            actual = [normalizer.record(record) for record in raw]
            if actual != case["expected"]:
                dest = ROOT / "rust/artifacts/object-execution-mismatch.json"
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError(f"object execution mismatch in {case['name']}: {dest}")
            count += len(actual)
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "operations": count,
        "corpus_sha256": identity,
        "state_request_event_and_identity_parity": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/tests/fixtures/object-execution.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker command required after --")
        print(json.dumps(verify(args.corpus, command), indent=2))
