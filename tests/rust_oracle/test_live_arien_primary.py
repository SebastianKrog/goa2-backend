"""Source semantic contracts for complete Arien primary bodies and geometry."""

from live_arien_primary_traces import line_scenes, state_for
from live_defeat_traces import operations
from object_execution_traces import reference


def run_action(effect, empty_hand=False):
    from goa2.domain.input import selection_value
    from goa2.engine.steps import ResolveCardStep, SetContextFlagStep

    state, hero, card, _ = state_for(effect)
    if empty_hand:
        state.get_hero("hero_brogan").hand = []
    state.execution_stack = [
        SetContextFlagStep(key="tail", value=True),
        ResolveCardStep(hero_id=str(hero.id)),
    ]

    def select(request, _mode):
        if request.request_type.value == "CHOOSE_ACTION":
            return card.current_primary_action.value
        if request.request_type.value == "SELECT_CARD_OR_PASS":
            return "PASS"
        return selection_value(request.options[0])

    ops, _ = operations(state, select=select)
    return state, reference(state, ops)


def test_arien_effect_creation_keeps_card_binding_and_movement():
    for effect in ("spell_break", "slippery_ground", "deluge"):
        state, records = run_action(effect)
        assert state.execution_context["tail"]
        created = state.active_effects[0]
        assert created.source_card_id == effect
        assert created.source_id == "hero_arien"
        if effect == "spell_break":
            assert [a.value for a in created.restrictions] == ["SKILL"]
            assert [c.value for c in created.except_card_colors] == ["GOLD"]
        else:
            assert created.max_value == 1 and created.limit_actions_only
            assert [a.value for a in created.restrictions] == ["FAST_TRAVEL"]
            assert any(e["event_type"] == "UNIT_MOVED" for row in records for e in row["events"])


def test_ebb_repeats_and_backstab_attacks_discard_real_cards():
    state, records = run_action("ebb_and_flow")
    assert sum(e["event_type"] == "UNITS_SWAPPED" for row in records for e in row["events"]) == 2
    for effect in ("dangerous_current", "raging_stream", "violent_torrent"):
        state, records = run_action(effect)
        context = state.execution_context
        assert context["discarded_card_owner_id"] in ("hero_brogan", "hero_tigerclaw")
        victim = state.get_hero(context["discarded_card_owner_id"])
        assert context["discarded_card_id"] in {card.id for card in victim.discard_pile}
        assert context["discarded_card_id"] not in {card.id for card in victim.hand}
        assert any(e["event_type"] == "COMBAT_RESOLVED" for row in records for e in row["events"])


def test_backstab_defeats_a_hero_who_cannot_discard():
    state, records = run_action("dangerous_current", empty_hand=True)
    assert "hero_brogan" not in state.entity_locations
    assert any(
        e["event_type"] == "UNIT_DEFEATED" and e["target_id"] == "hero_brogan"
        for row in records
        for e in row["events"]
    )


def test_behind_target_uses_direct_identity_and_does_not_include_target_space():
    wanted = {
        "zero",
        "negative",
        "no-target",
        "unknown-target",
        "hex-target",
        "no-origin",
        "origin-unknown",
        "not-straight",
    }
    checked = set()
    for name, state, ops in line_scenes():
        if "-edge-" not in name:
            continue
        variant = name.rsplit("-edge-", 1)[1]
        record = reference(state, ops)[0]
        if variant in wanted:
            assert record["request"] is None, variant
            checked.add(variant)
        if variant == "on-target":
            request = record["request"]
            assert request is not None
            assert all(
                option["metadata"]["hex"] != {"q": 2, "r": 0, "s": -2}
                for option in request["options"]
            )
    assert checked == wanted
