"""Frozen real-handler combat/defeat traces; no attack/reaction shortcuts."""

from __future__ import annotations

from oracle import ROOT
from state_codec import export_state


def cases(advance, answer):
    from stack_traces import reference_trace

    from goa2.domain.board import Board
    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase, Hero, Minion, MinionType, Team, TeamColor
    from goa2.domain.models.marker import Marker, MarkerType
    from goa2.domain.models.unit import HeroPiece
    from goa2.domain.state import GameState
    from goa2.domain.tile import Tile
    from goa2.engine.handler import push_steps
    from goa2.engine.setup import GameSetup
    from goa2.engine.steps.combat import (
        DefeatUnitStep,
        RemoveUnitStep,
        ResolveCombatStep,
        TriggerGameOverStep,
    )
    from goa2.engine.steps.utility import SetContextFlagStep

    result = []

    def build():
        coords = [
            Hex(q=q, r=r, s=-q - r) for q in range(-3, 4) for r in range(-3, 4) if abs(q + r) <= 3
        ]
        red = [Hero(id=n, name=n, team=TeamColor.RED, deck=[]) for n in ("killer", "ally")]
        blue = [Hero(id="victim", name="Victim", team=TeamColor.BLUE, deck=[])]
        red.append(Hero(id="razzle", name="Razzle", team=TeamColor.RED, deck=[], piece_supply=2))
        state = GameState(
            board=Board(tiles={h: Tile(hex=h) for h in coords}),
            teams={
                TeamColor.RED: Team(color=TeamColor.RED, heroes=red),
                TeamColor.BLUE: Team(color=TeamColor.BLUE, heroes=blue),
            },
            phase=GamePhase.RESOLUTION,
            rules_version=9,
            current_actor_id="killer",
        )
        for uid, p in [("killer", (0, 0, 0)), ("ally", (-2, 0, 2)), ("victim", (2, -1, -1))]:
            state.place_entity(uid, Hex(q=p[0], r=p[1], s=p[2]))
        for i, p in enumerate([(-1, 0, 1), (-1, 1, 0)], 1):
            piece = HeroPiece(
                id=f"razzle_piece_{i}",
                name="Razzle piece",
                owner_hero_id="razzle",
                team=TeamColor.RED,
            )
            state.misc_entities[piece.id] = piece
            state.place_entity(piece.id, Hex(q=p[0], r=p[1], s=p[2]))
        for kind, p in [
            (MinionType.MELEE, (1, -1, 0)),
            (MinionType.RANGED, (1, 0, -1)),
            (MinionType.HEAVY, (2, -2, 0)),
        ]:
            minion = Minion(id=kind.value, name=kind.value, type=kind, team=TeamColor.BLUE)
            state.teams[TeamColor.BLUE].minions.append(minion)
            state.place_entity(minion.id, Hex(q=p[0], r=p[1], s=p[2]))
        state.unresolved_hero_ids = ["victim", "killer", "victim"]
        return state

    def add(name, state, steps):
        for i, step in enumerate(steps):
            step.step_id = f"fixture/combat/{name}/{i}"
        push_steps(state, steps)
        operations = [advance]
        result.append(
            {
                "name": f"combat-{name}",
                "envelope": export_state(state),
                "operations": operations,
                "expected": reference_trace(state, operations),
            }
        )

    contexts = [
        ("none", {}),
        ("tie", {"defense_value": 4, "minion_defense_modifier": 0}),
        ("hit", {"defense_value": 3, "minion_defense_modifier": 0}),
        ("invalid-before-auto", {"defense_invalid": True, "auto_block": True, "defense_value": 99}),
        ("auto", {"auto_block": True}),
        ("cached", {"defense_value": 2, "minion_defense_modifier": 2}),
        ("negative-cached", {"defense_value": 5, "minion_defense_modifier": -2}),
        (
            "ignore",
            {"defense_value": 3, "minion_defense_modifier": 99, "ignore_minion_defense": True},
        ),
        ("bonus-string", {"defense_value": 2, "defense_bonus": "2", "minion_defense_modifier": 0}),
        ("bonus-float", {"defense_value": 2, "defense_bonus": 2.9, "minion_defense_modifier": 0}),
        (
            "initiative",
            {
                "defense_value": 2,
                "defense_bonus": 99,
                "defense_uses_initiative": True,
                "minion_defense_modifier": 0,
            },
        ),
        ("bool", {"defense_value": True, "minion_defense_modifier": True, "defense_bonus": 2}),
        ("computed-modifier", {"defense_value": 3}),
        ("null-cache", {"defense_value": 3, "minion_defense_modifier": None}),
        ("no-defense-bonus-ignored", {"defense_bonus": "bad"}),
    ]
    for name, ctx in contexts:
        state = build()
        state.execution_context = {"victim_id": "victim", **ctx}
        add(name, state, [ResolveCombatStep(damage=4)])
    for name, value in [("missing", None), ("empty", ""), ("offboard", "offboard")]:
        state = build()
        if value is not None:
            state.execution_context["victim_id"] = value
        add(name, state, [ResolveCombatStep(damage=4)])
    for kind in MinionType:
        for killer in ["killer", "razzle_piece_1", "MELEE", None, "unknown"]:
            state = build()
            add(
                f"minion-{kind.value}-{killer}",
                state,
                [DefeatUnitStep(victim_id=kind.value, killer_id=killer)],
            )
    for level in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]:
        state = build()
        state.teams[TeamColor.BLUE].heroes[0].level = level
        add(
            f"level-{level}",
            state,
            [DefeatUnitStep(victim_id="victim", killer_id="killer", assist_multiplier=3)],
        )
    for killer in ["razzle_piece_1", "MELEE", "victim", None, "unknown"]:
        state = build()
        add(f"hero-killer-{killer}", state, [DefeatUnitStep(victim_id="victim", killer_id=killer)])
    for target in ["victim", "razzle_piece_1", "razzle"]:
        state = build()
        for kind in MarkerType:
            state.markers[kind] = Marker(type=kind, target_id=target, value=-2, source_id="killer")
        add(f"markers-{target}", state, [DefeatUnitStep(victim_id=target, killer_id="killer")])
    state = build()
    state.markers[MarkerType.BOUNTY] = Marker(
        type=MarkerType.BOUNTY, target_id="ally", value=1, source_id="victim"
    )
    add("placed-marker-remains", state, [DefeatUnitStep(victim_id="victim", killer_id="killer")])
    for target in ["victim", "razzle_piece_1", "razzle"]:
        state = build()
        add(
            f"repeat-{target}",
            state,
            [
                DefeatUnitStep(victim_id=target, killer_id="killer"),
                DefeatUnitStep(victim_id=target, killer_id="killer"),
            ],
        )
    for remaining in [0, 1, 2]:
        state = build()
        state.teams[TeamColor.BLUE].life_counters = remaining
        state.teams[TeamColor.BLUE].heroes[0].level = 8
        add(
            f"annihilation-{remaining}",
            state,
            [
                DefeatUnitStep(victim_id="victim", killer_id="killer"),
                SetContextFlagStep(key="must_not_run", value=True),
            ],
        )
    for name, step in [
        ("remove", RemoveUnitStep(unit_id="victim")),
        ("remove-offboard", RemoveUnitStep(unit_id="unknown")),
        ("remove-key-overrides", RemoveUnitStep(unit_id="victim", unit_key="absent")),
        ("defeat-key", DefeatUnitStep(victim_key="chosen", killer_id="killer")),
        (
            "defeat-literal-wins",
            DefeatUnitStep(victim_id="ally", victim_key="chosen", killer_id="killer"),
        ),
        ("no-victim", DefeatUnitStep()),
        ("team-win", TriggerGameOverStep(winner=TeamColor.RED, condition="TEST")),
        ("hero-win", TriggerGameOverStep(individual_winner_id="victim", condition="TEST")),
    ]:
        state = build()
        state.execution_context["chosen"] = "victim"
        add(name, state, [step])
    # Real registered card instances exercise turn-slot completion and aliases.
    path = sorted((ROOT / "src/goa2/data/maps").glob("*.json"))[0]
    for variant in ["ordinary", "retrieved", "alias", "annihilation"]:
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        hero = state.get_hero("hero_brogan")
        card = hero.hand[0]
        hero.current_turn_card = card
        if variant != "alias":
            hero.hand.remove(card)
        if variant == "retrieved":
            hero.current_turn_card = None
        state.unresolved_hero_ids = ["hero_brogan", "hero_hanu"]
        if variant == "annihilation":
            state.teams[TeamColor.BLUE].life_counters = 1
        add(f"card-{variant}", state, [DefeatUnitStep(victim_id=hero.id, killer_id="hero_arien")])
    return result
