"""Ordinary retrieval/model alias/expiry and discard-expiry handler traces."""

from __future__ import annotations


def cases(advance, answer):
    from aura_traces import _board, _pieces
    from model_traces import setup
    from stack_traces import reference_trace
    from state_codec import export_state

    from goa2.domain.models import CardContainerType, GamePhase
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import DiscardCardStep, RetrieveCardStep

    result = []
    variants = [
        "discard",
        "played",
        "current",
        "current-facedown",
        "played-facedown",
        "all",
        "all-empty",
        "all-ignore-key",
        "actor-none",
        "hero-key",
        "hero-key-piece",
        "hero-missing",
        "hero-falsy",
        "unknown-hero",
        "missing-card",
        "unknown-card",
        "hand-only",
        "card-number",
        "card-list",
        "card-object",
        "empty-card-key",
        "all-shared-alias",
        "enraged",
        "global-shadow",
        "played-discard-alias",
        "history",
    ]
    for variant in variants:
        state = setup(["Sabina", "Arien"], ["Brogan", "Razzle"])
        _board(state)
        state.phase = GamePhase.RESOLUTION
        who = "hero_razzle" if variant == "hero-key-piece" else "hero_brogan"
        hero = state.get_hero(who)
        state.current_actor_id = who
        card = hero.hand[0]
        card.is_active_base = True
        all_cards = variant in ["all", "all-empty", "all-ignore-key", "all-shared-alias"]
        if variant in ["current", "current-facedown"]:
            hero.play_card(card)
            card.is_facedown = variant == "current-facedown"
        elif variant in ["played", "played-facedown", "enraged"]:
            hero.play_card(card)
            card.is_facedown = False
            hero.resolve_current_card(turn_number=2)
            card.is_facedown = variant == "played-facedown"
            card.enraged_active_override = variant == "enraged"
        elif variant not in ["hand-only", "all-empty"]:
            hero.discard_card(card)
            if all_cards:
                other = hero.hand[-1]
                other.is_active_base = True
                hero.discard_card(other)
            if variant in ["played-discard-alias", "all-shared-alias"]:
                hero.played_cards = [None, card]
        if variant == "history":
            state.record_public_revealed_card(hero.id, str(card.id))
            state.turn_discard_log[hero.id] = [card.id]
        if variant == "global-shadow":
            shadow = card.model_copy(deep=True)
            shadow.is_active_base = True
            state.get_hero("hero_sabina").deck.append(shadow)
        hero_key = None
        state.execution_context = {"chosen": str(card.id), "who": who}
        if variant == "actor-none":
            state.current_actor_id = None
        if variant in ["hero-key", "hero-key-piece", "hero-missing", "hero-falsy", "unknown-hero"]:
            hero_key = "who"
            state.current_actor_id = "hero_sabina"
            if variant == "hero-key-piece":
                state.execution_context["who"] = _pieces(state, who)[0]
            if variant == "hero-missing":
                state.execution_context.pop("who")
            if variant == "hero-falsy":
                state.execution_context["who"] = False
            if variant == "unknown-hero":
                state.execution_context["who"] = "missing"
        if variant == "missing-card":
            state.execution_context.pop("chosen")
        if variant == "unknown-card":
            state.execution_context["chosen"] = "missing"
        if variant == "card-number" or variant == "all-ignore-key":
            state.execution_context["chosen"] = 7
        if variant == "card-list":
            state.execution_context["chosen"] = ["not-a-card"]
        if variant == "card-object":
            state.execution_context["chosen"] = {"id": str(card.id)}
        key = "chosen"
        if variant == "empty-card-key":
            key = ""
            state.execution_context[""] = str(card.id)
        step = RetrieveCardStep(
            card_key=key,
            hero_key=hero_key,
            retrieve_all_discarded=all_cards,
            step_id=f"fixture/retrieval/{variant}",
        )
        push_steps(state, [step])
        envelope = export_state(state)
        expected = reference_trace(state, [advance])
        events = expected[-1]["events"]
        no_op = variant in [
            "all-empty",
            "actor-none",
            "hero-missing",
            "hero-falsy",
            "unknown-hero",
            "missing-card",
            "unknown-card",
            "hand-only",
            "card-number",
            "card-list",
            "card-object",
        ]
        assert len(events) == (0 if no_op else 2 if all_cards else 1), variant
        if not no_op:
            assert card in hero.hand and not card.is_facedown, variant
            assert card.is_active_base == (variant == "global-shadow"), variant
        if variant == "global-shadow":
            assert not shadow.is_active_base
        if variant == "enraged":
            assert card.enraged_active_override
        if variant == "history":
            assert state.turn_discard_log[hero.id] == [card.id]
        if variant == "hero-key-piece":
            assert events[0]["actor_id"] == state.execution_context["who"]
        result.append(
            {
                "name": f"retrieval-{variant}",
                "envelope": envelope,
                "operations": [advance],
                "expected": expected,
            }
        )
    for variant in ["hand", "played", "current", "global-shadow", "enraged"]:
        state = setup(["Sabina", "Arien"], ["Brogan", "Razzle"])
        state.phase = GamePhase.RESOLUTION
        hero = state.get_hero("hero_brogan")
        state.current_actor_id = hero.id
        card = hero.hand[0]
        card.is_active_base = True
        if variant in ["played", "current", "enraged"]:
            hero.play_card(card)
            card.is_facedown = False
            if variant in ["played", "enraged"]:
                hero.resolve_current_card(turn_number=1)
            card.enraged_active_override = variant == "enraged"
        if variant == "global-shadow":
            shadow = card.model_copy(deep=True)
            shadow.is_active_base = True
            state.get_hero("hero_sabina").deck.append(shadow)
        step = DiscardCardStep(
            hero_id=hero.id,
            card_id=card.id,
            source=(
                CardContainerType.HAND
                if variant in ["hand", "global-shadow"]
                else CardContainerType.PLAYED
            ),
            step_id=f"fixture/discard-expiry/{variant}",
        )
        push_steps(state, [step])
        envelope = export_state(state)
        expected = reference_trace(state, [advance])
        assert card.is_active_base == (variant == "global-shadow"), variant
        if variant == "global-shadow":
            assert not shadow.is_active_base
        if variant == "enraged":
            assert card.enraged_active_override
        result.append(
            {
                "name": f"discard-expiry-{variant}",
                "envelope": envelope,
                "operations": [advance],
                "expected": expected,
            }
        )
    return result
