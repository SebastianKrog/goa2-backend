"""Spellbook lifecycle boundaries with physical spell/step aliases.

Raw setup is deliberate: casting construction is checked separately from the
separately verified spell bodies, and retained collection/model identities are part
of the contract.
"""

from __future__ import annotations


def base(ally="Hanu"):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase

    state = setup(["Gydion", ally], ["Arien", "Tigerclaw"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.battle_zones = {}
    state.current_actor_id = "hero_gydion"
    for hero, point in (
        ("hero_gydion", h(0, 0)),
        ("hero_razzle_piece_1" if ally == "Razzle" else "hero_hanu", h(0, 1)),
        ("hero_arien", h(1, 0)),
        ("hero_tigerclaw", h(2, 0)),
    ):
        state.place_entity(hero, point)
    owner = state.get_hero("hero_gydion")
    for spell in owner.spells:
        spell.state = CardState.SPELLBOOK
        spell.is_facedown = True
    state.execution_context.update(
        owner_alias=owner, spells_alias=owner.spells, effects_alias=state.active_effects
    )
    return state, owner


def retain(state, step, ops):
    from goa2.engine.steps import SetContextFlagStep

    step.step_id = "fixture/spellbook-main"
    state.execution_context["spellbook_step"] = step
    state.execution_stack = [
        SetContextFlagStep(step_id="fixture/spellbook-tail", key="spellbook_tail", value=True),
        step,
    ]
    return state, ops


def cast_scene(variant):
    from goa2.domain.models import CardState
    from goa2.engine.steps import CastSpellStep

    state, owner = base("Razzle" if variant == "piece" else "Hanu")
    step = CastSpellStep(allowed_spell_ids=["magic_missile", "wish", "magic_missile"])
    if variant == "no-owner":
        owner.spells.clear()
    if variant == "spent":
        for spell in owner.spells:
            spell.state = CardState.OUTSIDE_SPELLBOOK
    if variant == "no-actor":
        state.current_actor_id = None
    if variant == "piece":
        step.caster_id = "hero_razzle_piece_1"
    if variant == "literal":
        step.caster_id = "hero_hanu"
    if variant == "key":
        step.caster_id = "hero_gydion"
        step.caster_key = "caster"
        state.execution_context["caster"] = "hero_hanu"
    if variant == "false-key":
        step.caster_id = "hero_hanu"
        step.caster_key = "caster"
        state.execution_context["caster"] = 0
    if variant == "unknown":
        step.caster_id = "absent"
    if variant == "empty":
        step.caster_id = ""
    if variant == "skip":
        step.skip_if_key = "owner_alias"
    if variant == "inactive":
        step.active_if_key = "absent"
    ops = [{"kind": "advance_prefix", "steps": 1}]
    if variant in ("ordinary", "literal", "key", "false-key", "invalid", "wish", "piece"):
        selection = "wish" if variant == "wish" else "magic_missile"
        if variant == "invalid":
            ops += [
                {"kind": "input", "request_id": "current", "selection": "not-offered"},
                {"kind": "advance_prefix", "steps": 1},
            ]
        ops += [
            {"kind": "input", "request_id": "current", "selection": selection},
            {"kind": "advance_prefix", "steps": 1},
        ]
        if variant == "wish":
            ops += [{"kind": "advance_prefix", "steps": 1}]
    ops += [{"kind": "advance_prefix", "steps": 0}]
    return retain(state, step, ops)


def wish_scene(variant):
    from goa2.engine.steps import WishSequenceStep

    state, owner = base("Razzle" if variant == "piece" else "Hanu")
    owner.wish_cast_count = (
        2 if variant in ("third", "started-third", "wrong-marker", "right-marker") else 0
    )
    step = WishSequenceStep(caster_id="hero_gydion")
    if variant in ("started", "started-third", "right-marker", "wrong-marker"):
        step.started = True
    if variant == "right-marker":
        state.execution_context["wish_cast_counted_caster_id"] = "hero_gydion"
    if variant == "wrong-marker":
        state.execution_context["wish_cast_counted_caster_id"] = "hero_hanu"
    if variant == "piece":
        step.caster_id = "hero_razzle_piece_1"
    if variant == "perform":
        step.perform_wish_action = True
    if variant == "missing":
        step.caster_id = "absent"
    if variant == "third":
        owner.spells.clear()
    if variant == "skip":
        step.skip_if_key = "owner_alias"
    # Stop before unported spell action bodies. Empty nested casts and resumed
    # sequences can run all the way through the actual terminal step.
    ops = [{"kind": "advance_prefix", "steps": 1}]
    if variant == "third":
        ops += [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 1},
        ]
    if variant == "started-third":
        owner.wish_cast_count = 3
        ops += [{"kind": "advance_prefix", "steps": 1}]
    ops += [{"kind": "advance_prefix", "steps": 0}]
    return retain(state, step, ops)


def remove_scene(variant):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState
    from goa2.engine.steps import RemovePreparedSpellsStep

    state, owner = base()
    step = RemovePreparedSpellsStep()
    if variant == "one":
        for spell in owner.spells[1:]:
            spell.state = CardState.OUTSIDE_SPELLBOOK
    if variant == "no-owner":
        owner.spells.clear()
    if variant == "unknown":
        step.caster_id = "absent"
    if variant == "literal":
        step.caster_id = "hero_hanu"
    if variant == "no-actor":
        state.current_actor_id = None
    if variant == "zero":
        step.max_removals = 0
    if variant == "negative":
        step.max_removals = -1
    if variant == "one-limit":
        step.max_removals = 1
    if variant == "pre-removed":
        step.removed_spell_ids = ["magic_missile"]
    if variant == "skip-step":
        step.skip_if_key = "owner_alias"
    retain(state, step, [])
    state.execution_context["removed_alias"] = step.removed_spell_ids
    bad = False

    def select(request, _mode):
        nonlocal bad
        if variant == "invalid" and not bad:
            bad = True
            return "not-offered"
        if variant == "skip":
            return "SKIP"
        return selection_value(request.options[-1] if variant == "last" else request.options[0])

    ops, _ = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}]


def prepare_scene(variant):
    from goa2.domain.models import CardState
    from goa2.engine.steps import PrepareSpellbookStep

    state, owner = base()
    for spell in owner.spells[::2]:
        spell.state = CardState.OUTSIDE_SPELLBOOK
        spell.is_facedown = False
    if variant == "empty":
        for spell in owner.spells:
            spell.state = CardState.SPELLBOOK
            spell.is_facedown = True
    if variant == "no-owner":
        owner.spells.clear()
    if variant == "no-actor":
        state.current_actor_id = None
    if variant == "foreign-actor":
        state.current_actor_id = "hero_hanu"
    if variant == "effects":
        from goa2.domain.models.effect import (
            ActiveEffect,
            DurationType,
            EffectScope,
            EffectType,
            Shape,
        )

        for spell in owner.spells[:3]:
            spell.is_active = True
            state.active_effects.append(
                ActiveEffect(
                    id="spell-effect-" + spell.id,
                    source_id=owner.id,
                    source_card_id=spell.id,
                    effect_type=EffectType.ATTACK_IMMUNITY,
                    scope=EffectScope(shape=Shape.GLOBAL),
                    duration=DurationType.THIS_ROUND,
                    is_active=True,
                    created_at_round=state.round,
                    created_at_turn=state.turn,
                )
            )
    step = PrepareSpellbookStep()
    if variant == "skip":
        step.skip_if_key = "owner_alias"
    return retain(
        state,
        step,
        [{"kind": "advance_prefix", "steps": 1}, {"kind": "advance_prefix", "steps": 0}],
    )


def scenes():
    for variant in (
        "ordinary",
        "no-owner",
        "spent",
        "no-actor",
        "literal",
        "key",
        "false-key",
        "unknown",
        "empty",
        "skip",
        "inactive",
        "invalid",
        "wish",
        "piece",
    ):
        state, ops = cast_scene(variant)
        yield "spellbook-cast-" + variant, state, ops
    for variant in (
        "ordinary",
        "third",
        "started",
        "started-third",
        "right-marker",
        "wrong-marker",
        "perform",
        "missing",
        "skip",
        "piece",
    ):
        state, ops = wish_scene(variant)
        yield "spellbook-wish-" + variant, state, ops
    for variant in (
        "ordinary",
        "last",
        "skip",
        "one",
        "no-owner",
        "unknown",
        "literal",
        "no-actor",
        "zero",
        "negative",
        "one-limit",
        "pre-removed",
        "skip-step",
        "invalid",
    ):
        state, ops = remove_scene(variant)
        yield "spellbook-remove-" + variant, state, ops
    for variant in (
        "ordinary",
        "empty",
        "no-owner",
        "no-actor",
        "foreign-actor",
        "effects",
        "skip",
    ):
        state, ops = prepare_scene(variant)
        yield "spellbook-prepare-" + variant, state, ops
    yield from abort_scenes()


def abort_scenes():
    from live_defeat_traces import operations

    from goa2.domain.models import TargetType
    from goa2.engine.steps import SelectStep, SetContextFlagStep, WishSequenceStep

    for count in (1, 3):
        state, owner = base()
        owner.wish_cast_count = count
        wish = WishSequenceStep(caster_id=owner.id, started=True, step_id="fixture/wish-abort")
        abort = SelectStep(
            target_type=TargetType.NUMBER,
            prompt="Unperformable nested action",
            number_options=[],
            step_id="fixture/wish-empty",
        )
        tail = SetContextFlagStep(
            key="after_wish_abort", value=True, step_id="fixture/wish-abort-tail"
        )
        state.execution_stack = [tail, wish, abort]
        state.execution_context.update(wish_alias=wish, wish_cast_counted_caster_id=owner.id)
        ops, _ = operations(state)
        yield f"spellbook-wish-abort-{count}", state, [*ops, {"kind": "advance_prefix", "steps": 0}]
