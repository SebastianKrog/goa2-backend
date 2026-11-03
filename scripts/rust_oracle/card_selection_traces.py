"""Real-handler CARD selection, forced discard, and projectile on-block flows."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer, active_projectiles=False, emit=None):
    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import (
        ActionType,
        CardColor,
        CardContainerType,
        CardState,
        GamePhase,
        StatType,
        TargetType,
        TeamColor,
    )
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        AttackSequenceStep,
        ForceDiscardStep,
        SelectStep,
        SetContextFlagStep,
    )

    result = []
    setup()

    def build(name="Arien"):
        others = [n for n in ["Arien", "Hanu", "Brogan", "Tigerclaw"] if n != name]
        state = setup([name, others[0]], others[1:3])
        state.phase = GamePhase.RESOLUTION
        hero = state.teams[TeamColor.RED].heroes[0]
        state.current_actor_id = hero.id
        return state, hero

    def add(name, state, steps, ops):
        if active_projectiles and name.startswith("projectile-"):
            from combat_effect_traces import effect

            from goa2.domain.models.effect import EffectType

            state.active_effects.append(
                effect(
                    state,
                    "projectile-context",
                    EffectType.MOVEMENT_ZONE,
                    source=str(state.current_actor_id),
                    max_value=8,
                )
            )
        for i, step in enumerate(steps):
            step.step_id = f"fixture/card-select/{name}/{i}"
        push_steps(state, steps)
        if emit is not None:
            emit(name, state, ops)
            return
        result.append(
            {
                "name": f"card-select-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

    def selection_ops(state, step, invalid=False):
        clone = state.model_copy(deep=True)
        probe = step.model_copy(deep=True).resolve(clone, clone.execution_context)
        if not probe.input_request:
            return [advance]
        choice = probe.input_request.options[0].id
        return (
            [advance, answer("not_a_card"), advance, answer(choice), advance]
            if invalid
            else [advance, answer(choice), advance]
        )

    for name in sorted(HeroRegistry._heroes):
        state, hero = build(name)
        step = SelectStep(
            target_type=TargetType.CARD,
            prompt="Choose",
            output_key="chosen",
            selected_card_color_key="chosen_color",
            selected_card_tier_key="chosen_tier",
        )
        add(f"hero-{name}", state, [step], selection_ops(state, step, True))
    for variant in [
        "hand",
        "reverse",
        "alias",
        "played",
        "discard",
        "deck",
        "merge",
        "merge-hidden",
        "include-hidden",
        "hidden-identity",
        "actions",
        "empty-actions",
        "colors",
        "color-key",
        "tier-key",
        "basic",
        "nonbasic",
        "active",
        "enraged-active",
        "item",
        "no-item",
        "allowed",
        "empty-allowed",
        "excluded",
        "states",
        "empty-states",
        "exclude-states",
        "literal-wins",
        "hero-key",
        "owner-literal",
        "owner-key",
        "piece-owner",
        "no-actor",
        "missing-hero",
        "empty",
        "optional-empty",
        "optional-skip",
        "auto",
        "optional-auto",
        "shield-only",
        "invalid-null",
        "invalid-dict",
        "imported",
        "pending",
    ]:
        state, hero = build("Razzle" if variant == "piece-owner" else "Arien")
        step = SelectStep(
            target_type=TargetType.CARD,
            prompt="Choose",
            output_key="chosen",
            selected_card_color_key="chosen_color",
            selected_card_tier_key="chosen_tier",
        )
        state.execution_context = {
            "chosen": "old",
            "chosen_color": "old-color",
            "chosen_tier": "old-tier",
        }
        first = hero.hand[0]
        second = hero.hand[1]
        if variant == "reverse":
            hero.hand.reverse()
        elif variant == "alias":
            hero.hand = [first, first, second]
        elif variant in (
            "played",
            "discard",
            "include-hidden",
            "hidden-identity",
            "shield-only",
            "merge-hidden",
        ):
            cards = hero.hand[:2]
            hero.hand = hero.hand[2:]
            for card in cards:
                card.state = (
                    CardState.RESOLVED
                    if variant in ("played", "shield-only")
                    else CardState.DISCARD
                )
                card.is_facedown = variant not in ("played", "discard", "shield-only")
            if variant in ("played", "shield-only"):
                hero.played_cards = [cards[0], None, cards[1]]
                step.card_container = CardContainerType.PLAYED
                step.restrict_played_to_shields = variant == "shield-only"
            else:
                hero.discard_pile = cards
                step.card_container = CardContainerType.DISCARD
            if variant in ("include-hidden", "hidden-identity"):
                step.include_facedown = True
            if variant == "hidden-identity":
                step.card_is_basic = True
            if variant == "merge-hidden":
                hero.hand.append(cards[0])
                step.card_containers = [CardContainerType.HAND, CardContainerType.DISCARD]
        elif variant == "deck":
            step.card_container = CardContainerType.DECK
        elif variant == "merge":
            step.card_containers = [
                CardContainerType.HAND,
                CardContainerType.DECK,
                CardContainerType.HAND,
            ]
        elif variant == "actions":
            step.card_action_types = [ActionType.DEFENSE, ActionType.DEFENSE_SKILL]
        elif variant == "empty-actions":
            step.card_action_types = []
        elif variant == "colors":
            step.card_colors = [CardColor.RED, CardColor.BLUE]
        elif variant == "color-key":
            step.card_color_key = "color"
            state.execution_context["color"] = "GREEN"
        elif variant == "tier-key":
            step.card_tier_key = "tier"
            state.execution_context["tier"] = "I"
        elif variant in ("basic", "nonbasic"):
            step.card_is_basic = variant == "basic"
        elif variant == "active":
            first.is_active = True
            step.card_is_active = True
        elif variant == "enraged-active":
            first.state = CardState.RESOLVED
            first.enraged_active_override = True
            first.is_active = False
            step.card_is_active = True
        elif variant in ("item", "no-item"):
            step.card_container = CardContainerType.DECK
            step.card_has_item = variant == "item"
        elif variant == "allowed":
            step.allowed_card_ids = [second.id]
        elif variant == "empty-allowed":
            step.allowed_card_ids = []
        elif variant == "excluded":
            step.exclude_card_id_keys = ["excluded"]
            state.execution_context["excluded"] = first.id
        elif variant == "states":
            step.card_states = [CardState.HAND]
        elif variant == "empty-states":
            step.card_states = []
        elif variant == "exclude-states":
            step.exclude_card_states = [CardState.HAND]
        elif variant == "literal-wins":
            step.context_hero_id = "hero_hanu"
            step.context_hero_id_key = "who"
            state.execution_context["who"] = "missing"
        elif variant == "hero-key":
            step.context_hero_id_key = "who"
            state.execution_context["who"] = "hero_hanu"
        elif variant == "owner-literal":
            step.override_player_id = "hero_hanu"
            step.override_player_id_key = "who"
            state.execution_context["who"] = "missing"
        elif variant == "owner-key":
            step.override_player_id_key = "who"
            state.execution_context["who"] = "hero_hanu"
        elif variant == "piece-owner":
            step.context_hero_id = state.get_piece_ids(hero.id)[0]
            step.override_player_id = step.context_hero_id
        elif variant == "no-actor":
            state.current_actor_id = None
            step.context_hero_id = hero.id
        elif variant == "missing-hero":
            step.context_hero_id = "missing"
        elif variant in ("empty", "optional-empty"):
            hero.hand = []
            step.is_mandatory = variant == "empty"
        elif variant in ("auto", "optional-auto"):
            hero.hand = [first]
            step.auto_select_if_one = True
            step.is_mandatory = variant == "auto"
        ops = selection_ops(state, step)
        if variant == "optional-skip":
            step.is_mandatory = False
            ops = [advance, answer("SKIP"), advance]
        elif variant in ("invalid-null", "invalid-dict"):
            ops = [
                advance,
                answer(None if variant == "invalid-null" else {"bad": True}),
                advance,
                answer(str(first.id)),
                advance,
            ]
        elif variant == "imported":
            step.pending_request_id = "fixture/card-request"
            ops = [answer(str(first.id), "fixture/card-request"), advance]
        elif variant == "pending":
            step.pending_input = {"selection": str(first.id)}
            ops = [advance]
        add(variant, state, [step, SetContextFlagStep(key="tail", value=True)], ops)
    for variant in [
        "literal",
        "key",
        "empty-literal",
        "literal-wins",
        "piece",
        "no-actor",
        "basic",
        "nonbasic",
        "no-hand",
        "no-match",
        "unknown",
        "missing-key",
        "immunity-no-effects",
        "atomic",
        "imported-children",
    ]:
        state, hero = build("Razzle" if variant == "piece" else "Arien")
        victim = state.teams[TeamColor.BLUE].heroes[0]
        raw = state.get_piece_ids(hero.id)[0] if variant == "piece" else victim.id
        if variant == "piece":
            victim = hero
        state.execution_context = {"victim": raw}
        step = ForceDiscardStep(victim_id=raw)
        if variant in ("key", "empty-literal"):
            step.victim_id = "" if variant == "empty-literal" else None
            step.victim_key = "victim"
        elif variant == "literal-wins":
            step.victim_key = "victim"
            state.execution_context["victim"] = hero.id
        elif variant == "no-actor":
            state.current_actor_id = None
        elif variant in ("basic", "nonbasic", "no-match"):
            step.card_is_basic = variant == "basic" or variant == "no-match"
        if variant == "no-hand":
            victim.hand = []
        if variant == "no-match":
            victim.hand = [c for c in victim.hand if not c.is_basic]
        if variant == "unknown":
            step.victim_id = "missing"
        if variant == "missing-key":
            step.victim_id = None
            step.victim_key = "absent"
        if variant == "immunity-no-effects":
            step.immunity_source_id = hero.id
        eligible = [
            c for c in victim.hand if step.card_is_basic is None or c.is_basic == step.card_is_basic
        ]
        ops = (
            [advance, answer(str(eligible[0].id)), advance]
            if eligible and variant not in ("unknown", "missing-key")
            else [advance]
        )
        steps = [step]
        if variant == "atomic":
            ops = [
                {"kind": "advance_prefix", "steps": 1},
                advance,
                answer(str(eligible[0].id)),
                {"kind": "advance_prefix", "steps": 1},
                advance,
            ]
        if variant == "imported-children":
            state.execution_context["victim"] = raw
            step.victim_id = None
            step.victim_key = "victim"
            steps = list(step.resolve(state, state.execution_context).new_steps)
            state.execution_context["victim"] = hero.id
        add(f"force-{variant}", state, steps, ops)
    for effect in ["reflect_projectiles", "deflect_projectiles"]:
        for variant in [
            "discard",
            "skip",
            "empty-hand",
            "melee",
            "range-item",
            "zero-range",
            "piece",
            "atomic",
            "invalid-attacker",
            "no-targets",
        ]:
            state = setup(["Wasp", "Arien"], ["Brogan", "Razzle" if variant == "piece" else "Hanu"])
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = "hero_brogan"
            wasp = state.get_hero("hero_wasp")
            victim = state.teams[TeamColor.BLUE].heroes[1]
            target = state.get_piece_ids(victim.id)[0] if variant == "piece" else victim.id
            card = next(c for c in wasp.deck if c.effect_id == effect)
            card.is_facedown = False
            card.state = CardState.HAND
            if card not in wasp.hand:
                wasp.hand.append(card)
            origin = state.entity_locations[wasp.id]
            for uid in ["hero_brogan", target]:
                loc = next(
                    h
                    for h, t in state.board.tiles.items()
                    if not t.is_terrain and not t.occupant_id and origin.distance(h) <= 3
                )
                state.place_entity(uid, loc)
            if variant == "range-item":
                wasp.items[StatType.RANGE] = 2
                loc = next(
                    h
                    for h, t in state.board.tiles.items()
                    if not t.is_terrain and not t.occupant_id and 3 < origin.distance(h) <= 5
                )
                state.place_entity(target, loc)
            if variant == "zero-range":
                wasp.items[StatType.RANGE] = -3
            if variant == "empty-hand":
                victim.hand = []
            if variant == "no-targets":
                state.remove_unit("hero_brogan")
                state.remove_unit(target)
            state.execution_context = {
                "chosen": wasp.id,
                "current_action_type": "ATTACK",
                "reflect_victim": "hero_brogan",
                "deflect_victim": "hero_brogan",
            }
            ops = [advance, answer(str(card.id)), advance]
            if variant not in ("melee", "no-targets"):
                if variant == "invalid-attacker" and effect == "deflect_projectiles":
                    ops += [answer("hero_brogan"), advance]
                ops += [answer("SKIP" if variant == "skip" else target), advance]
                if variant not in ("skip", "empty-hand"):
                    ops += [answer(str(victim.hand[0].id)), advance]
            if variant == "atomic":
                ops = [
                    advance,
                    answer(str(card.id)),
                    advance,
                    answer(target),
                    {"kind": "advance_prefix", "steps": 1},
                    advance,
                    answer(str(victim.hand[0].id)),
                    {"kind": "advance_prefix", "steps": 1},
                    advance,
                ]
            add(
                f"projectile-{effect}-{variant}",
                state,
                [
                    AttackSequenceStep(
                        damage=20, target_id_key="chosen", is_ranged=variant != "melee"
                    )
                ],
                ops,
            )
    return result
