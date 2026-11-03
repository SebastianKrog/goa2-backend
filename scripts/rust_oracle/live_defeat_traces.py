"""Frozen defeat/removal/protection/terminal flows with retained model aliases."""

from __future__ import annotations


def operations(state, select=None, mode="first"):
    import copy

    from goa2.domain.input import InputResponse, selection_value
    from goa2.engine.handler import process_stack, submit_input

    probe = copy.deepcopy(state)
    result = []
    bad = False
    for _ in range(100):
        if not probe.execution_stack or probe.phase.value == "GAME_OVER":
            return result, probe
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
            if select:
                value = select(request, mode)
            elif mode == "invalid" and not bad:
                value = "not-offered"
                bad = True
            elif mode == "skip" and request.can_skip:
                value = "SKIP"
            else:
                value = selection_value(
                    request.options[-1] if mode == "last" else request.options[0]
                )
            submit_input(probe, InputResponse(request_id=request.id, selection=value))
            result.append({"kind": "input", "request_id": "current", "selection": value})
    raise AssertionError("defeat trace did not finish")


def retain(state, steps, mode="first", select=None):
    from goa2.engine.steps import SetContextFlagStep

    tail = SetContextFlagStep(step_id="fixture/defeat/tail", key="tail", value=True)
    for index, step in enumerate(steps):
        step.step_id = f"fixture/defeat/{index}"
    state.execution_stack = list(reversed([*steps, tail]))
    state.execution_context.update(
        defeat_steps=steps,
        effects_alias=state.active_effects,
        locations_alias=state.entity_locations,
        unresolved_alias=state.unresolved_hero_ids,
        defeated_alias=state.heroes_defeated_this_round,
        input_alias=state.input_stack,
    )
    ops, probe = operations(state, select, mode)
    return [
        *ops,
        {"kind": "advance_prefix", "steps": 1},
        {"kind": "advance_prefix", "steps": 0},
    ], probe


def scenes():
    from unittest.mock import patch

    import combat_effect_traces
    from aura_traces import _board, _pieces
    from combat_effect_traces import effect, state_for
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, Marker, MarkerType, Turret
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import (
        DefeatUnitStep,
        RemoveUnitStep,
        TriggerGameOverStep,
    )

    emitted = []

    def capture(name, state, steps, mode="first", unordered_keys=(), select=None):
        # Reuse source scenario construction/assertions, without serializing its
        # separate value-only trace corpus. Each selected world is retained live.
        ops, final = retain(state, steps, mode, select)
        if not name.startswith(("combat-effect-negative-", "combat-effect-create-")):
            emitted.append(("live-" + name, state, ops))
        return {"name": name}, final

    with patch.object(combat_effect_traces, "drive", capture):
        combat_effect_traces.cases()
    yield from emitted

    for variant in (
        "plain",
        "current",
        "retrieved",
        "played-alias",
        "duplicate-unresolved",
        "shared-tracking",
        "existing-log",
        "offboard",
        "repeat",
        "bounty",
        "marker-source",
        "card-active",
        "snapshot",
        "key",
        "literal-wins",
        "missing-key",
        "empty-key",
        "skip",
        "active",
    ):
        state, h, _ = state_for()
        hero = state.get_hero("hero_brogan")
        step = DefeatUnitStep(victim_id=str(hero.id), killer_id="hero_swift")
        state.unresolved_hero_ids = [hero.id, "hero_hanu"]
        state.execution_context["hero_alias"] = hero
        state.execution_context["played_alias"] = hero.played_cards
        if variant in {"current", "played-alias", "card-active"}:
            hero.current_turn_card = hero.hand.pop(0)
            hero.current_turn_card.state = CardState.UNRESOLVED
            hero.current_turn_card.initiative_override = 19
        if variant == "played-alias":
            state.get_hero("hero_arien").played_cards = hero.played_cards
        if variant == "retrieved":
            hero.current_turn_card = None
            state.turn = 2
            hero.resolved_turn_count = 0
        if variant == "duplicate-unresolved":
            state.unresolved_hero_ids = [hero.id, hero.id]
        if variant == "shared-tracking":
            state.heroes_defeated_this_round = state.unresolved_hero_ids
        if variant == "existing-log":
            state.execution_context["defeat_log"] = [["before", None]]
            state.execution_context["old_log"] = state.execution_context["defeat_log"]
        if variant == "offboard":
            state.remove_entity(hero.id)
        if variant in {"bounty", "marker-source"}:
            marker = state.markers.setdefault(MarkerType.BOUNTY, Marker(type=MarkerType.BOUNTY))
            marker.place(str(hero.id) if variant == "bounty" else "hero_hanu", 2, str(hero.id))
            state.execution_context["marker_alias"] = marker
        if variant == "card-active":
            state.active_effects = [
                effect(
                    state,
                    "owned",
                    EffectType.MOVEMENT_ZONE,
                    source_card_id=hero.current_turn_card.id,
                )
            ]
            hero.current_turn_card.is_active = True
        if variant == "snapshot":
            state.last_turn_positions = state.entity_locations
        if variant in {"key", "literal-wins", "missing-key", "empty-key"}:
            step.victim_key = "victim"
            state.execution_context["victim"] = "hero_arien"
            if variant != "literal-wins":
                step.victim_id = ""
            if variant == "missing-key":
                state.execution_context.pop("victim")
            if variant == "empty-key":
                step.victim_key = ""
        if variant == "skip":
            step.skip_if_key = "gate"
            state.execution_context["gate"] = False
        if variant == "active":
            step.active_if_key = "absent"
        steps = [step, step] if variant == "repeat" else [step]
        yield "defeat-hero-" + variant, state, retain(state, steps)[0]

    for level in (-1, 1, 2, 3, 4, 5, 6, 7, 8, 9):
        for killer in ("hero_swift", "hero_brogan", None, "", "unknown"):
            state, _, _ = state_for()
            state.get_hero("hero_brogan").level = level
            step = DefeatUnitStep(
                victim_id="hero_brogan", killer_id=killer, assist_multiplier=-2 if level == 9 else 3
            )
            yield f"defeat-rewards-{level}-{killer}", state, retain(state, [step])[0]

    for variant in (
        "piece",
        "owner",
        "offboard-piece",
        "offboard-owner",
        "killer-piece",
        "marker-piece",
        "one-piece",
        "repeat",
    ):
        state = setup(["Swift", "Razzle"], ["Brogan", "Arien"])
        h = _board(state)
        pieces = _pieces(state, "hero_razzle")
        for index, piece in enumerate(pieces):
            state.place_entity(piece, h(index, 0))
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_brogan"
        victim = pieces[0]
        killer = "hero_brogan"
        if variant in {"owner", "offboard-owner"}:
            victim = "hero_razzle"
        if variant in {"offboard-piece", "one-piece"}:
            state.remove_entity(pieces[0] if variant == "offboard-piece" else pieces[1])
        if variant == "offboard-owner":
            for piece in pieces:
                state.remove_entity(piece)
        if variant == "killer-piece":
            victim = "hero_brogan"
            killer = pieces[0]
        if variant == "marker-piece":
            state.markers.setdefault(MarkerType.BOUNTY, Marker(type=MarkerType.BOUNTY)).place(
                pieces[1], 1, "hero_brogan"
            )
        step = DefeatUnitStep(victim_id=victim, killer_id=killer)
        steps = [step, step] if variant == "repeat" else [step]
        yield "defeat-pieces-" + variant, state, retain(state, steps)[0]

    for target in ("hero_brogan", "minion", "token", "turret", "unknown", None):
        for keyed in (False, True):
            state, h, minion = state_for()
            tid = str(minion.id) if target == "minion" else target
            if target == "token":
                from goa2.domain.models import Token, TokenType

                token = Token(id="remove_token", name="Removed", token_type=TokenType.TOTEM)
                state.register_entity(token)
                state.token_pool.setdefault(TokenType.TOTEM, []).append(token)
                tid = str(token.id)
                state.place_entity(tid, h(3, 0))
                state.active_effects = [
                    effect(
                        state,
                        "retained-token-effect",
                        EffectType.MOVEMENT_ZONE,
                        source=tid,
                        token_id=tid,
                    )
                ]
            if target == "turret":
                tid = "trinkets_turret"
                state.register_entity(Turret(id=tid, name="Turret"), "board_entity")
                state.place_entity(tid, h(3, 0))
            state.last_turn_positions = state.entity_locations
            state.execution_context["chosen"] = tid
            step = RemoveUnitStep(
                unit_id="hero_swift" if keyed else tid, unit_key="chosen" if keyed else None
            )
            yield f"defeat-remove-{target}-{keyed}", state, retain(state, [step])[0]

    for variant in ("beneficiary-team", "killer-team", "victim-team", "minion-team"):
        from goa2.domain.models import TeamColor

        state, _, minion = state_for()
        bounty = effect(
            state,
            "team-bounty",
            EffectType.MINION_DEFEAT_BOUNTY,
            source="hero_swift",
            subject_id="hero_swift",
            max_value=2,
        )
        bounty.scope.range = 3
        state.active_effects = [bounty]
        victim = str(minion.id)
        if variant == "beneficiary-team":
            state.get_hero("hero_swift").team = TeamColor.BLUE
        if variant == "killer-team":
            state.get_hero("hero_swift").team = TeamColor.BLUE
            victim = "hero_brogan"
        if variant == "victim-team":
            state.get_hero("hero_brogan").team = TeamColor.RED
            victim = "hero_brogan"
        if variant == "minion-team":
            minion.team = TeamColor.RED
        yield "defeat-roster-" + variant, state, retain(
            state, [DefeatUnitStep(victim_id=victim, killer_id="hero_swift")]
        )[0]

    from goa2.engine.steps import ForceDiscardOrDefeatStep

    for variant in ("literal", "key", "empty-key", "no-actor", "empty-literal", "missing-key"):
        state, _, _ = state_for()
        state.get_hero("hero_brogan").hand = []
        state.execution_context.update(forced_victim="hero_brogan", who="hero_hanu")
        step = ForceDiscardOrDefeatStep(victim_key="forced_victim", killer_key="who")
        if variant == "literal":
            step.killer_id = "hero_brogan"
        if variant == "empty-key":
            state.execution_context["who"] = ""
        if variant == "no-actor":
            state.current_actor_id = None
            step.killer_key = None
        if variant == "empty-literal":
            step.killer_id = ""
        if variant == "missing-key":
            state.execution_context.pop("who")
        yield "defeat-credit-" + variant, state, retain(state, [step])[0]

    from live_passive_traces import build

    from goa2.domain.hex import Hex
    from goa2.engine.effects import CardEffectRegistry

    for effect_id in ("self_destruct", "emergency_protocol"):
        for empty in ("hanu", "both"):
            for annihilate in (False, True):
                for mode in ("first", "last", "invalid"):
                    state, hero = build("Trinkets")
                    turret = Turret(id="trinkets_turret", name="Turret", owner_id=str(hero.id))
                    state.register_entity(turret, "board_entity")
                    state.place_entity(turret.id, Hex(q=1, r=0, s=-1))
                    state.place_entity("hero_hanu", Hex(q=2, r=0, s=-2))
                    state.place_entity("hero_brogan", Hex(q=1, r=1, s=-2))
                    state.get_hero("hero_hanu").hand = []
                    if empty == "both":
                        state.get_hero("hero_brogan").hand = []
                    if annihilate:
                        from goa2.domain.models import TeamColor

                        state.teams[TeamColor.BLUE].life_counters = 1
                    card = next(c for c in [*hero.hand, *hero.deck] if c.effect_id == effect_id)
                    hero.current_turn_card = card
                    card.state = CardState.UNRESOLVED
                    card.is_facedown = False
                    body = CardEffectRegistry.get(effect_id).get_steps(state, hero, card)
                    yield f"defeat-card-{effect_id}-{empty}-{annihilate}-{mode}", state, retain(
                        state, body, mode
                    )[0]

    for winner in ("RED", "BLUE", "hero_swift", "", "unknown"):
        for phase in (GamePhase.RESOLUTION, GamePhase.GAME_OVER):
            state, _, _ = state_for()
            state.phase = phase
            step = TriggerGameOverStep(
                winner=winner if winner in {"RED", "BLUE"} else None,
                individual_winner_id=None if winner in {"RED", "BLUE"} else winner,
                condition="test-condition",
            )
            from goa2.domain.input import InputRequestType, create_input_request

            queued = create_input_request(
                request_type=InputRequestType.SELECT_UNIT,
                player_id="hero_swift",
                prompt="Queued",
                options=[],
            )
            queued.id = "fixture/queued-input"
            state.input_stack = [queued]
            state.execution_context["queued_model"] = queued
            ops, _ = retain(state, [step])
            state.execution_context["stack_alias"] = state.execution_stack
            yield f"defeat-terminal-{winner}-{phase.value}", state, ops

    yield from shared_bounty_scenes()


def export(path):
    from object_execution_traces import reference
    from oracle import verify_sources
    from shared_objects import export_state_with_shared_objects
    from stack_traces import write_corpus_stream

    manifest = verify_sources()

    def cases():
        from itertools import chain

        from live_multi_selection_traces import scenes as multi_scenes

        replacements = (row for row in multi_scenes() if row[0].startswith("multi-card-"))
        for name, state, ops in chain(scenes(), replacements):
            envelope, _ = export_state_with_shared_objects(state)
            yield {
                "name": "object-execution-" + name,
                "envelope": envelope,
                "operations": ops,
                "expected": reference(state, ops),
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


def shared_bounty_scenes():
    from combat_effect_traces import effect, state_for

    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import DefeatUnitStep

    for budget, count, shared in (
        (1, 2, True),
        (2, 2, True),
        (1, 3, True),
        (2, 3, True),
        (1, 2, False),
        (2, 2, False),
    ):
        state, _, minion = state_for()
        bounty = effect(
            state,
            "shared-bounty",
            EffectType.MINION_DEFEAT_BOUNTY,
            source="hero_swift",
            subject_id="hero_swift",
            max_value=budget,
        )
        bounty.scope.range = 3
        state.active_effects = (
            [bounty] * count if shared else [bounty.model_copy(deep=True) for _ in range(count)]
        )
        state.execution_context["bounty_alias"] = state.active_effects[0]
        yield f"defeat-bounty-model-{budget}-{count}-{shared}", state, retain(
            state, [DefeatUnitStep(victim_id=str(minion.id), killer_id="hero_swift")]
        )[0]
