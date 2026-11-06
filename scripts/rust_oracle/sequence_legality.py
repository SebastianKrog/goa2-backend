"""Compare native sequence verdicts with the frozen Python legality checker."""

import argparse
import copy
from collections import Counter
from itertools import chain

from live_defeat_traces import retain
from live_hero_respawn_traces import scenes as respawn_scenes
from live_rune_traces import scenes as rune_scenes
from live_upgrade_traces import scenes as upgrade_scenes
from live_spellbook_traces import cast_scene
from model_traces import setup
from object_execution_traces import (
    ObjectTraceNormalizer, confirmation_scenes, iter_reference, planning_command_scenes,
    reference as reference_trace,
)
from oracle import Worker, verify_sources
from shared_objects import export_state_with_shared_objects
from streamed_games import StreamWorker

from automata.runtime.legality import DecisionMove, check_moves, check_sequences
from goa2.domain.models import CardState, GamePhase, TargetType
from goa2.domain.input import selection_value
from goa2.engine.handler import process_stack
from goa2.engine.filters_hex import MovementPathFilter, ObstacleFilter
from goa2.engine.steps import (
    FindNextActorStep, MoveUnitStep, MultiSelectStep, PlaceRunesStep, ResolveCardStep,
    ResolveCardTextStep, SelectStep,
)


def scenes():
    for name, state, ops in planning_command_scenes(lambda red=None: setup(red=red)):
        if name not in {"planning-command-pass", "planning-command-pass-nonempty"}:
            continue
        move = {"kind": "pass", "player_id": ops[0]["hero"]}
        yield name, state, [[], [move], [move, move], [move], [move, move, move]]

    state = setup()
    state.phase = GamePhase.PLANNING
    envelope, _ = export_state_with_shared_objects(state)
    first, other = [
        envelope["state"]["teams"][team]["heroes"][0] for team in ("RED", "BLUE")
    ]
    commit = {"kind": "card", "player_id": first["id"], "card": first["hand"][0]["$goa2_instance"]}
    foreign = dict(commit, card=other["hand"][0]["$goa2_instance"])
    yield "planning-card-membership", state, [
        [commit], [foreign], [dict(commit, player_id="missing")], [commit, commit],
    ]

    state = setup()
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_arien"
    state.execution_stack = [
        SelectStep(
            step_id=f"fixture/sequence/number-{index}",
            target_type=TargetType.NUMBER,
            number_options=[1, 2] if index else [2, 3],
            prompt="Choose a number",
            output_key=f"sequence_choice_{index}",
        )
        for index in range(2)
    ]
    first = {"kind": "input", "player_id": "hero_arien", "selection": 1}
    second = dict(first, selection=3)
    yield "two-number-requests", state, [
        [], [first], [first, second], [first, dict(second, selection=9)],
        [first, dict(second, request_id="stale")],
        [dict(first, player_id="not-a-player")], [second, first],
    ]


    state = setup()
    state.phase = GamePhase.RESOLUTION
    hero = "hero_arien"
    state.current_actor_id = hero
    origin = state.get_position(hero)

    def clear(hex):
        return hex in state.board.tiles and not state.board.tiles[hex].is_obstacle

    first_hex, second_hex = next(
        (first, second)
        for first in origin.neighbors() if clear(first)
        for second in first.neighbors() if clear(second) and origin.distance(second) == 2
    )
    path = MovementPathFilter(range_val=1, unit_id=hero)

    def movement_selector(label):
        return SelectStep(
            step_id=f"fixture/sequence/movement-{label}",
            target_type=TargetType.HEX,
            prompt=f"Choose the {label} destination",
            output_key="target_hex",
            filters=[path, ObstacleFilter()],
        )

    state.execution_stack = [
        movement_selector("second"),
        MoveUnitStep(step_id="fixture/sequence/movement-apply", unit_id=hero),
        movement_selector("first"),
    ]
    first = dict(kind="input", player_id=hero, selection=first_hex.model_dump())
    second = dict(first, selection=second_hex.model_dump())
    yield "movement-updates-reachability", state, [[second], [first, second], [second, first], [first]]

    for name, state, ops in upgrade_scenes({
        "upgrade-live-Arien-3-batch-0",
        "upgrade-live-Arien-3-single-0",
    }):
        decisions = [
            {"kind": "input", "player_id": value["hero_id"], "selection": value}
            for value in (op["selection"] for op in ops if op["kind"] == "input")
        ]
        first = decisions[0]
        hero = first["player_id"]
        card = first["selection"].get("card_id") or first["selection"]["card_ids"][0]
        yield name, state, [
            decisions,
            [first],
            [dict(first, player_id="hero_hanu")],
            [dict(first, selection={"hero_id": hero, "card_ids": [card, card]})],
            [dict(first, selection={"hero_id": hero, "card_id": "missing"})],
            [dict(first, request_id="stale")],
        ]

    name, state, _ = next(iter(rune_scenes()))
    rune = {"kind": "input", "player_id": str(state.current_actor_id), "selection": "axe"}
    yield name, state, [[rune], [dict(rune, selection="missing")], [dict(rune, player_id="hero_hanu")]]


    name, state, _ = next(respawn_scenes({"hero-respawn-live-ordinary-first"}))
    probe = copy.deepcopy(state)
    request = process_stack(probe).input_request
    hex_choice = selection_value(request.options[0])
    if hasattr(hex_choice, "model_dump"):
        hex_choice = hex_choice.model_dump(mode="json")
    move = {"kind": "input", "player_id": request.player_id, "selection": hex_choice}
    yield name, state, [
        [move], [dict(move, selection={"q": 30, "r": 0, "s": -30})],
        [dict(move, player_id="hero_hanu")], [dict(move, request_id="stale")],
    ]

    state = setup()
    state.phase = GamePhase.RESOLUTION
    heroes = [hero for team in state.teams.values() for hero in team.heroes]
    for index, hero in enumerate(heroes):
        card = hero.hand[0]
        hero.play_card(card)
        card.is_facedown = False
        card.initiative_override = 20 if index < 2 else 10
    state.unresolved_hero_ids = [hero.id for hero in heroes]
    state.execution_stack = [FindNextActorStep(step_id="fixture/sequence/team-tie")]
    request = process_stack(copy.deepcopy(state)).input_request
    assert request.player_id.startswith("team:")
    actor = {"kind": "input", "player_id": str(heroes[0].id), "selection": request.options[0].id}
    yield "team-owned-actor", state, [
        [actor], [dict(actor, player_id=str(heroes[1].id))],
        [dict(actor, player_id=str(heroes[2].id))], [dict(actor, selection="missing")],
    ]

    state = setup()
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_arien"
    state.execution_stack = [MultiSelectStep(
        step_id="fixture/sequence/multiselect", target_type=TargetType.UNIT,
        prompt="Pick units", output_key="chosen", min_selections=1, max_selections=2,
    )]
    request = process_stack(copy.deepcopy(state)).input_request
    first = {"kind": "input", "player_id": request.player_id, "selection": request.options[0].id}
    done = dict(first, selection="DONE")
    skip = dict(first, selection="SKIP")
    yield "multi-selection", state, [[first], [done], [skip], [first, done], [first, skip]]


    confirmation_name, confirmation_state, _ = next(
        iter(confirmation_scenes(lambda red=None: setup(red=red)))
    )
    spell_state, _ = cast_scene("ordinary")
    for name, state in [(confirmation_name, confirmation_state), ("spellbook-cast", spell_state)]:
        request = process_stack(copy.deepcopy(state)).input_request
        move = {
            "kind": "input",
            "player_id": request.player_id,
            "selection": selection_value(request.options[0]),
        }
        yield name, state, [
            [move],
            [dict(move, selection="not-offered")],
            [dict(move, selection="SKIP")],
            [dict(move, request_id="stale")],
        ]


    state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_snorri"
    state.execution_stack = [PlaceRunesStep(step_id="fixture/sequence/inscription", hero_id="hero_snorri")]
    move = {"kind": "input", "player_id": "hero_snorri", "selection": "axe"}
    yield "rune-inscription", state, [
        [move], [move, dict(move, selection=["bird", "anvil"])],
        [dict(move, selection=["axe", "bird", "anvil", "horn"])],
        [dict(move, selection=["axe", "bird", "anvil"])],
        [dict(move, selection=["axe", "bird"])],
        [move, dict(move, selection=["axe", "bird"])],
        [dict(move, selection=["axe", "axe", "horn"])],
        [dict(move, player_id="hero_hanu")],
    ]


def inscription_card_traces():
    for entry in ("text", "action"):
        state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_snorri")
        state.current_actor_id = state.resolution_owner_id = hero.id
        card = next(card for card in hero.deck if card.effect_id == "inscribe_the_runes")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        state.execution_context["inscription_card"] = card
        step = (
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
            if entry == "text" else ResolveCardStep(hero_id=hero.id)
        )

        def choose(request, _mode):
            if request.request_type.value == "CHOOSE_ACTION":
                return card.primary_action.value
            if request.decision_context and request.decision_context.purpose == "rune_inscription":
                return ["horn", "anvil", "bird"]
            return selection_value(request.options[0])

        operations, final = retain(state, [step], select=choose)
        assert [rune.value for rune in final.get_hero(hero.id).rune_slots.values()] == [
            "horn", "anvil", "bird", "axe",
        ]
        yield f"inscription-card-{entry}", state, operations


def passage_card_traces():
    from goa2.domain.models import RuneType
    from snorri_builders import SNORRI_STATEFUL_EFFECTS

    for effect in (effect for effect in SNORRI_STATEFUL_EFFECTS if effect.endswith("passage")):
        for mask in range(8):
            state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
            state.phase = GamePhase.RESOLUTION
            state.turn = 2
            hero = state.get_hero("hero_snorri")
            state.current_actor_id = state.resolution_owner_id = hero.id
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            if mask & 1:
                hero.rune_slots[2] = RuneType.BIRD
            if mask & 2:
                state.execution_context["snorri_ult_rune_action"] = RuneType.ANVIL.value
            if mask & 4:
                state.execution_context["snorri_ult_rune_defense"] = RuneType.HORN.value
            step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
            operations, _ = retain(state, [step])
            yield f"{effect}-{mask}", state, operations


def runic_melee_traces():
    from aura_traces import _board
    from goa2.domain.models import RuneType

    for effect in ("runic_dagger", "runic_hammer", "runic_battleaxe"):
        for mask in range(8):
            state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.battle_zones = {}
            state.phase = GamePhase.RESOLUTION
            state.turn = 2
            hero = state.get_hero("hero_snorri")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            minions = state.teams["BLUE"].minions
            state.place_entity(minions[0].id, h(1, 0))
            state.place_entity(minions[1].id, h(0, 1))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            hero.discard_card(hero.hand[0])
            if mask & 1:
                hero.rune_slots[2] = RuneType.AXE
            if mask & 2:
                state.execution_context["snorri_ult_rune_action"] = RuneType.ANVIL.value
            if mask & 4:
                state.execution_context["snorri_ult_rune_defense"] = RuneType.HORN.value

            def choose(request, _mode):
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                if request.prompt.startswith("You may move"):
                    return "SKIP"
                return selection_value(request.options[0])

            operations, _ = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            yield f"{effect}-{mask}", state, operations


def runic_ranged_traces():
    from aura_traces import _board
    from goa2.domain.models import RuneType

    masks = ((), (RuneType.BIRD,), (RuneType.AXE,), (RuneType.ANVIL,), (RuneType.HORN,),
             (RuneType.BIRD, RuneType.HORN, RuneType.AXE),
             (RuneType.HORN, RuneType.AXE, RuneType.ANVIL))
    for effect in ("runecaster", "runeblaster", "runetrap", "runebomb"):
        for index, runes in enumerate(masks):
            state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.battle_zones = {}
            state.phase = GamePhase.RESOLUTION
            state.turn = 2
            hero = state.get_hero("hero_snorri")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_hanu", h(2, 0))
            target = state.teams["BLUE"].minions[0]
            state.place_entity(target.id, h(3, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            if runes:
                hero.rune_slots[2] = runes[0]
            for key, rune in zip(("snorri_ult_rune_action", "snorri_ult_rune_defense"), runes[1:]):
                state.execution_context[key] = rune.value

            def choose(request, _mode):
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                if request.prompt == "Select target in range":
                    return next(option.id for option in request.options if option.id == "hero_hanu")
                return selection_value(request.options[0])

            operations, _ = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            yield f"{effect}-{index}", state, operations


def ancestral_boon_traces():
    from aura_traces import _board
    from combat_effect_traces import effect
    from goa2.domain.models import RuneType
    from goa2.domain.models.effect import EffectType

    for mask in range(4):
        state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        state.turn = 2
        hero = state.get_hero("hero_snorri")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(1, 0))
        card = next(card for card in hero.deck if card.effect_id == "ancestral_boon")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        resolved = ally.hand.pop(0)
        resolved.state, resolved.is_facedown, resolved.played_this_round = CardState.RESOLVED, False, True
        ally.played_cards = [resolved]
        ally.discard_card(ally.hand[0])
        state.active_effects = [
            effect(state, "ancestral-swap", EffectType.MOVEMENT_ZONE, source_card_id=resolved.id),
        ]
        state.execution_context.update(ally_hand=ally.hand, ally_played=ally.played_cards)
        if mask & 1:
            hero.rune_slots[2] = RuneType.AXE
        if mask & 2:
            state.execution_context["snorri_ult_rune_action"] = RuneType.ANVIL.value

        def choose(request, _mode):
            if request.prompt == "Select a friendly hero in radius":
                return ally.id
            return selection_value(request.options[0])

        operations, _ = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        yield f"ancestral-boon-{mask}", state, operations


def ancestral_grace_traces():
    from aura_traces import _board
    from goa2.domain.models import RuneType

    for zone in ("hand", "discard", "played", "current", "deck"):
        state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        state.turn = 2
        hero = state.get_hero("hero_snorri")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(1, 0))
        card = next(card for card in hero.deck if card.effect_id == "ancestral_grace")
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        hero.rune_slots[2] = RuneType.BIRD
        item = next(card for card in ally.deck if card.tier.value == "II" and card.item)
        target = next(card for card in ally.deck if card is not item and card.tier == item.tier
                      and card.color == item.color and card.item)
        item.state = CardState.ITEM
        ally.items[item.item] = 1
        if zone == "hand":
            ally.hand.append(target)
            target.state = CardState.HAND
        elif zone == "discard":
            ally.discard_pile.append(target)
            target.state = CardState.DISCARD
        elif zone == "played":
            ally.played_cards = [target]
            target.state = CardState.RESOLVED
        elif zone == "current":
            ally.current_turn_card = target
            target.state = CardState.UNRESOLVED
        else:
            target.state = CardState.DECK
        state.execution_context.update(ally_items=ally.items, ally_hand=ally.hand)

        def choose(request, _mode):
            if request.prompt == "Select a friendly hero in radius":
                return ally.id
            return selection_value(request.options[0])

        if zone in ("discard", "played"):
            from goa2.engine.steps import SwapItemCardStep

            state.execution_context.update(swap_hero=ally.id, swap_item=item.id, swap_target=target.id)
            step = SwapItemCardStep(hero_key="swap_hero", item_card_key="swap_item", target_card_key="swap_target")
        else:
            step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
        operations, final = retain(state, [step], select=choose)
        final_ally = final.get_hero(ally.id)
        assert next(card for card in final_ally.deck if card.id == target.id).state == CardState.ITEM
        yield f"ancestral-grace-{zone}", state, operations


def rune_sigils_traces():
    from aura_traces import _board
    from goa2.domain.models import RuneType

    variants = (
        ("plain", ()),
        ("axe", (RuneType.AXE,)),
        ("anvil", (RuneType.ANVIL,)),
        ("horn", (RuneType.HORN,)),
        ("axe-anvil-horn", (RuneType.AXE, RuneType.ANVIL, RuneType.HORN)),
        ("bird-current", (RuneType.BIRD,)),
        ("bird-horn", (RuneType.BIRD, RuneType.HORN)),
    )
    for variant, runes in variants:
        state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        state.turn = 2
        hero = state.get_hero("hero_snorri")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity("hero_hanu", h(1, 0))
        state.place_entity("hero_tigerclaw", h(2, 0))
        target = state.teams["BLUE"].minions[0]
        state.place_entity(target.id, h(0, 2))
        card = next(card for card in hero.deck if card.effect_id == "rune_sigils")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        if runes:
            hero.rune_slots[2] = runes[0]
        for key, rune in zip(("snorri_ult_rune_action", "snorri_ult_rune_defense"), runes[1:]):
            state.execution_context[key] = rune.value

        def choose(request, _mode):
            if request.request_type.value == "SELECT_CARD_OR_PASS":
                return "PASS"
            if RuneType.BIRD in runes and any(option.id == target.id for option in request.options):
                return target.id
            return selection_value(request.options[0])

        operations, _ = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        yield f"rune-sigils-{variant}", state, operations


def snorri_integration_traces():
    from aura_traces import _board
    from goa2.domain.models import RuneType
    from goa2.engine.steps import PerformCardActionStep

    for mode in ("mastery-action", "copied-passage"):
        state = setup(["Snorri", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        state.turn = 2
        owner = state.get_hero("hero_snorri")
        actor = owner if mode == "mastery-action" else state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = actor.id
        state.place_entity(actor.id, h(0, 0))
        card = next(card for card in owner.deck if card.effect_id == "deep_passage")
        if card in owner.hand:
            owner.hand.remove(card)
        owner.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        owner.rune_slots = {1: RuneType.HORN, 2: RuneType.BIRD, 3: RuneType.ANVIL}
        if mode == "mastery-action":
            owner.level = 8
            owner.ultimate_card.is_facedown = False
            owner.ultimate_card.state = CardState.PASSIVE
            step = ResolveCardStep(hero_id=actor.id)
        else:
            state.execution_context.update(copied_passage=card.id, passage_owner=owner.id)
            step = PerformCardActionStep(
                card_key="copied_passage", card_owner_key="passage_owner",
                hero_id=actor.id, suppress_after_resolve_card=True,
            )

        def choose(request, _mode):
            if request.request_type.value == "CHOOSE_ACTION":
                return card.primary_action.value
            if request.prompt.startswith("Rune Mastery:"):
                return RuneType.HORN.value
            return selection_value(request.options[0])

        operations, _ = retain(state, [step], select=choose)
        if mode == "mastery-action":
            assert any(op.get("selection") == RuneType.HORN.value for op in operations)
        yield mode, state, operations


def whisper_shadow_traces():
    from aura_traces import _board
    from goa2.domain.board import Zone
    from goa2.domain.models import MinionType, TeamColor
    from goa2.domain.models.spawn import SpawnPoint, SpawnType

    for effect in ("shadow_step", "shadow_walk", "creeping_shadow"):
        for mode in ("spawn", "adjacent", "occupied", "wrong-zone", "hero-spawn"):
            state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_whisper")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            spawn = h(2, 0)
            state.board.tiles[spawn].zone_id = "quiet" if mode == "wrong-zone" else "battle"
            state.board.zones = {
                zone: Zone(id=zone, hexes={spawn} if state.board.tiles[spawn].zone_id == zone else set())
                for zone in ("battle", "quiet")
            }
            state.battle_zones = {"lane": "battle"}
            state.wave_counters = {"lane": 0}
            is_hero = mode == "hero-spawn"
            point = SpawnPoint(location=spawn, team=TeamColor.BLUE,
                               type=SpawnType.HERO if is_hero else SpawnType.MINION,
                               minion_type=None if is_hero else MinionType.MELEE)
            state.board.tiles[spawn].spawn_point = point
            state.board.spawn_points = [point]
            if mode == "occupied":
                state.place_entity("hero_hanu", spawn)

            def choose(request, _mode):
                wanted = h(1, 0) if mode == "adjacent" and effect == "creeping_shadow" else spawn
                return next(value for option in request.options
                            if (value := selection_value(option)) == wanted.model_dump())

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            expected = h(0, 0)
            if mode in ("spawn", "adjacent"):
                expected = h(1, 0) if mode == "adjacent" and effect == "creeping_shadow" else spawn
            assert final.entity_locations[hero.id] == expected
            yield f"{effect}-{mode}", state, operations


def whisper_fear_traces():
    from aura_traces import _board
    from goa2.domain.models import MinionType, TeamColor
    from goa2.domain.models.spawn import SpawnPoint, SpawnType

    for distance, effect in enumerate(("seeds_of_fear", "sprouts_of_panic", "blooming_nightmare"), 1):
        for mode in ("minion-spawn", "hero-spawn", "no-spawn", "friendly"):
            state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            hero = state.get_hero("hero_whisper")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            spawn = h(1, 0)
            target = "hero_brogan" if mode == "friendly" else "hero_hanu"
            state.place_entity(target, spawn)
            if mode != "no-spawn":
                is_hero = mode == "hero-spawn"
                point = SpawnPoint(location=spawn, team=TeamColor.BLUE,
                                   type=SpawnType.HERO if is_hero else SpawnType.MINION,
                                   minion_type=None if is_hero else MinionType.MELEE)
                state.board.tiles[spawn].spawn_point = point
                state.board.spawn_points = [point]
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            destination = h(1 + distance, 0)

            def choose(request, _mode):
                if request.prompt == "Select destination for the target":
                    return next(value for option in request.options
                                if (value := selection_value(option)) == destination.model_dump())
                return selection_value(request.options[0])

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.entity_locations[target] == (destination if mode.endswith("-spawn") and mode != "no-spawn" else spawn)
            yield f"{effect}-{mode}", state, operations


def whisper_trail_traces():
    from aura_traces import _board
    from goa2.domain.board import Zone
    from goa2.domain.models import MinionType, TeamColor
    from goa2.domain.models.spawn import SpawnPoint, SpawnType
    from primary_templates import WHISPER_TRAIL_EFFECTS

    for effect in WHISPER_TRAIL_EFFECTS:
        for mode in ("zero", "one", "two", "capped", "occupied", "wrong-zone", "hero-spawn"):
            state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_whisper")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_hanu", h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            count = {"zero": 0, "one": 1, "two": 2, "capped": 4}.get(mode, 1)
            spawns = [h(0, 2), h(1, 1), h(-1, 1), h(2, -1)][:count]
            zone = "quiet" if mode == "wrong-zone" else "battle"
            state.board.zones = {name: Zone(id=name, hexes=set(spawns) if name == zone else set())
                                 for name in ("quiet", "battle")}
            state.battle_zones = {"lane": "battle"}
            state.wave_counters = {"lane": 0}
            for spawn in spawns:
                is_hero = mode == "hero-spawn"
                point = SpawnPoint(location=spawn, team=TeamColor.BLUE,
                                   type=SpawnType.HERO if is_hero else SpawnType.MINION,
                                   minion_type=None if is_hero else MinionType.MELEE)
                state.board.tiles[spawn].zone_id = zone
                state.board.tiles[spawn].spawn_point = point
                state.board.spawn_points.append(point)
            if mode == "occupied":
                state.place_entity("hero_tigerclaw", spawns[0])

            def choose(request, _mode):
                if request.prompt == "Select destination to move to":
                    return h(0, 1).model_dump()
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                return selection_value(request.options[0])

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.entity_locations[hero.id] == (h(0, 1) if mode in ("one", "two", "capped") else h(0, 0))
            yield f"{effect}-{mode}", state, operations


def whisper_swap_traces():
    from aura_traces import _board

    for effect, distance in (("cruel_twist", 2), ("sealed_fate", 3)):
        for mode in ("adjacent-ally", "ranged-enemy", "skip-move"):
            state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_whisper")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_brogan", h(0, 1))
            state.place_entity(enemy.id, h(2, 0))
            state.place_entity("hero_tigerclaw", h(0, 2))
            enemy.discard_card(enemy.hand[0])
            target = "hero_brogan" if mode == "adjacent-ally" else enemy.id
            target_hex = state.entity_locations[target]
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            destination = h(-distance, 1) if mode == "adjacent-ally" else h(2, -distance)

            def choose(request, _mode):
                if request.prompt == "Select a unit to swap with":
                    offered = [selection_value(option) for option in request.options]
                    assert "hero_tigerclaw" not in offered
                    assert target in offered, (effect, mode, offered)
                    return target
                if request.prompt == "Select destination to move to":
                    offered = [selection_value(option) for option in request.options]
                    assert destination.model_dump() in offered or mode == "skip-move", (effect, mode, destination, offered)
                    return "SKIP" if mode == "skip-move" else destination.model_dump()
                return selection_value(request.options[0])

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.entity_locations[target] == h(0, 0)
            assert final.entity_locations[hero.id] == (target_hex if mode == "skip-move" else destination)
            yield f"{effect}-{mode}", state, operations


def whisper_repeat_attack_traces():
    from aura_traces import _board

    for effect in ("blood_fury", "blood_frenzy"):
        for mode in ("no-discard", "decline", "repeat"):
            state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_whisper")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, h(1, 0))
            state.place_entity("hero_tigerclaw", h(0, 1))
            if mode != "no-discard":
                enemy.discard_card(enemy.hand[0])
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            targets = []
            prompts = []

            def choose(request, _mode):
                if request.prompt.startswith("Repeat attack"):
                    prompts.append(request.prompt)
                    return "YES" if mode == "repeat" and len(prompts) == 1 else "NO"
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                options = [selection_value(option) for option in request.options]
                if any(target in options for target in ("hero_hanu", "hero_tigerclaw")):
                    # Leave the hero with the discarded card in range after the first attack.
                    target = enemy.id if targets else "hero_tigerclaw"
                    assert target in options and not any(previous in options for previous in targets)
                    targets.append(target)
                    return target
                return options[0]

            operations, _ = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert len(targets) == (2 if mode == "repeat" else 1)
            assert bool(prompts) == (mode != "no-discard")
            yield f"{effect}-{mode}", state, operations


def whisper_victim_choice_traces():
    from aura_traces import _board

    for effect in ("lesser_evil", "greater_good"):
        for mode in ("discard", "empty-hand", "defeat-minions", "skip-minions"):
            state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_whisper")
            victim = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(victim.id, h(1, 0))
            minions = state.teams["BLUE"].minions[:3]
            for minion, coordinates in zip(minions, ((0, 1), (-1, 1), (0, -1)), strict=True):
                state.place_entity(minion.id, h(*coordinates))
            if mode == "empty-hand":
                for held in list(victim.hand):
                    victim.discard_card(held)
            hand_size = len(victim.hand)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            defeated = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Choose one":
                    assert request.player_id == victim.id
                    return 1 if mode in ("discard", "empty-hand") else 2
                if "adjacent minion" in request.prompt:
                    assert request.player_id == hero.id
                    if mode == "skip-minions":
                        return "DONE" if effect == "greater_good" else "SKIP"
                    target = next(minion.id for minion in minions if minion.id in options)
                    defeated.append(target)
                    return target
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert len(final.get_hero(victim.id).hand) == hand_size - (mode == "discard")
            assert (victim.id not in final.entity_locations) == (effect == "greater_good" and mode == "empty-hand")
            expected_defeats = (3 if effect == "greater_good" else 1) if mode == "defeat-minions" else 0
            assert len(defeated) == expected_defeats
            assert all(target not in final.entity_locations for target in defeated)
            yield f"{effect}-{mode}", state, operations


def whisper_death_seeker_traces():
    from aura_traces import _board

    for level, mode in ((7, "ineligible"), (7, "immunity"), (7, "retrieve"),
                        (8, "immunity"), (8, "retrieve"), (8, "decline"),
                        (8, "no-ultimate"), (8, "ultimate-face")):
        state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_whisper")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(1, 0))
        hero.level = level
        if mode == "no-ultimate":
            hero.ultimate_card = None
        elif level == 8:
            hero.ultimate_card.is_facedown = False
            hero.ultimate_card.state = CardState.PASSIVE
        if mode != "ineligible":
            enemy.discard_card(enemy.hand[0])
        discarded = next(held for held in hero.hand if held.effect_id != "death_seeker")
        hero.discard_card(discarded)
        card = hero.ultimate_card if mode == "ultimate-face" else next(
            card for card in hero.deck if card.effect_id == "death_seeker"
        )
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False

        def choose(request, _mode):
            if request.prompt == "Choose one":
                return 2 if mode in ("retrieve", "decline") else 1
            if "Also become immune" in request.prompt:
                return 2 if mode == "decline" else 1
            return selection_value(request.options[0])

        operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        both = level == 8 and mode in ("immunity", "retrieve")
        immunity = any(effect.effect_type.value == "immunity_enemy_actions" for effect in final.active_effects)
        assert immunity == (both or mode in ("immunity", "no-ultimate")), (level, mode, final.active_effects)
        assert (discarded.id in {held.id for held in final.get_hero(hero.id).hand}) == (
            both or mode in ("retrieve", "decline")
        )
        yield f"death-seeker-{level}-{mode}", state, operations


def whisper_forced_movement_traces():
    from aura_traces import _board
    from goa2.engine.steps.movement import ForceDefenseCardMovementStep

    for mode in ("secondary-discard", "secondary-played", "primary", "facedown", "missing", "blocked"):
        state = setup(["Whisper", "Brogan"], ["Hanu", "Arien"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        attacker = state.get_hero("hero_whisper")
        defender = state.get_hero("hero_arien" if mode == "primary" else "hero_hanu")
        state.current_actor_id = state.resolution_owner_id = attacker.id
        state.place_entity(attacker.id, h(0, 0))
        origin = h(1, 0)
        state.place_entity(defender.id, origin)
        card_id = "slippery_ground" if mode == "primary" else "safe_travels"
        defense = next(card for card in defender.deck if card.id == card_id)
        if defense in defender.hand:
            defender.hand.remove(defense)
        defense.state, defense.is_facedown = CardState.RESOLVED, mode == "facedown"
        if mode == "secondary-played":
            defender.played_cards.append(defense)
        elif mode != "missing":
            defender.discard_pile.append(defense)
        state.execution_context.update(
            defense_card_id=defense.id,
            swift_target=defender.id,
            current_card_id="swift_justice",
        )
        destination = h(1, -3)
        if mode == "blocked":
            state.board.tiles = {point: state.board.tiles[point] for point in (h(0, 0), origin)}

        def choose(request, _mode):
            if request.prompt.startswith("Select Movement Destination"):
                assert request.player_id == defender.id and not request.can_skip
                options = [selection_value(option) for option in request.options]
                assert destination.model_dump() in options
                return destination.model_dump()
            return selection_value(request.options[0])

        operations, final = retain(
            state, [ForceDefenseCardMovementStep(defender_key="swift_target")], select=choose
        )
        assert final.current_actor_id == attacker.id
        assert final.execution_context["current_card_id"] == "swift_justice"
        expected = origin if mode in ("facedown", "missing", "blocked") else destination
        assert final.entity_locations[defender.id] == expected, (mode, final.entity_locations)
        yield f"swift-justice-force-{mode}", state, operations


def whisper_swift_justice_traces():
    from aura_traces import _board

    for mode in ("ranged", "adjacent", "no-ranged-target", "both-ranged-first",
                 "both-adjacent-first", "decline-extra"):
        state = setup(["Whisper", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_whisper")
        ranged = state.get_hero("hero_hanu")
        adjacent = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ranged.id, h(2, 0))
        state.place_entity(adjacent.id, h(0, 1))
        if mode.startswith("both-") or mode == "decline-extra":
            hero.level = 8
            hero.ultimate_card.is_facedown = False
            hero.ultimate_card.state = CardState.PASSIVE
        if mode == "no-ranged-target":
            ranged.discard_card(ranged.hand[0])
        defense = next(held for held in ranged.deck if held.id == "unexpected_journey")
        if defense not in ranged.hand:
            ranged.hand.append(defense)
        defense.is_facedown = False
        card = next(card for card in hero.deck if card.effect_id == "swift_justice")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        destination = h(2, -3)

        def choose(request, _mode):
            if request.prompt == "Choose one":
                return 2 if mode in ("adjacent", "both-adjacent-first") else 1
            if request.request_type.value == "SELECT_CARD_OR_PASS":
                return defense.id
            if request.prompt.startswith("Select Movement Destination"):
                assert request.player_id == ranged.id
                return destination.model_dump()
            if mode == "decline-extra" and request.can_skip:
                return "SKIP"
            return selection_value(request.options[0])

        operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        assert final.current_actor_id == hero.id
        moved = mode not in ("adjacent", "no-ranged-target")
        assert final.entity_locations[ranged.id] == (destination if moved else h(2, 0))
        defeated = mode in ("adjacent", "both-ranged-first", "both-adjacent-first")
        assert (adjacent.id not in final.entity_locations) == defeated
        yield f"swift-justice-{mode}", state, operations


def brynn_swap_traces():
    from aura_traces import _board

    for effect, distance in (("tread_lightly", 0), ("cover_tracks", 1), ("hide_traces", 2)):
        for mode in ("adjacent", "two-obstacles", "three-obstacles", "ultimate", "hidden-ultimate"):
            state = setup(["Brynn", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_brynn")
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin = h(0, 0)
            state.place_entity(hero.id, origin)
            target, position = ("hero_brogan", h(0, 1)) if mode == "adjacent" else ("hero_hanu", h(2, 0))
            state.place_entity(target, position)
            if mode in ("two-obstacles", "three-obstacles"):
                state.board.tiles[h(2, 1)].is_terrain = True
                state.board.tiles[h(3, -1)].is_terrain = True
                if mode == "three-obstacles":
                    state.place_entity(state.teams["BLUE"].minions[0].id, h(3, 0))
            if mode in ("ultimate", "hidden-ultimate"):
                hero.level = 8
                hero.ultimate_card.state = CardState.PASSIVE
                hero.ultimate_card.is_facedown = mode == "hidden-ultimate"
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            destination = h(2, -distance)
            should_swap = mode in ("adjacent", "three-obstacles", "ultimate")
            should_move = distance > 0 and mode in ("three-obstacles", "ultimate")

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Select a unit to swap with":
                    assert should_swap and target in options
                    return target
                if request.prompt.startswith("You may move"):
                    if not should_move:
                        return "SKIP"
                    assert destination.model_dump() in options
                    return destination.model_dump()
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.entity_locations[target] == (origin if should_swap else position)
            expected = destination if should_move else position if should_swap else origin
            assert final.entity_locations[hero.id] == expected
            yield f"{effect}-{mode}", state, operations


def brynn_obstacle_attack_traces():
    from aura_traces import _board

    for effect in ("brynn_high_ground", "elevated_ambush", "peak_precision", "familiar_ground"):
        for mode in ("unqualified", "qualified", "ultimate", "minion"):
            state = setup(["Brynn", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_brynn")
            enemy = state.get_hero("hero_hanu")
            ranged = effect == "familiar_ground" and mode != "minion"
            target = state.teams["BLUE"].minions[0].id if mode == "minion" else enemy.id
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            x = 2 if ranged else 1
            state.place_entity(target, h(x, 0))
            if mode == "qualified":
                for point in (h(x, 1), h(x + 1, -1), h(x + 1, 0)):
                    state.board.tiles[point].is_terrain = True
            if mode in ("ultimate", "minion"):
                hero.level = 8
                hero.ultimate_card.state = CardState.PASSIVE
                hero.ultimate_card.is_facedown = False
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                if request.prompt == "Choose one":
                    return 2 if ranged else 1
                if request.prompt == "You may retrieve this card":
                    return 1 if mode == "qualified" else 0
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                return selection_value(request.options[0])

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            qualified = mode in ("qualified", "ultimate")
            if effect != "familiar_ground":
                assert final.execution_context.get("ha_bonus", 0) == (2 if qualified else 0)
            elif mode == "unqualified":
                assert target in final.entity_locations
            else:
                assert target not in final.entity_locations
            yield f"{effect}-{mode}", state, operations


def brynn_followup_traces():
    from aura_traces import _board

    groups = (
        (("bear_trap", "log_trap", "deadfall_trap"), ("adjacent", "qualified", "empty-hand", "unqualified")),
        (("true_grit", "die_hard"), ("qualified", "unqualified", "skip")),
        (("split_attack", "split_throw"), ("qualified", "unqualified", "skip")),
    )
    for effects, modes in groups:
        for effect in effects:
            for mode in modes:
                state = setup(["Brynn", "Brogan"], ["Hanu", "Tigerclaw"])
                h = _board(state)
                state.phase = GamePhase.RESOLUTION
                hero = state.get_hero("hero_brynn")
                enemy = state.get_hero("hero_hanu")
                state.current_actor_id = state.resolution_owner_id = hero.id
                origin = h(0, 0)
                state.place_entity(hero.id, origin)
                x = 1 if mode == "adjacent" or effect.startswith("split_") else 2
                state.place_entity(enemy.id, h(x, 0))
                if mode != "unqualified":
                    for point in (h(x, 1), h(x + 1, -1), h(x + 1, 0)):
                        state.board.tiles[point].is_terrain = True
                if mode == "empty-hand":
                    for held in list(enemy.hand):
                        enemy.discard_card(held)
                hand_size = len(enemy.hand)
                minion = state.teams["BLUE"].minions[0]
                if effect.startswith("split_"):
                    state.place_entity(minion.id, h(0, 1))
                card = next(card for card in hero.deck if card.effect_id == effect)
                if card in hero.hand:
                    hero.hand.remove(card)
                hero.current_turn_card = card
                card.state, card.is_facedown = CardState.UNRESOLVED, False
                retrieved = None
                if effect in ("true_grit", "die_hard"):
                    retrieved = next(item for item in hero.deck if item.effect_id == "familiar_ground")
                    if retrieved not in hero.hand:
                        hero.hand.append(retrieved)
                    hero.discard_card(retrieved)
                destination = h(-4 if effect == "die_hard" else -3, 0)

                def choose(request, _mode):
                    if request.prompt == "Choose one":
                        return 1 if mode == "adjacent" else 2
                    if request.prompt.startswith("You may retrieve"):
                        return "SKIP" if mode == "skip" else retrieved.id
                    if request.prompt.startswith("Move up to"):
                        return "SKIP" if mode == "skip" else destination.model_dump()
                    if request.prompt.startswith("Repeat the attack"):
                        return "NO" if mode == "skip" else "YES"
                    if request.request_type.value == "SELECT_CARD_OR_PASS":
                        return "PASS"
                    options = [selection_value(option) for option in request.options]
                    if request.prompt == "Target a unit in range":
                        assert enemy.id in options
                        return enemy.id
                    if request.prompt == "Repeat on a different unit":
                        assert enemy.id not in options and minion.id in options
                        return minion.id
                    return options[0]

                operations, final = retain(
                    state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose
                )
                if effect.endswith("_trap"):
                    assert (enemy.id not in final.entity_locations) == (effect == "deadfall_trap" and mode == "empty-hand")
                    assert len(final.get_hero(enemy.id).hand) == hand_size - (mode in ("adjacent", "qualified"))
                elif retrieved is not None:
                    assert (retrieved.id in {held.id for held in final.get_hero(hero.id).hand}) == (mode != "skip")
                    assert final.entity_locations[hero.id] == (destination if mode == "qualified" else origin)
                else:
                    assert (minion.id not in final.entity_locations) == (mode == "qualified")
                yield f"{effect}-{mode}", state, operations


def brynn_displacement_traces():
    from aura_traces import _board

    for effect in ("mountain_guide", "expedition_leader", "decoy"):
        for mode in ("qualified", "unqualified", "skip-first", "skip-destination"):
            state = setup(["Brynn", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_brynn")
            enemy = state.get_hero("hero_hanu")
            decoy = effect == "decoy"
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            minion = state.teams["BLUE" if decoy else "RED"].minions[0]
            first = minion.id if decoy else "hero_brogan"
            second = enemy.id if decoy else minion.id
            first_origin = h(0, 1)
            second_origin = h(2, 0) if decoy else h(0, -2)
            state.place_entity(first, first_origin)
            state.place_entity(second, second_origin)
            if not decoy:
                state.place_entity(enemy.id, h(2, 0))
            if mode != "unqualified":
                for point in (h(2, 1), h(3, -1), h(3, 0)):
                    state.board.tiles[point].is_terrain = True
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            distance = 3 if effect == "expedition_leader" else 1 if decoy else 2
            first_dest = h(-distance, 1)
            second_dest = h(2, -1) if decoy else h(distance, -2)
            choices = 0
            selected = None

            def choose(request, _mode):
                nonlocal choices, selected
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith(("You may move", "Move a different", "Move an enemy")):
                    choices += 1
                    if choices == 1 and mode == "skip-first":
                        return "SKIP"
                    if choices == 2 and mode != "skip-first":
                        assert first not in options
                    selected = first if choices == 1 else second
                    assert selected in options
                    return selected
                if request.prompt.startswith(("Move the unit", "Move it")):
                    if selected == first and mode == "skip-destination":
                        return "SKIP"
                    destination = first_dest if selected == first else second_dest
                    assert destination.model_dump() in options
                    return destination.model_dump()
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.entity_locations[first] == (
                first_origin if mode in ("skip-first", "skip-destination") else first_dest
            )
            assert final.entity_locations[second] == (second_origin if mode == "unqualified" else second_dest)
            yield f"{effect}-{mode}", state, operations


def garrus_charge_traces():
    from aura_traces import _board

    for effect, distance in (("trace", 1), ("chase", 2), ("hunt_down", 3)):
        for mode in ("charge", "skip-move", "empty-discard", "standard"):
            state = setup(["Garrus", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_garrus")
            enemy = state.get_hero("hero_hanu")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin = h(0, 0)
            destination = h(distance, 0)
            state.place_entity(hero.id, origin)
            state.place_entity(enemy.id, h(distance + 1 if mode == "charge" else 1, 0))
            state.place_entity(minion.id, h(0, 1))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            if mode in ("charge", "skip-move"):
                hero.discard_card(hero.hand[0])

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Choose one":
                    return 2 if mode == "standard" else 1
                if request.prompt.startswith("Move up to"):
                    return "SKIP" if mode == "skip-move" else destination.model_dump()
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                target = minion.id if mode == "standard" else enemy.id
                assert target in options
                if mode != "standard":
                    assert minion.id not in options
                return target

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.entity_locations[hero.id] == (destination if mode == "charge" else origin)
            assert (minion.id not in final.entity_locations) == (mode == "standard")
            assert (enemy.id not in final.entity_locations) == (mode != "standard")
            yield f"{effect}-{mode}", state, operations


def brynn_ultimate_traces():
    state = setup(["Brynn", "Brogan"], ["Hanu", "Tigerclaw"])
    state.phase = GamePhase.RESOLUTION
    hero = state.get_hero("hero_brynn")
    state.current_actor_id = state.resolution_owner_id = hero.id
    card = hero.ultimate_card
    hero.current_turn_card = card
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    operations, _ = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)])
    yield "over-the-top-primary", state, operations


def garrus_push_traces():
    from aura_traces import _board

    for effect, maximum in (("menace", 0), ("threaten", 1), ("terrify", 2)):
        for repeats in range(maximum + 1):
            state = setup(["Garrus", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_garrus")
            enemy = state.get_hero("hero_hanu")
            other = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, h(1, 0))
            state.place_entity(other.id, h(0, 1))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            selected = 0
            accepted = 0

            def choose(request, _mode):
                nonlocal selected, accepted
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Push another enemy"):
                    if accepted == repeats:
                        return "NO"
                    accepted += 1
                    return "YES"
                if request.prompt.startswith("Select an enemy unit"):
                    selected += 1
                    target = enemy.id if selected < 3 else other.id
                    if selected == 3:
                        assert enemy.id not in options
                    assert target in options
                    return target
                if request.prompt == "Select a space 1 farther from you":
                    destination = h(selected + 1, 0) if selected < 3 else h(0, 2)
                    assert destination.model_dump() in options
                    return destination.model_dump()
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert selected == 1 + repeats
            assert final.entity_locations[enemy.id] == h(2 + min(repeats, 1), 0)
            assert final.entity_locations[other.id] == h(0, 2 if repeats == 2 else 1)
            yield f"{effect}-repeat-{repeats}", state, operations


def garrus_retrieval_traces():
    from aura_traces import _board

    for effect, maximum in (("hold_ground", 1), ("make_a_stand", 1), ("battle_ready", 2)):
        for mode in ("one-hero", "two-heroes", "skip-first", "one-card"):
            state = setup(["Garrus", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_garrus")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_hanu", h(1, 0))
            if mode == "one-hero":
                state.place_entity(state.teams["BLUE"].minions[0].id, h(0, 1))
            else:
                state.place_entity("hero_tigerclaw", h(0, 1))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            discarded = list(hero.hand[:1 if mode == "one-card" else 2])
            for held in discarded:
                hero.discard_card(held)
            selected = []

            def choose(request, _mode):
                assert request.player_id == hero.id
                if mode == "skip-first" and request.prompt.endswith("(1)"):
                    return "SKIP"
                options = [selection_value(option) for option in request.options]
                assert all(previous not in options for previous in selected)
                selected.append(options[0])
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            expected = {"one-hero": 0, "two-heroes": maximum, "skip-first": maximum - 1, "one-card": 1}[mode]
            assert len(selected) == expected
            remaining = {held.id for held in final.get_hero(hero.id).discard_pile}
            assert remaining == {held.id for held in discarded} - set(selected)
            yield f"{effect}-{mode}", state, operations


def garrus_pilum_traces():
    from aura_traces import _board

    for effect, distance in (("light_pilum", 1), ("heavy_pilum", 2)):
        for mode in ("discard", "empty-hand", "skip-target", "skip-move", "no-target"):
            state = setup(["Garrus", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_garrus")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin, destination = h(0, 0), h(-distance, 0)
            state.place_entity(hero.id, origin)
            if mode != "no-target":
                state.place_entity(enemy.id, h(1, 0))
            state.place_entity(state.teams["BLUE"].minions[0].id, h(0, 1))
            if mode == "empty-hand":
                for held in list(enemy.hand):
                    enemy.discard_card(held)
            hand_size = len(enemy.hand)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Select an enemy hero in range":
                    assert request.player_id == hero.id and options == [enemy.id]
                    return "SKIP" if mode == "skip-target" else enemy.id
                if request.prompt.startswith("Move "):
                    assert request.player_id == hero.id and destination.model_dump() in options
                    return "SKIP" if mode == "skip-move" else destination.model_dump()
                assert request.player_id == enemy.id
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert len(final.get_hero(enemy.id).hand) == hand_size - (mode in ("discard", "skip-move"))
            absent = mode == "no-target" or (effect == "heavy_pilum" and mode == "empty-hand")
            assert (enemy.id not in final.entity_locations) == absent
            assert final.entity_locations[hero.id] == (origin if mode == "skip-move" else destination)
            yield f"{effect}-{mode}", state, operations


def garrus_untiered_traces():
    from aura_traces import _board

    for effect in ("angry_strike", "chilling_howl"):
        for mode in (range(3) if effect == "angry_strike" else (0, 1, 2, "fury-decline", "fury-accept")):
            state = setup(["Garrus", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_garrus")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_hanu", h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            if effect == "angry_strike":
                for held in list(hero.hand[:mode]):
                    hero.discard_card(held)
            elif mode:
                resolved = next(held for held in hero.deck if held.effect_id == "angry_strike")
                if resolved in hero.hand:
                    hero.hand.remove(resolved)
                resolved.state, resolved.is_facedown = CardState.RESOLVED, False
                hero.played_cards.append(resolved)
                if isinstance(mode, str):
                    hero.level = 8
                    hero.ultimate_card.state = CardState.PASSIVE
                    hero.ultimate_card.is_facedown = False

            def choose(request, _mode):
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                if request.prompt.startswith("Optionally discard"):
                    return "SKIP" if mode == 1 else resolved.id
                if request.prompt.startswith("Battle Fury:"):
                    return "YES" if mode == "fury-accept" else "NO"
                return selection_value(request.options[0])

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            if effect == "angry_strike":
                assert final.execution_context["angry_discard_count"] == mode
            else:
                assert any(effect.effect_type.value == "movement_zone" for effect in final.active_effects)
                assert len(final.get_hero(hero.id).discard_pile) == (mode not in (0, 1))
                assert (final.execution_context.get("angry_discard_count") == 1) == (mode == "fury-accept")
            yield f"{effect}-{mode}", state, operations


def garrus_displacement_traces():
    from aura_traces import _board

    for effect, maximum in (("form_up", 1), ("testudo", 2), ("blunt_force", 1), ("send_flying", 2)):
        dash = effect in ("blunt_force", "send_flying")
        for count in range(maximum + 1 if not dash else 2):
            state = setup(["Garrus", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_garrus")
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin = h(-2, 0) if dash else h(0, 0)
            state.place_entity(hero.id, origin)
            target = "hero_hanu" if dash else "hero_brogan"
            other = state.teams["BLUE" if dash else "RED"].minions[0].id
            target_x = maximum - 1 if dash else 3
            state.place_entity(target, h(target_x, 0))
            state.place_entity(other, h(-2, 1) if dash else h(0, 2))
            if dash:
                state.board.tiles[h(target_x + 1, 0)].is_terrain = True
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            selected = 0
            accepted = 0

            def choose(request, _mode):
                nonlocal selected, accepted
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Pull another"):
                    if accepted == count:
                        return "NO"
                    accepted += 1
                    return "YES"
                if request.prompt.startswith("Select a friendly"):
                    selected += 1
                    return target if selected < 3 else other
                if request.prompt == "Select a space 1 closer to you":
                    return (h(3 - selected, 0) if selected < 3 else h(0, 1)).model_dump()
                if request.prompt.startswith("Move up to"):
                    return h(target_x - 1, 0).model_dump() if count else "SKIP"
                if request.prompt.startswith("Select an adjacent enemy hero"):
                    return target
                if dash and other in options:
                    return other
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            if dash:
                assert other not in final.entity_locations
                assert final.entity_locations[hero.id] == (h(target_x - 1, 0) if count else origin)
                assert final.entity_locations[target] == h(target_x + 3 * count, 0)
            else:
                assert final.entity_locations[target] == h(2 - min(count, 1), 0)
                assert final.entity_locations[other] == h(0, 1 if count == 2 else 2)
            yield f"{effect}-{count}", state, operations


def silverarrow_snipe_traces():
    from aura_traces import _board

    for effect in ("clear_shot", "opportunity_shot", "snap_shot", "long_shot"):
        for mode in ("open", "screened", "adjacent", "beyond"):
            state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_silverarrow")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(-1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            distance = 1 if mode == "adjacent" else card.range_value + (mode == "beyond")
            state.place_entity(enemy.id, h(distance - 1, 0))
            if mode in ("screened", "adjacent"):
                state.place_entity("hero_brogan", h(distance - 1, -1))
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            eligible = mode == "open" or (mode == "screened" if effect == "long_shot" else mode == "adjacent")

            def choose(request, _mode):
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                options = [selection_value(option) for option in request.options]
                assert eligible and options == [enemy.id]
                return enemy.id

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert (enemy.id not in final.entity_locations) == eligible
            yield f"{effect}-{mode}", state, operations


def silverarrow_followup_traces():
    from aura_traces import _board

    for effect in ("grappling_branches", "entangling_vines", "grasping_roots", "rain_of_arrows"):
        modes = ("root-zone",) if effect != "rain_of_arrows" else ("all", "skip-minion", "minion-first", "one-hero")
        for mode in modes:
            state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_silverarrow")
            enemy = state.get_hero("hero_hanu")
            second = state.get_hero("hero_tigerclaw")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            steps = [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)]
            state.place_entity(hero.id, h(0, 0))
            if effect == "rain_of_arrows":
                state.place_entity(enemy.id, h(3, 0))
                if mode != "one-hero":
                    state.place_entity(second.id, h(0, 3))
                state.place_entity(minion.id, h(-3, 0))

            def choose(request, _mode):
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    return "PASS"
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select a different enemy hero"):
                    assert options == [second.id]
                    return second.id
                if request.prompt.startswith("You may target a different enemy minion"):
                    assert options == [minion.id]
                    return "SKIP" if mode == "skip-minion" else minion.id
                target = minion.id if mode == "minion-first" else enemy.id
                assert target in options
                return target

            operations, final = retain(state, steps, select=choose)
            if effect == "rain_of_arrows":
                assert (minion.id not in final.entity_locations) == (mode in ("all", "minion-first"))
                assert (enemy.id not in final.entity_locations) == (mode != "minion-first")
            else:
                zone, = final.active_effects
                assert zone.max_value == 1 and zone.limit_actions_only
                assert zone.effect_type.value == "movement_zone"
            yield f"{effect}-{mode}", state, operations


def silverarrow_gift_traces():
    from aura_traces import _board

    for effect, coins in (("fae_healing", 1), ("natures_blessing", 2)):
        for mode in ("friendly", "enemy", "skip", "empty"):
            state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_silverarrow")
            recipient = state.get_hero("hero_brogan" if mode == "friendly" else "hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(recipient.id, h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            gift = recipient.hand[0]
            if mode != "empty":
                recipient.discard_card(gift)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select a hero"):
                    assert request.player_id == hero.id and options == [recipient.id]
                    return recipient.id
                assert request.player_id == recipient.id and gift.id in options
                return "SKIP" if mode == "skip" else gift.id

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            result = final.get_hero(recipient.id)
            assert (gift.id in {held.id for held in result.discard_pile}) == (mode == "skip")
            assert result.gold == recipient.gold + (coins if mode in ("friendly", "enemy") else 0)
            yield f"{effect}-{mode}", state, operations


def silverarrow_disorient_traces():
    from aura_traces import _board

    for skip in (False, True):
        state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_silverarrow")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(1, 0))
        card = next(card for card in hero.deck if card.effect_id == "disorient")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False

        def choose(request, _mode):
            if request.prompt.startswith("Select an adjacent enemy"):
                return enemy.id
            if request.prompt.startswith("Select destination"):
                return h(2, 0).model_dump()
            assert request.prompt == "You may move 1 space"
            return "SKIP" if skip else h(1, 0).model_dump()

        operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        assert final.entity_locations[enemy.id] == h(2, 0)
        assert final.entity_locations[hero.id] == h(0 if skip else 1, 0)
        yield f"disorient-skip-{skip}", state, operations


def silverarrow_drag_traces():
    from aura_traces import _board

    for effect, maximum in (("lead_astray", 3), ("divert_attention", 2)):
        for mode in ("short", "full", "skip"):
            state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_silverarrow")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, h(1, 0))
            distance = 1 if mode == "short" else maximum
            target = h(1, distance)
            destination = h(-distance, 0)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select an adjacent enemy"):
                    return enemy.id
                if request.prompt.startswith("Select destination"):
                    assert target.model_dump() in options
                    return target.model_dump()
                assert request.prompt == "You may move in a straight line"
                assert destination.model_dump() in options
                assert h(-distance - 1, 0).model_dump() not in options
                assert h(-1, -1).model_dump() not in options
                return "SKIP" if mode == "skip" else destination.model_dump()

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.execution_context["drag_distance_moved"] == distance
            assert final.entity_locations[enemy.id] == target
            assert final.entity_locations[hero.id] == (h(0, 0) if mode == "skip" else destination)
            yield f"{effect}-{mode}", state, operations


def silverarrow_travel_traces():
    from aura_traces import _board
    from goa2.domain.board import Zone

    for effect in ("shoot_and_scoot", "trailblazer"):
        for mode in ("travel", "skip", "same-zone"):
            state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_silverarrow")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin, destination, local = h(0, 0), h(2, 0), h(-1, 0)
            state.place_entity(hero.id, origin)
            if effect == "shoot_and_scoot":
                state.place_entity(minion.id, h(-2, 0))
            state.board.zones = {
                "A": Zone(id="A", hexes=set(), neighbors=[] if mode == "same-zone" else ["B"]),
                "B": Zone(id="B", hexes=set(), neighbors=["A"]),
            }
            for point, tile in state.board.tiles.items():
                tile.zone_id = "A" if point.q <= 0 else "B"
                state.board.zones[tile.zone_id].hexes.add(point)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            traveled = local if mode == "same-zone" and effect == "trailblazer" else destination

            def choose(request, _mode):
                if request.prompt == "Select Fast Travel Destination":
                    options = [selection_value(option) for option in request.options]
                    assert (local.model_dump() in options) == (effect == "trailblazer")
                    assert traveled.model_dump() in options
                    return "SKIP" if mode == "skip" else traveled.model_dump()
                return selection_value(request.options[0])

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            moved = mode == "travel" or (mode == "same-zone" and effect == "trailblazer")
            assert final.entity_locations[hero.id] == (traveled if moved else origin)
            if effect == "trailblazer":
                aura, = final.active_effects
                assert aura.grants_pass_through_obstacles
            else:
                assert minion.id not in final.entity_locations
            yield f"{effect}-{mode}", state, operations


def silverarrow_delayed_traces():
    from aura_traces import _board

    for effect in ("warning_shot", "treetop_sentinel"):
        for mode in ("discard", "empty-hand", "skip", "absent"):
            state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_silverarrow")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            if mode != "absent":
                state.place_entity(enemy.id, h(1, 0))
            if mode == "empty-hand":
                for held in list(enemy.hand):
                    enemy.discard_card(held)
            count = len(enemy.hand)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("End of turn"):
                    assert request.player_id == hero.id and options == [enemy.id]
                    return "SKIP" if mode == "skip" else enemy.id
                assert request.player_id == enemy.id
                return options[0]

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id), FindNextActorStep()],
                select=choose,
            )
            assert final.turn == state.turn + 1
            assert not final.active_effects
            defeated = effect == "treetop_sentinel" and mode == "empty-hand"
            assert (enemy.id not in final.entity_locations) == (mode == "absent" or defeated)
            assert len(final.get_hero(enemy.id).hand) == count - (mode == "discard")
            yield f"{effect}-{mode}", state, operations


def silverarrow_wild_hunt_traces():
    from aura_traces import _board

    for mode in ("move", "decline", "skip-destination", "blocked", "low-level"):
        state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_silverarrow")
        state.current_actor_id = state.resolution_owner_id = hero.id
        origin, destination = h(0, 0), h(2, 0)
        state.place_entity(hero.id, origin)
        hero.level = 7 if mode == "low-level" else 8
        hero.ultimate_card.is_facedown = False
        hero.ultimate_card.state = CardState.PASSIVE
        if mode == "blocked":
            for point in origin.neighbors():
                state.board.tiles[point].is_terrain = True
        card = next(card for card in hero.deck if card.effect_id == "warning_shot")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        prompts = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Wild Hunt: Move 2 spaces in a straight line?":
                prompts.append("offer")
                return "NO" if mode == "decline" else "YES"
            if request.prompt == "Wild Hunt: Select a straight-line destination":
                prompts.append("destination")
                assert destination.model_dump() in options
                assert h(1, 0).model_dump() not in options and h(1, 1).model_dump() not in options
                return "SKIP" if mode == "skip-destination" else destination.model_dump()
            assert card.primary_action.value in options
            return card.primary_action.value

        operations, final = retain(state, [ResolveCardStep(hero_id=hero.id)], select=choose)
        assert final.entity_locations[hero.id] == (destination if mode == "move" else origin)
        assert ("offer" in prompts) == (mode != "low-level")
        assert ("destination" in prompts) == (mode in ("move", "skip-destination"))
        assert any(effect.effect_type.value == "delayed_trigger" for effect in final.active_effects)
        yield f"wild-hunt-before-warning-shot-{mode}", state, operations


def silverarrow_root_enforcement_traces():
    from aura_traces import _board
    from goa2.domain.board import Zone
    from goa2.engine.steps import FinalizeHeroTurnStep

    for effect in ("grappling_branches", "entangling_vines", "grasping_roots"):
        for inside, travel in ((True, False), (False, False), (False, True)):
            state = setup(["Silverarrow", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_silverarrow")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.unresolved_hero_ids.append(enemy.id)
            state.place_entity(hero.id, h(0, 0) if inside else h(-4, 0))
            origin = h(1, 0) if inside else h(4, 0)
            state.place_entity(enemy.id, origin)
            state.board.zones = {
                "A": Zone(id="A", hexes=set(), neighbors=["B"]),
                "B": Zone(id="B", hexes=set(), neighbors=["A"]),
            }
            for point, tile in state.board.tiles.items():
                tile.zone_id = "A" if point.q <= 0 else "B"
                state.board.zones[tile.zone_id].hexes.add(point)
            for player, effect_id in ((hero, effect), (enemy, "unexpected_journey")):
                card = next(card for card in player.deck if card.effect_id == effect_id)
                if card in player.hand:
                    player.hand.remove(card)
                player.current_turn_card = card
                card.state, card.is_facedown = CardState.UNRESOLVED, False
            distance = 1 if inside else 3
            destination = h(origin.q, -distance)
            if travel:
                destination = h(2, -2)
            chosen = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Select Fast Travel Destination":
                    assert travel and request.player_id == enemy.id
                    assert destination.model_dump() in options
                    chosen.append("travel")
                    return destination.model_dump()
                if request.prompt.startswith("Select Movement Destination"):
                    assert request.player_id == enemy.id
                    assert request.prompt.endswith(f"Range {distance})"), request.prompt
                    assert destination.model_dump() in options
                    chosen.append("movement")
                    return destination.model_dump()
                if request.player_id == hero.id and "SKILL" in options:
                    return "SKILL"
                if request.player_id == enemy.id and "MOVEMENT" in options:
                    assert ("FAST_TRAVEL" in options) == (not inside), options
                    return "FAST_TRAVEL" if travel else "MOVEMENT"
                return options[0]

            operations, final = retain(
                state, [ResolveCardStep(hero_id=hero.id), FinalizeHeroTurnStep(hero_id=hero.id)],
                select=choose,
            )
            assert chosen == ["travel" if travel else "movement"]
            assert final.entity_locations[enemy.id] == destination
            assert not final.active_effects
            yield f"{effect}-inside-{inside}-travel-{travel}", state, operations


def takahide_support_traces():
    from aura_traces import _board

    for effect, distance, coins in (
        ("come_to_aid", 3, 0), ("bring_the_relief", 4, 0), ("commit_reserves", 4, 0),
        ("pledge_of_allegiance", 0, 1), ("loyal_retainer", 0, 2),
        ("calculated_risk", 2, 0), ("tactical_gambit", 2, 0),
    ):
        ally_moves = effect in ("calculated_risk", "tactical_gambit")
        for mode in ("discard", "existing", "decline", "no-ally"):
            state = setup(["Takahide", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_takahide")
            ally = state.get_hero("hero_brogan")
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin, ally_origin = h(0, 0), h(1, 0)
            destination = h(1 + distance, 0) if ally_moves else h(-distance, 0)
            state.place_entity(hero.id, origin)
            state.place_entity("hero_hanu", h(0, 1))
            if mode != "no-ally":
                state.place_entity(ally.id, ally_origin)
            if mode == "existing":
                existing = next(held for held in ally.hand if not ally_moves or held.primary_action.value != "ATTACK")
                ally.discard_card(existing)
            if effect in ("commit_reserves", "tactical_gambit"):
                for point, tile in state.board.tiles.items():
                    if point.q == (2 if ally_moves else -1):
                        tile.is_terrain = True
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            gift = hero.hand[0]
            if coins:
                hero.discard_card(gift)
                gift.is_facedown = True

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Choose a friendly hero":
                    assert request.player_id == hero.id and options == [ally.id]
                    return ally.id
                if request.prompt.startswith("You may discard"):
                    assert request.player_id == ally.id
                    if ally_moves:
                        attacks = {held.id for held in ally.hand if held.primary_action.value == "ATTACK"}
                        assert set(options) == attacks
                    return options[0] if mode == "discard" else "SKIP"
                if request.prompt.startswith("You may retrieve"):
                    assert request.player_id == hero.id and gift.id in options
                    return gift.id
                assert request.player_id == (ally.id if ally_moves else hero.id)
                assert destination.model_dump() in options
                return destination.model_dump()

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            enabled = mode in ("discard", "existing")
            assert final.entity_locations[hero.id] == (destination if enabled and not ally_moves else origin)
            if ally_moves and mode != "no-ally":
                assert final.entity_locations[ally.id] == (destination if enabled else ally_origin)
            assert final.get_hero(hero.id).gold == hero.gold + (coins if enabled else 0)
            assert final.get_hero(ally.id).gold == ally.gold + (coins if enabled else 0)
            if coins:
                assert (gift.id in {held.id for held in final.get_hero(hero.id).hand}) == enabled
            yield f"{effect}-{mode}", state, operations


def takahide_color_traces():
    from aura_traces import _board

    for effect in ("proven_warrior", "chosen_champion", "the_right_hand"):
        for mode in ("match", "no-match", "skip", "hidden-source"):
            state = setup(["Takahide", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_takahide")
            ally = state.get_hero("hero_brogan")
            enemies = [state.get_hero(name) for name in ("hero_hanu", "hero_tigerclaw")]
            state.current_actor_id = state.resolution_owner_id = hero.id
            for unit, point in zip((hero, ally, *enemies), (h(0, 0), h(1, 0), h(0, 1), h(-1, 0)), strict=True):
                state.place_entity(unit.id, point)
            shared_colors = set.intersection(*({held.color for held in unit.hand} for unit in (ally, *enemies)))
            source = next(held for held in ally.hand if held.color in shared_colors)
            ally.discard_card(source)
            source.is_facedown = mode == "hidden-source"
            if mode == "no-match":
                for enemy in enemies:
                    for held in list(enemy.hand):
                        if held.color == source.color:
                            enemy.discard_card(held)
            sizes = {enemy.id: len(enemy.hand) for enemy in enemies}
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            targets = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Choose a friendly"):
                    assert request.player_id == hero.id and options == [ally.id]
                    return ally.id
                if request.prompt == "Choose a card in their discard":
                    assert request.player_id == hero.id and options == [source.id]
                    return source.id
                if request.prompt.startswith("Choose an enemy hero"):
                    assert request.player_id == hero.id and all(target not in options for target in targets)
                    if mode == "skip":
                        return "SKIP"
                    target = enemies[len(targets)].id
                    assert target in options
                    targets.append(target)
                    return target
                enemy = next(enemy for enemy in enemies if enemy.id == request.player_id)
                assert enemy.id == targets[-1]
                assert set(options) == {held.id for held in enemy.hand if held.color == source.color}
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert len(targets) == (0 if mode in ("skip", "hidden-source") else 2 if effect == "the_right_hand" else 1)
            for enemy in enemies:
                assert len(final.get_hero(enemy.id).hand) == sizes[enemy.id] - (mode == "match" and enemy.id in targets)
            yield f"{effect}-{mode}", state, operations


def takahide_swap_traces():
    from aura_traces import _board

    for effect in ("set_an_example", "lead_from_the_front", "hold_my_sake"):
        modes = ("hand", "skip", "no-attack-target", "no-unresolved")
        if effect == "hold_my_sake":
            modes += ("discard",)
        for mode in modes:
            state = setup(["Takahide", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_takahide")
            ally = state.get_hero("hero_brogan")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(ally.id, h(-1, 0))
            if mode != "no-attack-target":
                state.place_entity(minion.id, h(1, 0))
            outgoing, incoming, discarded = ally.hand[:3]
            if mode != "no-unresolved":
                ally.hand.remove(outgoing)
                ally.current_turn_card = outgoing
                outgoing.state, outgoing.is_facedown = CardState.UNRESOLVED, False
                state.unresolved_hero_ids.append(ally.id)
            ally.discard_card(discarded)
            replacement = discarded if mode == "discard" else incoming
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("A friendly hero"):
                    assert request.player_id == hero.id and options == [ally.id]
                    return ally.id
                if request.prompt.startswith("Swap your unresolved card"):
                    assert request.player_id == ally.id and replacement.id in options
                    assert (discarded.id in options) == (effect == "hold_my_sake")
                    return "SKIP" if mode == "skip" else replacement.id
                assert options == [minion.id]
                return minion.id

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            result = final.get_hero(ally.id)
            if mode in ("hand", "discard"):
                assert result.current_turn_card.id == replacement.id
                assert result.current_turn_card.state == CardState.UNRESOLVED
                assert not result.current_turn_card.is_facedown
                assert result.id in final.unresolved_hero_ids
                collection = result.discard_pile if mode == "discard" else result.hand
                assert outgoing.id in {held.id for held in collection}
            else:
                assert result.current_turn_card is None if mode == "no-unresolved" else result.current_turn_card.id == outgoing.id
            yield f"{effect}-{mode}", state, operations


def takahide_blade_traces():
    from aura_traces import _board
    from goa2.engine.steps import FinalizeHeroTurnStep

    for effect in ("spinning_blade", "blade_helix"):
        for attack_target in (True, False):
            state = setup(["Takahide", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_takahide")
            enemy = state.get_hero("hero_hanu")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.unresolved_hero_ids.append(enemy.id)
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, h(2, 0))
            if attack_target:
                state.place_entity(minion.id, h(0, 1))
            for player, effect_id in ((hero, effect), (enemy, "unexpected_journey")):
                card = next(card for card in player.deck if card.effect_id == effect_id)
                if card in player.hand:
                    player.hand.remove(card)
                player.current_turn_card = card
                card.state, card.is_facedown = CardState.UNRESOLVED, False
            destination = h(3, 0)
            move_requests = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select Movement Destination"):
                    assert request.player_id == enemy.id and destination.model_dump() in options
                    assert (h(1, 0).model_dump() in options) == (not attack_target)
                    move_requests.append(request.prompt)
                    return destination.model_dump()
                if request.player_id == hero.id and "ATTACK" in options:
                    return "ATTACK"
                if request.player_id == enemy.id and "MOVEMENT" in options:
                    return "MOVEMENT"
                return options[0]

            operations, final = retain(
                state, [ResolveCardStep(hero_id=hero.id), FinalizeHeroTurnStep(hero_id=hero.id)],
                select=choose,
            )
            assert len(move_requests) == 1
            assert final.entity_locations[enemy.id] == destination
            assert not final.active_effects
            yield f"{effect}-attack-{attack_target}", state, operations


def takahide_deck_swap_traces():
    from goa2.engine.steps.cards import SwapWithDeckCardStep

    for slot in ("hand", "current", "discard", "played", "invalid-incoming"):
        state = setup(["Takahide", "Brogan"], ["Hanu", "Tigerclaw"])
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_takahide")
        state.current_actor_id = state.resolution_owner_id = hero.id
        outgoing = next(card for card in hero.hand if card.color.value == "GOLD")
        incoming = next(card for card in hero.deck if card.color.value == "GOLD" and card.state == CardState.DECK)
        outgoing.played_this_round = True
        if slot == "current":
            hero.hand.remove(outgoing)
            hero.current_turn_card = outgoing
            outgoing.state = CardState.UNRESOLVED
        elif slot == "discard":
            hero.discard_card(outgoing)
        elif slot == "played":
            hero.hand.remove(outgoing)
            hero.played_cards.extend([None, outgoing])
            outgoing.state = CardState.RESOLVED
        elif slot == "invalid-incoming":
            incoming.state = CardState.HAND
            hero.hand.append(incoming)
        state.execution_context.update(swap_owner=hero.id, old_gold=outgoing.id, new_gold=incoming.id)
        operations, final = retain(state, [SwapWithDeckCardStep(
            hero_key="swap_owner", outgoing_card_key="old_gold", incoming_card_key="new_gold",
            facedown_if_from_discard_or_resolved=True,
        )])
        result = final.get_hero(hero.id)
        assert [card.id for card in result.deck] == [card.id for card in hero.deck]
        new = next(card for card in result.deck if card.id == incoming.id)
        old = next(card for card in result.deck if card.id == outgoing.id)
        if slot != "invalid-incoming":
            assert new.state == outgoing.state and new.played_this_round == outgoing.played_this_round
            assert new.is_facedown == (outgoing.is_facedown or slot in ("discard", "played"))
            assert old.state == CardState.DECK and not old.is_facedown and not old.played_this_round
        else:
            assert old.state == outgoing.state
        yield f"takahide-deck-swap-{slot}", state, operations


def takahide_gold_cycle_traces():
    from aura_traces import _board

    for effect in ("float_like_a_butterfly", "sting_like_a_bee", "strike_like_a_tiger", "bushido"):
        modes = ("swap", "empty-deck") if effect != "bushido" else ("swap", "empty-deck", "discard", "played", "no-outgoing")
        for mode in modes:
            state = setup(["Takahide", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_takahide")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            golds = [item for item in hero.deck if item.color.value == "GOLD"]
            hero.hand[:] = [item for item in hero.hand if item not in golds and item is not card]
            for gold in golds:
                gold.state = CardState.DECK
            outgoing = golds[0] if effect == "bushido" else card
            incoming = next(gold for gold in golds if gold is not outgoing)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            if effect == "bushido" and mode != "no-outgoing":
                outgoing.state = {"discard": CardState.DISCARD, "played": CardState.RESOLVED}.get(mode, CardState.HAND)
                collection = {"discard": hero.discard_pile, "played": hero.played_cards}.get(mode, hero.hand)
                collection.append(outgoing)
            if mode == "empty-deck":
                for gold in golds:
                    if gold.state == CardState.DECK:
                        gold.state = CardState.HAND
                        hero.hand.append(gold)
            if effect in ("sting_like_a_bee", "strike_like_a_tiger"):
                distance = card.range_value if effect == "sting_like_a_bee" else 1
                state.place_entity(state.teams["BLUE"].minions[0].id, h(distance, 0))
            swaps = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Choose a gold card in your deck":
                    assert set(options) == {gold.id for gold in golds if gold.state == CardState.DECK}
                    swaps.append(incoming.id)
                    return incoming.id
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            expected = mode not in ("empty-deck", "no-outgoing")
            assert bool(swaps) == expected
            result = final.get_hero(hero.id)
            if expected:
                new = next(gold for gold in result.deck if gold.id == incoming.id)
                old = next(gold for gold in result.deck if gold.id == outgoing.id)
                assert new.state == outgoing.state and old.state == CardState.DECK
                assert new.is_facedown == (mode in ("discard", "played"))
            yield f"{effect}-{mode}", state, operations


def takahide_unlock_traces():
    from goa2.engine.steps import EndPhaseCleanupStep

    for mode in ("hand", "discard", "played", "already-in-deck", "insufficient-gold"):
        state = setup(["Takahide", "Brogan"], ["Hanu", "Tigerclaw"])
        state.phase = GamePhase.CLEANUP
        hero = state.get_hero("hero_takahide")
        hero.level = 7
        hero.gold = 6 if mode == "insufficient-gold" else 7
        silver = next(card for card in hero.deck if card.effect_id == "bushido")
        if mode == "discard":
            hero.discard_card(silver)
        elif mode == "played":
            hero.hand.remove(silver)
            hero.played_cards.append(silver)
            silver.state = CardState.RESOLVED
        elif mode == "already-in-deck":
            hero.hand.remove(silver)
            silver.state = CardState.DECK
        golds = {card.id for card in hero.deck if card.color.value == "GOLD"}
        operations, final = retain(state, [EndPhaseCleanupStep()])
        result = final.get_hero(hero.id)
        unlocked = mode != "insufficient-gold"
        assert result.level == (8 if unlocked else 7)
        assert result.gold == (0 if unlocked else 7)
        assert (silver.id in {card.id for card in result.hand}) == (not unlocked)
        assert len(golds & {card.id for card in result.hand}) == (
            len(golds) if unlocked and mode != "already-in-deck" else 1
        )
        yield f"ready-for-war-{mode}", state, operations


def emmitt_temporal_attack_traces():
    from aura_traces import _board

    for effect in ("temporal_punch", "temporal_slam", "temporal_judgment"):
        for mode in ("initiative-defense", "pass", "out-of-range", "minion"):
            state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_emmitt")
            enemy = state.get_hero("hero_hanu")
            target = state.teams["BLUE"].minions[0].id if mode == "minion" else enemy.id
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(target, h(2 if mode == "out-of-range" else 1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            defense = next(card for card in enemy.deck if card.effect_id == "unexpected_journey")
            if defense not in enemy.hand:
                enemy.hand.append(defense)
            defense.is_facedown = False
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            blocked = mode == "initiative-defense" and effect == "temporal_punch"

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.request_type.value == "SELECT_CARD_OR_PASS":
                    if mode == "initiative-defense":
                        assert defense.id in options
                        return defense.id
                    return "PASS"
                assert options == [target]
                return target

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert (target in final.entity_locations) == (blocked or mode == "out-of-range")
            yield f"{effect}-{mode}", state, operations


def emmitt_turn_effect_traces():
    from aura_traces import _board

    for effect in ("reverse_time", "time_loop"):
        for eligible in (True, False):
            state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.turn = 2
            hero = state.get_hero("hero_emmitt")
            enemy = state.get_hero("hero_hanu")
            target = state.teams["BLUE"].minions[0].id if effect == "reverse_time" else enemy.id
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin = h(0, 0)
            destination = h(1 if eligible or effect == "time_loop" else 2, 0)
            state.place_entity(hero.id, origin)
            state.place_entity(target, destination)
            if effect == "time_loop":
                resolved = enemy.hand.pop(0)
                resolved.state, resolved.is_facedown = CardState.RESOLVED, False
                enemy.played_cards.extend([None, resolved] if eligible else [resolved])
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                assert eligible and options == [target]
                return target

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            if effect == "time_loop":
                assert final.entity_locations[hero.id] == (destination if eligible else origin)
                assert final.entity_locations[enemy.id] == (origin if eligible else destination)
            elif eligible:
                inversion, = final.active_effects
                assert inversion.effect_type.value == "reversed_initiative"
                assert inversion.duration.value == "NEXT_TURN" and inversion.source_card_id == card.id
            else:
                assert not final.active_effects
            yield f"{effect}-eligible-{eligible}", state, operations


def emmitt_reverse_order_traces():
    from aura_traces import _board
    from goa2.engine.handler import process_stack

    for mode in ("same-turn", "next-turn", "later-turn", "next-round", "inactive"):
        state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.turn = 2
        emmitt = state.get_hero("hero_emmitt")
        state.current_actor_id = state.resolution_owner_id = emmitt.id
        state.place_entity(emmitt.id, h(0, 0))
        state.place_entity(state.teams["BLUE"].minions[0].id, h(1, 0))
        reverse = next(card for card in emmitt.deck if card.effect_id == "reverse_time")
        if reverse in emmitt.hand:
            emmitt.hand.remove(reverse)
        emmitt.current_turn_card = reverse
        reverse.state, reverse.is_facedown = CardState.UNRESOLVED, False
        _, state = retain(state, [ResolveCardTextStep(hero_id=emmitt.id, card_id=reverse.id)])
        emmitt = state.get_hero(emmitt.id)
        emmitt.resolve_current_card(turn_number=state.turn)
        state.turn = {"same-turn": 2, "later-turn": 4}.get(mode, 3)
        if mode == "next-round":
            state.round += 1
        if mode == "inactive":
            state.active_effects[0].is_active = False
        actors = [state.get_hero(name) for name in ("hero_hanu", "hero_tigerclaw")]
        for actor, initiative, position in zip(actors, (5, 12), (h(2, 0), h(0, 2)), strict=True):
            state.place_entity(actor.id, position)
            played = actor.hand.pop(0)
            actor.current_turn_card = played
            played.state, played.is_facedown = CardState.UNRESOLVED, False
            played.initiative = initiative
        state.current_actor_id = state.resolution_owner_id = None
        state.unresolved_hero_ids[:] = [actor.id for actor in actors]
        state.execution_stack = [FindNextActorStep(step_id="fixture/reverse-time/next-actor")]
        probe = copy.deepcopy(state)
        process_stack(probe, stop_before_step=lambda _, step: not isinstance(step, FindNextActorStep))
        assert probe.current_actor_id == actors[0 if mode == "next-turn" else 1].id
        yield f"reverse-time-order-{mode}", state, [{"kind": "advance_prefix", "steps": 1}]


def emmitt_time_warp_traces():
    from aura_traces import _board

    for mode in ("positions", "cards", "old-position-target", "no-resolved-card"):
        state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.turn = 2
        hero = state.get_hero("hero_emmitt")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == "time_warp")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        resolved = enemy.hand[0]
        if mode != "no-resolved-card":
            enemy.hand.remove(resolved)
            resolved.state, resolved.is_facedown = CardState.RESOLVED, False
            enemy.played_cards[:] = [None, resolved] if mode == "positions" else [resolved]
        outgoing = None
        if mode in ("cards", "no-resolved-card"):
            outgoing = enemy.hand.pop(0)
            outgoing.state, outgoing.is_facedown = CardState.UNRESOLVED, False
            enemy.current_turn_card = outgoing
            state.unresolved_hero_ids.append(enemy.id)

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Time Warp: choose one":
                return 1 if mode in ("positions", "old-position-target") else 2
            if request.prompt.startswith("Time Warp: choose one of your"):
                assert request.player_id == enemy.id and options == [resolved.id]
                return resolved.id
            assert request.player_id == hero.id and options == [enemy.id]
            return enemy.id

        operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        assert final.entity_locations[hero.id] == (h(2, 0) if mode == "positions" else h(0, 0))
        result = final.get_hero(enemy.id)
        if mode == "cards":
            assert result.current_turn_card.id == resolved.id
            assert result.played_cards[0].id == outgoing.id
            assert enemy.id in final.unresolved_hero_ids
        elif outgoing is not None:
            assert result.current_turn_card.id == outgoing.id
        yield f"time-warp-{mode}", state, operations


def emmitt_stationary_target_traces():
    from aura_traces import _board
    from goa2.engine.filters import HasStraightLineDestinationFilter, RemainedInPlaceFilter, TeamFilter
    from goa2.engine.steps import SelectStep

    for mode in ("stationary", "moved", "no-snapshot", "blocked"):
        state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero, enemy = "hero_emmitt", "hero_hanu"
        state.current_actor_id = state.resolution_owner_id = hero
        origin = h(1, 0)
        state.place_entity(hero, h(-1, 0))
        state.place_entity(enemy, origin)
        if mode != "no-snapshot":
            state.last_turn_positions[enemy] = h(2, 0) if mode == "moved" else origin
        if mode == "blocked":
            for point in origin.neighbors():
                state.board.tiles[point].is_terrain = True
        select = SelectStep(
            target_type=TargetType.UNIT, prompt="Choose a stationary enemy", output_key="stationary_target",
            filters=[TeamFilter(relation="ENEMY"), RemainedInPlaceFilter(), HasStraightLineDestinationFilter()],
        )
        operations, final = retain(state, [select])
        assert final.execution_context.get("stationary_target") == (enemy if mode == "stationary" else None)
        yield f"emmitt-stationary-{mode}", state, operations


def emmitt_shove_traces():
    from aura_traces import _board

    for effect in ("time_walk", "fast_forward"):
        for mode in ("stationary", "moved", "blocked"):
            state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_emmitt")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin, destination = h(1, 0), h(3, 0)
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, origin)
            state.last_turn_positions[enemy.id] = h(2, 0) if mode == "moved" else origin
            if mode == "blocked":
                for point in origin.neighbors():
                    if point != h(0, 0):
                        state.board.tiles[point].is_terrain = True
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select an enemy hero"):
                    assert options == [enemy.id]
                    return enemy.id
                assert destination.model_dump() in options
                assert h(2, 0).model_dump() not in options
                return destination.model_dump()

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert final.entity_locations[enemy.id] == (destination if mode == "stationary" else origin)
            yield f"{effect}-{mode}", state, operations


def emmitt_recall_traces():
    from aura_traces import _board

    for mode in ("recall", "occupied", "no-snapshot", "shove"):
        state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_emmitt")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        origin = h(1, 0)
        start = h(1, 1)
        state.place_entity(enemy.id, origin)
        state.last_turn_positions.clear()
        if mode != "no-snapshot":
            state.last_turn_positions[enemy.id] = origin if mode == "shove" else start
        if mode == "occupied":
            state.place_entity("hero_brogan", start)
        card = next(card for card in hero.deck if card.effect_id == "back_to_the_future")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False

        def choose(request, _mode):
            if request.prompt == "Choose one":
                return 2 if mode == "shove" else 1
            if request.prompt.startswith("Move them"):
                return h(3, 0).model_dump()
            assert [selection_value(option) for option in request.options] == [enemy.id]
            return enemy.id

        operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        assert final.entity_locations[enemy.id] == {"recall": start, "shove": h(3, 0)}.get(mode, origin)
        if mode == "recall":
            assert final.execution_context[f"{card.id}_target_hex"] is final.last_turn_positions[enemy.id]
        yield f"back-to-the-future-{mode}", state, operations


def emmitt_resolved_discard_traces():
    from aura_traces import _board

    for effect in ("time_snare", "time_trap", "time_bomb"):
        for mode in ("discard", "empty-hand", "previous-turn"):
            state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.turn = 2
            hero = state.get_hero("hero_emmitt")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, h(1, 0))
            resolved = enemy.hand.pop(0)
            resolved.state, resolved.is_facedown = CardState.RESOLVED, False
            enemy.played_cards[:] = [resolved] if mode == "previous-turn" else [None, resolved]
            if mode == "empty-hand":
                for held in list(enemy.hand):
                    enemy.discard_card(held)
            hand_size = len(enemy.hand)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select an enemy hero"):
                    assert request.player_id == hero.id and options == [enemy.id]
                    return enemy.id
                assert request.player_id == enemy.id
                return options[0]

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            assert len(final.get_hero(enemy.id).hand) == hand_size - (mode == "discard")
            assert (enemy.id not in final.entity_locations) == (effect == "time_bomb" and mode == "empty-hand")
            yield f"{effect}-{mode}", state, operations


def emmitt_retrieval_traces():
    from aura_traces import _board

    for effect in ("time_capsule", "future_proof"):
        modes = ("both", "decline-ally", "ally-outside", "empty")
        if effect == "future_proof":
            modes += ("immunity",)
        for mode in modes:
            state = setup(["Brogan", "Emmitt"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_emmitt")
            ally = state.get_hero("hero_brogan")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(-3, 0))
            state.place_entity(ally.id, h(3 if mode == "ally-outside" else -2, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            for player in (hero, ally):
                logged = player.hand[:2]
                for held in logged:
                    player.discard_card(held)
                if mode != "empty":
                    state.turn_discard_log[player.id] = [held.id for held in logged]
                player.discard_card(player.hand[0])
            offered = []

            def choose(request, _mode):
                if request.prompt == "Future Proof: choose one":
                    return 2 if mode == "immunity" else 1
                assert request.prompt.startswith("Time Capsule: retrieve all")
                offered.append(request.player_id)
                return 0 if mode == "decline-ally" and request.player_id == ally.id else 1

            operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
            expected = [] if mode in ("empty", "immunity") else [hero.id] if mode == "ally-outside" else [hero.id, ally.id]
            assert offered == expected
            for player in (hero, ally):
                accepted = player.id in expected and not (mode == "decline-ally" and player.id == ally.id)
                assert len(final.get_hero(player.id).discard_pile) == (1 if accepted else 3)
            assert bool(final.active_effects) == (mode == "immunity")
            yield f"{effect}-{mode}", state, operations


def emmitt_glitch_traces():
    from aura_traces import _board

    for effect, count in (("flashback", 3), ("deja_vu", 2), ("unstable_timeline", 2)):
        modes = ("swap", "blocked") if effect == "unstable_timeline" else ("swap", "decline", "blocked")
        for mode in modes:
            state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_emmitt")
            enemy = state.get_hero("hero_hanu")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, h(2, 0))
            if effect != "unstable_timeline":
                state.place_entity(state.teams["BLUE"].minions[0].id, h(0, 1))
            if mode == "blocked":
                for point, tile in state.board.tiles.items():
                    if not tile.occupant_id:
                        tile.is_terrain = True
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            chosen_token = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Place ") and "tokens in radius?" in request.prompt:
                    return 0 if mode == "decline" else 1
                if request.prompt.startswith(("Choose the Glitch token", "Choose one of the Glitch tokens")):
                    assert request.player_id == enemy.id and len(options) == count
                    chosen_token.append(options[0])
                return options[0]

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id), FindNextActorStep()],
                select=choose,
            )
            assert bool(chosen_token) == (mode == "swap")
            assert not any(getattr(token, "token_type", None) == "GLITCH" and token.id in final.entity_locations
                           for token in final.misc_entities.values())
            yield f"{effect}-{mode}", state, operations


def emmitt_glitch_defense_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import AttackSequenceStep

    for mode in ("swap", "blocked"):
        state = setup(["Emmitt", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_emmitt")
        attacker = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = attacker.id
        origin = h(0, 0)
        state.place_entity(hero.id, origin)
        state.place_entity(attacker.id, h(1, 0))
        if mode == "blocked":
            for tile in state.board.tiles.values():
                if tile.occupant_id is None:
                    tile.is_terrain = True
        defense = next(card for card in hero.hand if card.effect_id == "unstable_timeline")
        token_choices = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.request_type.value == "SELECT_CARD_OR_PASS":
                assert request.player_id == hero.id and defense.id in options
                return defense.id
            if request.prompt == "Choose one of the Glitch tokens":
                assert request.player_id == attacker.id and len(options) == 3
                token_choices.extend(options)
            return options[0]

        operations, final = retain(state, [AttackSequenceStep(damage=5, range_val=1), FindNextActorStep()], select=choose)
        assert hero.id in final.entity_locations
        assert (final.entity_locations[hero.id] != origin) == (mode == "swap")
        assert bool(token_choices) == (mode == "swap")
        assert defense.id in {card.id for card in final.get_hero(hero.id).discard_pile}
        assert not any(getattr(token, "token_type", None) == TokenType.GLITCH
                       and token.id in final.entity_locations for token in final.misc_entities.values())
        yield f"unstable-timeline-defense-{mode}", state, operations


def emmitt_timeline_reveal_traces():
    from object_execution_traces import card_planning_scenes

    def base(red):
        return setup(red or ["Arien", "Hanu"], ["Brogan", "Tigerclaw"])

    _, state, operations = next(
        scene for scene in card_planning_scenes(base)
        if scene[0] == "card-planning-dual-second-last"
    )
    request = reference_trace(copy.deepcopy(state), operations)[-1]["request"]
    assert request["player_id"] == "hero_emmitt"
    assert request["prompt"].startswith("Alternative Timelines:")
    choices = [option["id"] for option in request["options"]]
    assert len(choices) == 2
    for choice in choices:
        yield f"alternative-timelines-retrieve-{choice}", copy.deepcopy(state), [
            *operations,
            {"kind": "input", "request_id": "current", "selection": choice},
            {"kind": "advance_prefix", "steps": 1},
        ]


def mortimer_zombie_traces():
    from aura_traces import _board
    from goa2.domain.models import MinionType, TeamColor, TokenType
    from goa2.domain.models.spawn import SpawnPoint, SpawnType

    for mode in ("skip", "place", "move", "swap", "spawn", "drift", "surf", "ultimate-surf", "skip-surf"):
        state = setup(["Mortimer", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mortimer")
        state.current_actor_id = state.resolution_owner_id = hero.id
        origin = h(-1, 0) if mode == "spawn" else h(0, 0)
        state.place_entity(hero.id, origin)
        spawn_targets = [h(2, 0), h(0, 0), h(-1, 1), h(-2, 1)]
        far_spawn = h(4, 0)
        if mode == "spawn":
            for point in (spawn_targets[0], far_spawn):
                spawn = SpawnPoint(location=point, team=TeamColor.BLUE,
                                   type=SpawnType.MINION, minion_type=MinionType.MELEE)
                state.board.tiles[point].spawn_point = spawn
                state.board.spawn_points.append(spawn)
        chosen_spawns = []
        repeats = {"drift": 2, "surf": 3, "ultimate-surf": 5, "skip-surf": 1}
        choice_prompts = []
        awaken = next(card for card in hero.deck if card.effect_id == "awaken")
        if awaken in hero.hand:
            hero.hand.remove(awaken)
        hero.current_turn_card = awaken
        awaken.state, awaken.is_facedown = CardState.UNRESOLVED, False
        steps = [ResolveCardTextStep(hero_id=hero.id, card_id=awaken.id)]

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select hex for Zombie"):
                if mode == "spawn":
                    assert far_spawn.model_dump() not in options
                    assert h(2, -1).model_dump() not in options
                    assert all(point.model_dump() not in options for point in chosen_spawns)
                    point = spawn_targets[len(chosen_spawns)]
                    assert point.model_dump() in options
                    chosen_spawns.append(point)
                    return point.model_dump()
                if mode != "skip" and "1/4" in request.prompt:
                    assert h(1, 0).model_dump() in options
                    return h(1, 0).model_dump()
                assert h(1, 0).model_dump() not in options or mode == "skip"
                return "SKIP"
            if request.prompt.startswith("Choose one (up to"):
                choice_prompts.append(request.prompt)
                return "SKIP" if mode == "skip-surf" else 2
            if request.prompt == "Choose one":
                return 1 if mode == "move" else 2
            if request.prompt == "Select a Zombie token":
                assert len(options) == 1
                return options[0]
            assert h(2, 0).model_dump() in options
            return h(2, 0).model_dump()

        if mode in ("move", "swap") or mode in repeats:
            _, state = retain(state, steps, select=choose)
            hero = state.get_hero(hero.id)
            hero.resolve_current_card(turn_number=state.turn)
            if mode == "ultimate-surf":
                hero.level = 8
            effect = "crowd_drift" if mode == "drift" else "crowd_surf" if mode in repeats else "stage_dive"
            stage = next(card for card in hero.deck if card.effect_id == effect)
            if stage in hero.hand:
                hero.hand.remove(stage)
            hero.current_turn_card = stage
            stage.state, stage.is_facedown = CardState.UNRESOLVED, False
            steps = [ResolveCardTextStep(hero_id=hero.id, card_id=stage.id)]
        operations, final = retain(state, steps, select=choose)
        zombies = [token for token in final.misc_entities.values()
                   if getattr(token, "token_type", None) == TokenType.ZOMBIE
                   and token.id in final.entity_locations]
        assert len(zombies) == (4 if mode == "spawn" else 0 if mode == "skip" else 1)
        if mode == "spawn":
            assert {final.entity_locations[token.id] for token in zombies} == set(spawn_targets)
        elif zombies:
            swapped = mode in ("swap", "surf", "ultimate-surf")
            assert final.entity_locations[zombies[0].id] == (h(2, 0) if mode == "move" else origin if swapped else h(1, 0))
        assert len(choice_prompts) == repeats.get(mode, 0)
        assert final.entity_locations[hero.id] == (h(1, 0) if mode in ("swap", "surf", "ultimate-surf") else origin)
        yield f"mortimer-zombie-{mode}", state, operations


def mortimer_horde_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType

    for effect, level, limit in (("gathering_horde", 7, 2), ("army_of_darkness", 7, 3), ("army_of_darkness", 8, 5)):
        state = setup(["Mortimer", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mortimer")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        targets = state.teams["BLUE"].minions[:2]
        for unit, point in zip(targets, (h(1, 1), h(-1, 2)), strict=True):
            state.place_entity(unit.id, point)
        awaken = next(card for card in hero.deck if card.effect_id == "awaken")
        if awaken in hero.hand:
            hero.hand.remove(awaken)
        hero.current_turn_card = awaken
        awaken.state, awaken.is_facedown = CardState.UNRESOLVED, False
        placements = iter((h(1, 0), h(0, 1), h(-1, 1)))

        def place(request, _mode):
            point = next(placements, None)
            return point.model_dump() if point else "SKIP"

        _, state = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=awaken.id)], select=place)
        hero = state.get_hero(hero.id)
        hero.resolve_current_card(turn_number=state.turn)
        hero.level = level
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        choices = []
        replacements = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose one (up to"):
                choices.append(request.prompt)
                return 2
            assert request.prompt.startswith("Select enemy minion")
            assert set(options) == {unit.id for unit in targets}
            replacements.append(targets[0].id)
            return targets[0].id

        operations, final = retain(state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose)
        assert len(choices) == limit and replacements == [targets[0].id]
        assert targets[0].id not in final.entity_locations and targets[1].id in final.entity_locations
        token = final.misc_entities[final.board.tiles[h(1, 1)].occupant_id]
        assert token.token_type == TokenType.ZOMBIE
        yield f"{effect}-level-{level}", state, operations


def mortimer_dead_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, level, limit in (("crawling_dead", 7, 1), ("walking_dead", 7, 2),
                                 ("racing_dead", 7, 3), ("racing_dead", 8, 5)):
        state = setup(["Mortimer", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mortimer")
        minion = state.teams["BLUE"].minions[0]
        enemies = [state.get_hero(name) for name in ("hero_hanu", "hero_tigerclaw")]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(minion.id, h(-1, 0))
        state.place_entity(enemies[0].id, h(2, 0))
        state.place_entity(enemies[1].id, h(1, 1))
        hero.level = level
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        state.execution_context["dead_fixture_zombie"] = h(1, 0)
        selected = []
        choices = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose one"):
                choices.append(request.prompt)
                return 2
            if request.prompt.startswith("Select an enemy hero adjacent"):
                assert all(enemy not in options for enemy in selected)
                enemy = enemies[len(selected)].id
                assert enemy in options
                selected.append(enemy)
                return enemy
            if request.player_id in selected:
                return options[0]
            assert options == [minion.id]
            return minion.id

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.ZOMBIE, hex_key="dead_fixture_zombie"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert len(choices) == limit
        assert selected == [enemy.id for enemy in enemies[:min(limit, 2)]]
        for enemy in enemies:
            assert len(final.get_hero(enemy.id).hand) == len(enemy.hand) - (enemy.id in selected)
        yield f"{effect}-level-{level}", state, operations


def mortimer_knife_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for mode in ("remove", "skip", "no-target"):
        state = setup(["Mortimer", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mortimer")
        target = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        if mode != "no-target":
            state.place_entity(target.id, h(1, 0))
        hero.level = 8 if mode == "remove" else 7
        card = next(card for card in hero.deck if card.effect_id == "knife_of_the_living_dead")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        steps = []
        for index, point in enumerate((h(1, -1), h(0, 1))):
            key = f"knife_fixture_zombie_{index}"
            state.execution_context[key] = point
            steps.append(PlaceTokenStep(token_type=TokenType.ZOMBIE, hex_key=key))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        removed = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose one"):
                return 2 if mode == "remove" and len(removed) < 2 else "SKIP"
            if request.prompt.startswith("Select Zombie token"):
                assert all(token not in options for token in removed)
                removed.append(options[0])
                return options[0]
            assert options == [target.id]
            return target.id

        operations, final = retain(state, steps, select=choose)
        assert len(removed) == (2 if mode == "remove" else 0)
        assert final.execution_context["knife_atk_bonus"] == len(removed)
        assert all(token not in final.entity_locations for token in removed)
        assert target.id not in final.entity_locations
        yield f"knife-of-the-living-dead-{mode}", state, operations


def mortimer_brains_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect in ("braains", "braaaaaaaains"):
        for eligible in (True, False):
            state = setup(["Mortimer", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_mortimer")
            target = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(target.id, h(-1, 0))
            state.place_entity("hero_hanu", h(2, 0) if eligible else h(0, -2))
            hero.level = 8 if effect == "braaaaaaaains" else 7
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            discarded = hero.hand[0]
            hero.discard_card(discarded)
            state.execution_context["brains_fixture_zombie"] = h(1, 0)
            choices = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Choose one"):
                    choices.append(request.prompt)
                    return 2 if len(choices) == 1 else "SKIP"
                if request.prompt == "Select a discarded card to retrieve":
                    assert eligible and options == [discarded.id]
                    return discarded.id
                assert options == [target.id]
                return target.id

            operations, final = retain(state, [
                PlaceTokenStep(token_type=TokenType.ZOMBIE, hex_key="brains_fixture_zombie"),
                ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
            ], select=choose)
            assert (discarded.id in {held.id for held in final.get_hero(hero.id).hand}) == eligible
            assert target.id not in final.entity_locations
            assert len(choices) == 2
            yield f"{effect}-eligible-{eligible}", state, operations


def mortimer_slam_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, level, limit, gains_coin, skip in (
        ("corpse_slam", 7, 1, False, False),
        ("morbid_mosh", 8, 2, False, False),
        ("macabre_mayhem", 7, 3, False, True),
        ("macabre_mayhem", 8, 5, False, False),
        ("robbing_zombies", 8, 2, True, False),
        ("stalking_scalpers", 7, 3, True, False),
        ("stalking_scalpers", 8, 5, True, False),
    ):
        state = setup(["Mortimer", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mortimer")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity("hero_hanu", h(2, 0))
        hero.level = level
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        state.execution_context["slam_fixture_zombie"] = h(1, 0)
        prompts = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose one"):
                prompts.append(request.prompt)
                if len(prompts) == 1:
                    return 1
                return "SKIP" if skip else 2
            if request.prompt == "Select a Zombie token":
                assert len(options) == 1
                return options[0]
            if request.prompt == "Select Zombie destination":
                assert h(1, 0).model_dump() in options
                return h(1, 0).model_dump()
            if request.prompt == "Select adjacent unit or token to push":
                assert "hero_hanu" in options
                return "hero_hanu"
            if request.prompt.startswith("Select Movement Destination"):
                return "SKIP"
            if request.prompt.startswith("Select") and h(3, 0).model_dump() in options:
                return h(3, 0).model_dump()
            raise AssertionError((request.prompt, options))

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.ZOMBIE, hex_key="slam_fixture_zombie"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert len(prompts) == (2 if skip else limit)
        if limit > 1:
            assert prompts[0] == f"Choose one (up to {limit}, choice 1)"
        assert final.get_hero(hero.id).gold == hero.gold + int(gains_coin)
        assert final.entity_locations["hero_hanu"] == h(2 if gains_coin else 3, 0)
        yield f"{effect}-level-{level}", state, operations


def bain_retrieval_traces():
    from aura_traces import _board
    from goa2.engine.steps import FindNextActorStep

    for effect in ("drinking_buddies", "another_one"):
        for skip, repeat in ((False, False), (True, False), (True, True), (False, True)):
            if repeat and effect != "another_one":
                continue
            state = setup(["Bain", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_bain")
            ally = state.get_hero("hero_brogan")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(ally.id, h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            ally_card, own_card = ally.hand[0], hero.hand[0]
            ally.discard_card(ally_card)
            hero.discard_card(own_card)

            target_prompts = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("You may have a hero"):
                    assert options == [ally.id]
                    target_prompts.append(request.player_id)
                    return "SKIP" if skip and len(target_prompts) == 1 else ally.id
                expected = own_card.id if request.prompt.startswith("You may also") else ally_card.id
                assert options == [expected]
                return expected

            steps = [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)]
            turn = state.turn
            if repeat:
                state.unresolved_hero_ids.clear()
                steps.append(FindNextActorStep())
            operations, final = retain(state, steps, select=choose)
            assert len(target_prompts) == (2 if repeat and skip else 1)
            if repeat:
                assert final.turn == turn + 1
                assert final.phase == GamePhase.PLANNING
            for owner, retrieved in ((hero, own_card), (ally, ally_card)):
                assert (retrieved.id in {held.id for held in final.get_hero(owner.id).hand}) == (repeat or not skip)
            yield f"{effect}-skip-{skip}-turn-boundary-{repeat}", state, operations


def bain_guess_traces():
    from aura_traces import _board

    for effect, mode, gold in (
        ("a_game_of_chance", "correct", 0),
        ("a_game_of_chance", "wrong", 1),
        ("dead_mans_hand", "wrong", 2),
        ("were_not_done_yet", "coins", 2),
        ("were_not_done_yet", "repeat-correct", 0),
        ("were_not_done_yet", "repeat-wrong", 2),
    ):
        state = setup(["Bain", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_bain")
        victim = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(victim.id, h(1, 0))
        state.place_entity("hero_tigerclaw", h(4, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        chosen = victim.hand[0]
        guesses = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select an enemy"):
                assert options == [victim.id]
                return victim.id
            if request.prompt == "Choose one of your cards":
                return chosen.id
            if request.prompt == "Choose consolation":
                return 2 if mode == "coins" else 1
            assert request.prompt == "Guess the card's color"
            assert options == sorted({held.color.value for held in victim.hand})
            correct = mode == "correct" or (mode == "repeat-correct" and bool(guesses))
            guesses.append(correct)
            return chosen.color.value if correct else next(color for color in options if color != chosen.color.value)

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert len(guesses) == (2 if mode.startswith("repeat-") else 1)
        assert final.get_hero(hero.id).gold == hero.gold + gold
        assert (chosen.id in {held.id for held in final.get_hero(victim.id).discard_pile}) == guesses[-1]
        assert [attempt["correct"] for attempt in final.card_guess["attempts"]] == guesses
        yield f"{effect}-{mode}", state, operations


def bain_bounty_movement_traces():
    from aura_traces import _board
    from goa2.domain.models import Marker, MarkerType

    for effect, bonus in (("vantage_point", 1), ("high_ground", 2)):
        for mode in ("absent", "unplaced", "miskeyed", "placed"):
            state = setup(["Bain", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_bain")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity("hero_hanu", h(-1, 0))
            state.board.tiles[h(1, 0)].is_terrain = True
            state.markers.pop(MarkerType.BOUNTY, None)
            if mode != "absent":
                key = MarkerType.POISON if mode == "miskeyed" else MarkerType.BOUNTY
                state.markers[key] = Marker(
                    type=MarkerType.BOUNTY,
                    source_id=hero.id,
                    target_id=None if mode == "unplaced" else "hero_hanu",
                )
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            distance = 2 + (bonus if mode == "placed" else 0)
            destination = h(distance, 0)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                assert h(1, 0).model_dump() not in options
                assert destination.model_dump() in options
                if distance < 4:
                    assert h(distance + 1, 0).model_dump() not in options
                return destination.model_dump()

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert final.entity_locations[hero.id] == destination
            yield f"{effect}-{mode}", state, operations


def swift_shot_traces():
    from aura_traces import _board

    for effect, mode in (
        ("snipe", "maximum"),
        ("prepared_shot", "maximum"),
        ("killshot", "maximum"),
        ("shotgun", "skip"),
        ("shotgun", "discard"),
        ("super_shotgun", "discard"),
        ("super_shotgun", "empty"),
    ):
        state = setup(["Swift", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_swift")
        bystander = state.get_hero("hero_hanu")
        target, adjacent, nearer = state.teams["BLUE"].minions[:3]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        state.place_entity(target.id, h(card.range_value - 1, 0))
        if mode == "maximum":
            state.place_entity(adjacent.id, h(0, 0))
            state.place_entity(nearer.id, h(1, 0))
            state.place_entity(bystander.id, h(1, -1))
        else:
            state.place_entity(bystander.id, h(2, 0))
        if mode == "empty":
            for held in list(bystander.hand):
                bystander.discard_card(held)
        before = len(bystander.hand)
        targeted = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.lower().startswith("select attack target"):
                targeted.append(True)
                if mode == "maximum":
                    expected = {target.id} if effect == "snipe" else {target.id, nearer.id}
                    assert set(options) == expected
                return target.id
            if request.prompt == "Select an enemy hero adjacent to the target":
                assert options == [bystander.id]
                return "SKIP" if mode == "skip" else bystander.id
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert targeted == [True]
        assert len(final.get_hero(bystander.id).hand) == before - int(mode == "discard")
        assert (bystander.id in final.entity_locations) == (mode != "empty")
        yield f"{effect}-{mode}", state, operations


def swift_jump_suppress_traces():
    from aura_traces import _board

    for effect, mode in (
        ("steam_jump", "push"), ("assault_jump", "push"), ("assault_jump", "skip"),
        ("suppress", "terrain"), ("pin_down", "edge"), ("killing_ground", "empty"),
    ):
        state = setup(["Swift", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_swift")
        target = state.get_hero("hero_hanu")
        other = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        jump = mode in ("push", "skip")
        state.place_entity(hero.id, h(0, 0) if jump else h(1, 0))
        state.place_entity(target.id, h(2, 0) if jump else h(0, 0))
        if jump:
            state.board.tiles[h(-1, 0)].is_terrain = True
        elif mode == "edge":
            state.place_entity(other.id, h(4, 0))
        else:
            state.place_entity(other.id, h(2, 0))
            state.board.tiles[h(2, 1)].is_terrain = True
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        if mode == "empty":
            for held in list(target.hand):
                target.discard_card(held)
        before = len(target.hand)
        distance = 1 if effect == "steam_jump" else 2

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Place yourself"):
                assert h(-1, 0).model_dump() not in options
                assert h(1, -2).model_dump() not in options
                return h(1, 0).model_dump()
            if request.prompt.startswith(("Push an enemy", "Select an enemy hero in radius")):
                assert options == [target.id]
                return "SKIP" if mode == "skip" else target.id
            if request.prompt == "Choose push distance":
                assert options == list(range(distance + 1))
                return distance
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        if jump:
            assert final.entity_locations[hero.id] == h(1, 0)
            assert final.entity_locations[target.id] == h(2 + (distance if mode == "push" else 0), 0)
        elif mode == "empty":
            assert target.id not in final.entity_locations
        else:
            assert len(final.get_hero(target.id).hand) == before - 1
        yield f"{effect}-{mode}", state, operations


def swift_special_traces():
    from aura_traces import _board

    for effect in ("mark_for_death", "hunting_season", "delayed_jump", "mobile_scout", "drop_trooper"):
        state = setup(["Swift", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_swift")
        first, second = state.teams["BLUE"].minions[:2]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(first.id, h(2, 0))
        state.place_entity(second.id, h(1, 1))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        steps = [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)]
        delayed = effect in ("delayed_jump", "mobile_scout")
        if delayed:
            state.unresolved_hero_ids.clear()
            steps.append(FindNextActorStep())
        destinations = iter((h(0, 1), h(0, 2)))
        pushes = iter((2, 1))

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith(("Select up to two", "Push up to two")):
                return first.id if first.id in options else second.id
            if request.prompt == "Select an enemy minion in radius to move":
                return first.id
            if request.prompt.startswith("Move the minion"):
                return next(destinations).model_dump()
            if request.prompt.startswith("Place yourself"):
                if delayed:
                    assert h(1, 0).model_dump() not in options
                    return h(1, 1 if effect == "mobile_scout" else -2).model_dump()
                return h(1, 0).model_dump()
            if request.prompt == "Choose push distance":
                return next(pushes)
            raise AssertionError((request.prompt, options))

        if effect == "mobile_scout":
            state.place_entity(second.id, h(2, -1))
        operations, final = retain(state, steps, select=choose)
        if delayed:
            assert final.turn == state.turn + 1 and final.phase == GamePhase.PLANNING
            assert final.entity_locations[hero.id] == h(1, 1 if effect == "mobile_scout" else -2)
            assert not final.active_effects
        elif effect == "drop_trooper":
            assert final.entity_locations[hero.id] == h(1, 0)
            assert final.entity_locations[first.id] == h(4, 0)
            assert final.entity_locations[second.id] == h(1, 2)
        else:
            assert final.entity_locations[first.id] == h(0, 1)
            assert final.entity_locations[second.id] == (h(0, 2) if effect == "hunting_season" else h(1, 1))
            bounty, = final.active_effects
            assert bounty.effect_type.value == "minion_defeat_bounty"
            assert bounty.duration.value == "NEXT_TURN"
            assert bounty.max_value == (2 if effect == "hunting_season" else 1)
        yield effect, state, operations


def swift_bounce_traces():
    from aura_traces import _board
    from goa2.engine.steps import PerformPrimaryActionStep

    for mode in ("initial", "current-repeat", "resolved-repeat", "decline"):
        state = setup(["Swift", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_swift")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.board.tiles[h(1, 0)].is_terrain = True
        state.board.tiles[h(3, 0)].is_terrain = True
        card = next(card for card in hero.deck if card.effect_id == "bounce")
        hero.hand.remove(card)
        card.is_facedown = False
        if mode == "resolved-repeat":
            hero.played_cards.append(card)
            card.state = CardState.RESOLVED
        else:
            hero.current_turn_card = card
            card.state = CardState.UNRESOLVED
        if mode == "initial":
            step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
        else:
            state.execution_context["bounce_fixture_card"] = card.id
            step = PerformPrimaryActionStep(card_key="bounce_fixture_card", hero_id=hero.id)
        destinations = iter((h(2, 0), h(4, 0)))
        repeats = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if options == ["YES", "NO"]:
                repeats.append(request.prompt)
                return "NO" if mode == "decline" else "YES"
            destination = next(destinations).model_dump()
            assert destination in options
            return destination

        operations, final = retain(state, [step], select=choose)
        assert len(repeats) == int(mode != "initial")
        assert final.entity_locations[hero.id] == h(4 if mode.endswith("-repeat") else 2, 0)
        yield f"bounce-{mode}", state, operations


def swift_reload_traces():
    from aura_traces import _board

    for mode in ("empty", "rightmost", "defeat"):
        state = setup(["Swift", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_swift")
        victim = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(victim.id, h(0, 1))
        card = next(card for card in hero.deck if card.effect_id == "reload")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        if mode == "rightmost":
            earlier, bounce = (
                next(held for held in hero.deck if held.effect_id == effect)
                for effect in ("snipe", "bounce")
            )
            for held in (earlier, bounce):
                hero.hand.remove(held)
                held.state, held.is_facedown = CardState.RESOLVED, False
            hero.played_cards = [earlier, None, bounce, None]
        destinations = iter((h(2, 0), h(4, 0)))
        repeats = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Choose one":
                return 2 if mode == "defeat" else 1
            if request.prompt == "Defeat a minion adjacent to you":
                assert options == [victim.id]
                return victim.id
            if options == ["YES", "NO"]:
                repeats.append(request.prompt)
                return "YES"
            assert mode == "rightmost" and request.prompt.startswith("Move 2 spaces")
            return next(destinations).model_dump()

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert len(repeats) == int(mode == "rightmost")
        assert final.entity_locations[hero.id] == h(4 if mode == "rightmost" else 0, 0)
        assert (victim.id in final.entity_locations) == (mode != "defeat")
        yield f"reload-{mode}", state, operations


def swift_bullet_time_traces():
    from aura_traces import _board

    for mode in ("accept", "decline", "locked"):
        state = setup(["Swift", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        state.turn = 2
        hero = state.get_hero("hero_swift")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-3, 0))
        card = next(card for card in hero.hand if card.effect_id == "bounce")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        hero.level = 7 if mode == "locked" else 8
        hero.ultimate_card.state = CardState.PASSIVE
        hero.ultimate_card.is_facedown = False
        destinations = iter((h(-1, 0), h(1, 0), h(3, 0)))
        offers = []

        def choose(request, _mode):
            if request.request_type.value == "CHOOSE_ACTION":
                return card.primary_action.value
            if request.prompt.startswith("Bullet Time:"):
                offers.append(request.prompt)
                return "YES" if mode == "accept" else "NO"
            if request.prompt.startswith("Move 2 spaces"):
                destination = next(destinations).model_dump()
                assert destination in [selection_value(option) for option in request.options]
                return destination
            assert [selection_value(option) for option in request.options] == ["YES", "NO"]
            return "YES"

        operations, final = retain(state, [ResolveCardStep(hero_id=hero.id)], select=choose)
        assert len(offers) == int(mode != "locked")
        assert final.entity_locations[hero.id] == h(3 if mode == "accept" else -1, 0)
        yield f"bullet-time-{mode}", state, operations


def swift_bullet_time_exclusion_trace():
    from aura_traces import _board
    from goa2.engine.steps import OfferPassiveStep

    state = setup(["Swift", "Brogan"], ["Hanu", "Tigerclaw"])
    h = _board(state)
    state.battle_zones = {}
    state.phase = GamePhase.RESOLUTION
    hero = state.get_hero("hero_swift")
    state.current_actor_id = state.resolution_owner_id = hero.id
    state.place_entity(hero.id, h(0, 0))
    previous, eligible = state.teams["BLUE"].minions[:2]
    state.place_entity(previous.id, h(1, 0))
    state.place_entity(eligible.id, h(0, 1))
    card = next(card for card in hero.deck if card.effect_id == "reload")
    if card in hero.hand:
        hero.hand.remove(card)
    hero.current_turn_card = card
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    hero.level = 8
    hero.ultimate_card.state = CardState.PASSIVE
    hero.ultimate_card.is_facedown = False
    state.execution_context.update(
        basic_action_card_id=card.id, last_combat_target=previous.id,
    )

    def choose(request, _mode):
        if [selection_value(option) for option in request.options] == ["YES", "NO"]:
            return "YES"
        if request.prompt == "Choose one":
            return 2
        assert request.prompt == "Defeat a minion adjacent to you", request.prompt
        assert [selection_value(option) for option in request.options] == [eligible.id]
        return eligible.id

    operations, final = retain(state, [OfferPassiveStep(
        card_id=hero.ultimate_card.id, trigger="after_basic_action",
    )], select=choose)
    assert previous.id in final.entity_locations
    assert eligible.id not in final.entity_locations
    yield "bullet-time-excludes-prior-target", state, operations


def rowenna_surrender_opening_traces():
    from aura_traces import _board

    for effect, mode in (
        ("accept_surrender", "defeat"), ("accept_surrender", "hand"),
        ("glorious_triumph", "defeat"), ("opening_shots", "discard"),
        ("opening_shots", "actor-discard"), ("opening_volley", "repeat"),
    ):
        state = setup(["Rowenna", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_rowenna")
        ally = state.get_hero("hero_brogan")
        first = state.get_hero("hero_hanu")
        second = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(first.id, h(1, 0))
        state.place_entity(second.id, h(0, 1))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        if mode == "defeat":
            for held in list(first.hand):
                first.discard_card(held)
        if mode == "actor-discard":
            hero.discard_card(hero.hand[0])
        if mode == "discard":
            second.discard_card(second.hand[0])
        chosen = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select an enemy hero with no cards"):
                assert options == [first.id]
                return first.id
            if request.prompt == "Select an enemy hero with no discarded cards":
                target = second if chosen else first
                expected = {first.id, second.id} if not chosen and mode == "repeat" else {target.id}
                assert set(options) == expected
                chosen.append(target.id)
                return target.id
            if options == ["YES", "NO"]:
                return "YES"
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert (first.id in final.entity_locations) == (mode != "defeat")
        assist = (3 if effect == "glorious_triumph" else 1) if mode == "defeat" else 0
        assert final.get_hero(ally.id).gold == ally.gold + assist
        assert chosen == ([first.id, second.id] if mode == "repeat" else [first.id] if mode == "discard" else [])
        for victim in (first, second):
            assert len(final.get_hero(victim.id).discard_pile) == len(victim.discard_pile) + int(victim.id in chosen)
        yield f"{effect}-{mode}", state, operations


def rowenna_swap_traces():
    from aura_traces import _board

    for effect, mode in (
        ("stand_guard", "adjacent"), ("stand_guard", "no-target"),
        ("protect_the_weak", "discard"),
        ("defend_the_innocent", "retrieve"), ("defend_the_innocent", "skip-retrieve"),
    ):
        state = setup(["Rowenna", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_rowenna")
        ally = state.get_hero("hero_brogan")
        minion = state.teams["RED"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(1, 0))
        state.place_entity(minion.id, h(0, 1))
        if mode == "adjacent":
            state.place_entity("hero_hanu", h(0, 2))
        elif mode != "no-target":
            ally.discard_card(ally.hand[0])
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        discarded = hero.hand[0]
        if effect == "defend_the_innocent":
            hero.discard_card(discarded)
        target = minion if mode == "adjacent" else ally

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select a friendly unit to swap with":
                assert mode != "no-target" and options == [target.id]
                return target.id
            assert request.prompt == "You may retrieve a discarded card"
            assert options == [discarded.id]
            return discarded.id if mode == "retrieve" else "SKIP"

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert final.entity_locations[hero.id] == (
            h(0, 0) if mode == "no-target" else state.entity_locations[target.id]
        )
        if mode != "no-target":
            assert final.entity_locations[target.id] == h(0, 0)
        if effect == "defend_the_innocent":
            assert (discarded.id in {held.id for held in final.get_hero(hero.id).hand}) == (mode == "retrieve")
        yield f"{effect}-{mode}", state, operations


def rowenna_lance_traces():
    from aura_traces import _board
    from goa2.domain.models import StatType
    from primary_templates import ROWENNA_LANCE_ATTACKS

    for effect in (effect for effect in ROWENNA_LANCE_ATTACKS if effect != "code_of_chivalry"):
        for level in (7, 8):
            state = setup(["Rowenna", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.battle_zones = {}
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_rowenna")
            ally = state.get_hero("hero_brogan")
            first, second = state.teams["BLUE"].minions[:2]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(ally.id, h(-1, 0))
            state.place_entity(first.id, h(3 if level == 8 else 1, 0))
            state.place_entity(second.id, h(0, 1))
            hero.level = level
            hero.items[StatType.RANGE] = 1
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            retrieval = effect in ("feat_of_bravery", "paragon_of_valor")
            discarded = ally.hand[:2] if retrieval else []
            for held in discarded:
                ally.discard_card(held)
            attacked = []
            retrieved = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Select Attack Target":
                    expected = {first.id, second.id} if not attacked else {second.id}
                    assert set(options) == expected
                    target = second if attacked else first
                    attacked.append(target.id)
                    return target.id
                if "friendly hero in radius" in request.prompt:
                    assert ally.id in options
                    return ally.id
                if request.prompt == "Select a discarded card to retrieve":
                    choice = discarded[len(retrieved)].id
                    assert choice in options
                    retrieved.append(choice)
                    return choice
                if options == ["YES", "NO"]:
                    return "YES"
                return options[0]

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            repeats = effect.startswith("paragon")
            assert len(attacked) == 1 + repeats
            if retrieval:
                assert len(retrieved) == len(attacked)
            else:
                # Minion rewards can also grant assist coins, so count the card's
                # explicit benefit in the state deltas on the Rust/Python trace.
                assert final.get_hero(ally.id).gold >= ally.gold + len(attacked)
            yield f"{effect}-level-{level}", state, operations


def rowenna_melee_traces():
    from aura_traces import _board

    for effect, mode, choices in (
        ("close_quarters", "friendly", [1]),
        ("close_quarters", "no-adjacent-hero", []),
        ("melee", "enemy", [2]),
        ("grand_melee", "both", [1, 2]),
        ("grand_melee", "skip-first", ["SKIP", 2]),
    ):
        state = setup(["Rowenna", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_rowenna")
        enemy = state.get_hero("hero_hanu")
        friendly_minion = state.teams["RED"].minions[0]
        enemy_minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(4 if not choices else 2, 0))
        state.place_entity(friendly_minion.id, h(0, 1))
        state.place_entity(enemy_minion.id, h(1, -1))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        pending = iter(choices)
        placements = {
            "Select a space adjacent to the enemy hero": h(2, -1),
            "Select a space adjacent to you": h(1, 1),
        }

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            prompt = request.prompt
            if prompt == "Choose one":
                return next(pending)
            if prompt in placements:
                destination = placements[prompt].model_dump()
                assert destination in options
                return destination
            for prefix, target in (
                ("Select the adjacent enemy hero", enemy),
                ("Select a friendly minion", friendly_minion),
                ("Select an enemy minion", enemy_minion),
            ):
                if prompt.startswith(prefix):
                    assert options == [target.id]
                    return target.id
            assert prompt.startswith("Select Movement Destination"), prompt
            return h(1, 0).model_dump()

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert next(pending, None) is None
        assert final.entity_locations[hero.id] == h(1, 0)
        assert final.entity_locations[friendly_minion.id] == (h(2, -1) if 1 in choices else h(0, 1))
        assert final.entity_locations[enemy_minion.id] == (h(1, 1) if 2 in choices else h(1, -1))
        yield f"{effect}-{mode}", state, operations


def rowenna_basic_traces():
    from aura_traces import _board
    from goa2.domain.models import StatType

    for effect, mode in (
        ("code_of_chivalry", "melee-minion"), ("code_of_chivalry", "ranged-hero"),
        ("throw_the_gauntlet", "move"), ("throw_the_gauntlet", "decline"),
    ):
        state = setup(["Rowenna", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_rowenna")
        enemy = state.get_hero("hero_hanu")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(minion.id, h(0, 1))
        state.place_entity(enemy.id, h(3 if mode == "ranged-hero" else 2, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        retrieved = []
        if mode == "ranged-hero":
            hero.level = 8
            hero.items[StatType.RANGE] = 1
            for owner in (enemy, hero):
                owner.discard_card(owner.hand[0])

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            prompt = request.prompt
            if prompt == "Target a unit adjacent to you":
                assert set(options) == ({minion.id, enemy.id} if mode == "ranged-hero" else {minion.id})
                return enemy.id if mode == "ranged-hero" else minion.id
            if prompt == "You may retrieve a discarded card":
                owner = hero if retrieved else enemy
                assert options == [owner.discard_pile[0].id]
                retrieved.append(owner.id)
                return options[0]
            if prompt == "Select an enemy hero in range":
                assert options == [enemy.id]
                return enemy.id
            if prompt.startswith("Select a space in range adjacent"):
                return h(1, 0).model_dump()
            if prompt.startswith("You may move 1 space"):
                assert str(request.player_id) == enemy.id
                return h(3, 0).model_dump() if mode == "move" else "SKIP"
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert retrieved == ([enemy.id, hero.id] if mode == "ranged-hero" else [])
        if effect == "throw_the_gauntlet":
            assert final.entity_locations[hero.id] == h(1, 0)
            assert final.entity_locations[enemy.id] == h(3 if mode == "move" else 2, 0)
            assert final.get_hero(hero.id).gold == hero.gold + (2 if mode == "move" else 0)
        yield f"{effect}-{mode}", state, operations


def mrak_terrain_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, mode in (
        ("seismic_slam", "rock"), ("seismic_assault", "empty"),
        ("epicenter", "repeat"), ("treacherous_ground", "move"),
        ("rockslide", "skip"), ("avalanche", "move-twice"),
    ):
        state = setup(["Mrak", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mrak")
        first = state.get_hero("hero_hanu")
        second = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(first.id, h(1, 0))
        state.place_entity(second.id, h(0, -1))
        state.board.tiles[h(0, -2)].is_terrain = mode in ("empty", "repeat", "move", "skip", "move-twice")
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        if mode == "empty":
            for held in list(second.hand):
                second.discard_card(held)
        steps = []
        if mode in ("rock", "repeat", "move", "move-twice"):
            state.execution_context["fixture_rock"] = h(2, -1)
            steps.append(PlaceTokenStep(token_type=TokenType.ROCK, hex_key="fixture_rock"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        chosen = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if options == ["YES", "NO"]:
                return "YES"
            if request.prompt.startswith("An enemy hero in radius"):
                expected = {second.id} if mode == "empty" or chosen else {first.id}
                if mode == "repeat" and not chosen:
                    expected.add(second.id)
                assert set(options) == expected
                target = second if chosen or mode == "empty" else first
                chosen.append(target.id)
                return target.id
            if request.prompt == "You may move a unit in range":
                if mode == "skip":
                    return "SKIP"
                target = second if chosen else first
                chosen.append(target.id)
                return target.id
            if request.prompt.startswith("Move it 1 space"):
                destination = h(1, -1) if len(chosen) == 1 else h(1, -2)
                assert destination.model_dump() in options
                return destination.model_dump()
            return options[0]

        operations, final = retain(state, steps, select=choose)
        if mode in ("move", "move-twice"):
            assert final.entity_locations[first.id] == h(1, -1)
            assert final.entity_locations[second.id] == (h(1, -2) if mode == "move-twice" else h(0, -1))
        elif mode == "empty":
            assert second.id not in final.entity_locations
        else:
            for target in (first, second):
                assert len(final.get_hero(target.id).hand) == len(target.hand) - int(target.id in chosen)
        yield f"{effect}-{mode}", state, operations


def mrak_rock_placement_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType

    for effect in ("stomping_step", "ground_shaker", "stone_grip", "fissure"):
        state = setup(["Mrak", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mrak")
        first = state.get_hero("hero_hanu")
        second = state.get_hero("hero_tigerclaw")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        stomp = effect in ("stomping_step", "ground_shaker")
        if stomp:
            state.place_entity(first.id, h(1, 0))
            state.place_entity(second.id, h(0, -1))
            state.board.tiles[h(2, 0)].is_terrain = True
            state.board.tiles[h(0, -2)].is_terrain = True
        elif effect == "stone_grip":
            state.place_entity(first.id, h(1, 0))
            hero.level = 8
        else:
            state.place_entity(minion.id, h(1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        moved = []
        ultimate = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            prompt = request.prompt
            if prompt.startswith("Rock and a Hard Place"):
                ultimate.append(prompt)
                return 1
            if options == ["YES", "NO"]:
                return "YES"
            if prompt.startswith("Move a unit in radius"):
                if moved:
                    assert first.id not in options
                target = second if moved else first
                moved.append(target.id)
                return target.id
            if prompt == "Move it 1 space":
                return h(1, 1).model_dump() if len(moved) == 1 else h(-1, 0).model_dump()
            if prompt == "Target an enemy hero in range":
                return first.id
            if prompt == "Target a unit adjacent to you":
                return minion.id
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        rocks = {final.entity_locations[token.id] for token in final.misc_entities.values()
                 if getattr(token, "token_type", None) == TokenType.ROCK
                 and token.id in final.entity_locations}
        if stomp:
            assert rocks == ({h(1, 0), h(0, -1)} if effect == "ground_shaker" else {h(1, 0)})
        elif effect == "stone_grip":
            assert len(rocks) == 3 and len(ultimate) == 1
            assert len(final.get_hero(first.id).hand) == len(first.hand) - 1
        else:
            start = 2 if minion.id in final.entity_locations else 1
            assert rocks == {h(q, 0) for q in range(start, start + 3)}
        yield effect, state, operations


def mrak_shield_traces():
    from aura_traces import _board
    from goa2.engine.steps import ForceDiscardStep

    for effect, mode in (
        ("stone_carapace", "shield"), ("stone_carapace", "consume"), ("rock_solid", "retrieve"), ("rock_solid", "skip"),
    ):
        state = setup(["Mrak", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mrak")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        discarded = hero.hand[0]
        hero.discard_card(discarded)

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select Movement Destination"):
                return h(1, 0).model_dump()
            if request.prompt == "You may retrieve a discarded card":
                assert options == [discarded.id]
                return discarded.id if mode == "retrieve" else "SKIP"
            assert mode == "consume" and card.id in options
            return card.id

        steps = [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)]
        if mode == "consume":
            steps.append(ForceDiscardStep(victim_id=hero.id))
        operations, final = retain(state, steps, select=choose)
        if mode == "consume":
            assert not final.active_effects
            assert card.id in {held.id for held in final.get_hero(hero.id).discard_pile}
        else:
            shield, = final.active_effects
            assert shield.effect_type.value == "discard_shield"
            assert shield.duration.value == "THIS_ROUND"
            assert shield.source_card_id == card.id
        assert final.entity_locations[hero.id] == h(1, 0)
        assert (discarded.id in {held.id for held in final.get_hero(hero.id).hand}) == (mode == "retrieve")
        yield f"{effect}-{mode}", state, operations


def mrak_boulder_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, cap, token, follow in (
        ("boulder_rush", 2, False, True),
        ("boulder_blitz", 3, True, True),
        ("boulderdozer", 4, False, False),
    ):
        state = setup(["Mrak", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_mrak")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-1, 0))
        state.board.tiles[h(1, 0)].is_terrain = True
        steps = []
        if token:
            state.execution_context["boulder_rock"] = h(0, 0)
            steps.append(PlaceTokenStep(token_type=TokenType.ROCK, hex_key="boulder_rock"))
        else:
            state.place_entity("hero_hanu", h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        targets = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Push a token"):
                assert len(options) == 1
                targets.append(options[0])
                return options[0]
            if request.prompt == "Push how many spaces?":
                assert options == list(range(1, cap + 1))
                return cap
            assert request.prompt.startswith("You may move up to")
            assert h(-2, 0).model_dump() not in options
            return h(0, 0).model_dump() if follow else "SKIP"

        operations, final = retain(state, steps, select=choose)
        assert final.entity_locations[targets[0]] == h(cap, 0)
        assert final.entity_locations[hero.id] == h(0 if follow else -1, 0)
        yield effect, state, operations


def mrak_rolling_traces():
    from aura_traces import _board

    for effect, limit in (("rolling_stone", 1), ("strolling_stone", 2)):
        for terrain in (False, True):
            state = setup(["Mrak", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_mrak")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.board.tiles[h(1, 0)].is_terrain = terrain
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            maximum = limit + 1 + int(terrain)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                assert request.prompt.startswith("Move any number of spaces")
                assert (h(1, 0).model_dump() in options) == (not terrain)
                assert h(1, 1).model_dump() not in options
                assert [q for q in range(2, 5) if h(q, 0).model_dump() in options] == list(range(2, maximum + 1))
                return h(maximum, 0).model_dump()

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert final.entity_locations[hero.id] == h(maximum, 0)
            yield f"{effect}-terrain-{terrain}", state, operations


def dodger_debuff_traces():
    from aura_traces import _board
    from goa2.domain.models import StatType
    from goa2.engine.stats import get_computed_stat
    from goa2.engine.steps import MayRepeatOnceStep, SetActorStep, SetContextFlagStep

    for effect, outside in (("weakness", False), ("enfeeblement", False), ("enfeeblement", True)):
        state = setup(["Dodger", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_dodger")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-2, 0))
        state.place_entity(enemy.id, h(4 if outside else -1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        can_repeat = outside or effect == "weakness"
        prompts = []

        def choose(request, _mode):
            assert [selection_value(option) for option in request.options] == ["YES", "NO"]
            prompts.append(request.prompt)
            return "YES"

        operations, final = retain(state, [
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
            SetActorStep(actor_id=enemy.id),
            MayRepeatOnceStep(steps_template=[SetContextFlagStep(key="fixture_repeated", value=True)]),
        ], select=choose)
        assert bool(prompts) == can_repeat
        assert bool(final.execution_context.get("fixture_repeated")) == can_repeat
        assert get_computed_stat(final, enemy.id, StatType.ATTACK, 2) == (
            2 if outside else -2 if effect == "weakness" else -4
        )
        yield f"{effect}-outside-{outside}", state, operations


def dodger_skull_traces():
    from aura_traces import _board

    for effect, mode in (
        ("burning_skull", "move"), ("burning_skull", "skip"),
        ("blazing_skull", "both"), ("blazing_skull", "skip-second"),
    ):
        state = setup(["Dodger", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_dodger")
        victim, second = state.teams["BLUE"].minions[:2]
        first = state.teams["RED"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(victim.id, h(2, 0))
        state.place_entity(first.id, h(0, 1))
        if effect == "blazing_skull":
            state.place_entity(second.id, h(-1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        destinations = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select Attack Target":
                return victim.id
            if request.prompt.startswith("Select a minion adjacent"):
                assert options == [first.id]
                return "SKIP" if mode == "skip" else first.id
            if request.prompt.startswith("Select up to 2 minions"):
                return first.id if first.id in options else second.id
            if request.prompt.startswith("Select destination"):
                assert h(1, 0).model_dump() not in options
                destination = h(-2, 0) if destinations else h(0, 2)
                destinations.append(destination)
                return "SKIP" if mode == "skip-second" and len(destinations) == 2 else destination.model_dump()
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert final.entity_locations[first.id] == h(0, 1 if mode == "skip" else 2)
        if effect == "blazing_skull":
            assert final.entity_locations[second.id] == h(-2 if mode == "both" else -1, 0)
            assert len(destinations) == 2
        yield f"{effect}-{mode}", state, operations


def dodger_respawn_traces():
    from aura_traces import _board

    for effect, team, mode in (
        ("necromancy", "RED", "lane"),
        ("necromastery", "BLUE", "tide"),
        ("necromastery", "RED", "full"),
    ):
        rosters = (["Dodger", "Brogan"], ["Hanu", "Tigerclaw"])
        state = setup(*(rosters if team == "RED" else rosters[::-1]))
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_dodger")
        state.current_actor_id = state.resolution_owner_id = hero.id
        minions = state.teams[team].minions
        if mode == "lane":
            minion = next(m for m in minions if m.id in state.entity_locations
                          and state.board.tiles[state.entity_locations[m.id]].zone_id in state.battle_zone_ids())
            destination = state.entity_locations[minion.id]
            state.remove_entity(minion.id)
            origin = next(point for point in destination.neighbors()
                          if point in state.board.tiles and not state.board.tiles[point].is_obstacle)
            state.place_entity(hero.id, origin)
        else:
            h = _board(state)
            state.battle_zones = {}
            state.place_entity(hero.id, h(0, 0))
            destination = h(1, 0)
            hero.level = 8
            if mode == "full":
                free = (point for point in state.board.tiles if point != h(0, 0))
                for minion in minions:
                    state.place_entity(minion.id, next(free))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        placed_before = sum(m.id in state.entity_locations for m in minions)

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if destination.model_dump() in options:
                return destination.model_dump()
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert sum(m.id in final.entity_locations for m in minions) == placed_before + int(mode != "full")
        if mode != "full":
            assert destination in {final.entity_locations.get(m.id) for m in minions}
        yield f"{effect}-{mode}", state, operations


def dodger_ritual_traces():
    from aura_traces import _board
    from goa2.domain.models import StatType
    from goa2.scripts.dodger_effects import _count_empty_spawn_points

    for effect, coins in (("dark_ritual", 1), ("darker_ritual", 2), ("darkest_ritual", 2)):
        modes = ("lane", "tide", "short", "item-only") if effect == "darkest_ritual" else ("lane", "tide", "short")
        for mode in modes:
            state = setup(["Dodger", "Brogan"], ["Hanu", "Tigerclaw"])
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_dodger")
            state.current_actor_id = state.resolution_owner_id = hero.id
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            if mode == "lane":
                for team in state.teams.values():
                    for minion in team.minions:
                        state.remove_entity(minion.id)
                for point, tile in state.board.tiles.items():
                    if not tile.is_obstacle:
                        state.place_entity(hero.id, point)
                        if _count_empty_spawn_points(state, hero.id, card.radius_value) >= 2:
                            break
                else:
                    raise AssertionError("no eligible battle-zone position")
            else:
                h = _board(state)
                state.battle_zones = {}
                if mode != "item-only":
                    state.place_entity(hero.id, h(0, 0))
                hero.level = 8 if mode in ("tide", "item-only") else 7
            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)],
            )
            result = final.get_hero(hero.id)
            assert result.gold == hero.gold + (0 if mode in ("short", "item-only") else coins)
            assert result.items.get(StatType.ATTACK, 0) == int(effect == "darkest_ritual" and hero.level >= 8)
            yield f"{effect}-{mode}", state, operations


def dodger_razor_traces():
    from goa2.scripts.dodger_effects import _is_adjacent_to_empty_spawn_in_battle_zone

    for mode in ("no-zone", "adjacent", "ranged"):
        state = setup(["Dodger", "Brogan"], ["Hanu", "Tigerclaw"])
        for entity in list(state.entity_locations):
            state.remove_entity(entity)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_dodger")
        state.current_actor_id = state.resolution_owner_id = hero.id
        card = next(card for card in hero.deck if card.effect_id == "dread_razor")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        for point, tile in state.board.tiles.items():
            if tile.is_obstacle or not _is_adjacent_to_empty_spawn_in_battle_zone(state, point, hero.id):
                continue
            nearby = [hex for hex in point.neighbors() if hex in state.board.tiles
                      and not state.board.tiles[hex].is_obstacle
                      and not state.board.tiles[hex].spawn_point]
            distant = [hex for hex, tile in state.board.tiles.items() if not tile.is_obstacle
                       and 1 < point.distance(hex) <= card.range_value]
            if nearby and distant:
                break
        else:
            raise AssertionError("no eligible Dread Razor position")
        near = state.teams["BLUE"].minions[0]
        far = state.get_hero("hero_hanu")
        state.place_entity(hero.id, point)
        state.place_entity(near.id, nearby[0])
        state.place_entity(far.id, distant[0])
        if mode == "no-zone":
            hero.level = 8
            state.battle_zones = {}
        choices = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Choose attack mode":
                assert options == [1, 2]
                choices.append(True)
                return 2 if mode == "ranged" else 1
            if request.prompt == "Select Attack Target":
                assert set(options) == ({near.id, far.id} if mode == "ranged" else {near.id})
                return far.id if mode == "ranged" else near.id
            return options[0]

        operations, _ = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert bool(choices) == (mode != "no-zone")
        yield f"dread-razor-{mode}", state, operations


def dodger_death_trap_traces():
    from goa2.scripts.dodger_effects import _is_adjacent_to_empty_spawn_in_battle_zone

    for mode in ("one", "both", "no-zone", "piece"):
        state = setup(["Dodger", "Brogan"], ["Razzle" if mode == "piece" else "Hanu", "Tigerclaw"])
        for entity in list(state.entity_locations):
            state.remove_entity(entity)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_dodger")
        first = state.get_hero("hero_razzle" if mode == "piece" else "hero_hanu")
        second = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        card = next(card for card in hero.deck if card.effect_id == "death_trap")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        open_floor = [point for point, tile in state.board.tiles.items()
                      if not tile.is_obstacle and not tile.spawn_point]
        for origin in open_floor:
            qualifying = [point for point in open_floor
                          if 0 < origin.distance(point) <= card.radius_value
                          and _is_adjacent_to_empty_spawn_in_battle_zone(state, point, hero.id)]
            if len(qualifying) >= 2:
                break
        else:
            raise AssertionError("no eligible Death Trap position")
        state.place_entity(hero.id, origin)
        first_target = "hero_razzle_piece_1" if mode == "piece" else first.id
        state.place_entity(first_target, qualifying[0])
        if mode == "piece":
            distant = [point for point in open_floor if origin.distance(point) > card.radius_value]
            state.place_entity("hero_razzle_piece_2", distant[1])
        state.place_entity(second.id, qualifying[1] if mode == "both" else next(
            point for point in open_floor if origin.distance(point) > card.radius_value
        ))
        if mode == "no-zone":
            hero.level = 8
            state.battle_zones = {}
        choices = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if "Death Trap" in request.prompt:
                assert set(options) == ({first.id, second.id} if mode == "both" else {first_target})
                choices.append(first_target)
                return first_target
            if mode == "piece":
                assert request.player_id == first.id
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert bool(choices) == (mode != "no-zone")
        assert len(final.get_hero(first.id).hand) == len(first.hand) - int(mode != "no-zone")
        yield f"death-trap-{mode}", state, operations


def dodger_finger_traces():
    from aura_traces import _board

    for effect, first_mode, follow in (
        ("littlefinger_of_death", 1, False), ("finger_of_death", 2, False),
        ("middlefinger_of_death", 1, True), ("middlefinger_of_death", 2, True),
        ("middlefinger_of_death", 2, False),
    ):
        state = setup(["Dodger", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_dodger")
        near = state.get_hero("hero_hanu")
        far = state.get_hero("hero_tigerclaw")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(near.id, h(1, 0))
        state.place_entity(far.id, h(2, 0))
        state.place_entity(minion.id, h(0, 1))
        far.discard_card(far.hand[0])
        middle = effect == "middlefinger_of_death"
        if middle:
            near.discard_card(near.hand[0])
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        targeted = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose"):
                if options == [1, 2]:
                    return first_mode
            if request.prompt == "Select Attack Target":
                expected = {near.id, minion.id} if first_mode == 1 else {far.id}
                if first_mode == 2 and middle:
                    expected.add(near.id)
                assert set(options) == expected
                target = near.id if middle else minion.id if first_mode == 1 else far.id
                targeted.append(target)
                return target
            if request.prompt.startswith("Optionally target"):
                target = far.id if first_mode == 1 else minion.id
                assert options == [target]
                targeted.append(target if follow else "SKIP")
                return targeted[-1]
            return options[0]

        operations, _ = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert len(targeted) == 1 + middle
        yield f"{effect}-mode-{first_mode}-follow-{follow}", state, operations


def wuk_throw_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, mode in (
        ("toss_away", "enemy"), ("mighty_throw", "token"),
        ("monstrous_throw", "repeat"), ("monstrous_throw", "decline"),
    ):
        state = setup(["Wuk", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_wuk")
        enemy = state.get_hero("hero_hanu")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(-1, 0))
        state.place_entity(enemy.id, h(1, 0) if mode != "token" else h(3, 0))
        state.board.tiles[h(0, -1)].is_terrain = True
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        steps = []
        if mode in ("token", "repeat"):
            state.execution_context["fixture_throw_tree"] = h(0, 1)
            steps.append(PlaceTokenStep(token_type=TokenType.TREE, hex_key="fixture_throw_tree"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        targets = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select a token"):
                assert ally.id not in options
                target = enemy.id if enemy.id in options else options[0]
                targets.append(target)
                return target
            if options == ["YES", "NO"]:
                return "YES" if mode == "repeat" else "NO"
            assert request.prompt == "Select a space in range"
            assert h(0, -1).model_dump() not in options
            return (h(0, 2) if len(targets) == 2 else h(2, 0)).model_dump()

        operations, final = retain(state, steps, select=choose)
        assert len(targets) == 1 + (mode == "repeat")
        assert final.entity_locations[targets[0]] == h(2, 0)
        if mode == "repeat":
            assert final.entity_locations[targets[1]] == h(0, 2)
        yield f"{effect}-{mode}", state, operations


def wuk_canopy_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, modes in (
        ("into_the_canopy", [1]), ("into_the_canopy", [2]),
        ("treetop_ride", [1, 2]), ("treetop_ride", []),
    ):
        state = setup(["Wuk", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_wuk")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(0, 1))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        state.execution_context["fixture_canopy_tree"] = h(1, 0)
        selected = []
        trees = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Resolve a Canopy swap"):
                return "YES" if len(selected) < len(modes) else "NO"
            if request.prompt == "Choose one":
                selected.append(modes[len(selected)])
                return selected[-1]
            if request.prompt == "Select a friendly unit in radius":
                assert options == [ally.id]
                return ally.id
            assert request.prompt == "Select a Tree token in radius" and len(options) == 1
            trees.append(options[0])
            return options[0]

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.TREE, hex_key="fixture_canopy_tree"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert selected == modes
        # Python retains the first selection key and runs that swap again
        # before the ally swap when the branches are mixed.
        assert final.entity_locations[hero.id] == (h(1, 0) if modes == [1] else h(0, 0))
        assert final.entity_locations[ally.id] == (h(1, 0) if 2 in modes else h(0, 1))
        if trees:
            assert len(set(trees)) == 1
            assert final.entity_locations[trees[-1]] == (h(0, 1) if 2 in modes else h(0, 0))
        yield f"{effect}-swaps-{modes}", state, operations


def wuk_dominance_traces():
    from battle_effect_traces import exclusion_states
    from goa2.engine.steps import MinionBattleStep

    for effect, cap in (("claim_dominance", 1), ("assert_dominance", 2)):
        _, state = next(exclusion_states())
        state.active_effects.clear()
        hero = state.get_hero("hero_wuk")
        state.resolution_owner_id = hero.id
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        battle = MinionBattleStep()
        operations, final = retain(state, [
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id), battle,
        ])
        exclusion, = final.active_effects
        assert exclusion.max_value == cap
        assert exclusion.source_card_id == card.id
        assert exclusion.duration.value == "PASSIVE"
        yield effect, state, operations


def wuk_retrieval_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, mode, self_gets, ally_gets in (
        ("gifts_of_nature", "self", True, False),
        ("gifts_of_nature", "no-tree", False, False),
        ("tree_of_plenty", "ally", False, True),
        ("abundance", "both", True, True),
        ("abundance", "skip-self", False, True),
    ):
        state = setup(["Wuk", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_wuk")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(0, 1))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        own_card, ally_card = hero.hand[0], ally.hand[0]
        hero.discard_card(own_card)
        ally.discard_card(ally_card)
        steps = []
        if mode != "no-tree":
            state.execution_context["fixture_retrieve_tree"] = h(1, 0)
            steps.append(PlaceTokenStep(token_type=TokenType.TREE, hex_key="fixture_retrieve_tree"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        paid = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Remove a Tree token in radius":
                assert len(options) == 1
                paid.append(options[0])
                return options[0]
            if request.prompt == "Choose one":
                return 2
            if request.prompt == "You may retrieve a discarded card":
                assert options == [own_card.id]
                return own_card.id if self_gets else "SKIP"
            if request.prompt.startswith("Select a friendly hero"):
                assert options == [ally.id]
                return ally.id
            assert request.prompt == "Select a card to retrieve"
            assert request.player_id == ally.id and options == [ally_card.id]
            return ally_card.id

        operations, final = retain(state, steps, select=choose)
        assert len(paid) == int(mode != "no-tree")
        assert all(token not in final.entity_locations for token in paid)
        for owner, held, retrieved in ((hero, own_card, self_gets), (ally, ally_card, ally_gets)):
            assert (held.id in {card.id for card in final.get_hero(owner.id).hand}) == retrieved
        yield f"{effect}-{mode}", state, operations


def wuk_nature_attack_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, order in (
        ("natures_protector", 1), ("natures_guardian", 2),
        ("natures_champion", 1), ("natures_champion", 2),
    ):
        state = setup(["Wuk", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_wuk")
        near = state.get_hero("hero_hanu")
        far = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(near.id, h(1, 0))
        state.place_entity(far.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        state.execution_context["fixture_nature_tree"] = h(2, -1)
        targeted = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose"):
                if options == [1, 2]:
                    return order
            if request.prompt == "Target an adjacent enemy hero":
                assert options == [near.id]
                targeted.append(near.id)
                return near.id
            if request.prompt == "Target a unit in range adjacent to a Tree token":
                assert set(options) == ({far.id} if targeted else {near.id, far.id})
                targeted.append(far.id)
                return far.id
            return options[0]

        operations, _ = retain(state, [
            PlaceTokenStep(token_type=TokenType.TREE, hex_key="fixture_nature_tree"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        if effect == "natures_champion":
            assert targeted == ([near.id, far.id] if order == 1 else [far.id, near.id])
        else:
            assert targeted == [near.id if order == 1 else far.id]
        yield f"{effect}-order-{order}", state, operations


def wuk_sapling_slam_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, mode in (
        ("mystic_saplings", "three"), ("mystic_saplings", "stop"),
        ("tree_slam", "minion"), ("tree_slam", "tree"),
    ):
        state = setup(["Wuk", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_wuk")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        if effect == "tree_slam":
            state.place_entity(minion.id, h(2 if mode == "tree" else 1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        points = [h(1, 0), h(0, 1), h(-1, 0)]
        selected = []
        targets = []
        steps = []
        if mode == "tree":
            state.execution_context["fixture_slam_tree"] = points[0]
            steps.append(PlaceTokenStep(token_type=TokenType.TREE, hex_key="fixture_slam_tree"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Place a Tree token in radius":
                assert all(point.model_dump() not in options for point in selected)
                if mode == "stop" and selected:
                    return "SKIP"
                selected.append(points[len(selected)])
                return selected[-1].model_dump()
            if request.prompt == "Choose one":
                return 2 if mode == "tree" else 1
            if request.prompt == "Remove an adjacent Tree token":
                assert len(options) == 1
                return options[0]
            if request.prompt.startswith("Target "):
                assert options == [minion.id]
                targets.append(minion.id)
                return minion.id
            return options[0]

        operations, final = retain(state, steps, select=choose)
        trees = {final.entity_locations[token.id] for token in final.misc_entities.values()
                 if getattr(token, "token_type", None) == TokenType.TREE
                 and token.id in final.entity_locations}
        assert trees == set(points if mode == "three" else points[:1] if mode == "stop" else [])
        assert targets == ([minion.id] if effect == "tree_slam" else [])
        yield f"{effect}-{mode}", state, operations


def wuk_trample_traces():
    from aura_traces import _board

    for effect, mode in (
        ("trample", "normal"), ("trample", "discard"),
        ("angry_stampede", "defeat"), ("angry_stampede", "skip-minion"),
    ):
        state = setup(["Wuk", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_wuk")
        enemy = state.get_hero("hero_hanu")
        first, second = state.teams["BLUE"].minions[:2]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-2, 0))
        state.place_entity(enemy.id, h(-1, 0))
        state.place_entity(first.id, h(0, 0))
        long = effect == "angry_stampede"
        if long:
            state.place_entity(second.id, h(1, 0))
        if mode == "defeat":
            for held in list(enemy.hand):
                enemy.discard_card(held)
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        destination = h(-2, 1) if mode == "normal" else h(2 if long else 1, 0)
        defeated = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Choose how to move":
                return 1 if mode == "normal" else 2
            if request.prompt.startswith("Select Movement Destination"):
                assert destination.model_dump() in options
                return destination.model_dump()
            if request.prompt == "Defeat an enemy minion you moved through":
                if mode == "skip-minion" and defeated:
                    return "SKIP"
                victim = second if defeated else first
                assert victim.id in options
                defeated.append(victim.id)
                return victim.id
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert final.entity_locations[hero.id] == destination
        assert (enemy.id in final.entity_locations) == (mode != "defeat")
        if mode in ("discard", "skip-minion"):
            assert len(final.get_hero(enemy.id).hand) == len(enemy.hand) - 1
        assert defeated == ([first.id, second.id] if mode == "defeat" else [] if mode == "normal" else [first.id])
        assert all(victim not in final.entity_locations for victim in defeated)
        yield f"{effect}-{mode}", state, operations


def tali_token_traces():
    from aura_traces import _board
    from goa2.domain.models import MinionType, TokenType

    for effect, count, skip in (
        ("glacial_barrier", 1, False), ("wall_of_frost", 2, False),
        ("pack_ice", 3, False), ("pack_ice", 1, True),
        ("ancestral_totem", 1, False), ("venerated_totem", 1, False),
    ):
        state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_tali")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        points = [h(1, 0), h(0, 1), h(-1, 0)]
        selected = []
        totem = effect.endswith("totem")

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            assert request.prompt == ("Select a space for the Totem token" if totem else "Select a space for an Ice token")
            assert all(point.model_dump() not in options for point in selected)
            if skip and selected:
                return "SKIP"
            selected.append(points[len(selected)])
            return selected[-1].model_dump()

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        kind = TokenType.TOTEM if totem else TokenType.ICE
        tokens = [token for token in final.misc_entities.values()
                  if getattr(token, "token_type", None) == kind and token.id in final.entity_locations]
        assert len(tokens) == count and {final.entity_locations[token.id] for token in tokens} == set(points[:count])
        assert len(final.active_effects) == count
        assert {row.scope.origin_id for row in final.active_effects} == {token.id for token in tokens}
        if totem:
            upgraded = effect == "venerated_totem"
            assert tokens[0].is_immune_to_enemy_actions == upgraded
            assert final.active_effects[0].protected_minion_types == ([] if upgraded else [MinionType.MELEE])
        else:
            assert all(row.stat_value == -1 for row in final.active_effects)
        yield f"{effect}-count-{count}", state, operations


def tali_winter_weapon_traces():
    from aura_traces import _board
    from goa2.engine.steps import PerformPrimaryActionStep
    from primary_templates import TALI_WINTER_WEAPONS

    for effect in TALI_WINTER_WEAPONS:
        for discarded in (False, True):
            state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.battle_zones = {}
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_tali")
            target = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(target.id, h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            card.is_facedown = False
            if discarded:
                if card not in hero.hand:
                    hero.return_card_to_hand(card)
                hero.discard_card(card)
                state.execution_context["fixture_winter_card"] = card.id
                step = PerformPrimaryActionStep(card_key="fixture_winter_card", hero_id=hero.id)
            else:
                if card in hero.hand:
                    hero.hand.remove(card)
                hero.current_turn_card = card
                card.state = CardState.UNRESOLVED
                step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Choose an action to perform"):
                    return "ATTACK"
                assert options == [target.id]
                return target.id

            operations, final = retain(state, [step], select=choose)
            assert final.execution_context["attack_damage"] == card.primary_action_value + (0 if discarded else 3)
            yield f"{effect}-discard-{discarded}", state, operations


def tali_spirit_retrieval_traces():
    from aura_traces import _board
    from goa2.engine.steps import PerformPrimaryActionStep

    for effect in ("guardian_spirit", "warrior_spirit"):
        for mode in ("played", "discarded", "skip"):
            state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_tali")
            ally = state.get_hero("hero_brogan")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(ally.id, h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            card.is_facedown = False
            if mode == "played":
                if card in hero.hand:
                    hero.hand.remove(card)
                hero.current_turn_card = card
                card.state = CardState.UNRESOLVED
                step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
            else:
                if card not in hero.hand:
                    hero.return_card_to_hand(card)
                hero.discard_card(card)
                state.execution_context["fixture_spirit_card"] = card.id
                step = PerformPrimaryActionStep(card_key="fixture_spirit_card", hero_id=hero.id)
            ally_card = ally.hand[0]
            own_card = hero.hand[0]
            ally.discard_card(ally_card)
            hero.discard_card(own_card)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select a friendly hero"):
                    assert options == [ally.id]
                    return "SKIP" if mode == "skip" else ally.id
                if request.prompt == "Select a card to retrieve":
                    assert request.player_id == ally.id and options == [ally_card.id]
                    return ally_card.id
                assert request.prompt == "Select one of your discarded cards to retrieve"
                return "SKIP" if mode == "skip" else own_card.id

            operations, final = retain(state, [step], select=choose)
            held = {item.id for item in final.get_hero(hero.id).hand}
            assert (card.id in held) == (mode != "played" and effect == "guardian_spirit")
            assert (own_card.id in held) == (mode == "discarded" and effect == "warrior_spirit")
            assert (ally_card.id in {item.id for item in final.get_hero(ally.id).hand}) == (mode != "skip")
            yield f"{effect}-{mode}", state, operations


def tali_spirit_attack_traces():
    from aura_traces import _board
    from goa2.engine.steps import PerformPrimaryActionStep

    for effect, discarded, order in (
        ("spirit_wolf", False, 1), ("spirit_wolf", True, 2),
        ("spirit_bear", False, 1), ("spirit_bear", False, 2), ("spirit_bear", True, 2),
    ):
        state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_tali")
        near = state.get_hero("hero_hanu")
        far = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(near.id, h(1, 0))
        state.place_entity(far.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        card.is_facedown = False
        if discarded:
            hero.return_card_to_hand(card)
            hero.discard_card(card)
            state.execution_context["fixture_spirit_attack"] = card.id
            step = PerformPrimaryActionStep(card_key="fixture_spirit_attack", hero_id=hero.id)
        else:
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state = CardState.UNRESOLVED
            step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
        targets = []
        repeats = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose an action to perform"):
                return "ATTACK"
            if request.prompt == "Choose Spirit attack mode":
                assert options == ([2] if discarded else [1, 2])
                return order
            if request.prompt.startswith("Resolve the other Spirit Bear"):
                repeats.append(True)
                return "YES"
            if request.prompt == "Select a unit in range":
                assert set(options) == ({far.id} if targets else {near.id, far.id})
                targets.append(far.id)
                return far.id
            if request.prompt == "Select an adjacent enemy hero":
                assert options == [near.id]
                targets.append(near.id)
                return near.id
            return options[0]

        operations, _ = retain(state, [step], select=choose)
        both = effect == "spirit_bear" and not discarded
        assert len(targets) == 1 + both and bool(repeats) == both
        yield f"{effect}-discard-{discarded}-order-{order}", state, operations


def tali_ice_blast_traces():
    from aura_traces import _board
    from goa2.domain.models import ActionType

    for mode in ("match", "skip", "no-match"):
        state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_tali")
        defender = state.get_hero("hero_hanu")
        other = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(defender.id, h(1, 0))
        state.place_entity(other.id, h(0, 1))
        card = next(card for card in hero.deck if card.effect_id == "ice_blast")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        matching = {held.color: held for held in other.hand}
        defense = next(held for held in defender.hand if held.color in matching
                       and ActionType.DEFENSE in held.secondary_actions)
        matched = matching[defense.color]
        if mode == "no-match":
            other.discard_card(matched)
        prompts = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select Attack Target":
                return defender.id
            if request.prompt.startswith("Select another enemy hero"):
                assert options == [other.id]
                prompts.append(True)
                return "SKIP" if mode == "skip" else other.id
            if defense.id in options:
                return defense.id
            assert mode == "match" and options == [matched.id]
            return matched.id

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert prompts == [True]
        assert (matched.id in {held.id for held in final.get_hero(other.id).hand}) == (mode == "skip")
        yield f"ice-blast-{mode}", state, operations


def tali_commune_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType

    for mode in ("discard", "already-discarded"):
        state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_tali")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(1, -1))
        card = next(card for card in hero.deck if card.effect_id == "commune_with_spirits")
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        barrier = next(held for held in hero.hand if held.effect_id == "glacial_barrier")
        for held in list(hero.hand):
            if held is not barrier and held.color == barrier.color:
                hero.hand.remove(held)
                held.state = CardState.DECK
        if mode == "already-discarded":
            hero.discard_card(barrier)
        asked = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            prompt = request.prompt
            asked.append(prompt)
            if prompt == "Select a friendly hero to name a color":
                assert options == [ally.id]
                return ally.id
            if prompt == "Name a card color":
                assert request.player_id == ally.id
                assert options == ["BLUE", "GOLD", "GREEN", "RED", "SILVER"]
                return barrier.color.value
            if prompt == "You may discard a card of the named color":
                assert mode == "discard" and options == [barrier.id]
                return barrier.id
            if prompt == "Select a discarded card of the named color to perform":
                assert options == [barrier.id]
                return barrier.id
            assert prompt == "Select a space for an Ice token"
            return h(1, 0).model_dump()

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert ("You may discard a card of the named color" in asked) == (mode == "discard")
        assert barrier.id in {held.id for held in final.get_hero(hero.id).discard_pile}
        assert any(getattr(token, "token_type", None) == TokenType.ICE
                   and final.entity_locations.get(token.id) == h(1, 0)
                   for token in final.misc_entities.values())
        yield f"commune-{mode}", state, operations


def tali_directional_traces():
    from aura_traces import _board

    for effect in ("cold_snap", "snowstorm"):
        for blocked in (False, True):
            state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_tali")
            first = state.get_hero("hero_hanu")
            second = state.get_hero("hero_tigerclaw")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 1))
            state.place_entity(first.id, h(0, 0))
            state.place_entity(second.id, h(1, 0))
            state.board.tiles[h(2, 0)].is_terrain = blocked
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                assert request.prompt == "Choose a direction"
                assert [selection_value(option) for option in request.options] == list(range(6))
                return 0

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert final.entity_locations[first.id] == h(0 if blocked else 1, 0)
            assert final.entity_locations[second.id] == h(1 if blocked else 2, 0)
            yield f"{effect}-blocked-{blocked}", state, operations


def tali_blizzard_traces():
    from aura_traces import _board
    from goa2.engine.steps import PerformPrimaryActionStep

    for mode in ("played", "repeat", "decline"):
        state = setup(["Tali", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_tali")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.unresolved_hero_ids.clear()
        state.place_entity(hero.id, h(0, 1))
        state.place_entity(enemy.id, h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == "blizzard")
        card.is_facedown = False
        if mode == "played":
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state = CardState.UNRESOLVED
            step = ResolveCardTextStep(hero_id=hero.id, card_id=card.id)
        else:
            hero.return_card_to_hand(card)
            hero.discard_card(card)
            state.execution_context["fixture_blizzard"] = card.id
            step = PerformPrimaryActionStep(card_key="fixture_blizzard", hero_id=hero.id)
        prompts = []

        def choose(request, _mode):
            prompts.append(request.prompt)
            if request.prompt.startswith("Repeat Blizzard?"):
                return "YES" if mode == "repeat" else "NO"
            assert request.prompt == "Choose a direction"
            return 0

        operations, final = retain(state, [step, FindNextActorStep()], select=choose)
        assert len(prompts) == {"played": 1, "decline": 2, "repeat": 3}[mode]
        assert final.entity_locations[enemy.id] == h(2 if mode == "repeat" else 1, 0)
        assert final.turn == state.turn + 1 and not final.active_effects
        yield f"blizzard-{mode}", state, operations


def ursafar_rage_traces():
    from aura_traces import _board
    from combat_effect_traces import effect as make_effect
    from goa2.domain.models import EffectType

    for effect, mode in (
        ("sniff_out", "calm"), ("sniff_out", "active"),
        ("eyes_on_the_prey", "dormant"), ("eyes_on_the_prey", "ultimate"),
        ("apex_predator", "defeat"), ("rip", "calm"), ("rip", "active"),
    ):
        state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_ursafar")
        enemy = state.get_hero("hero_hanu")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(2, 0))
        state.place_entity(minion.id, h(1, 0))
        if mode in ("ultimate", "defeat"):
            hero.level = 8
        if mode in ("active", "dormant"):
            rage = make_effect(state, "fixture-rage", EffectType.ENRAGED, source=hero.id)
            rage.is_active = mode == "active"
            state.active_effects.append(rage)
        if mode == "defeat":
            for held in list(enemy.hand):
                enemy.discard_card(held)
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        chosen = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select an enemy hero"):
                assert options == [enemy.id]
                chosen.append(enemy.id)
                return enemy.id
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        if effect == "rip":
            assert any(row.effect_type == EffectType.ENRAGED and row.source_card_id == card.id
                       for row in final.active_effects)
            assert ("self_hero" in final.execution_context) == (mode == "active")
        else:
            assert bool(chosen) == (mode not in ("calm", "dormant"))
            assert (enemy.id in final.entity_locations) == (mode != "defeat")
            if mode in ("active", "ultimate"):
                assert len(final.get_hero(enemy.id).hand) == len(enemy.hand) - 1
        yield f"{effect}-{mode}", state, operations


def ursafar_rage_movement_traces():
    from aura_traces import _board
    from combat_effect_traces import effect as make_effect
    from goa2.domain.models import EffectType, StatType

    for effect, bonus in (("cold_ire", 1), ("eyes_of_flame", 2)):
        for mode in ("calm", "active", "ultimate"):
            state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_ursafar")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            hero.items[StatType.MOVEMENT] = 1
            if mode == "ultimate":
                hero.level = 8
            if mode == "active":
                state.active_effects.append(make_effect(
                    state, "fixture-rage", EffectType.ENRAGED, source=hero.id,
                ))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            distance = card.primary_action_value + 1 + (0 if mode == "calm" else bonus)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                assert request.prompt.startswith("Select Movement Destination")
                assert h(distance, 0).model_dump() in options
                if distance < 4:
                    assert h(distance + 1, 0).model_dump() not in options
                return h(distance, 0).model_dump()

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert final.entity_locations[hero.id] == h(distance, 0)
            assert any(row.effect_type == EffectType.ENRAGED and row.source_card_id == card.id
                       for row in final.active_effects)
            yield f"{effect}-{mode}", state, operations


def ursafar_prey_traces():
    from aura_traces import _board
    from goa2.domain.models import ActionType, StatType

    for effect, mode, removals in (
        ("prey_drive", "survives", 1), ("prey_abundance", "removed", 0),
        ("feeding_frenzy", "survives", 2), ("feeding_frenzy", "calm", 0),
    ):
        state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_ursafar")
        enemy = state.get_hero("hero_hanu")
        first, second, defeated = state.teams["BLUE"].minions[:3]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(1, 0))
        state.place_entity(first.id, h(0, 1))
        state.place_entity(second.id, h(-1, 0))
        if mode == "removed":
            state.place_entity(defeated.id, h(0, -1))
        hero.level = 7 if mode == "calm" else 8
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        if mode != "removed":
            hero.items[StatType.ATTACK] = 1 - card.primary_action_value
        defense = next(held for held in enemy.hand if ActionType.DEFENSE in held.secondary_actions)
        removed = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select Attack Target":
                return defeated.id if mode == "removed" else enemy.id
            if request.prompt.startswith(("Select an enemy minion", "Select enemy minions")):
                assert len(removed) < removals
                target = second.id if removed else first.id
                assert target in options
                removed.append(target)
                return target
            if defense.id in options:
                return defense.id
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert len(removed) == removals
        assert enemy.id in final.entity_locations
        assert all((target.id in final.entity_locations) == (target.id not in removed)
                   for target in (first, second))
        if mode == "removed":
            assert defeated.id not in final.entity_locations
        yield f"{effect}-{mode}", state, operations


def ursafar_swap_traces():
    from aura_traces import _board

    for effect, mode, extra in (
        ("prowling_brute", "calm", 0), ("prowling_brute", "swap", 0),
        ("rampaging_beast", "swap", 1), ("rampaging_beast", "skip", 0),
        ("unstoppable_force", "swap", 2), ("unstoppable_force", "stay", 0),
    ):
        state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_ursafar")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(2, 0))
        hero.level = 7 if mode == "calm" else 8
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        prompts = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            prompts.append(request.prompt)
            if request.prompt.startswith("Select Movement Destination"):
                return h(1, 0).model_dump()
            if request.prompt.startswith("Select a unit or token"):
                assert options == [ally.id]
                return "SKIP" if mode == "skip" else ally.id
            assert request.prompt.startswith("Move up to")
            assert h(2 + extra, 0).model_dump() in options
            return h(2 + extra, 0).model_dump()

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        swapped = mode in ("swap", "stay")
        assert final.entity_locations[hero.id] == h(2 + extra if swapped else 1, 0)
        assert final.entity_locations[ally.id] == h(1 if swapped else 2, 0)
        assert sum(prompt.startswith("Move up to") for prompt in prompts) == int(
            swapped and effect != "prowling_brute"
        )
        yield f"{effect}-{mode}", state, operations


def ursafar_claws_traces():
    from aura_traces import _board

    for mode in ("calm", "move", "skip"):
        state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_ursafar")
        enemy = state.get_hero("hero_hanu")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(2, 0))
        state.place_entity(minion.id, h(0, -1))
        hero.level = 7 if mode == "calm" else 8
        card = next(card for card in hero.deck if card.effect_id == "claws_that_catch")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        prompts = []
        targets = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Move 1 space"):
                prompts.append(True)
                assert h(1, 0).model_dump() in options
                assert h(-1, 0).model_dump() not in options
                return "SKIP" if mode == "skip" else h(1, 0).model_dump()
            if request.prompt == "Select Attack Target":
                target = enemy.id if mode == "move" else minion.id
                assert options == [target]
                targets.append(target)
                return target
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert prompts == ([] if mode == "calm" else [True])
        assert targets == [enemy.id if mode == "move" else minion.id]
        assert final.entity_locations[hero.id] == h(1 if mode == "move" else 0, 0)
        yield f"claws-that-catch-{mode}", state, operations


def ursafar_tear_traces():
    from aura_traces import _board

    for mode, life_loss, reward in (
        ("calm", 1, 1), ("hero", 2, 3), ("minion", 0, 2), ("terminal", 2, 3),
    ):
        state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.battle_zones = {}
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_ursafar")
        enemy = state.get_hero("hero_hanu")
        victim = state.teams["BLUE"].minions[0] if mode == "minion" else enemy
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(victim.id, h(1, 0))
        hero.level = 7 if mode == "calm" else 8
        for held in list(enemy.hand):
            enemy.discard_card(held)
        if mode == "terminal":
            state.teams["BLUE"].life_counters = 2
        card = next(card for card in hero.deck if card.effect_id == "tear")
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)],
        )
        assert victim.id not in final.entity_locations
        assert final.teams["BLUE"].life_counters == state.teams["BLUE"].life_counters - life_loss
        assert final.get_hero(hero.id).gold >= hero.gold + reward
        assert (final.phase == GamePhase.GAME_OVER) == (mode == "terminal")
        yield f"tear-{mode}", state, operations



def ursafar_discard_action_traces():
    from aura_traces import _board

    for effect, choice in (
        ("instinctive_reaction", 0), ("instinctive_reaction", 1),
        ("instinctive_reaction", 2), ("evolutionary_response", 3),
        ("evolutionary_response", 4),
    ):
        state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_ursafar")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        hero.level = 8 if choice else 7
        move = next(card for card in hero.hand if card.effect_id == "prowling_brute")
        retrieve = next(card for card in hero.hand if card.color.value == "GOLD")
        hero.discard_card(move)
        hero.discard_card(retrieve)
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        actions = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Choose one"):
                assert choice in options
                return choice
            if request.prompt == "Select a discarded card to retrieve.":
                assert retrieve.id in options
                actions.append("retrieve")
                return retrieve.id
            if request.prompt == "Select a discarded card to perform its primary action.":
                assert move.id in options
                actions.append("perform")
                return move.id
            if request.prompt.startswith("Select Movement Destination"):
                return h(1, 0).model_dump()
            return "SKIP" if request.can_skip else options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert actions == ([], ["perform"], ["retrieve"], ["perform", "retrieve"],
                           ["retrieve", "perform"])[choice]
        assert final.entity_locations[hero.id] == h(int(choice in (1, 3, 4)), 0)
        assert (retrieve.id in {card.id for card in final.get_hero(hero.id).hand}) == (
            choice in (2, 3, 4)
        )
        yield f"{effect}-{choice}", state, operations



def ursafar_angry_roar_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType

    for mode in ("calm", "no-eligible", "eligible"):
        state = setup(["Ursafar", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_ursafar")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        hero.level = 7 if mode == "calm" else 8
        cards = {card.effect_id: card for card in hero.deck}
        card = cards["angry_roar"]
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        move = cards["prowling_brute"]
        for played in (move, cards["sniff_out"]):
            if played in hero.hand:
                hero.hand.remove(played)
            hero.played_cards.append(played)
            played.state, played.is_facedown = CardState.RESOLVED, False
            played.enraged_active_override = played != move or mode != "no-eligible"
        selected = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select an active card"):
                assert options == [move.id]
                selected.append(move.id)
                return move.id
            if request.prompt.startswith("Select Movement Destination"):
                return h(1, 0).model_dump()
            return "SKIP" if request.can_skip else options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert selected == ([move.id] if mode == "eligible" else [])
        assert final.entity_locations[hero.id] == h(int(mode == "eligible"), 0)
        assert any(row.effect_type == EffectType.ENRAGED and row.source_card_id == card.id
                   for row in final.active_effects)
        yield f"angry-roar-{mode}", state, operations



def ignatia_coin_traces():
    from aura_traces import _board
    from combat_effect_traces import effect as make_effect
    from goa2.domain.models import EffectType, TeamColor

    for effect, branch in (
        ("equilibrium", "create"),
        ("playing_with_fire", "blue"),
        ("erratic_fireblast", "orange"),
        ("loosely_aimed_firebolts", "orange"),
        ("playing_with_fire", "equilibrium"),
        ("playing_with_fire", "ultimate"),
    ):
        state = setup(["Ignatia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        state.tie_breaker_team = TeamColor.RED if branch == "orange" else TeamColor.BLUE
        hero = state.get_hero("hero_ignatia")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        straight = state.get_hero("hero_hanu")
        angled = state.get_hero("hero_tigerclaw")
        for target, at in ((straight, h(1, 0)), (angled, h(1, 1))):
            state.place_entity(target.id, at)
            for held in list(target.hand):
                target.discard_card(held)
        if branch == "equilibrium":
            state.active_effects.append(
                make_effect(state, "fixture-equilibrium", EffectType.EQUILIBRIUM, source=hero.id),
            )
        hero.level = 8 if branch == "ultimate" else 7
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        targets = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Equilibrium:"):
                return 2
            if request.prompt.startswith("Chaos Incarnate:"):
                return "YES"
            if straight.id in options or angled.id in options:
                target = angled if branch in ("blue", "ultimate") and not targets else straight
                assert target.id in options
                assert (angled if target == straight else straight).id not in options
                targets.append(target.id)
                return target.id
            return "SKIP" if request.can_skip else options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        if branch == "create":
            assert any(row.effect_type == EffectType.EQUILIBRIUM and row.source_card_id == card.id
                       for row in final.active_effects)
        else:
            expected = [angled.id] if branch in ("blue", "ultimate") else [straight.id]
            if branch == "ultimate":
                expected.append(straight.id)
                assert final.tie_breaker_team == TeamColor.RED
            assert targets == expected
        yield f"{effect}-{branch}", state, operations



def ignatia_range_traces():
    from aura_traces import _board
    from goa2.domain.models import TeamColor

    for effect, orange in (
        ("crack_of_doom", False), ("crack_of_doom", True),
        ("imminent_eruption", False), ("imminent_eruption", True),
    ):
        state = setup(["Ignatia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        state.tie_breaker_team = TeamColor.RED if orange else TeamColor.BLUE
        hero = state.get_hero("hero_ignatia")
        near = state.get_hero("hero_hanu")
        far = state.get_hero("hero_tigerclaw")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        for unit, at in ((hero, h(-2, 0)), (near, h(-1, 0)),
                         (far, h(3, 0)), (minion, h(-2, 1))):
            state.place_entity(unit.id, at)
        for target in (near, far):
            for held in list(target.hand):
                target.discard_card(held)
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        target = far if orange else near
        repeat = not orange and effect == "imminent_eruption"
        selected = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Repeat once on a minion?":
                assert repeat
                return "YES"
            if request.prompt == "Target an adjacent minion":
                assert options == [minion.id]
                selected.append(minion.id)
                return minion.id
            if near.id in options or far.id in options:
                assert set(options) == ({far.id} if orange else {near.id, minion.id})
                selected.append(target.id)
                return target.id
            return "SKIP" if request.can_skip else options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert selected == [target.id] + ([minion.id] if repeat else [])
        assert target.id not in final.entity_locations
        assert (minion.id not in final.entity_locations) == repeat
        yield f"{effect}-{'orange' if orange else 'blue'}", state, operations



def ignatia_bolt_and_swap_traces():
    from aura_traces import _board
    from goa2.domain.models import TeamColor

    for effect in ("chaos_bolt", "unstable_portal", "chaos_gate"):
        for orange in (False, True):
            state = setup(["Ignatia", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            state.tie_breaker_team = TeamColor.RED if orange else TeamColor.BLUE
            hero = state.get_hero("hero_ignatia")
            enemy = state.get_hero("hero_hanu")
            ally = state.get_hero("hero_brogan")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            origin = h(0, 0)
            ally_pos, enemy_pos, minion_pos = h(0, 1), h(2, 0), h(-1, 0)
            for unit, at in ((hero, origin), (enemy, enemy_pos),
                             (ally, ally_pos), (minion, minion_pos)):
                state.place_entity(unit.id, at)
            for held in list(enemy.hand):
                enemy.discard_card(held)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            selected = []
            swap_target = enemy if orange else ally
            swap_pos = enemy_pos if orange else ally_pos
            move_dest = h(3, 0) if orange else h(1, 0)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Swap with"):
                    assert swap_target.id in options
                    assert (ally if orange else enemy).id not in options
                    selected.append(swap_target.id)
                    return swap_target.id
                if request.prompt.startswith("You may move"):
                    assert effect == "chaos_gate"
                    selected.append(move_dest)
                    assert move_dest.model_dump() in options
                    return move_dest.model_dump()
                if enemy.id in options or minion.id in options:
                    target = enemy if orange else minion
                    assert options == [target.id]
                    selected.append(target.id)
                    return target.id
                return "SKIP" if request.can_skip else options[0]

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            if effect == "chaos_bolt":
                assert selected == [enemy.id if orange else minion.id]
            else:
                moved = effect == "chaos_gate"
                assert selected == [swap_target.id] + ([move_dest] if moved else [])
                assert final.entity_locations[hero.id] == (move_dest if moved and orange else swap_pos)
                assert final.entity_locations[swap_target.id] == (
                    move_dest if moved and not orange else origin
                )
            yield f"{effect}-{'orange' if orange else 'blue'}", state, operations



def ignatia_combustion_traces():
    from aura_traces import _board
    from goa2.domain.models import TeamColor, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect in ("abrupt_combustion", "spontaneous_immolation", "violent_conflagration"):
        for orange in (False, True):
            state = setup(["Ignatia", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            state.tie_breaker_team = TeamColor.RED if orange else TeamColor.BLUE
            hero = state.get_hero("hero_ignatia")
            enemy = state.get_hero("hero_hanu")
            isolated = state.get_hero("hero_tigerclaw")
            minion, other = state.teams["BLUE"].minions[:2]
            state.current_actor_id = state.resolution_owner_id = hero.id
            for unit, at in (
                (hero, h(0, 0)), (enemy, h(1, 0)), (minion, h(1, 1)),
                (isolated, h(-2, 0)), (other, h(0, -2)),
            ):
                state.place_entity(unit.id, at)
            steps = []
            if not orange and effect == "spontaneous_immolation":
                state.remove_entity(minion.id)
                state.execution_context["fixture_magma"] = h(1, 1)
                steps.append(PlaceTokenStep(token_type=TokenType.MAGMA, hex_key="fixture_magma"))
            violent = effect == "violent_conflagration"
            if violent and not orange:
                for held in list(enemy.hand):
                    enemy.discard_card(held)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            target = minion if orange else enemy
            selected = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("An enemy"):
                    assert options == [target.id]
                    selected.append(target.id)
                return options[0]

            steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
            operations, final = retain(state, steps, select=choose)
            assert selected == [target.id]
            if orange:
                assert minion.id not in final.entity_locations
                assert (final.get_hero(hero.id).gold > hero.gold) == violent
            elif violent:
                assert enemy.id not in final.entity_locations
            else:
                assert len(final.get_hero(enemy.id).hand) == len(enemy.hand) - 1
            yield f"{effect}-{'orange' if orange else 'blue'}", state, operations



def ignatia_straight_movement_traces():
    from aura_traces import _board
    from goa2.domain.models import TeamColor

    for effect, distance in (("searing_heat", 2), ("scorching_blaze", 3)):
        for orange in (False, True):
            state = setup(["Ignatia", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.tie_breaker_team = TeamColor.RED if orange else TeamColor.BLUE
            hero = state.get_hero("hero_ignatia")
            ally = state.get_hero("hero_brogan")
            enemy = state.get_hero("hero_hanu")
            mover, other = (enemy, ally) if orange else (ally, enemy)
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(mover.id, h(1, 0))
            state.place_entity(other.id, h(-1, 0))
            state.board.tiles[h(2, 0)].is_terrain = True
            destination = h(1, distance)
            card = next(card for card in hero.deck if card.effect_id == effect)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            chosen = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select a"):
                    assert mover.id in options and other.id not in options
                    chosen.append(mover.id)
                    return mover.id
                assert request.prompt == "Move it in a straight line"
                assert destination.model_dump() in options
                assert h(3, 0).model_dump() not in options  # Terrain blocks the path.
                assert h(1, 1).model_dump() not in options  # Too short.
                assert (h(1, 3).model_dump() in options) == (distance == 3)
                chosen.append(destination)
                return destination.model_dump()

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert chosen == [mover.id, destination]
            assert final.entity_locations[mover.id] == destination
            assert final.entity_locations[other.id] == h(-1, 0)
            yield f"{effect}-{'orange' if orange else 'blue'}", state, operations



def ignatia_path_traces():
    from aura_traces import _board
    from goa2.domain.models import TeamColor, Token, TokenType

    for effect, distance in (("path_of_ashes", 2), ("path_of_cinders", 3), ("path_of_flames", 4)):
        for orange in (False, True):
            state = setup(["Ignatia", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.tie_breaker_team = TeamColor.RED if orange else TeamColor.BLUE
            hero = state.get_hero("hero_ignatia")
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            destination = h(distance, 0)
            placements = [h(0, 1), h(-1, 1), h(-1, 0), h(0, -1)][:distance]
            prompts = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if orange:
                    assert request.prompt == "You may place a Magma token in radius"
                    at = placements[len(prompts)]
                    assert all(prior.model_dump() not in options for prior in prompts)
                else:
                    assert request.prompt == f"Move up to {distance} spaces in a straight line"
                    at = destination
                    assert h(1, 1).model_dump() not in options
                assert at.model_dump() in options
                prompts.append(at)
                return at.model_dump()

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert prompts == (placements if orange else [destination])
            assert final.entity_locations[hero.id] == (h(0, 0) if orange else destination)
            magma = {
                final.entity_locations[token.id]
                for token in final.misc_entities.values()
                if isinstance(token, Token) and token.token_type == TokenType.MAGMA
                and token.id in final.entity_locations
            }
            assert magma == set(placements if orange else [h(q, 0) for q in range(distance)])
            yield f"{effect}-{'orange' if orange else 'blue'}", state, operations



def ignatia_ultimate_traces():
    from aura_traces import _board
    from combat_effect_traces import effect as make_effect
    from goa2.domain.models import EffectType, TeamColor

    for mode in ("decline", "different-target", "ultimate-primary"):
        state = setup(["Ignatia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.tie_breaker_team = TeamColor.BLUE
        hero = state.get_hero("hero_ignatia")
        first = state.get_hero("hero_hanu")
        second = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(first.id, h(1, 0))
        state.place_entity(second.id, h(0, 1))
        state.active_effects.append(
            make_effect(state, "fixture-equilibrium", EffectType.EQUILIBRIUM, source=hero.id),
        )
        hero.level = 8
        card = hero.ultimate_card if mode == "ultimate-primary" else next(
            card for card in hero.deck if card.effect_id == "chaos_gate"
        )
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        targets = []
        repeats = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Equilibrium:"):
                return 2
            if request.prompt.startswith("Chaos Incarnate:"):
                repeats.append(request.prompt)
                return "NO" if mode == "decline" else "YES"
            if request.prompt.startswith("Swap with"):
                target = second if targets else first
                assert set(options) == ({second.id} if targets else {first.id, second.id})
                targets.append(target.id)
                return target.id
            assert request.prompt == "You may move 1 space"
            return "SKIP"

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert len(repeats) == int(mode != "ultimate-primary")
        assert targets == {
            "ultimate-primary": [], "decline": [first.id],
            "different-target": [first.id, second.id],
        }[mode]
        assert final.tie_breaker_team == (
            TeamColor.RED if mode == "different-target" else TeamColor.BLUE
        )
        assert final.entity_locations[hero.id] == {
            "ultimate-primary": h(0, 0), "decline": h(1, 0),
            "different-target": h(0, 1),
        }[mode]
        yield f"chaos-incarnate-{mode}", state, operations



def cordelia_broom_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType
    from goa2.domain.models.effect import DurationType

    for effect, bonus in (
        ("broom_for_improvement", 1), ("broomstick_beatdown", 2),
        ("this_is_my_broomstick", 3),
    ):
        for has_target in (False, True):
            state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            hero = state.get_hero("hero_cordelia")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            if has_target:
                state.place_entity(minion.id, h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)],
            )
            effects = [
                row for row in final.active_effects
                if row.effect_type == EffectType.BASIC_ACTION_STAT_BONUS
            ]
            if has_target:
                assert len(effects) == 1
                row = effects[0]
                assert row.source_card_id == card.id and row.source_id == hero.id
                assert row.is_active and row.stat_value == bonus
                assert row.duration == DurationType.THIS_ROUND
                assert minion.id not in final.entity_locations
            else:
                assert not effects
            yield f"{effect}-{'attack' if has_target else 'no-target'}", state, operations



def cordelia_bonds_traces():
    from aura_traces import _board

    for effect in ("collateral_misfortune", "fatal_bonds"):
        for choice in (1, 2):
            state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            hero = state.get_hero("hero_cordelia")
            enemy = state.get_hero("hero_hanu")
            adjacent, linked = state.teams["BLUE"].minions[:2]
            state.current_actor_id = state.resolution_owner_id = hero.id
            for unit, at in ((hero, h(0, 0)), (enemy, h(2, 1)),
                             (adjacent, h(-1, 0)), (linked, h(1, 1))):
                state.place_entity(unit.id, at)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            first, second = (adjacent, linked) if choice == 1 else (linked, adjacent)
            targets = []

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Choose which attack"):
                    return choice
                if request.prompt.startswith("You may choose a different target"):
                    assert options == [second.id]
                    targets.append(second.id)
                    return second.id
                assert options == [first.id]
                targets.append(first.id)
                return first.id

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            both = effect == "fatal_bonds"
            assert targets == [first.id] + ([second.id] if both else [])
            assert first.id not in final.entity_locations
            assert (second.id not in final.entity_locations) == both
            yield f"{effect}-{choice}", state, operations



def cordelia_reveal_traces():
    from aura_traces import _board
    from goa2.domain.models import CardTier

    for effect, tier, discarded in (
        ("vile_vial", CardTier.I, True), ("vile_vial", CardTier.II, False),
        ("potion_explosion", CardTier.II, True), ("potion_explosion", CardTier.III, False),
    ):
        state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_cordelia")
        victim = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(victim.id, h(1, 0))
        revealed = next(card for card in victim.deck if card.tier == tier)
        if revealed not in victim.hand:
            victim.hand.append(revealed)
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select an enemy"):
                assert options == [victim.id]
                return victim.id
            assert request.prompt == "Choose a card from your hand to reveal faceup"
            assert request.player_id == victim.id
            assert revealed.id in options
            return revealed.id

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        value = {CardTier.I: 1, CardTier.II: 2, CardTier.III: 3}[tier]
        assert final.get_hero(hero.id).gold == hero.gold + value
        assert final.card_reveal["card_id"] == revealed.id
        assert len(final.public_revealed_cards) == 1
        assert (revealed.id in {card.id for card in final.get_hero(victim.id).hand}) != discarded
        yield f"{effect}-{tier.value}", state, operations



def cordelia_spores_traces():
    from aura_traces import _board

    for effect in ("healing_spores", "fungal_favor", "toxic_tranquility"):
        for retrieve in (False, True):
            state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_cordelia")
            ally = state.get_hero("hero_brogan")
            enemy = state.get_hero("hero_hanu")
            other = state.get_hero("hero_tigerclaw")
            state.current_actor_id = state.resolution_owner_id = hero.id
            for unit, at in ((hero, h(0, 0)), (ally, h(1, 0)),
                             (enemy, h(0, 1)), (other, h(-1, 0))):
                state.place_entity(unit.id, at)
            enemy.gold = other.gold = 2
            discarded = ally.hand[0]
            ally.discard_card(discarded)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select a friendly"):
                    assert options == [ally.id]
                    return ally.id
                if request.prompt.startswith("You may retrieve"):
                    assert request.player_id == ally.id and options == [discarded.id]
                    return discarded.id if retrieve else "SKIP"
                assert request.prompt.startswith("Select an enemy") and retrieve
                assert set(options) == {enemy.id, other.id}
                return enemy.id

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert (discarded.id in {card.id for card in final.get_hero(ally.id).hand}) == retrieve
            assert final.get_hero(enemy.id).gold == 2 - int(retrieve)
            assert final.get_hero(other.id).gold == 2 - int(retrieve and effect == "toxic_tranquility")
            yield f"{effect}-{'retrieve' if retrieve else 'skip'}", state, operations



def cordelia_path_traces():
    from aura_traces import _board

    for effect, distance in (
        ("charmed_step", 1), ("candy_trail", 1),
        ("enchanted_path", 1), ("enchanted_path", 2),
    ):
        state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_cordelia")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        destination = h(1, 0).model_dump()
        choices = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select an enemy hero in range":
                assert options == [enemy.id]
                return enemy.id
            if request.prompt == "Select an empty space adjacent to that hero":
                assert destination in options
                return destination
            if request.prompt == "Choose how far to push that hero":
                assert options == [1, 2]
                choices.append(distance)
                return distance
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert choices == ([distance] if effect == "enchanted_path" else [])
        assert final.entity_locations[hero.id] == h(1, 0)
        assert final.entity_locations[enemy.id] == h(2 + distance, 0)
        yield f"{effect}-{distance}", state, operations



def cordelia_brewing_traces():
    from aura_traces import _board

    for effect, retrieve in (
        ("trouble_brewing", False), ("recipe_for_disaster", False),
        ("recipe_for_disaster", True),
    ):
        state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_cordelia")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(minion.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        basic = next(card for card in hero.hand if card.is_basic)
        tiered = next(card for card in hero.hand if not card.is_basic)
        hero.discard_card(basic)
        hero.discard_card(tiered)
        target_hex = h(1, 0)
        retrievals = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select an enemy minion in range":
                assert options == [minion.id]
                return minion.id
            if request.prompt == "Select an empty space adjacent to Cordelia":
                assert target_hex.model_dump() in options
                return target_hex.model_dump()
            assert request.prompt == "You may retrieve a discarded basic card"
            assert options == [basic.id]
            retrievals.append(basic.id)
            return basic.id if retrieve else "SKIP"

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert final.entity_locations[minion.id] == target_hex
        assert bool(retrievals) == (effect == "recipe_for_disaster")
        hand = {card.id for card in final.get_hero(hero.id).hand}
        assert (basic.id in hand) == retrieve and tiered.id not in hand
        yield f"{effect}-{'retrieve' if retrieve else 'place-only'}", state, operations



def cordelia_bewitch_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType, StatType

    for adjacent in (False, True):
        state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_cordelia")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(minion.id, h(1 if adjacent else 2, 0))
        card = next(card for card in hero.deck if card.effect_id == "bewitch")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)],
        )
        effects = [row for row in final.active_effects if row.source_card_id == card.id]
        assert (minion.id not in final.entity_locations) == adjacent
        assert len(effects) == int(adjacent)
        if adjacent:
            modifier = effects[0]
            assert modifier.effect_type == EffectType.AREA_STAT_MODIFIER
            assert modifier.stat_type == StatType.RANGE and modifier.stat_value == -1
            assert modifier.apply_stat_value_only_if_result_at_least == 1
        yield f"bewitch-{'adjacent' if adjacent else 'no-target'}", state, operations



def cordelia_jinx_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType, StatType
    from goa2.engine.steps import ForceDiscardStep

    for mode in ("own-move", "own-decline", "enemy-discard"):
        state = setup(["Cordelia", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_cordelia")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == "jinx")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        victim = enemy if mode == "enemy-discard" else hero
        state.execution_context["fixture_jinx_victim"] = victim.id
        offered = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Jinx:"):
                offered.append(request.player_id)
                return "YES" if mode == "own-move" else "NO"
            if request.prompt == "Move exactly 1 space":
                assert h(1, 0).model_dump() in options
                return h(1, 0).model_dump()
            return options[0]

        operations, final = retain(state, [
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
            ForceDiscardStep(victim_key="fixture_jinx_victim"),
        ], select=choose)
        assert offered == ([] if mode == "enemy-discard" else [hero.id])
        assert final.entity_locations[hero.id] == h(int(mode == "own-move"), 0)
        assert any(row.effect_type == EffectType.AREA_STAT_MODIFIER
                   and row.stat_type == StatType.ATTACK and row.stat_value == -10
                   and row.granted_passive_effect_id == "jinx" for row in final.active_effects)
        yield f"jinx-{mode}", state, operations



def widget_pyro_swap_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, target, move in (
        ("take_off", "self", False), ("all_aboard", "ally", False),
        ("take_off", "absent", False), ("safe_landing", "ally", False),
        ("safe_landing", "ally", True),
    ):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_widget")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(ally.id, h(0, 1))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        swap = ally if target == "ally" else hero
        steps = []
        if target != "absent":
            state.execution_context["fixture_pyro_hex"] = h(1, 0)
            steps.append(PlaceTokenStep(token_type=TokenType.PYRO, hex_key="fixture_pyro_hex"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        selected = []

        def choose(request, _mode):
            if request.prompt == "Select Pyro to move":
                return selection_value(request.options[0])
            if request.prompt.startswith("Select Pyro destination"):
                assert effect == "safe_landing"
                return h(2, 0).model_dump() if move else "SKIP"
            assert request.prompt == "Select yourself or a friendly hero to swap with Pyro"
            options = [selection_value(option) for option in request.options]
            assert set(options) == {hero.id, ally.id}
            selected.append(swap.id)
            return swap.id

        operations, final = retain(state, steps, select=choose)
        assert selected == ([] if target == "absent" else [swap.id])
        if target != "absent":
            pyro = next(token for token in final.misc_entities.values()
                        if isinstance(token, Token) and token.token_type == TokenType.PYRO)
            assert final.entity_locations[pyro.id] == state.entity_locations[swap.id]
            assert final.entity_locations[swap.id] == h(2 if move else 1, 0)
        yield f"{effect}-{target}{'-move' if move else ''}", state, operations



def widget_dragon_bond_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for order in (0, 1, 2):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_widget")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == "dragon_bond")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        steps = []
        if order:
            state.execution_context["fixture_pyro_hex"] = h(1, 0)
            steps.append(PlaceTokenStep(token_type=TokenType.PYRO, hex_key="fixture_pyro_hex"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        moves = []

        def choose(request, _mode):
            if request.prompt == "Choose one":
                return 2 if order else 1
            if request.prompt == "Choose who moves first":
                return order
            if request.prompt == "Select a space in radius for Pyro":
                return h(1, 0).model_dump()
            if request.prompt.startswith("Select Widget destination"):
                moves.append("Widget")
                return h(-2, 0).model_dump()
            assert request.prompt.startswith("Select Pyro destination")
            moves.append("Pyro")
            return h(3, 0).model_dump()

        operations, final = retain(state, steps, select=choose)
        assert moves == ([], ["Widget", "Pyro"], ["Pyro", "Widget"])[order]
        pyro = next(token for token in final.misc_entities.values()
                    if isinstance(token, Token) and token.token_type == TokenType.PYRO)
        assert final.entity_locations[hero.id] == h(-2 if order else 0, 0)
        assert final.entity_locations[pyro.id] == h(3 if order else 1, 0)
        yield f"dragon-bond-{order}", state, operations



def widget_minion_bite_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, eligible in (("nibble", True), ("gnaw", True), ("gnaw", False)):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_widget")
        near, far = state.teams["BLUE"].minions[:2]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-1, 0))
        state.place_entity(near.id, h(1, 0) if eligible else h(0, -1))
        state.place_entity(far.id, h(3, 0))
        state.execution_context["fixture_pyro_hex"] = h(2, 0)
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        selected = []

        def choose(request, _mode):
            assert request.prompt == "Select an enemy minion adjacent to Pyro"
            options = [selection_value(option) for option in request.options]
            assert options == [near.id]
            selected.append(near.id)
            return near.id

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.PYRO, hex_key="fixture_pyro_hex"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert selected == ([near.id] if eligible else [])
        assert (near.id not in final.entity_locations) == eligible
        assert far.id in final.entity_locations
        assert (final.get_hero(hero.id).gold > hero.gold) == (eligible and effect == "gnaw")
        pyro = next(token for token in final.misc_entities.values()
                    if isinstance(token, Token) and token.token_type == TokenType.PYRO)
        assert (pyro.id not in final.entity_locations) == eligible
        yield f"{effect}-{'bite' if eligible else 'no-target'}", state, operations



def widget_breath_traces():
    from aura_traces import _board
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, empty in (
        ("fiery_breath", False), ("flaming_breath", True),
        ("scorching_breath", False), ("scorching_breath", True),
    ):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_widget")
        target = state.get_hero("hero_hanu")
        decoy = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-2, 0))
        state.place_entity(target.id, h(3, 0))
        state.place_entity(decoy.id, h(2, 1))
        state.execution_context["fixture_pyro_hex"] = h(1, 0)
        if empty:
            for held in list(target.hand):
                target.discard_card(held)
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        selected = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select an enemy hero"):
                assert options == [target.id]
                selected.append(target.id)
                return target.id
            assert request.player_id == target.id
            return options[0]

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.PYRO, hex_key="fixture_pyro_hex"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert selected == [target.id]
        assert (target.id not in final.entity_locations) == (empty and effect == "scorching_breath")
        assert len(final.get_hero(target.id).hand) == len(target.hand) - int(not empty)
        assert len(final.get_hero(decoy.id).hand) == len(decoy.hand)
        yield f"{effect}-{'empty' if empty else 'discard'}", state, operations



def widget_diversionary_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, distance, mode in (
        ("diversionary_strike", 2, "move"), ("diversionary_attack", 3, "move"),
        ("diversionary_assault", 4, "move"), ("diversionary_assault", 4, "skip"),
        ("diversionary_strike", 2, "no-target"),
    ):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_widget")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        if mode != "no-target":
            state.place_entity(minion.id, h(0, -1))
        start = h(-2, 1)
        end = h(-2 + distance, 1)
        state.execution_context["fixture_pyro_hex"] = start
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        offered = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select Pyro to move":
                return options[0]
            if request.prompt.startswith("Select Pyro destination"):
                assert request.prompt == f"Select Pyro destination (up to {distance})"
                assert end.model_dump() in options
                offered.append(distance)
                return end.model_dump() if mode == "move" else "SKIP"
            return options[0]

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.PYRO, hex_key="fixture_pyro_hex"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert offered == ([] if mode == "no-target" else [distance])
        assert minion.id not in final.entity_locations
        pyro = next(token for token in final.misc_entities.values()
                    if isinstance(token, Token) and token.token_type == TokenType.PYRO)
        assert final.entity_locations[pyro.id] == (end if mode == "move" else start)
        yield f"{effect}-{mode}", state, operations



def widget_airborne_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, before, after in (
        ("airborne_attack", False, False), ("airborne_attack", True, False),
        ("airborne_assault", True, True), ("airborne_assault", True, False),
    ):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_widget")
        left, right = state.teams["BLUE"].minions[:2]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(left.id, h(-1, 0))
        state.place_entity(right.id, h(3, 0))
        state.execution_context["fixture_pyro_hex"] = h(2, 0)
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        victim = right if before else left
        swaps = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select Pyro":
                return options[0]
            if request.prompt == "Select yourself or a friendly hero to swap with Pyro":
                assert options == [hero.id]
                accept = after if swaps else before
                swaps.append(accept)
                return hero.id if accept else "SKIP"
            assert options == [victim.id]
            return victim.id

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.PYRO, hex_key="fixture_pyro_hex"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert swaps == [before] + ([after] if effect == "airborne_assault" else [])
        assert victim.id not in final.entity_locations
        assert (left if before else right).id in final.entity_locations
        pyro = next(token for token in final.misc_entities.values()
                    if isinstance(token, Token) and token.token_type == TokenType.PYRO)
        moved = before != after
        assert final.entity_locations[hero.id] == h(2 if moved else 0, 0)
        assert final.entity_locations[pyro.id] == h(0 if moved else 2, 0)
        yield f"{effect}-{before}-{after}", state, operations



def widget_drag_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect in ("drag_off", "carry_away"):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_widget")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, -1))
        state.place_entity(enemy.id, h(0, 1))
        state.board.tiles[h(1, 0)].is_terrain = True
        state.board.tiles[h(3, 1)].is_terrain = True
        state.execution_context["fixture_pyro_hex"] = h(0, 0)
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        flies = effect == "carry_away"
        delta = 2 if flies else -2

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select an enemy unit adjacent to Pyro":
                assert options == [enemy.id]
                return enemy.id
            assert request.prompt.startswith("Select Pyro destination")
            assert (h(2, 0).model_dump() in options) == flies
            assert h(3, 0).model_dump() not in options
            assert h(delta, 0).model_dump() in options
            return h(delta, 0).model_dump()

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.PYRO, hex_key="fixture_pyro_hex"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        pyro = next(token for token in final.misc_entities.values()
                    if isinstance(token, Token) and token.token_type == TokenType.PYRO)
        assert final.entity_locations[pyro.id] == h(delta, 0)
        assert final.entity_locations[enemy.id] == h(delta, 1)
        yield effect, state, operations



def widget_fight_as_one_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType

    for mode in ("perform", "decline", "facedown"):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_widget")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(minion.id, h(1, 0))
        cards = {card.effect_id: card for card in hero.deck}
        current = cards["fight_as_one"]
        skill = cards["dragon_bond"]
        for card in (current, skill):
            if card in hero.hand:
                hero.hand.remove(card)
        hero.current_turn_card = current
        current.state, current.is_facedown = CardState.UNRESOLVED, False
        skill.state, skill.is_facedown = CardState.RESOLVED, mode == "facedown"
        hero.played_cards.append(skill)
        offered = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select a resolved skill card to perform":
                assert options == [skill.id]
                offered.append(skill.id)
                return skill.id if mode == "perform" else "SKIP"
            if request.prompt == "Choose one":
                return 1
            if request.prompt == "Select a space in radius for Pyro":
                return h(0, 1).model_dump()
            return options[0]

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=current.id)], select=choose,
        )
        assert offered == ([] if mode == "facedown" else [skill.id])
        assert minion.id not in final.entity_locations
        pyro = [token.id for token in final.misc_entities.values()
                if isinstance(token, Token) and token.token_type == TokenType.PYRO
                and token.id in final.entity_locations]
        assert bool(pyro) == (mode == "perform")
        yield f"fight-as-one-{mode}", state, operations



def widget_dragon_knight_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType

    for mode in ("perform", "decline", "locked"):
        state = setup(["Widget", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_widget")
        hero.level = 7 if mode == "locked" else 8
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        skill = next(card for card in hero.deck if card.effect_id == "dragon_bond")
        hero.hand.remove(skill)
        hero.played_cards.append(skill)
        skill.state, skill.is_facedown = CardState.RESOLVED, False
        current = next(card for card in hero.hand if card.effect_id == "fiery_breath")
        hero.hand.remove(current)
        hero.current_turn_card = current
        current.state, current.is_facedown = CardState.UNRESOLVED, False
        offered = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if "MOVEMENT" in options:
                return "MOVEMENT"
            if request.prompt.startswith("Select Movement Destination"):
                return h(1, 0).model_dump()
            if request.prompt.startswith("Dragon Knight:"):
                offered.append(request.player_id)
                return "YES" if mode == "perform" else "NO"
            if request.prompt == "Select a faceup skill card to perform":
                assert options == [skill.id]
                return skill.id
            if request.prompt == "Choose one":
                return 1
            assert request.prompt == "Select a space in radius for Pyro"
            return h(1, 1).model_dump()

        operations, final = retain(state, [ResolveCardStep(hero_id=hero.id)], select=choose)
        assert offered == ([] if mode == "locked" else [hero.id])
        assert final.entity_locations[hero.id] == h(1, 0)
        placed = [final.entity_locations[token.id] for token in final.misc_entities.values()
                  if isinstance(token, Token) and token.token_type == TokenType.PYRO
                  and token.id in final.entity_locations]
        assert placed == ([h(1, 1)] if mode == "perform" else [])
        yield f"dragon-knight-{mode}", state, operations


def mrak_ultimate_primary_trace():
    from aura_traces import _board

    state = setup(["Mrak", "Brogan"], ["Hanu", "Tigerclaw"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    hero = state.get_hero("hero_mrak")
    hero.level = 8
    state.current_actor_id = state.resolution_owner_id = hero.id
    state.place_entity(hero.id, h(0, 0))
    card = hero.ultimate_card
    hero.current_turn_card = card
    card.state, card.is_facedown = CardState.UNRESOLVED, False
    operations, final = retain(
        state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)],
    )
    assert final.execution_context["tail"] is True
    assert not final.active_effects
    yield "mrak-ultimate-primary", state, operations



def trinkets_barrier_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType, StatType, Token, TokenType

    for effect, count in (
        ("deployable_barrier", 2), ("deployable_bastion", 3),
        ("deployable_bastion", 1), ("deployable_barrier", 0),
    ):
        state = setup(["Trinkets", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_trinkets")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity("trinkets_turret", h(1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        placements = [h(1, 1), h(-1, 0), h(0, -1)]
        prompts = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            index = len(prompts)
            if index == 0:
                assert request.prompt.startswith("Select a space adjacent to the Turret")
                assert placements[1].model_dump() not in options
            else:
                assert request.prompt.startswith("Select a space in radius")
            prompts.append(request.prompt)
            if index == count:
                return "SKIP"
            assert placements[index].model_dump() in options
            return placements[index].model_dump()

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        cap = 2 if effect == "deployable_barrier" else 3
        assert len(prompts) == min(count + 1, cap)
        barriers = {
            token.id: final.entity_locations[token.id]
            for token in final.misc_entities.values()
            if isinstance(token, Token) and token.token_type == TokenType.BARRIER
            and token.id in final.entity_locations
        }
        assert set(barriers.values()) == set(placements[:count])
        auras = [row for row in final.active_effects
                 if row.effect_type == EffectType.AREA_STAT_MODIFIER and row.token_id in barriers]
        assert len(auras) == count
        assert {row.scope.origin_id for row in auras} == set(barriers)
        assert all(row.stat_type == StatType.DEFENSE and row.stat_value == 1 for row in auras)
        yield f"{effect}-{count}", state, operations



def trinkets_disruptor_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType
    from goa2.engine.effect_manager import EffectManager
    from goa2.engine.steps import ResolvePreActionDiscardStep

    for effect, mode in (
        ("disruptor_jolt", "discard"), ("disruptor_pulse", "empty"),
        ("disruptor_grid", "empty"), ("disruptor_grid", "outside"),
    ):
        state = setup(["Trinkets", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_trinkets")
        target = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-3, 0) if mode != "outside" else h(0, 0))
        state.place_entity("trinkets_turret", h(2, 0) if mode != "outside" else h(-4, 0))
        state.place_entity(target.id, h(3, 0) if mode != "outside" else h(1, 0))
        if mode == "empty":
            for held in list(target.hand):
                target.discard_card(held)
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        operations, active = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)],
        )
        yield f"{effect}-{mode}-create", state, operations

        active.get_hero(hero.id).resolve_current_card(turn_number=active.turn)
        EffectManager.activate_effects_by_card(active, card.id)
        active.current_actor_id = active.resolution_owner_id = target.id
        active.execution_context.clear()
        operations, final = retain(active, [
            ResolvePreActionDiscardStep(hero_id=target.id),
            ResolvePreActionDiscardStep(hero_id=target.id),
        ])
        assert len(final.get_hero(target.id).hand) == len(target.hand) - int(mode == "discard")
        assert (target.id not in final.entity_locations) == (
            effect == "disruptor_grid" and mode == "empty"
        )
        disruptors = [row for row in final.active_effects
                      if row.effect_type == EffectType.PRE_ACTION_DISCARD]
        assert len(disruptors) == int(mode != "discard")
        if disruptors:
            assert disruptors[0].scope.origin_id == "trinkets_turret"
            assert disruptors[0].is_active
        yield f"{effect}-{mode}-trigger", active, operations



def trinkets_self_destruct_traces():
    from aura_traces import _board
    from goa2.engine.steps import PlaceTurretStep

    for effect, choose_targets in (
        ("self_destruct", True), ("emergency_protocol", True),
        ("emergency_protocol", False),
    ):
        state = setup(["Trinkets", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_trinkets")
        full = state.get_hero("hero_hanu")
        empty = state.get_hero("hero_tigerclaw")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(-3, 0))
        state.execution_context["fixture_turret_hex"] = h(2, 0)
        state.place_entity(full.id, h(3, 0))
        state.place_entity(empty.id, h(2, 1))
        for held in list(empty.hand):
            empty.discard_card(held)
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        chosen = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Select up to 2 enemy"):
                if not choose_targets or len(chosen) == 2:
                    return "DONE"
                target = (full, empty)[len(chosen)]
                assert target.id in options
                chosen.append(target.id)
                return target.id
            assert request.player_id == full.id
            return options[0]

        operations, final = retain(
            state, [
                PlaceTurretStep(hex_key="fixture_turret_hex"),
                ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
            ], select=choose,
        )
        assert chosen == ([full.id, empty.id] if choose_targets else [])
        assert len(final.get_hero(full.id).hand) == len(full.hand) - int(choose_targets)
        assert (empty.id not in final.entity_locations) == (
            choose_targets and effect == "emergency_protocol"
        )
        assert "trinkets_turret" not in final.entity_locations
        yield f"{effect}-{'two-targets' if choose_targets else 'no-targets'}", state, operations



def trinkets_design_traces():
    from aura_traces import _board
    from goa2.engine.steps import PlaceTurretStep

    for effect, mode in (
        ("early_prototype", "swap"), ("updated_design", "swap"),
        ("perfected_design", "swap"), ("perfected_design", "place"),
        ("early_prototype", "absent"), ("updated_design", "outside"),
    ):
        state = setup(["Trinkets", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_trinkets")
        ally = state.get_hero("hero_brogan")
        state.current_actor_id = state.resolution_owner_id = hero.id
        origin = h(-4 if mode == "outside" else 0, 0)
        state.place_entity(hero.id, origin)
        state.place_entity(ally.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        steps = []
        if mode != "absent":
            state.execution_context["fixture_turret_hex"] = h(1, 0)
            steps.append(PlaceTurretStep(hex_key="fixture_turret_hex"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        destination = h(1, 1) if mode == "place" else h(2, 0)

        def choose(request, _mode):
            assert mode in ("swap", "place")
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Choose one":
                return 2 if mode == "place" else 1
            if mode == "place":
                assert destination.model_dump() in options
                return destination.model_dump()
            assert ally.id in options
            return ally.id

        operations, final = retain(state, steps, select=choose)
        assert final.entity_locations[hero.id] == (
            destination if mode in ("swap", "place") else origin
        )
        assert final.entity_locations[ally.id] == (origin if mode == "swap" else h(2, 0))
        assert ("trinkets_turret" in final.entity_locations) == (
            mode != "absent" and not (effect == "early_prototype" and mode == "swap")
        )
        yield f"{effect}-{mode}", state, operations



def trinkets_setup_traces():
    from aura_traces import _board
    from goa2.engine.steps import PlaceTurretStep

    for effect, choice in (
        ("salvage_parts", 1), ("salvage_parts", 2), ("salvage_parts", 3),
        ("rapid_redeployment", 1), ("rapid_redeployment", 2),
    ):
        state = setup(["Trinkets", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_trinkets")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(minion.id, h(-1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        discarded = hero.hand[0]
        hero.discard_card(discarded)
        salvage = effect == "salvage_parts"
        move = (salvage and choice == 2) or (not salvage and choice == 1)
        place = choice == 1
        steps = []
        if not place:
            state.execution_context["fixture_turret_hex"] = h(0, 1)
            steps.append(PlaceTurretStep(hex_key="fixture_turret_hex"))
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        movement_dest = h(2, 0)
        turret_dest = h(3 if move else 1, 0)

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Choose one":
                return choice
            if request.prompt.startswith("Select destination"):
                assert move and movement_dest.model_dump() in options
                return movement_dest.model_dump()
            if request.prompt == "Select an adjacent space for the Turret":
                assert place and turret_dest.model_dump() in options
                return turret_dest.model_dump()
            if request.prompt == "Select a discarded card to retrieve":
                assert salvage and choice == 3 and options == [discarded.id]
                return discarded.id
            assert not salvage and choice == 2 and options == [minion.id]
            return minion.id

        operations, final = retain(state, steps, select=choose)
        assert final.entity_locations[hero.id] == (movement_dest if move else h(0, 0))
        assert final.entity_locations.get("trinkets_turret") == (
            turret_dest if place else None if salvage else h(0, 1)
        )
        assert (minion.id in final.entity_locations) == (salvage or choice == 1)
        assert (discarded.id in {card.id for card in final.get_hero(hero.id).hand}) == (
            salvage and choice == 3
        )
        yield f"{effect}-{choice}", state, operations



def trinkets_turret_repeat_traces():
    from aura_traces import _board
    from goa2.engine.steps import PlaceTurretStep

    for effect, count in (
        ("steam_discharge", 2), ("flame_belcher", 3), ("flame_belcher", 1),
    ):
        state = setup(["Trinkets", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_trinkets")
        minions = state.teams["BLUE"].minions[:3]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        for minion, at in zip(minions, (h(1, 1), h(2, 0), h(2, -1)), strict=True):
            state.place_entity(minion.id, at)
        state.execution_context["fixture_turret_hex"] = h(1, 0)
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        chosen = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Repeat the attack"):
                return "YES" if len(chosen) < count else "NO"
            target = minions[len(chosen)]
            assert set(options) == {minion.id for minion in minions[len(chosen):]}
            chosen.append(target.id)
            return target.id

        operations, final = retain(state, [
            PlaceTurretStep(hex_key="fixture_turret_hex"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert chosen == [minion.id for minion in minions[:count]]
        assert final.execution_context["turret_attack_victims"] == chosen
        assert all((minion.id not in final.entity_locations) == (index < count)
                   for index, minion in enumerate(minions))
        yield f"{effect}-{count}", state, operations



def trinkets_cannon_traces():
    from aura_traces import _board
    from goa2.engine.steps import PlaceTurretStep

    for effect, bonus in (
        ("makeshift_minigun", 2), ("gatling_gun", 2), ("supercharged_cannon", 3),
    ):
        for aligned in (False, True):
            state = setup(["Trinkets", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            hero = state.get_hero("hero_trinkets")
            target = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(target.id, h(2, 0) if aligned else h(1, 1))
            state.execution_context["fixture_turret_hex"] = h(1, 0)
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt.startswith("Select a unit in range of both"):
                    assert options == [target.id]
                    return target.id
                return options[0]

            operations, final = retain(state, [
                PlaceTurretStep(hex_key="fixture_turret_hex"),
                ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
            ], select=choose)
            assert target.id not in final.entity_locations
            assert bool(final.execution_context["cannon_aligned"]) == aligned
            assert final.execution_context.get("cannon_bonus", 0) == (bonus if aligned else 0)
            yield f"{effect}-{'aligned' if aligned else 'off-axis'}", state, operations



def nebkher_doubt_traces():
    from aura_traces import _board
    from goa2.domain.models import CardColor, EffectType
    from goa2.domain.models.effect import DurationType

    for effect, color in (
        ("imbue_doubt", CardColor.RED), ("time_to_reconsider", CardColor.BLUE),
        ("an_illusion_of_choice", CardColor.BLUE),
    ):
        state = setup(["NebKher", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_nebkher")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False

        def choose(request, _mode):
            assert request.prompt.endswith(": Name a card color")
            return color.value

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        trigger = next(row for row in final.active_effects
                       if row.effect_type == EffectType.AFTER_CARDS_PLAYED_TRIGGER)
        assert trigger.duration == DurationType.NEXT_TURN
        assert trigger.named_color == color and trigger.source_card_id == card.id
        double = effect == "an_illusion_of_choice"
        assert [step.type.value for step in trigger.finishing_steps] == [
            "select_step", "force_discard_by_color",
        ] * (2 if double else 1)
        yield effect, state, operations


        # Resume the same resolved effect at the following turn's reveal.
        final.get_hero(hero.id).resolve_current_card(turn_number=final.turn)
        final.turn += 1
        final.phase = GamePhase.PLANNING
        final.current_actor_id = final.resolution_owner_id = None
        final.execution_context.clear()
        final.execution_stack.clear()
        target = final.get_hero("hero_hanu")
        final.place_entity(target.id, h(1, 0))
        matching = next(card for card in target.hand if card.color == color)
        committed = next(card for card in target.hand if card.color != color)
        target.play_card(committed)
        final.pending_inputs[target.id] = committed
        operations = [
            {"kind": "start_revelation"},
            {"kind": "advance"},
            {"kind": "input", "selection": target.id},
            {"kind": "advance"},
            {"kind": "input", "selection": matching.id},
            {"kind": "advance"},
        ]
        if double:
            other = final.get_hero("hero_tigerclaw")
            final.place_entity(other.id, h(0, 1))
            other_matching = next(card for card in other.hand if card.color == color)
            operations.extend([
                {"kind": "input", "selection": other.id},
                {"kind": "advance"},
                {"kind": "input", "selection": other_matching.id},
                {"kind": "advance"},
            ])
        probe = copy.deepcopy(final)
        assert all(row["error"] is None for row in reference_trace(probe, operations))
        assert matching.id in {card.id for card in probe.get_hero(target.id).discard_pile}
        if double:
            assert other_matching.id in {card.id for card in probe.get_hero(other.id).discard_pile}
        yield f"{effect}-next-turn", final, operations

        stale = copy.deepcopy(final)
        stale.round += 1
        stale_ops = [{"kind": "start_revelation"}]
        probe = copy.deepcopy(stale)
        assert all(row["error"] is None for row in reference_trace(probe, stale_ops))
        assert not any(row.effect_type == EffectType.AFTER_CARDS_PLAYED_TRIGGER
                       for row in probe.active_effects)
        assert not any(step.type.value == "force_discard_by_color" for step in probe.execution_stack)
        yield f"{effect}-cross-round", stale, stale_ops


def nebkher_illusion_supply_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType, Token, TokenType
    from goa2.domain.models.effect import DurationType

    for effect, cap, count in (
        ("illusionary_force", 2, 2), ("illusionary_army", 3, 3),
        ("illusionary_army", 3, 1), ("illusionary_force", 2, 0),
    ):
        state = setup(["NebKher", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_nebkher")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        placements = [h(1, 0), h(0, 1), h(-1, 0)]
        prompts = []

        def choose(request, _mode):
            index = len(prompts)
            assert request.prompt == f"Place Illusion token {index + 1}/{cap} in radius"
            prompts.append(request.prompt)
            return placements[index].model_dump() if index < count else "SKIP"

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert len(prompts) == min(count + 1, cap)
        illusions = {
            final.entity_locations[token.id] for token in final.misc_entities.values()
            if isinstance(token, Token) and token.token_type == TokenType.ILLUSION
            and token.id in final.entity_locations
        }
        assert illusions == set(placements[:count])
        aura = next(row for row in final.active_effects
                    if row.effect_type == EffectType.ILLUSION_MINION_EQUIVALENCE)
        assert aura.source_card_id == card.id and aura.duration == DurationType.THIS_ROUND
        yield f"{effect}-{count}", state, operations


def nebkher_swap_attack_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, swap in (("twist_fate", True), ("devious_scheme", True), ("devious_scheme", False)):
        state = setup(["NebKher", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_nebkher")
        enemy = state.get_hero("hero_hanu")
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(1, 1))
        state.place_entity(minion.id, h(-1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        near, far = h(1, 0), h(2, 0)
        state.execution_context.update(fixture_near=near, fixture_far=far)
        advanced = effect == "devious_scheme"

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select an enemy unit in range to swap":
                assert options == [enemy.id]
                return enemy.id if swap else "SKIP"
            if request.prompt == "Select an Illusion token to swap with":
                assert len(options) == (2 if advanced else 1)
                return options[-1]
            assert minion.id in options
            return minion.id

        operations, final = retain(state, [
            PlaceTokenStep(token_type=TokenType.ILLUSION, hex_key="fixture_near"),
            PlaceTokenStep(token_type=TokenType.ILLUSION, hex_key="fixture_far"),
            ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
        ], select=choose)
        assert minion.id not in final.entity_locations
        assert final.entity_locations[enemy.id] == ((far if advanced else near) if swap else h(1, 1))
        if swap:
            token = final.get_entity(final.board.tiles[h(1, 1)].occupant_id)
            assert isinstance(token, Token) and token.token_type == TokenType.ILLUSION
        yield f"{effect}-{'swap' if swap else 'decline'}", state, operations


def nebkher_projection_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, cap, place in (
        ("fleeting_image", 1, True), ("multiple_projections", 2, True),
        ("master_of_illusions", 3, True), ("master_of_illusions", 3, False),
    ):
        state = setup(["NebKher", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_nebkher")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.place_entity(hero.id, h(0, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        state.execution_context["fixture_existing_illusion"] = h(4, 0)
        placements = [h(1, 0), h(0, 1), h(-1, 0)]
        prompts = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Place Illusion token"):
                index = len(prompts)
                assert request.prompt == f"Place Illusion token {index + 1}/{cap} in radius"
                prompts.append(request.prompt)
                return placements[index].model_dump() if place else "SKIP"
            assert request.prompt == "Select an Illusion token in play to swap with"
            assert len(options) == (cap if place else 1)
            return options[0]

        steps = [] if place else [
            PlaceTokenStep(token_type=TokenType.ILLUSION, hex_key="fixture_existing_illusion"),
        ]
        steps.append(ResolveCardTextStep(hero_id=hero.id, card_id=card.id))
        operations, final = retain(state, steps, select=choose)
        assert len(prompts) == (cap if place else 1)
        assert final.entity_locations[hero.id] == (placements[0] if place else h(4, 0))
        illusions = {
            final.entity_locations[token.id] for token in final.misc_entities.values()
            if isinstance(token, Token) and token.token_type == TokenType.ILLUSION
            and token.id in final.entity_locations
        }
        assert illusions == {h(0, 0), *(placements[1:cap] if place else [])}
        yield f"{effect}-{'place' if place else 'skip-placement'}", state, operations


def nebkher_phantasmal_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType
    from goa2.engine.steps import PlaceTokenStep

    for effect, distance in (
        ("phantasmal_sentry", 0), ("phantasmal_warrior", 1),
        ("phantasmal_champion", 2),
    ):
        for adjacent in (False, True):
            state = setup(["NebKher", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.battle_zones = {}
            hero = state.get_hero("hero_nebkher")
            enemy = state.get_hero("hero_hanu")
            minion = state.teams["BLUE"].minions[0]
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.place_entity(hero.id, h(0, 0))
            state.place_entity(enemy.id, h(2, 0))
            state.place_entity(minion.id, h(-1, 0))
            for held in list(enemy.hand):
                enemy.discard_card(held)
            destination = h(1, 0)
            start = (destination, h(0, 1), h(-1, 1))[distance]
            state.execution_context["fixture_illusion"] = start
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            victim = minion if adjacent else enemy

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if "choose one" in request.prompt:
                    return 2 if adjacent else 1
                if request.prompt.startswith("Select destination hex"):
                    assert destination.model_dump() in options
                    return destination.model_dump()
                if request.prompt.startswith("Select an Illusion"):
                    assert len(options) == 1
                    return options[0]
                if not request.prompt.startswith("Select"):
                    return options[0]
                assert options == [victim.id], (request.prompt, options)
                return victim.id

            operations, final = retain(state, [
                PlaceTokenStep(token_type=TokenType.ILLUSION, hex_key="fixture_illusion"),
                ResolveCardTextStep(hero_id=hero.id, card_id=card.id),
            ], select=choose)
            assert victim.id not in final.entity_locations
            illusion = next(token for token in final.misc_entities.values()
                            if isinstance(token, Token) and token.token_type == TokenType.ILLUSION
                            and token.id in final.entity_locations)
            assert final.entity_locations[illusion.id] == (start if adjacent else destination)
            yield f"{effect}-{'adjacent' if adjacent else 'illusion'}", state, operations


def nebkher_topology_traces():
    from aura_traces import _board
    from goa2.domain.models import EffectType
    from goa2.scripts.nebkher_effects import _AXES

    for effect, kind in (
        ("crack_in_reality", EffectType.TOPOLOGY_SPLIT),
        ("shift_reality", EffectType.TOPOLOGY_ISOLATION),
    ):
        for choice, axis in [*_AXES, (None, None)]:
            state = setup(["NebKher", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_nebkher")
            state.current_actor_id = state.resolution_owner_id = hero.id
            anchor = h(1, 2)
            if choice is not None:
                state.place_entity(hero.id, anchor)
            card = next(card for card in hero.deck if card.effect_id == effect)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False

            def choose(request, _mode):
                assert request.prompt == "Choose the line of spaces drawn through your space"
                assert choice in [selection_value(option) for option in request.options]
                return choice

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert len(final.active_effects) == int(choice is not None)
            if choice is not None:
                topology = final.active_effects[0]
                assert topology.effect_type == kind and topology.split_axis == axis
                assert topology.split_value == getattr(anchor, axis)
                assert topology.isolated_hex == anchor
            yield f"{effect}-{axis or 'off-board'}", state, operations


def nebkher_laughter_traces():
    from aura_traces import _board
    from goa2.domain.models import Token, TokenType

    for mode in ("decline", "laugh", "ultimate", "swap-cards"):
        state = setup(["NebKher", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_nebkher")
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        hero.level = 8 if mode == "ultimate" else 7
        state.place_entity(hero.id, h(0, 0))
        state.place_entity(enemy.id, h(2, 0))
        card = next(card for card in hero.deck if card.effect_id == "diabolical_laughter")
        hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        prompts = []
        swapped = []
        if mode == "swap-cards":
            for played in list(enemy.hand[:2]):
                enemy.hand.remove(played)
                enemy.played_cards.append(played)
                played.state, played.is_facedown = CardState.RESOLVED, False
                swapped.append(played.id)


        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Laugh diabolically?":
                return "NO" if mode == "decline" else "YES"
            if request.player_id == enemy.id:
                assert not prompts
                return options[0]
            if request.prompt.startswith("Diabolical Laughter"):
                prompts.append(request.prompt)
                return (3 if swapped else 2) if len(prompts) == 1 else "SKIP"
            if request.prompt.startswith("Select an enemy hero"):
                assert options == [enemy.id]
                return enemy.id
            if request.prompt.startswith("Select the first resolved"):
                assert set(options) == set(swapped)
                return swapped[0]
            if request.prompt.startswith("Select the second resolved"):
                assert options == [swapped[1]]
                return swapped[1]
            assert request.prompt == "Select an adjacent space for the Illusion token"
            return h(1, 0).model_dump()

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert len(prompts) == (0 if mode == "decline" else 2)
        assert len(final.get_hero(enemy.id).hand) == len(enemy.hand) - int(mode == "ultimate")
        illusions = [final.entity_locations[token.id] for token in final.misc_entities.values()
                     if isinstance(token, Token) and token.token_type == TokenType.ILLUSION
                     and token.id in final.entity_locations]
        assert illusions == ([h(1, 0)] if mode in ("laugh", "ultimate") else [])
        if swapped:
            assert [card.id for card in final.get_hero(enemy.id).played_cards] == swapped[::-1]
        yield f"diabolical-laughter-{mode}", state, operations


def razzle_swap_traces():
    from aura_traces import _board, _pieces

    for effect, index, repeat in (
        ("theatrics", 0, False), ("theatrics", 1, False),
        ("spectacle", 0, False), ("spectacle", 1, True),
    ):
        state = setup(["Razzle", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_razzle")
        pieces = sorted(_pieces(state, hero.id))
        actor, other = pieces[index], pieces[1 - index]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.acting_piece_id = actor
        state.place_entity(actor, h(0, 0))
        state.place_entity(other, h(-2, 0))
        first, second = state.teams["BLUE"].minions[:2]
        state.place_entity(first.id, h(1, 0))
        state.place_entity(second.id, h(-2, 1))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select a minion in range to swap with":
                assert first.id in options
                return first.id
            if request.prompt == "Select another one of you to repeat the swap":
                assert options == [other]
                return other if repeat else "SKIP"
            assert request.prompt == "Select a different minion in range"
            assert options == [second.id]
            return second.id

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert final.entity_locations[actor] == h(1, 0)
        assert final.entity_locations[first.id] == h(0, 0)
        assert final.entity_locations[other] == (h(-2, 1) if repeat else h(-2, 0))
        assert final.entity_locations[second.id] == (h(-2, 0) if repeat else h(-2, 1))
        yield f"{effect}-{index}-{repeat}", state, operations


def razzle_wire_traces():
    from aura_traces import _board, _pieces

    for effect, distance, swap in (
        ("tightrope", 1, False), ("high_wire", 2, False), ("wire_dancers", 3, False),
        ("alleyoop", 1, True), ("group_performance", 2, True), ("team_spirit", 3, True),
    ):
        for follow in (False, True):
            state = setup(["Razzle", "Brogan"], ["Hanu", "Tigerclaw"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            hero = state.get_hero("hero_razzle")
            actor, other, *_ = sorted(_pieces(state, hero.id))
            state.current_actor_id = state.resolution_owner_id = hero.id
            state.acting_piece_id = actor
            state.place_entity(actor, h(0, 0))
            state.place_entity(other, h(-1, 1))
            ally = state.get_hero("hero_brogan")
            if swap:
                state.place_entity(ally.id, h(1, 0))
            card = next(card for card in hero.deck if card.effect_id == effect)
            if card in hero.hand:
                hero.hand.remove(card)
            hero.current_turn_card = card
            card.state, card.is_facedown = CardState.UNRESOLVED, False
            destination = h(-1 + distance, 1)

            def choose(request, _mode):
                options = [selection_value(option) for option in request.options]
                if request.prompt == "Select a friendly hero in range to swap with":
                    assert swap and ally.id in options
                    return ally.id
                if request.prompt.startswith("Select Movement Destination"):
                    assert not swap
                    return h(1, 0).model_dump()
                if request.prompt == "Select another one of you to move":
                    assert options == [other]
                    return other if follow else "SKIP"
                assert request.prompt == f"Select where to move that piece (up to {distance})"
                assert destination.model_dump() in options
                return destination.model_dump()

            operations, final = retain(
                state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
            )
            assert final.entity_locations[actor] == h(1, 0)
            if swap:
                assert final.entity_locations[ally.id] == h(0, 0)
            assert final.entity_locations[other] == (destination if follow else h(-1, 1))
            assert final.acting_piece_id == actor
            yield f"{effect}-{'follow' if follow else 'skip'}", state, operations


def razzle_retrieval_traces():
    from aura_traces import _board, _pieces

    for effect, nearby in (("rummage", 0), ("rummage", 2), ("ransack", 2), ("ransack", 3)):
        state = setup(["Razzle", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_razzle")
        actor, *others = sorted(_pieces(state, hero.id))
        minion = state.teams["BLUE"].minions[0]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.acting_piece_id = actor
        state.place_entity(actor, h(0, 0))
        state.place_entity(minion.id, h(1, 0))
        for piece, at in zip(others[:nearby], (h(-1, 0), h(0, 1), h(0, -1))):
            state.place_entity(piece, at)
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        discarded = list(hero.hand[:3])
        for held in discarded:
            hero.discard_card(held)
        selected = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt == "Select a discarded card to retrieve":
                chosen = discarded[len(selected)].id
                assert chosen in options and all(prior not in options for prior in selected)
                selected.append(chosen)
                return chosen
            assert options == [minion.id]
            return minion.id

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        count = min(nearby, 1) if effect == "rummage" else nearby
        assert selected == [held.id for held in discarded[:count]]
        assert final.execution_context[f"{effect}_piece_count"] == nearby
        assert minion.id not in final.entity_locations
        yield f"{effect}-{nearby}", state, operations


def razzle_stunt_traces():
    from aura_traces import _board, _pieces

    for mode in ("base", "repeat", "skip-spawn"):
        state = setup(["Razzle", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_razzle")
        hero.level = 8 if mode == "repeat" else 7
        actor, other, third, fourth = sorted(_pieces(state, hero.id))
        first, second = state.teams["BLUE"].minions[:2]
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.acting_piece_id = actor
        for unit, at in ((actor, h(0, 0)), (other, h(-2, 0)),
                         (first.id, h(1, 0)), (second.id, h(-2, 1))):
            state.place_entity(unit, at)
        card = next(card for card in hero.deck if card.effect_id == "stunt_doubles")
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        spawned = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Spawn another Razzle"):
                if mode == "skip-spawn" or (spawned and len(spawned) == 1 and state.acting_piece_id == actor
                                            and h(-3, 0).model_dump() not in options):
                    return "SKIP"
                destination = h(0, 1) if not spawned else h(-3, 0)
                assert destination.model_dump() in options
                spawned.append(destination)
                return destination.model_dump()
            if request.prompt == "Select another one of you to repeat Stunt Doubles":
                assert options == [other]
                return other
            return first.id if first.id in options else second.id

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert first.id not in final.entity_locations
        assert (second.id not in final.entity_locations) == (mode == "repeat")
        assert final.entity_locations.get(third) == (None if mode == "skip-spawn" else h(0, 1))
        assert final.entity_locations.get(fourth) == (h(-3, 0) if mode == "repeat" else None)
        yield f"stunt-doubles-{mode}", state, operations


def razzle_removal_traces():
    from aura_traces import _board, _pieces

    for effect, count in (
        ("phantom_strike", 1), ("phantom_strike", 0),
        ("hit_and_gone", 2), ("into_thin_air", 3), ("crowd_control", 2),
    ):
        state = setup(["Razzle", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.battle_zones = {}
        hero = state.get_hero("hero_razzle")
        actor, second, third, *_ = sorted(_pieces(state, hero.id))
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.acting_piece_id = actor
        for piece, at in ((actor, h(0, 0)), (second, h(-1, 0)), (third, h(0, -1))):
            state.place_entity(piece, at)
        minion = state.teams["BLUE"].minions[0]
        state.place_entity(minion.id, h(1, 0))
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        targets = [second, third, actor]
        removed = []

        def choose(request, _mode):
            options = [selection_value(option) for option in request.options]
            if request.prompt.startswith("Remove one of you?"):
                assert "SKIP" in options and all(piece not in options for piece in removed)
                if len(removed) == count:
                    return "SKIP"
                piece = targets[len(removed)]
                assert piece in options
                removed.append(piece)
                return piece
            assert options == [minion.id]
            return minion.id

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert all((piece not in final.entity_locations) == (piece in targets[:count])
                   for piece in targets)
        assert final.acting_piece_id == (None if count == 3 else actor)
        assert (minion.id in final.entity_locations) == (effect == "crowd_control")
        yield f"{effect}-{count}", state, operations


def razzle_mirror_traces():
    from aura_traces import _board, _pieces

    for effect, intent, actual, wall in (
        ("magic_trick", 2, 2, None), ("aaaand_its_gone", 3, 3, None),
        ("aaaand_its_gone", 3, 1, 3), ("magic_trick", 1, 1, -2),
        ("magic_trick", 0, 0, None),
    ):
        state = setup(["Razzle", "Brogan"], ["Hanu", "Tigerclaw"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_razzle")
        actor = sorted(_pieces(state, hero.id))[0]
        enemy = state.get_hero("hero_hanu")
        state.current_actor_id = state.resolution_owner_id = hero.id
        state.acting_piece_id = actor
        state.place_entity(actor, h(0, 0))
        state.place_entity(enemy.id, h(1, 0))
        if wall is not None:
            state.board.tiles[h(wall, 0)].is_terrain = True
        card = next(card for card in hero.deck if card.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        hero.current_turn_card = card
        card.state, card.is_facedown = CardState.UNRESOLVED, False
        distances = []

        def choose(request, _mode):
            if request.prompt == "Select an adjacent unit to push":
                assert [selection_value(option) for option in request.options] == [enemy.id]
                return enemy.id
            assert request.prompt == "Choose push distance"
            previews = {int(option.id): option.metadata["actual_moved"] for option in request.options}
            assert previews[intent] == actual
            if wall == -2:
                assert 2 not in previews
            distances.append(previews[intent])
            return str(intent)

        operations, final = retain(
            state, [ResolveCardTextStep(hero_id=hero.id, card_id=card.id)], select=choose,
        )
        assert distances == [actual]
        assert final.entity_locations[enemy.id] == h(1 + actual, 0)
        assert final.entity_locations[actor] == h(-actual, 0)
        yield f"{effect}-{intent}-{wall}", state, operations


def verify_trace(worker, name, state, envelope, operations):
    records = worker.records(envelope, operations)
    normalizer = ObjectTraceNormalizer()
    expected = iter_reference(state, operations, inplace_records=True)
    for index, (record, oracle) in enumerate(zip(records, expected, strict=True)):
        actual = normalizer.record(record, inplace=True)
        if actual != oracle:
            fields = [key for key in oracle if actual.get(key) != oracle[key]]
            raise AssertionError(f"{name} operation {index}: different trace fields {fields}")


def python_move(move, envelope):
    fields = dict(move, kind=move["kind"].upper())
    if fields["kind"] == "CARD":
        instance = envelope["instances"][fields.pop("card")]
        fields["selection"] = envelope["definitions"][instance["definition"]]["fields"]["id"]
    return DecisionMove(**fields)


def verify_live_resolution(worker, root):
    from resolution_validation import validate_resolution

    state = copy.deepcopy(root)
    prompt = process_stack(state).input_request
    envelope, _ = export_state_with_shared_objects(state)
    result = worker.request(
        "search_resolution", timeout=12, envelope=envelope,
        player_id="hero_arien", time_ms=10_000, max_depth=1, max_nodes=16,
    )
    options = dict(
        player_id="hero_arien", controlled_ids={"hero_arien"},
        searched_revision=1, live_revision=1,
    )
    move = validate_resolution(state, prompt, result, **options)
    assert move.request_id == prompt.id

    cases = [
        (result, {"live_revision": 2}, "stale resolution result"),
        (result, {"controlled_ids": set()}, "resolution player is not an eligible bot"),
        (dict(result, decision=dict(result["decision"], request_id="stale")), {},
         "resolution result does not match the live prompt"),
        (dict(result, decision=dict(result["decision"], selection=99)), {},
         "Python rejected the resolution move"),
    ]
    for response, changes, expected in cases:
        try:
            validate_resolution(state, prompt, response, **(options | changes))
        except ValueError as error:
            assert str(error) == expected
        else:
            raise AssertionError(f"expected rejection: {expected}")


def verify_consecutive_allied_choices(worker):
    from goa2.domain.input import InputResponse
    from goa2.engine.filters_units import TeamFilter, UnitTypeFilter
    from goa2.engine.handler import submit_input
    from goa2.engine.steps import DefeatUnitStep

    state = setup()
    state.phase = GamePhase.RESOLUTION
    player, ally = "hero_arien", "hero_hanu"
    state.current_actor_id = player
    state.execution_stack = [
        DefeatUnitStep(step_id="fixture/sequence/allied-defeat", victim_key="victim", killer_id=player),
        SelectStep(
            step_id="fixture/sequence/allied-target", target_type=TargetType.UNIT,
            output_key="victim", prompt="Choose an enemy minion", override_player_id=ally,
            filters=[TeamFilter(relation="ENEMY"), UnitTypeFilter(unit_type="MINION")],
            is_mandatory=False,
        ),
        SelectStep(
            step_id="fixture/sequence/allied-start", target_type=TargetType.NUMBER,
            output_key="number", prompt="Choose a number", number_options=[1, 2],
        ),
    ]
    first = process_stack(state).input_request
    assert first.player_id == player
    envelope, _ = export_state_with_shared_objects(state)
    options = dict(envelope=envelope, player_id=player, time_ms=10_000, max_nodes=512)
    shallow = worker.request("search_resolution", max_depth=1, **options)
    deep = worker.request("search_resolution", max_depth=2, **options)
    assert shallow["completed_depth"] == 1 and deep["completed_depth"] == 2
    assert deep["score"] > shallow["score"], (shallow, deep)

    submit_input(state, InputResponse(request_id=first.id, selection=deep["decision"]["selection"]))
    second = process_stack(state).input_request
    assert second.player_id == ally and second.can_skip and second.options
    gold = state.get_hero(player).gold
    victim = selection_value(second.options[0])
    submit_input(state, InputResponse(request_id=second.id, selection=victim))
    process_stack(state)
    assert state.get_position(victim) is None and state.get_hero(player).gold > gold
    print("Matched consecutive allied choices with a strictly better defeat payoff")


def verify(command, *, verdicts_only=False):
    verify_sources()
    worker = Worker(command)
    trace_worker = None
    trace_complete = False
    count = 0
    batch_counts = Counter()
    try:
        trace_worker = StreamWorker(command)
        for name, state, sequences in scenes():
            envelope, _ = export_state_with_shared_objects(state)
            reference = check_sequences(
                state,
                [[python_move(move, envelope) for move in sequence]
                 for sequence in sequences],
            )
            native = worker.request("check_sequences", envelope=envelope, sequences=sequences)
            expected = [(row.legal, row.checked, row.failed_at) for row in reference.results]
            actual = [(row["legal"], row["checked"], row["failed_at"]) for row in native["results"]]
            if actual != expected:
                raise AssertionError(f"{name}: expected {expected}, got {actual}")
            # Single-move rows exercise native candidate batches without
            # applying one candidate before evaluating the next.
            candidates = [sequence[0] for sequence in sequences if sequence]
            if candidates:
                batch = check_moves(state, [python_move(move, envelope) for move in candidates])
                native_batch = worker.request(
                    "check_sequences", envelope=envelope,
                    sequences=[[move] for move in candidates],
                )
                expected_batch = [row.legal for row in batch.results]
                actual_batch = [row["legal"] for row in native_batch["results"]]
                assert actual_batch == expected_batch, (name, actual_batch, expected_batch)
                batch_counts.update(expected_batch)
            if name == "movement-updates-reachability":
                assert actual == [(False, 0, 0), (True, 2, None), (False, 0, 0), (True, 1, None)]
                repeated = worker.request(
                    "check_sequences", envelope=envelope, sequences=sequences * 16,
                )
                assert repeated["results"] == native["results"] * 16
                assert repeated["applied_decisions"] == native["applied_decisions"] == 1
            if name == "two-number-requests":
                verify_live_resolution(worker, state)
            if name == "rune-inscription":
                operations = [
                    {"kind": "advance"},
                    {"kind": "input", "selection": "axe"},
                    {"kind": "advance"},
                    {"kind": "input", "selection": ["bird", "anvil"]},
                    {"kind": "advance"},
                ]
                searched = worker.request(
                    "search_resolution", envelope=envelope, player_id="hero_snorri",
                    max_depth=3, max_nodes=4, time_ms=10_000,
                )
                assert searched["completed_depth"] == 1 and searched["nodes"] == 4
                assert searched["score"] is not None
                partial = worker.request(
                    "search_resolution", envelope=envelope, player_id="hero_snorri",
                    max_depth=3, max_nodes=1, time_ms=10_000,
                )
                assert partial["completed_depth"] == 0 and partial["nodes"] == 1
                assert partial["score"] is not None
                assert partial["decision"] == searched["decision"]
                decision = searched["decision"]
                replay = worker.request("check_sequences", envelope=envelope, sequences=[[decision]])
                assert replay["results"][0]["legal"] is True
                # The two engines use independent prompt tokens.
                oracle_move = python_move(dict(decision, request_id=None), envelope)
                assert check_sequences(state, [[oracle_move]]).results[0].legal is True
                verify_trace(trace_worker, name, state, envelope, operations)

            count += len(expected)
        verify_consecutive_allied_choices(worker)
        print(f"Matched {sum(batch_counts.values())} batch candidate verdicts: {dict(batch_counts)}")
        if verdicts_only:
            trace_complete = True
            return count
        for name, state, operations in inscription_card_traces():
            envelope, _ = export_state_with_shared_objects(state)
            verify_trace(trace_worker, name, state, envelope, operations)
            print(f"Matched {len(operations)} operations for {name}")
        passage_operations = 0
        for name, state, operations in passage_card_traces():
            envelope, _ = export_state_with_shared_objects(state)
            verify_trace(trace_worker, name, state, envelope, operations)
            passage_operations += len(operations)
        print(f"Matched {passage_operations} Passage operations across 24 rune configurations")
        melee_operations = 0
        for name, state, operations in runic_melee_traces():
            envelope, _ = export_state_with_shared_objects(state)
            verify_trace(trace_worker, name, state, envelope, operations)
            melee_operations += len(operations)
        print(f"Matched {melee_operations} Runic melee operations across 24 rune configurations")
        ranged_operations = 0
        for name, state, operations in runic_ranged_traces():
            envelope, _ = export_state_with_shared_objects(state)
            verify_trace(trace_worker, name, state, envelope, operations)
            ranged_operations += len(operations)
        print(f"Matched {ranged_operations} ranged and discard operations across 28 rune configurations")
        for name, state, operations in chain(
            ancestral_boon_traces(), ancestral_grace_traces(), rune_sigils_traces(),
            snorri_integration_traces(), whisper_shadow_traces(), whisper_fear_traces(),
            whisper_trail_traces(), whisper_swap_traces(), whisper_repeat_attack_traces(),
            whisper_victim_choice_traces(), whisper_death_seeker_traces(),
            whisper_forced_movement_traces(), whisper_swift_justice_traces(), brynn_swap_traces(),
            brynn_obstacle_attack_traces(), brynn_followup_traces(), brynn_displacement_traces(),
            brynn_ultimate_traces(), garrus_charge_traces(), garrus_push_traces(),
            garrus_retrieval_traces(), garrus_pilum_traces(), garrus_untiered_traces(),
            garrus_displacement_traces(), silverarrow_snipe_traces(), silverarrow_followup_traces(),
            silverarrow_gift_traces(), silverarrow_disorient_traces(), silverarrow_drag_traces(),
            silverarrow_travel_traces(), silverarrow_delayed_traces(), silverarrow_wild_hunt_traces(),
            silverarrow_root_enforcement_traces(), takahide_support_traces(), takahide_color_traces(),
            takahide_swap_traces(), takahide_blade_traces(), takahide_deck_swap_traces(),
            takahide_gold_cycle_traces(), takahide_unlock_traces(),
            emmitt_temporal_attack_traces(), emmitt_turn_effect_traces(), emmitt_reverse_order_traces(),
            emmitt_time_warp_traces(), emmitt_stationary_target_traces(), emmitt_shove_traces(), emmitt_recall_traces(),
            emmitt_resolved_discard_traces(), emmitt_retrieval_traces(), emmitt_glitch_traces(), emmitt_glitch_defense_traces(),
            emmitt_timeline_reveal_traces(), mortimer_zombie_traces(), mortimer_horde_traces(), mortimer_dead_traces(), mortimer_knife_traces(), mortimer_brains_traces(), mortimer_slam_traces(), bain_retrieval_traces(), bain_guess_traces(), bain_bounty_movement_traces(), swift_shot_traces(), swift_jump_suppress_traces(), swift_special_traces(), swift_bounce_traces(), swift_reload_traces(), swift_bullet_time_traces(), swift_bullet_time_exclusion_trace(), rowenna_surrender_opening_traces(), rowenna_swap_traces(), rowenna_lance_traces(), rowenna_melee_traces(), rowenna_basic_traces(), mrak_terrain_traces(), mrak_rock_placement_traces(), mrak_shield_traces(), mrak_boulder_traces(), mrak_rolling_traces(), dodger_debuff_traces(), dodger_skull_traces(), dodger_respawn_traces(), dodger_ritual_traces(), dodger_razor_traces(), dodger_death_trap_traces(), dodger_finger_traces(), wuk_throw_traces(), wuk_canopy_traces(), wuk_dominance_traces(), wuk_retrieval_traces(), wuk_nature_attack_traces(), wuk_sapling_slam_traces(), wuk_trample_traces(), tali_token_traces(), tali_winter_weapon_traces(), tali_spirit_retrieval_traces(), tali_spirit_attack_traces(), tali_ice_blast_traces(), tali_commune_traces(), tali_directional_traces(), tali_blizzard_traces(),
            ursafar_rage_traces(), ursafar_rage_movement_traces(), ursafar_prey_traces(),
            ursafar_swap_traces(), ursafar_claws_traces(), ursafar_tear_traces(),
            ursafar_discard_action_traces(), ursafar_angry_roar_traces(), ignatia_coin_traces(),
            ignatia_range_traces(), ignatia_bolt_and_swap_traces(), ignatia_combustion_traces(),
            ignatia_straight_movement_traces(), ignatia_path_traces(), ignatia_ultimate_traces(),
            cordelia_broom_traces(), cordelia_bonds_traces(), cordelia_reveal_traces(),
            cordelia_spores_traces(), cordelia_path_traces(), cordelia_brewing_traces(),
            cordelia_bewitch_traces(), cordelia_jinx_traces(), widget_pyro_swap_traces(),
            widget_dragon_bond_traces(), widget_minion_bite_traces(), widget_breath_traces(),
            widget_diversionary_traces(), widget_airborne_traces(), widget_drag_traces(),
            widget_fight_as_one_traces(), widget_dragon_knight_traces(), mrak_ultimate_primary_trace(), trinkets_barrier_traces(), trinkets_disruptor_traces(), trinkets_self_destruct_traces(), trinkets_design_traces(), trinkets_setup_traces(), trinkets_turret_repeat_traces(), trinkets_cannon_traces(), nebkher_doubt_traces(), nebkher_illusion_supply_traces(), nebkher_swap_attack_traces(), nebkher_projection_traces(), nebkher_phantasmal_traces(), nebkher_topology_traces(), nebkher_laughter_traces(), razzle_swap_traces(), razzle_wire_traces(), razzle_retrieval_traces(), razzle_stunt_traces(), razzle_removal_traces(), razzle_mirror_traces(),
        ):
            envelope, _ = export_state_with_shared_objects(state)
            verify_trace(trace_worker, name, state, envelope, operations)
            print(f"Matched {len(operations)} operations for {name}")
        trace_complete = True
    finally:
        try:
            if trace_worker is not None:
                if trace_complete:
                    trace_worker.close()
                else:
                    trace_worker.abort()
        finally:
            worker.close()
    return count


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--verdicts-only", action="store_true",
                        help="Skip the extended card transition replay corpus")
    args = parser.parse_args()
    print(f"Matched {verify([args.worker], verdicts_only=args.verdicts_only)} sequence verdicts")
