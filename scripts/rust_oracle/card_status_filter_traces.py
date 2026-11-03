"""Current-card and resolved-turn-slot predicates used by Hanu and Emmitt."""

from __future__ import annotations


def scenes():
    from model_traces import setup

    from goa2.domain.models import CardState, GamePhase
    from goa2.engine.filters_cards import HasResolvedCardFilter, HasUnresolvedCardFilter

    for status in [None, *CardState]:
        for facedown in (False, True):
            state = setup()
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = "hero_arien"
            hero = state.get_hero("hero_hanu")
            card = hero.hand[0]
            if status is not None:
                card.state = status
                card.is_facedown = facedown
                hero.current_turn_card = card
            for label, predicate in (
                ("unresolved", HasUnresolvedCardFilter()),
                ("resolved", HasResolvedCardFilter()),
            ):
                expected = status == (
                    CardState.UNRESOLVED if label == "unresolved" else CardState.RESOLVED
                )
                assert predicate.apply(hero.id, state, {}) == expected
                assert not predicate.apply("unknown", state, {})
                assert not predicate.apply({"q": 0, "r": 0, "s": 0}, state, {})
                assert not predicate.apply(1, state, {})
            yield _case(state, f"{status.value if status else 'missing'}-{facedown}")

    for turn in (0, 1, 2, 4, 5):
        for slot in (0, 1, 3):
            state = setup()
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = "hero_arien"
            state.turn = turn
            hero = state.get_hero("hero_hanu")
            hero.played_cards = [None] * 4
            hero.played_cards[slot] = hero.hand[0]
            # A filled slot is sufficient even when its card is not RESOLVED.
            assert HasResolvedCardFilter().apply(hero.id, state, {}) == (turn == slot + 1)
            assert not HasUnresolvedCardFilter().apply(hero.id, state, {})
            yield _case(state, f"turn-{turn}-slot-{slot}")


def _case(state, name):
    from goa2.domain.models import TargetType
    from goa2.engine.filters_cards import HasResolvedCardFilter, HasUnresolvedCardFilter
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import CollectUnitsStep, CountStep, SelectStep

    steps = []
    for label, predicate in (
        ("unresolved", HasUnresolvedCardFilter()),
        ("resolved", HasResolvedCardFilter()),
    ):
        steps.extend(
            [
                CollectUnitsStep(filters=[predicate], output_key=label),
                CountStep(filters=[predicate], output_key=label + "-count"),
                SelectStep(
                    prompt="Choose a hex",
                    target_type=TargetType.HEX,
                    filters=[predicate],
                    output_key=label + "-hex",
                    is_mandatory=False,
                ),
            ]
        )
    for index, step in enumerate(steps):
        step.step_id = f"fixture/card-status/{name}/{index}"
    push_steps(state, steps)
    state.execution_context["current-alias"] = state.get_hero("hero_hanu").current_turn_card
    return (
        "card-status-filter-" + name,
        state,
        [{"kind": "advance_prefix", "steps": 1}] * len(steps),
    )
