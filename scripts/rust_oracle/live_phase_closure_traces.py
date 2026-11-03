"""Full source turn/round flows, expanded into authoritative one-step boundaries."""

from __future__ import annotations

import copy
from contextlib import suppress


def expand_operations(state, operations):
    from goa2.domain.input import InputResponse
    from goa2.engine.handler import process_stack, submit_input

    probe = copy.deepcopy(state)
    result = []
    request = None
    for op in operations:
        if op["kind"] == "advance":
            if not probe.execution_stack:
                result.append({"kind": "advance_prefix", "steps": 0})
            for _ in range(2048):
                if not probe.execution_stack:
                    break
                remaining = 1

                def stop(_state, _step):
                    nonlocal remaining
                    if remaining == 0:
                        return True
                    remaining -= 1
                    return False

                out = process_stack(probe, stop_before_step=stop)
                result.append({"kind": "advance_prefix", "steps": 1})
                request = out.input_request
                if request or probe.phase.value == "GAME_OVER":
                    break
            else:
                raise AssertionError("phase flow did not reach input/empty stack")
        elif op["kind"] == "advance_prefix":
            remaining = op["steps"]

            def stop(_state, _step):
                nonlocal remaining
                if remaining == 0:
                    return True
                remaining -= 1
                return False

            out = process_stack(probe, stop_before_step=stop)
            request = out.input_request
            result.append(op)
        else:
            assert op["kind"] == "input" and request is not None
            with suppress(ValueError):
                submit_input(
                    probe, InputResponse(request_id=request.id, selection=op.get("selection"))
                )
            result.append(op)
    return result


def scenes(names=None):
    from round_traces import cases as round_cases
    from turn_traces import cases as turn_cases

    retained = []
    advance = {"kind": "advance"}

    def answer(value):
        return {"kind": "input", "selection": value}

    for family, build in (("turn", turn_cases), ("round", round_cases)):
        build(
            advance,
            answer,
            live_sink=lambda name, state, ops, family=family: retained.append(
                (family + "-" + name, state, ops)
            ),
        )
    for label, state, ops in retained:
        name = "phase-closure-live-" + label
        if names is not None and name not in names:
            continue
        state.execution_context.update(
            stack_alias=state.execution_stack,
            locations_alias=state.entity_locations,
            snapshot_alias=state.last_turn_positions,
            effects_alias=state.active_effects,
            pending_upgrades_alias=state.pending_upgrades,
            reveals_alias=state.upgrade_reveal_snapshot,
            unresolved_alias=state.unresolved_hero_ids,
            pending_inputs_alias=state.pending_inputs,
            defeated_alias=state.heroes_defeated_this_round,
        )
        for team in state.teams.values():
            for hero in team.heroes:
                state.execution_context[str(hero.id) + "-cards"] = [
                    hero.deck,
                    hero.hand,
                    hero.played_cards,
                    hero.discard_pile,
                ]
                state.execution_context[str(hero.id) + "-items"] = hero.items
                if hero.current_turn_card:
                    state.execution_context[str(hero.id) + "-current"] = hero.current_turn_card
        yield name, state, expand_operations(state, ops)
