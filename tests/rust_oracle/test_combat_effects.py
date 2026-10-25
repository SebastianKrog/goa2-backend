"""Rule assertions supplement full source-derived handler traces.

Raw steps isolate combat/protection primitives; registered decks supply cards.
"""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))
from combat_effect_traces import effect, state_for


@pytest.mark.parametrize("attack", [-16, -1, 0])
@pytest.mark.parametrize("defense", [None, -17, -16, 0])
def test_negative_attacks_are_legal_and_compare_without_a_zero_clamp(attack, defense):
    from goa2.engine.steps import ResolveCombatStep

    state, _, _ = state_for()
    context = {"victim_id": "hero_brogan", "defense_value": defense, "minion_defense_modifier": 0}
    out = ResolveCombatStep(damage=attack).resolve(state, context)
    assert out.is_finished
    assert context["block_succeeded"] == (defense is not None and defense >= attack)
    assert out.events[0].metadata["attack_value"] == attack


def test_bounty_uses_subject_then_creator_and_decrements_the_actual_effect():
    from goa2.domain.models.effect import EffectType
    from goa2.engine.effect_manager import EffectManager

    state, _, minion = state_for()
    bounty = effect(
        state, "bounty", EffectType.MINION_DEFEAT_BOUNTY, source="hero_swift", max_value=2
    )
    bounty.scope.range = 5
    state.active_effects = [bounty]
    for subject, beneficiary in [(None, "hero_swift"), ("hero_hanu", "hero_hanu")]:
        bounty.subject_id = subject
        events = EffectManager.award_minion_defeat_bounties(state, str(minion.id))
        assert events[0].actor_id == beneficiary
        assert state.get_hero(beneficiary).gold == 1
    assert bounty.max_value == 0
    assert EffectManager.award_minion_defeat_bounties(state, str(minion.id)) == []
