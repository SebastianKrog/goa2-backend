"""Source retrieval precedence, physical aliases, expiry, and full card chains."""

from __future__ import annotations


def scenes():
    from combat_effect_traces import effect
    from live_passive_traces import build
    from object_execution_traces import live_choice_operations

    from goa2.domain.models import CardState
    from goa2.domain.models.effect import EffectType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.steps import RetrieveCardStep, SetContextFlagStep

    def retain(state, steps, mode="first"):
        state.execution_stack = list(reversed(steps))
        state.execution_context["retrieval_steps"] = steps
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    variants = (
        "discard",
        "played",
        "current",
        "holes",
        "facedown",
        "initiative",
        "played-first",
        "discard-first",
        "equal-discard",
        "equal-current",
        "revision-current",
        "metadata-current",
        "all",
        "all-empty",
        "all-printed",
        "all-effects",
        "hero-key",
        "empty-hero-key",
        "missing-hero",
        "missing-card",
        "unknown-card",
        "unknown-hero",
        "no-actor",
        "skip",
        "active",
        "effects",
        "duplicate-effect-id",
        "unmatched-effects",
        "foreign-print",
        "shared-discard-played",
        "shared-hand-discard",
    )
    for variant in variants:
        state, hero = build("Arien")
        if variant == "foreign-print":
            hero = state.get_hero("hero_hanu")
        card = hero.hand.pop(0)
        card.state = CardState.DISCARD
        card.is_active = True
        card.is_facedown = variant == "facedown"
        hero.discard_pile = [card]
        hero.played_cards = []
        state.execution_context.update(chosen=card.id, who=str(hero.id), target=card)
        other = next(h for team in state.teams.values() for h in team.heroes if h.id != hero.id)
        step = RetrieveCardStep(step_id="fixture/retrieval/step", card_key="chosen")
        if variant in {
            "played",
            "holes",
            "played-first",
            "equal-discard",
            "equal-current",
            "revision-current",
            "metadata-current",
        }:
            hero.discard_pile = []
            card.state = CardState.RESOLVED
            hero.played_cards = [None, card, None] if variant == "holes" else [card]
        if variant in {"current", "initiative"}:
            hero.discard_pile = []
            card.state = CardState.UNRESOLVED
            card.initiative_override = 9
            hero.current_turn_card = card
        if variant in {
            "played-first",
            "discard-first",
            "equal-discard",
            "equal-current",
            "revision-current",
            "metadata-current",
        }:
            clone = card.model_copy(deep=True)
            clone.state = (
                CardState.HAND
                if variant.startswith(("equal-", "revision-", "metadata-"))
                else CardState.DISCARD
            )
            clone.is_facedown = False
            if variant == "revision-current":
                clone._definition_revision += 1
            if variant == "metadata-current":
                card.metadata = {"a": True, "b": [2]}
                clone.metadata = {"b": [2.0], "a": 1}
            state.execution_context["clone"] = clone
            if variant.endswith("current"):
                hero.current_turn_card = clone
            else:
                hero.discard_pile = [clone] if variant != "discard-first" else [card, clone]
        if variant in {"all", "all-printed", "all-effects"}:
            second = card.model_copy(deep=True) if variant == "all-printed" else hero.hand.pop(0)
            second.state = CardState.DISCARD
            second.is_active = True
            hero.discard_pile.append(second)
            step.retrieve_all_discarded = True
        if variant == "all-empty":
            hero.discard_pile = []
            step.retrieve_all_discarded = True
        if variant in {
            "hero-key",
            "empty-hero-key",
            "missing-hero",
            "unknown-hero",
            "foreign-print",
        }:
            step.hero_key = "who"
        if variant == "empty-hero-key":
            step.hero_key = ""
        if variant == "missing-hero":
            state.execution_context.pop("who")
        if variant == "unknown-hero":
            state.execution_context["who"] = "unknown"
        if variant == "missing-card":
            state.execution_context.pop("chosen")
        if variant == "unknown-card":
            state.execution_context["chosen"] = "unknown"
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "skip":
            step.skip_if_key = "skip"
            state.execution_context["skip"] = False
        if variant == "active":
            step.active_if_key = "missing"
        if variant in {"effects", "duplicate-effect-id", "unmatched-effects", "all-effects"}:
            finishing = SetContextFlagStep(
                step_id="fixture/retrieval/forbidden", key="forbidden", value=True
            )
            linked = effect(
                state,
                "duplicate",
                EffectType.DELAYED_TRIGGER,
                source=str(hero.id),
                source_card_id=card.id,
                finishing_steps=[finishing],
            )
            unrelated = effect(
                state,
                "duplicate" if variant == "duplicate-effect-id" else "other",
                EffectType.DELAYED_TRIGGER,
                source=str(hero.id),
                source_card_id=hero.hand[0].id,
            )
            state.active_effects = (
                [unrelated] if variant == "unmatched-effects" else [linked, unrelated]
            )
            if variant == "all-effects":
                second_linked = effect(
                    state,
                    "duplicate",
                    EffectType.DELAYED_TRIGGER,
                    source=str(hero.id),
                    source_card_id=second.id,
                    finishing_steps=[finishing],
                )
                state.active_effects.insert(1, second_linked)
        if variant == "foreign-print":
            clone = card.model_copy(deep=True)
            clone.state = CardState.HAND
            state.get_hero("hero_arien").hand.append(clone)
            state.execution_context["foreign"] = clone
        if variant == "shared-discard-played":
            other.played_cards = hero.discard_pile
        if variant == "shared-hand-discard":
            other.discard_pile = hero.hand
        state.execution_context.update(
            old_effects=state.active_effects,
            hand_alias=hero.hand,
            discard_alias=hero.discard_pile,
            played_alias=hero.played_cards,
        )
        yield "retrieve-" + variant, state, retain(state, [step])

    for hero_name, effect_id in (
        ("Xargatha", "fresh_converts"),
        ("Xargatha", "devoted_followers"),
        ("Sabina", "steady_advance"),
    ):
        for enabled in (False, True):
            for mode in ("first", "last", "invalid"):
                state, hero = build(hero_name)
                card = next(c for c in [*hero.hand, *hero.deck] if c.effect_id == effect_id)
                hero.hand = [c for c in hero.hand if c is not card]
                card.state = CardState.UNRESOLVED
                card.is_facedown = False
                hero.current_turn_card = card
                returned = hero.hand.pop(0)
                returned.state = CardState.DISCARD
                returned.is_active = True
                hero.discard_pile = [returned]
                population = 2 if hero_name == "Sabina" else 1
                team = (
                    state.teams[hero.team]
                    if hero_name == "Sabina"
                    else next(t for t in state.teams.values() if t.color != hero.team)
                )
                if enabled:
                    origin = state.get_position(hero.id)
                    for minion, point in zip(
                        team.minions[:population],
                        list(origin.neighbors())[:population],
                        strict=True,
                    ):
                        state.place_entity(minion.id, point)
                body = CardEffectRegistry.get(effect_id).get_steps(state, hero, card)
                for i, step in enumerate(body):
                    step.step_id = f"fixture/retrieval/card/{i}"
                yield f"retrieve-card-{effect_id}-{enabled}-{mode}", state, retain(
                    state, body, mode
                )
