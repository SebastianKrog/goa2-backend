"""Source contracts for candidate-mode differences and frozen pre-action recipes."""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))


def test_only_single_unit_selector_appends_equivalent_illusions_after_units():
    from targeting_rules import evaluate, states

    state = next(state for name, state in states() if name == "plain")
    spec = {
        "target_type": "UNIT",
        "filters": [],
        "skip_self_filter": True,
        "skip_immunity_filter": True,
    }
    single = evaluate(state, {**spec, "mode": "select"})
    multi = evaluate(state, {**spec, "mode": "multi"})
    collected = evaluate(state, {**spec, "mode": "collect"})
    assert single == [*multi, "illusion-0", "illusion-1", "illusion-2"]
    assert multi == collected
    assert evaluate(state, {**spec, "mode": "count"}) == len(multi)


def test_frozen_pre_action_movement_recipe_matches_the_public_source_step():
    from types import SimpleNamespace

    from combat_effect_traces import effect
    from reaction_effect_traces import illusion_state

    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import ResolvePreActionMovementStep

    state, _ = illusion_state("plain")
    provider = effect(
        state, "template", EffectType.PRE_ACTION_MOVEMENT, source="template", max_value=1
    )
    expected = (
        ResolvePreActionMovementStep(hero_id="template")
        .resolve(SimpleNamespace(active_effects=[provider], round=state.round, turn=state.turn), {})
        .new_steps
    )
    for step in expected:
        step.step_id = "template"
    root = Path(__file__).resolve().parents[2]
    frozen = json.loads((root / "rust/tests/fixtures/instruction-templates.json").read_text())
    assert frozen["pre_action_movement"] == [step.model_dump(mode="json") for step in expected]
