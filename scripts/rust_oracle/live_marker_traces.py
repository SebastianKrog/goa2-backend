"""Frozen singleton marker mutations and post-placement passive execution.

Raw setup preserves marker/dictionary aliases deliberately. Real registered Bain
cards supply the passive integration; the Python handler is the sole oracle.
"""

from __future__ import annotations


def scenes():
    from aura_traces import _board, _pieces
    from combat_effect_traces import state_for
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase, Marker, MarkerType
    from goa2.engine.steps import (
        PlaceMarkerStep,
        RemoveMarkerStep,
        SetActorStep,
        SetContextFlagStep,
    )

    for kind in MarkerType:
        for variant in (
            "new",
            "existing",
            "supply",
            "remove",
            "remove-supply",
            "remove-absent",
            "replace-target",
            "same-model",
            "shared-types",
            "literal-priority",
            "context",
            "empty-literal",
            "empty-key",
            "missing",
            "unknown",
            "skip-target",
            "no-actor",
            "empty-actor",
            "skip-performer",
            "skip-reactor",
            "skip-no-actor",
            "no-skip-performer",
            "inactive",
            "skip-gate",
            "repeat",
            "remove-repeat",
            "live-target",
            "live-actor",
        ):
            state, _, _ = state_for()
            state.markers = {}
            marker = Marker(type=kind, target_id="hero_hanu", source_id="hero_swift", value=-1)
            if variant not in {"new", "remove-absent"}:
                state.markers[kind] = marker
            if variant in {"supply", "remove-supply"}:
                marker.remove()
            step = PlaceMarkerStep(marker_type=kind, target_id="hero_brogan", value=-2)
            if variant.startswith("remove"):
                step = RemoveMarkerStep(marker_type=kind)
            if variant == "same-model":
                state.markers[kind] = marker
            if variant == "shared-types":
                other = next(item for item in MarkerType if item != kind)
                state.markers[other] = marker
            if variant == "literal-priority":
                step.target_key = "target"
                state.execution_context["target"] = "hero_arien"
            if variant in {"context", "empty-literal", "live-target"}:
                step.target_id = "" if variant == "empty-literal" else None
                step.target_key = "target"
                state.execution_context["target"] = "hero_brogan"
            if variant == "empty-key":
                step.target_id, step.target_key = None, ""
                state.execution_context[""] = "hero_brogan"
            if variant == "missing":
                step.target_id, step.target_key = None, "missing"
            if variant == "unknown":
                step.target_id = "unknown"
            if variant == "skip-target":
                step.target_id = "SKIP"
            if variant in {"no-actor", "empty-actor", "skip-no-actor"}:
                state.current_actor_id = "" if variant == "empty-actor" else None
            if variant in {"skip-performer", "skip-reactor", "skip-no-actor", "no-skip-performer"}:
                state.execution_context.update(
                    skip_markers=variant != "no-skip-performer",
                    substitution_actor_id=(
                        "hero_swift" if variant != "skip-reactor" else "hero_hanu"
                    ),
                )
            if variant == "inactive":
                step.active_if_key = "missing"
            if variant == "skip-gate":
                step.skip_if_key = "skip"
                state.execution_context["skip"] = True
            steps = [step]
            if variant in {"repeat", "remove-repeat"}:
                steps.append(step)
            if variant == "live-target":
                steps.extend([SetContextFlagStep(key="target", value="hero_arien"), step])
            if variant == "live-actor":
                steps.extend([SetActorStep(actor_id="hero_hanu"), step])
            state.execution_context.update(
                marker_alias=marker, markers_alias=state.markers, marker_step=step
            )
            ops, _ = retain(state, steps)
            yield f"live-marker-{kind.value}-{variant}", state, ops

    for target in ("owner", "piece", "minion"):
        state = setup(["Swift", "Hanu"], ["Brogan", "Razzle"])
        _board(state)
        pieces = _pieces(state, "hero_razzle")
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_swift"
        target_id = {
            "owner": "hero_razzle",
            "piece": pieces[0],
            "minion": str(next(iter(state.teams.values())).minions[0].id),
        }[target]
        step = PlaceMarkerStep(marker_type=MarkerType.POISON, target_id=target_id, value=-3)
        ops, _ = retain(state, [step])
        yield "live-marker-target-" + target, state, ops

    for kind in MarkerType:
        for variant in ("hand", "empty", "skip", "reactor"):
            state = setup(["Bain", "Hanu"], ["Brogan", "Arien"])
            h = _board(state)
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = state.resolution_owner_id = "hero_bain"
            for unit, point in [
                ("hero_bain", h(0, 0)),
                ("hero_hanu", h(-2, 0)),
                ("hero_brogan", h(2, 0)),
                ("hero_arien", h(2, -2)),
            ]:
                state.place_entity(unit, point)
            bain, victim = state.get_hero("hero_bain"), state.get_hero("hero_brogan")
            card = bain.ultimate_card
            card.state, card.is_facedown = CardState.RESOLVED, False
            bain.played_cards.append(card)
            if variant == "empty":
                victim.hand = []
            if variant in {"skip", "reactor"}:
                state.execution_context.update(skip_markers=True, substitution_actor_id="hero_bain")
            if variant == "reactor":
                state.current_actor_id = "hero_hanu"
            step = PlaceMarkerStep(marker_type=kind, target_id="hero_brogan", value=-1)
            state.execution_context.update(
                victim_alias=victim, card_alias=card, markers_alias=state.markers
            )
            ops, _ = retain(state, [step])
            yield f"live-marker-bain-{kind.value}-{variant}", state, ops
