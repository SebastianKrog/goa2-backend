"""Real primary-defense effects and shared singleton marker instructions."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.domain.models import CardState, GamePhase, TeamColor
    from goa2.domain.models.marker import Marker, MarkerType
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        AttackSequenceStep,
        PlaceMarkerStep,
        RemoveMarkerStep,
        ResolveDefenseTextStep,
    )

    result = []

    def build(name):
        state = setup([name, "Hanu"], ["Brogan", "Razzle"])
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_brogan"
        return state, state.teams[TeamColor.RED].heroes[0]

    def add(name, state, steps, ops):
        for i, step in enumerate(steps):
            step.step_id = f"fixture/defense-effect/{name}/{i}"
        push_steps(state, steps)
        result.append(
            {
                "name": f"defense-effect-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

    for effect, name in [
        ("aspiring_duelist", "Arien"),
        ("stop_projectiles", "Wasp"),
        ("perfect_getaway", "Bain"),
        ("close_call", "Bain"),
        ("narrow_escape", "Bain"),
    ]:
        variants = (
            ["ranged", "melee", "large-attack", "piece-attacker"]
            if name != "Bain"
            else [
                "absent",
                "supply",
                "hero",
                "defender",
                "offboard",
                "piece",
                "empty-target",
                "copied-attacker",
                "copied-defender",
            ]
        )
        for variant in variants:
            state, hero = build(name)
            card = next(c for c in hero.deck if c.effect_id == effect)
            if card not in hero.hand:
                hero.hand.append(card)
            card.state = CardState.HAND
            card.is_facedown = False
            if name == "Bain" and variant != "absent":
                target = {
                    "supply": None,
                    "hero": "hero_brogan",
                    "defender": hero.id,
                    "offboard": "hero_brogan",
                    "piece": state.get_piece_ids("hero_razzle")[0],
                    "empty-target": "",
                    "copied-attacker": "hero_brogan",
                    "copied-defender": "hero_brogan",
                }[variant]
                state.markers[MarkerType.BOUNTY] = Marker(
                    type=MarkerType.BOUNTY, target_id=target, value=2, source_id="hero_hanu"
                )
                if variant == "offboard":
                    state.remove_unit("hero_brogan")
            if variant == "piece-attacker":
                state.current_actor_id = state.get_piece_ids("hero_razzle")[0]
            state.execution_context = {"defense_target": hero.id, "current_action_type": "ATTACK"}
            if variant in ("copied-attacker", "copied-defender"):
                state.execution_context.update(
                    skip_markers=True,
                    substitution_actor_id=(
                        "hero_brogan" if variant == "copied-attacker" else hero.id
                    ),
                )
            damage = 20 if variant in ("large-attack", "melee") else 2
            step = AttackSequenceStep(
                damage=damage, target_id_key="defense_target", is_ranged=variant != "melee"
            )
            add(f"{effect}-{variant}", state, [step], [advance, answer(str(card.id)), advance])
    # The projectile family shares defense text, while upgraded on-block bodies
    # remain a separate port. Isolate the real defense handler for these cards.
    for effect in ["reflect_projectiles", "deflect_projectiles"]:
        for ranged in [False, True]:
            state, hero = build("Wasp")
            card = next(c for c in hero.deck if c.effect_id == effect)
            hero.discard_pile.append(card)
            hero.hand = [c for c in hero.hand if c is not card]
            card.state = CardState.DISCARD
            card.is_facedown = False
            state.execution_context = {
                "defender_id": hero.id,
                "defense_card_id": card.id,
                "is_primary_defense": True,
                "attack_is_ranged": ranged,
            }
            add(f"{effect}-text-{ranged}", state, [ResolveDefenseTextStep()], [advance])
    for variant in [
        "offboard",
        "no-primary",
        "missing-card",
        "missing-defender",
        "facedown",
        "hand-card",
    ]:
        state, hero = build("Arien")
        card = next(c for c in hero.deck if c.effect_id == "aspiring_duelist")
        hero.hand = [c for c in hero.hand if c is not card]
        hero.discard_pile.append(card)
        card.state = CardState.DISCARD
        card.is_facedown = False
        state.execution_context = {
            "defender_id": hero.id,
            "defense_card_id": card.id,
            "is_primary_defense": True,
        }
        if variant == "offboard":
            state.remove_unit(hero.id)
        elif variant == "no-primary":
            state.execution_context["is_primary_defense"] = False
        elif variant == "missing-card":
            state.execution_context["defense_card_id"] = "missing"
        elif variant == "missing-defender":
            state.execution_context["defender_id"] = "missing"
        elif variant == "facedown":
            card.is_facedown = True
        elif variant == "hand-card":
            hero.discard_pile.remove(card)
            hero.hand.append(card)
            card.state = CardState.HAND
        add(f"duelist-{variant}", state, [ResolveDefenseTextStep()], [advance])
    for effect, name in [
        ("aspiring_duelist", "Arien"),
        ("stop_projectiles", "Wasp"),
        ("close_call", "Bain"),
        ("narrow_escape", "Bain"),
    ]:
        state, hero = build(name)
        card = next(c for c in hero.deck if c.effect_id == effect)
        hero.hand = [c for c in hero.hand if c is not card]
        hero.discard_pile.append(card)
        card.state = CardState.DISCARD
        card.is_facedown = False
        state.markers[MarkerType.BOUNTY] = Marker(
            type=MarkerType.BOUNTY, target_id="hero_brogan", source_id="hero_hanu"
        )
        state.execution_context = {
            "defender_id": hero.id,
            "defense_card_id": card.id,
            "is_primary_defense": True,
            "attack_is_ranged": True,
        }
        add(
            f"atomic-{effect}",
            state,
            [ResolveDefenseTextStep()],
            [{"kind": "advance_prefix", "steps": 1} for _ in range(6)] + [advance],
        )
    for kind in MarkerType:
        state, hero = build("Arien")
        card = next(c for c in hero.deck if c.effect_id == "aspiring_duelist")
        if card not in hero.hand:
            hero.hand.append(card)
        state.execution_context = {"defense_target": hero.id, "current_action_type": "ATTACK"}
        add(
            f"live-stats-{kind.value}",
            state,
            [
                PlaceMarkerStep(marker_type=kind, target_id=hero.id, value=-2),
                AttackSequenceStep(damage=2, target_id_key="defense_target"),
            ],
            [advance, answer(str(card.id)), advance],
        )
    for kind in MarkerType:
        for variant in [
            "place",
            "context",
            "literal-wins",
            "replace",
            "no-target",
            "no-actor",
            "piece-target",
            "piece-source",
            "skip-copy",
            "skip-foreign-defense",
            "skip-piece-copy",
            "remove-absent",
            "remove-supply",
            "place-remove-place",
            "atomic",
        ]:
            state, hero = build("Arien")
            piece = state.get_piece_ids("hero_razzle")[0]
            step = PlaceMarkerStep(marker_type=kind, target_id=hero.id, value=-2)
            if variant == "context":
                step.target_id = None
                step.target_key = "chosen"
                state.execution_context["chosen"] = piece
            elif variant == "literal-wins":
                step.target_key = "chosen"
                state.execution_context["chosen"] = "hero_brogan"
            elif variant == "replace":
                state.markers[kind] = Marker(type=kind, target_id=piece, value=6, source_id="old")
            elif variant == "no-target":
                step.target_id = None
            elif variant == "no-actor":
                state.current_actor_id = None
            elif variant == "piece-target":
                step.target_id = piece
            elif variant == "piece-source":
                state.current_actor_id = piece
            elif variant.startswith("skip"):
                state.execution_context = {
                    "skip_markers": True,
                    "substitution_actor_id": (
                        "hero_brogan" if variant != "skip-foreign-defense" else "hero_arien"
                    ),
                }
                if variant == "skip-piece-copy":
                    state.current_actor_id = piece
                    state.execution_context["substitution_actor_id"] = "hero_razzle"
            steps = [step]
            if variant.startswith("remove"):
                steps = [RemoveMarkerStep(marker_type=kind)]
                if variant == "remove-supply":
                    state.markers[kind] = Marker(type=kind, value=7, source_id="old")
            if variant in ("place-remove-place", "atomic"):
                steps = [
                    step,
                    RemoveMarkerStep(marker_type=kind),
                    PlaceMarkerStep(marker_type=kind, target_id=piece, value=3),
                ]
            ops = (
                [{"kind": "advance_prefix", "steps": 1} for _ in range(6)] + [advance]
                if variant == "atomic"
                else [advance]
            )
            add(f"marker-{kind.value}-{variant}", state, steps, ops)
    return result
