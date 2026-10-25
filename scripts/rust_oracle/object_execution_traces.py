"""Real source stack execution with shared steps and context object identity."""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

from oracle import ROOT, Worker, digest, verify_sources
from shared_objects import export_state_with_shared_objects, instance_bindings
from stack_traces import TraceNormalizer, read_case, read_index, write_corpus


class ObjectTraceNormalizer(TraceNormalizer):
    def bind_originals(self, bindings, objects):
        self.original_step_ids = {
            obj.step_id
            for binding, obj in zip(bindings, objects, strict=True)
            if binding["kind"] == "step"
        }

    def record(self, record, live_steps=None):
        record = copy.deepcopy(record)
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
                elif live_steps is None and str(value.get("step_id", "")).startswith(
                    "native-step-"
                ):
                    step_id = value["step_id"]
                    if step_id not in self.steps:
                        self.steps[step_id] = f"dynamic-step/{len(self.steps)+1}"
                    value["step_id"] = self.steps[step_id]
                value["pending_request_id"] = self.request_id(value["pending_request_id"])
        return record


def scenes():
    from combat_effect_traces import effect
    from model_traces import setup

    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase, TargetType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import SelectStep
    from goa2.engine.steps.utility import AddContextValueStep, RecordTargetStep, SetContextFlagStep

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
    yield "live-set-payload", state, [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance"}]
    state = base()
    record = RecordTargetStep(
        step_id="fixture/objects/new-record", input_key="target", output_list_key="history"
    )
    state.execution_stack = [record, record]
    state.execution_context = {"target": {"nested": [1]}}
    yield "new-shared-list", state, [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance"}]
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

    for variant in ("missing", "present-null", "empty-stack", "all-fields", "physical", "model"):
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
                    step_id="fixture/scope/actor", actor_id="hero_hanu", save_key="previous_actor"
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
        state.execution_stack = [select(), SetActorStep(step_id="fixture/scope/actor", **values)]
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
    state.execution_context = {"items": [1, 2], "card": card, "old_effects": state.active_effects}
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
            step_id="fixture/lifecycle/old-list", input_key="extra", output_list_key="old_effects"
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
        state.execution_context = {"current_card_id": "outer", "old_effects": state.active_effects}
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
    from goa2.engine.steps.utility import AddContextValueStep, RecordTargetStep, SetContextFlagStep

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
        ops = [{"kind": "end_turn"}, {"kind": "advance_prefix", "steps": 1}, {"kind": "advance"}]
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
                state, "after-auto-pass", EffectType.AFTER_CARDS_PLAYED_TRIGGER, source="hero_arien"
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


def reference(state, operations):
    from state_codec import materialize

    from goa2.domain.input import InputResponse
    from goa2.engine.handler import process_stack, submit_input

    request = None
    records = []
    normalizer = ObjectTraceNormalizer()
    initial_envelope, original_objects = export_state_with_shared_objects(state)
    normalizer.bind_originals(initial_envelope["object_bindings"], original_objects)
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


def export(path):
    manifest = verify_sources()
    cases = []
    for name, state, operations in scenes():
        envelope, _ = export_state_with_shared_objects(state)
        cases.append(
            {
                "name": "object-execution-" + name,
                "envelope": envelope,
                "operations": operations,
                "expected": reference(state, operations),
            }
        )
    data = {
        "schema_version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "cases": cases,
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


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
            normalizer = ObjectTraceNormalizer()
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
