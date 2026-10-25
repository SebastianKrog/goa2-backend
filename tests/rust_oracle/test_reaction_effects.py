"""Source assertions for illusion and reaction primitive semantics."""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))
from reaction_effect_traces import illusion_state


@pytest.mark.parametrize(
    "variant, expected",
    [
        ("plain", -2),
        ("inactive", 0),
        ("other-actor", 0),
        ("no-actor", 0),
        ("empty-actor", 0),
        ("unknown-source", 0),
        ("token-source", 0),
        ("blue-source", 2),
        ("actor-piece", -2),
        ("source-piece", -2),
        ("both-pieces", -2),
        ("old-active", -2),
        ("off-scope", -2),
        ("wrong-subject", -2),
        ("source-offboard", -2),
        ("duplicate", -2),
        ("inactive-first", -2),
        ("wrong-first", -2),
        ("non-illusion", -1),
        ("offboard-token", -1),
        ("target-offboard", 0),
    ],
)
def test_illusion_defense_uses_actor_owner_and_direct_active_flag(variant, expected):
    from goa2.engine.stats import calculate_minion_defense_modifier

    state, target = illusion_state(variant)
    assert calculate_minion_defense_modifier(state, target) == expected


def test_reaction_basic_bonus_uses_the_offered_card_even_without_a_turn_card():
    from combat_effect_traces import effect
    from discard_effect_traces import state_for

    from goa2.domain.models import ActionType, StatType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import ReactionWindowStep

    state, hero, _ = state_for("plain")
    card = next(
        c for c in hero.hand if c.is_basic and ActionType.DEFENSE in c.current_secondary_actions
    )
    assert hero.current_turn_card is None
    state.active_effects.append(
        effect(
            state,
            "basic",
            EffectType.BASIC_ACTION_STAT_BONUS,
            source=str(hero.id),
            stat_type=StatType.DEFENSE,
            stat_value=3,
        )
    )
    req = ReactionWindowStep().resolve(state, {"target_id": str(hero.id)}).input_request
    option = next(o for o in req.options if o.id == card.id)
    assert option.metadata["defense_value"] == card.get_base_stat_value(StatType.DEFENSE) + 3


def test_reaction_rejects_a_shield_after_its_effect_is_removed():
    from discard_effect_traces import state_for

    from goa2.engine.steps import ReactionWindowStep

    state, hero, shield = state_for("plain")
    context = {"target_id": str(hero.id)}
    step = ReactionWindowStep()
    req = step.resolve(state, context).input_request
    assert any(o.id == shield.id for o in req.options)
    state.active_effects.pop(0)
    step.pending_input = {"selection": str(shield.id)}
    req = step.resolve(state, context).input_request
    assert req is not None and all(o.id != shield.id for o in req.options)
    assert shield in hero.played_cards
