"""Frozen Hanu behavior and cross-root model identity contracts."""

import pytest
from live_hanu_primary_traces import CO_MOVES, JOURNEYS, action_scene, base
from object_execution_traces import reference


def test_reviewed_hanu_recipes_include_all_eighteen_faces_and_cross_root_aliases():
    from primary_builders import catalog
    from primary_templates import HANU_EFFECTS

    data = catalog()
    assert len(HANU_EFFECTS) == 18
    assert set(HANU_EFFECTS) <= data["recipes"].keys()
    for effect in ("trusted_sidekick", "pile_on"):
        assert any(row.get("imports") for row in data["recipes"][effect])


@pytest.mark.parametrize("effect", CO_MOVES)
@pytest.mark.parametrize("mode", ["first", "last"])
def test_distance_is_delegated_to_partner_without_changing_movement_actor(effect, mode):
    state, ops, final = action_scene(effect, mode=mode)
    before = {unit: state.get_position(unit) for unit in ("hero_hanu", "hero_arien")}
    records = reference(state, ops)
    requests = [row["request"] for row in records if row["request"]]
    distance = next(row for row in requests if row["prompt"] == "Choose a distance to move")
    direction = next(row for row in requests if row["prompt"].startswith("Choose a direction"))
    assert distance["player_id"] == "hero_arien"
    assert direction["player_id"] == "hero_hanu"
    distance = final.execution_context["tw_distance"]
    assert distance == (1 if mode == "first" else 2 if effect == "this_way" else 3)
    changes = []
    for unit in before:
        after = final.get_position(unit)
        assert before[unit].distance(after) == distance
        changes.append(
            (after.q - before[unit].q, after.r - before[unit].r, after.s - before[unit].s)
        )
    assert changes[0] == changes[1]


@pytest.mark.parametrize("effect", JOURNEYS)
def test_journey_returns_after_real_turn_cleanup(effect):
    state, ops, final = action_scene(effect, "turn", "skip")
    before = {unit: state.get_position(unit) for unit in ("hero_hanu", "hero_brogan")}
    records = reference(state, ops)
    events = [e for row in records for e in row["events"]]
    assert sum(e["event_type"] == "UNITS_SWAPPED" for e in events) == 2
    assert not final.active_effects
    assert final.get_position("hero_hanu") == before["hero_hanu"]
    assert final.get_position("hero_brogan") == before["hero_brogan"]


@pytest.mark.parametrize("effect", ["trusted_sidekick", "pile_on"])
def test_two_alternate_attacks_share_the_real_filter_models(effect):
    from goa2.engine.effects import CardEffectRegistry

    state, hero, card = base(effect)
    steps = CardEffectRegistry.get(effect).get_steps(state, hero, card)
    attacks = [
        step for step in steps if step.type.value == "attack_sequence" and step.target_filters
    ]
    a, b = attacks
    assert a.target_filters is not b.target_filters
    assert all(x is y for x, y in zip(a.target_filters, b.target_filters, strict=True))


def test_hurry_up_ultimate_enables_control_without_binding_to_the_hurry_card():
    _, _, final = action_scene("hurry_up", variant="level-eight")
    target = final.get_hero("hero_brogan")
    assert target.current_turn_card.initiative_override == 11
    control = next(
        effect
        for effect in final.active_effects
        if effect.effect_type.value == "control_next_action"
    )
    assert control.source_card_id is None
    assert control.controlled_card_id == target.current_turn_card.id


@pytest.mark.parametrize(
    "effect", ["helping_hand", "even_the_odds", "trusted_sidekick", "outnumber", "pile_on"]
)
@pytest.mark.parametrize(
    "variant,expected", [("range-zero", 1), ("range-none", 1), ("range-negative", -1)]
)
def test_attack_builder_default_range_preserves_negative_values(effect, variant, expected):
    from goa2.engine.effects import CardEffectRegistry

    state, hero, card = base(effect, variant)
    steps = CardEffectRegistry.get(effect).get_steps(state, hero, card)
    attacks = [step for step in steps if step.type.value == "attack_sequence"]
    ranged = [step for step in attacks if step.target_filters]
    assert ranged and all(step.range_val == expected for step in ranged)
