"""Resource-step oracle boundaries with retained model/context aliases.

Raw setup deliberately exercises identity and live step operands rather than a
card-specific effect, so the card-effect builder fixture is not appropriate.
"""

from __future__ import annotations


def scenes():
    from aura_traces import _board
    from live_defeat_traces import retain
    from model_traces import setup

    from goa2.domain.models import GamePhase
    from goa2.engine.steps import GainCoinsStep, LoseCoinsStep, RestoreSpentLifeCounterStep

    def base():
        state = setup(["Arien", "Swift"], ["Brogan", "Hanu"])
        _board(state)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_swift"
        hero = state.get_hero("hero_arien")
        hero.gold = 5
        state.execution_context.update(
            target=str(hero.id), hero_alias=hero, team_alias=state.teams[hero.team]
        )
        return state, hero

    for kind, cls in (("gain", GainCoinsStep), ("lose", LoseCoinsStep)):
        for variant in (
            "plain",
            "negative",
            "zero",
            "dynamic",
            "missing-amount",
            "dynamic-bool",
            "missing-hero",
            "empty-hero",
            "unknown-hero",
            "numeric-hero",
            "skip",
            "inactive",
            "repeat",
            "empty-actor",
        ):
            state, hero = base()
            kwargs = {"hero_key" if kind == "gain" else "victim_key": "target", "amount": 3}
            if kind == "lose":
                kwargs["output_key"] = "actual"
            if variant == "negative":
                kwargs["amount"] = -8
            if variant == "zero":
                kwargs["amount"] = 0
            if variant in {"dynamic", "missing-amount", "dynamic-bool"}:
                kwargs["amount_key"] = "requested"
                if variant != "missing-amount":
                    state.execution_context["requested"] = True if variant == "dynamic-bool" else 9
            if variant == "missing-hero":
                state.execution_context.pop("target")
            if variant == "empty-hero":
                state.execution_context["target"] = ""
            if variant == "unknown-hero":
                state.execution_context["target"] = "absent"
            if variant == "numeric-hero":
                state.execution_context["target"] = 123
            if variant == "empty-actor":
                state.current_actor_id = ""
            if variant == "skip":
                kwargs["skip_if_key"] = "blocked"
                state.execution_context["blocked"] = True
            if variant == "inactive":
                kwargs["active_if_key"] = "enabled"
            steps = [cls(**kwargs)]
            if variant == "repeat":
                steps.append(steps[0])
            ops, _ = retain(state, steps)
            yield "resource-" + kind + "-" + variant, state, ops
        if kind == "lose":
            for label, value in (("float", 2.9), ("text", " +1_0 "), ("negative-float", -2.9)):
                state, _ = base()
                state.execution_context["requested"] = value
                steps = [cls(victim_key="target", amount_key="requested", output_key="actual")]
                ops, _ = retain(state, steps)
                yield "resource-lose-" + label, state, ops

    for variant in (
        "plain",
        "cap",
        "full",
        "overfull",
        "negative",
        "zero",
        "key",
        "missing-key",
        "empty-key",
        "missing-hero",
        "unknown-hero",
        "actor",
        "no-actor",
        "starting-none",
        "skip",
        "inactive",
        "repeat",
    ):
        state, hero = base()
        team = state.teams[hero.team]
        team.starting_life_counters = 10
        team.life_counters = 6
        kwargs = {"hero_id": str(hero.id), "amount": 3}
        if variant == "cap":
            kwargs["amount"] = 20
        if variant == "full":
            team.life_counters = 10
        if variant == "overfull":
            team.life_counters = 12
        if variant == "negative":
            kwargs["amount"] = -4
        if variant == "zero":
            kwargs["amount"] = 0
        if variant in {"key", "missing-key", "empty-key"}:
            kwargs["hero_key"] = "alternate"
            if variant == "key":
                state.execution_context["alternate"] = "hero_brogan"
                other = state.teams[state.get_hero("hero_brogan").team]
                other.starting_life_counters = 10
                other.life_counters = 6
            if variant == "empty-key":
                state.execution_context["alternate"] = ""
        if variant == "missing-hero":
            kwargs["hero_id"] = ""
        if variant == "unknown-hero":
            kwargs["hero_id"] = "absent"
        if variant in {"actor", "no-actor"}:
            kwargs["hero_id"] = None
            if variant == "no-actor":
                state.current_actor_id = None
        if variant == "starting-none":
            team.starting_life_counters = None
        if variant == "skip":
            kwargs["skip_if_key"] = "blocked"
            state.execution_context["blocked"] = True
        if variant == "inactive":
            kwargs["active_if_key"] = "enabled"
        steps = [RestoreSpentLifeCounterStep(**kwargs)]
        if variant == "repeat":
            steps.append(steps[0])
        ops, _ = retain(state, steps)
        yield "resource-life-" + variant, state, ops

    # Razzle piece targeting must resolve the owner, but keep raw piece event IDs.
    for kind in ("gain", "lose", "life"):
        state = setup(["Razzle", "Swift"], ["Brogan", "Hanu"])
        h = _board(state)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_swift"
        hero = state.get_hero("hero_razzle")
        hero.gold = 5
        piece = "hero_razzle_piece_1"
        state.place_entity(piece, h(0, 0))
        state.execution_context.update(target=piece, hero_alias=hero)
        if kind == "life":
            team = state.teams[hero.team]
            team.starting_life_counters = 10
            team.life_counters = 5
            step = RestoreSpentLifeCounterStep(hero_key="target")
        elif kind == "gain":
            step = GainCoinsStep(hero_key="target", amount=2)
        else:
            step = LoseCoinsStep(victim_key="target", amount=2, output_key="actual")
        ops, _ = retain(state, [step])
        yield "resource-" + kind + "-piece", state, ops
