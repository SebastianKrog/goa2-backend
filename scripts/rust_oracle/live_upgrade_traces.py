"""Simultaneous upgrade choices and resets retain real physical/list aliases."""

from __future__ import annotations

import copy


def operations(state, *, batch=False, side=0, invalid=None):
    from goa2.domain.input import InputResponse
    from goa2.engine.handler import process_stack, submit_input
    from goa2.engine.steps import ResolveUpgradesStep

    probe = copy.deepcopy(state)
    result = []
    rejected = False
    for _ in range(64):
        if not probe.execution_stack:
            return result
        remaining = 1

        def stop(_state, _step):
            nonlocal remaining
            if not remaining:
                return True
            remaining -= 1
            return False

        out = process_stack(probe, stop_before_step=stop)
        result.append({"kind": "advance_prefix", "steps": 1})
        if not out.input_request:
            continue
        req = out.input_request
        assert req.request_type.value == "UPGRADE_PHASE"
        hero = next(h for h, row in req.context["players"].items() if row["options"])
        row = req.context["players"][hero]
        card = row["options"][0]["pair"][side]
        if invalid is not None and not rejected:
            candidates = {
                "duplicate": {"hero_id": hero, "card_ids": [card, card]},
                "mixed": {"hero_id": hero, "card_id": card, "card_ids": [card]},
                "unknown-hero": {"hero_id": "missing", "card_id": card},
                "unknown-card": {"hero_id": hero, "card_id": "missing"},
                "wrong-type": {"hero_id": hero, "card_ids": card},
                "empty": {"hero_id": hero, "card_ids": []},
                "same-pair": {"hero_id": hero, "card_ids": row["options"][0]["pair"]},
                "partial": {
                    "hero_id": hero,
                    "card_ids": [o["pair"][0] for o in row["options"][:2]],
                },
                "null": None,
            }
            value = candidates[invalid]
            try:
                submit_input(probe, InputResponse(request_id=req.id, selection=value))
            except ValueError:
                pass
            else:
                raise AssertionError("invalid upgrade accepted")
            rejected = True
            result.append({"kind": "input", "selection": value})
        if batch:
            preview = probe.model_copy(deep=True)
            ids = []
            for _ in range(row["remaining"]):
                option = ResolveUpgradesStep()._get_upgrade_options(preview, hero)[0]
                chosen = option["pair"][side]
                ids.append(chosen)
                preview = ResolveUpgradesStep().preview_choices(preview, hero, [chosen])
            value = {"hero_id": hero, "card_ids": ids}
        else:
            value = {"hero_id": hero, "card_id": card}
        submit_input(probe, InputResponse(request_id=req.id, selection=value))
        result.append({"kind": "input", "selection": value})
    raise AssertionError("upgrade scene did not finish")


def scenes(names=None):
    from model_traces import setup

    from goa2.domain.models import GamePhase, StatType
    from goa2.engine.steps import EndPhaseCleanupStep, ResolveUpgradesStep, RoundResetStep

    def inputs():
        for roster in ("Arien", "Min", "Gydion", "Takahide", "Ursafar", "Misa", "Wasp"):
            for count in (1, 3, 6):
                for batch in (False, True):
                    for side in (0, 1):
                        state = setup([roster, "Brogan"], ["Hanu", "Tigerclaw"])
                        hero = state.teams["RED"].heroes[0]
                        state.phase = GamePhase.LEVEL_UP
                        state.pending_upgrades = {hero.id: count}
                        state.upgrade_reveal_snapshot = {hero.id: {StatType.ATTACK: -2}}
                        hero.items = {StatType.ATTACK: -2}
                        yield f"{roster}-{count}-{'batch' if batch else 'single'}-{side}", state, ResolveUpgradesStep(), dict(
                            batch=batch, side=side
                        )
        for invalid in (
            "duplicate",
            "mixed",
            "unknown-hero",
            "unknown-card",
            "wrong-type",
            "empty",
            "same-pair",
            "partial",
            "null",
        ):
            state = setup()
            state.phase = GamePhase.LEVEL_UP
            state.pending_upgrades = {"hero_arien": 3}
            yield "invalid-" + invalid, state, ResolveUpgradesStep(), dict(invalid=invalid)
        for variant in (
            "flags",
            "shared-items",
            "pending-valid",
            "pending-invalid",
            "pending-empty",
            "pending-missing-selection",
            "pending-null-selection",
        ):
            state = setup()
            state.phase = GamePhase.LEVEL_UP
            state.pending_upgrades = {"hero_arien": 1}
            hero = state.get_hero("hero_arien")
            step = ResolveUpgradesStep()
            chosen = step._get_upgrade_options(state, "hero_arien")[0]["pair"][0]
            if variant == "flags":
                for card in hero.deck:
                    card.is_facedown = True
                    card.played_this_round = True
                    card.is_active = True
                    card.initiative_override = 17
            if variant == "shared-items":
                state.get_hero("hero_hanu").items = hero.items
            if variant.startswith("pending-"):
                step.pending_input = {
                    "pending-valid": {"selection": {"hero_id": "hero_arien", "card_id": chosen}},
                    "pending-invalid": {
                        "selection": {"hero_id": "hero_arien", "card_id": "missing"}
                    },
                    "pending-empty": {},
                    "pending-missing-selection": {"other": True},
                    "pending-null-selection": {"selection": None},
                }[variant]
                state.execution_context["pending_input_alias"] = step.pending_input
            yield variant, state, step, {}
        for variant in (
            "empty",
            "defeated",
            "shared-snapshot",
            "shared-reveal",
            "null-coin",
            "skipped",
        ):
            state = setup()
            state.phase, state.turn, state.round = GamePhase.LEVEL_UP, 4, 7
            if variant == "defeated":
                state.heroes_defeated_this_round.extend(["hero_arien", "hero_hanu"])
            if variant == "shared-snapshot":
                state.last_turn_positions = state.entity_locations
            if variant == "shared-reveal":
                state.upgrade_reveal_snapshot = state.pending_upgrades
            if variant == "null-coin":
                state.tie_breaker_team = None
            step = RoundResetStep() if variant != "shared-reveal" else ResolveUpgradesStep()
            if variant == "skipped":
                step.skip_if_key = "skip_reset"
                state.execution_context["skip_reset"] = True
            yield "reset-" + variant, state, step, {}
        state = setup()
        state.phase, state.turn = GamePhase.CLEANUP, 4
        state.get_hero("hero_arien").gold = 28
        state.get_hero("hero_hanu").gold = 6
        yield "cleanup-multihero-complete-round", state, EndPhaseCleanupStep(), dict(batch=True)

    for name, state, step, mode in inputs():
        name = "upgrade-live-" + name
        if names is not None and name not in names:
            continue
        step.step_id = "fixture/upgrades/step"
        state.execution_stack = [step]
        state.execution_context.update(
            step_alias=step,
            pending_alias=state.pending_upgrades,
            reveal_alias=state.upgrade_reveal_snapshot,
            defeated_alias=state.heroes_defeated_this_round,
            locations_alias=state.entity_locations,
            snapshot_alias=state.last_turn_positions,
        )
        for team in state.teams.values():
            for hero in team.heroes:
                state.execution_context[str(hero.id) + "-lists"] = [
                    hero.hand,
                    hero.deck,
                    hero.played_cards,
                    hero.discard_pile,
                ]
                state.execution_context[str(hero.id) + "-items"] = hero.items
        yield name, state, operations(state, **mode)
