"""Spell lifecycle source contracts; raw setup preserves physical aliases."""

from live_spellbook_traces import abort_scenes, cast_scene, prepare_scene, remove_scene, wish_scene
from object_execution_traces import reference


def events(records):
    return [event for record in records for event in record["events"]]


def test_cast_spends_the_same_physical_spell_and_queues_suppressed_copied_action():
    from goa2.domain.models import CardState

    state, ops = cast_scene("ordinary")
    owner = state.get_hero("hero_gydion")
    spell = next(s for s in owner.spells if s.id == "magic_missile")
    records = reference(state, ops)
    assert spell.state == CardState.OUTSIDE_SPELLBOOK and not spell.is_facedown
    assert state.execution_context["spells_alias"] is owner.spells
    assert len(owner.spells) == 22
    child = state.execution_stack[-1]
    assert child.type.value == "perform_card_action"
    assert child.hero_id == owner.id and child.suppress_after_resolve_card
    assert child.card_key == "cast_spell_id" and child.card_owner_key == "spell_owner_id"
    assert [e["event_type"] for e in events(records)] == ["SPELL_CAST"]


def test_caster_key_overrides_literal_and_falsey_key_falls_back():
    for variant in ("literal", "key", "false-key"):
        state, ops = cast_scene(variant)
        records = reference(state, ops)
        assert state.execution_context["spell_caster_id"] == "hero_hanu"
        assert state.execution_context["spell_owner_id"] == "hero_gydion"
        event = events(records)[0]
        assert event["actor_id"] == "hero_hanu"
        assert event["metadata"]["owner_id"] == "hero_gydion"


def test_accepted_invalid_cast_reissues_request_without_spending():
    state, ops = cast_scene("invalid")
    records = reference(state, ops)
    prompts = [r["request"] for r in records if r["request"]]
    assert len(prompts) == 2 and prompts[0]["id"] != prompts[1]["id"]
    assert prompts[0]["decision_context"]["purpose"] == "cast_spell"
    assert len(events(records)) == 1


def test_wish_counts_once_and_checks_victory_after_the_nested_cast_finishes():
    state, ops = wish_scene("third")
    records = reference(state, ops)
    assert state.get_hero("hero_gydion").wish_cast_count == 3
    assert state.phase.value == "GAME_OVER"
    kinds = [e["event_type"] for e in events(records)]
    assert kinds[0] == "WISH_CAST_COUNT_CHANGED" and kinds[-1] == "GAME_OVER"
    state, ops = wish_scene("perform")
    reference(state, ops)
    assert state.get_hero("hero_gydion").wish_cast_count == 1
    assert state.execution_context["wish_cast_counted_caster_id"] == "hero_gydion"
    assert state.execution_stack[-1].type.value == "perform_card_action"


def test_wish_completion_removes_only_its_own_counted_marker():
    for variant in ("right-marker", "wrong-marker"):
        state, ops = wish_scene(variant)
        reference(state, ops)
        assert ("wish_cast_counted_caster_id" in state.execution_context) == (
            variant == "wrong-marker"
        )
        assert state.get_hero("hero_gydion").wish_cast_count == 2


def test_optional_removal_obeys_limits_and_never_casts():
    for variant, amount in (
        ("ordinary", 3),
        ("one-limit", 1),
        ("one", 1),
        ("skip", 0),
        ("invalid", 3),
        ("unknown", 3),
        ("pre-removed", 2),
    ):
        state, ops = remove_scene(variant)
        before = len(state.get_hero("hero_gydion").cast_spells)
        records = reference(state, ops)
        emitted = events(records)
        assert len(emitted) == amount
        assert all(e["event_type"] == "SPELL_REMOVED_FROM_SPELLBOOK" for e in emitted)
        assert (
            state.execution_context["removed_alias"]
            is state.execution_context["spellbook_step"].removed_spell_ids
        )
        assert len(state.get_hero("hero_gydion").cast_spells) == before + amount


def test_prepare_returns_spells_sorted_and_expires_only_their_effects_without_detaching_cards():
    state, ops = prepare_scene("effects")
    owner = state.get_hero("hero_gydion")
    old_effects = state.active_effects
    physical = owner.spells[0]
    records = reference(state, ops)
    assert state.active_effects is not old_effects
    assert state.execution_context["effects_alias"] is old_effects and len(old_effects) == 3
    assert len(state.active_effects) == 1
    assert owner.spells[0] is physical
    assert len(owner.spellbook) == 22 and not owner.cast_spells
    assert not physical.is_active and owner.spells[1].is_active
    event = events(records)[0]
    assert event["event_type"] == "SPELLBOOK_PREPARED"
    assert event["metadata"]["returned_spell_ids"] == sorted(
        event["metadata"]["returned_spell_ids"]
    )


def test_nested_mandatory_abort_preserves_wish_completion():
    for name, state, ops in abort_scenes():
        records = reference(state, ops)
        assert "wish_cast_counted_caster_id" not in state.execution_context
        if name.endswith("-3"):
            assert state.phase.value == "GAME_OVER"
            assert events(records)[-1]["metadata"]["condition"] == "WISH"
        else:
            assert state.execution_context["after_wish_abort"] is True


def test_piece_cast_preserves_raw_id_but_wish_uses_the_owner_counter_and_event():
    state, ops = cast_scene("piece")
    records = reference(state, ops)
    assert events(records)[0]["actor_id"] == "hero_razzle_piece_1"
    state, ops = wish_scene("piece")
    records = reference(state, ops)
    assert state.get_hero("hero_razzle").wish_cast_count == 1
    assert state.get_hero("hero_gydion").wish_cast_count == 0
    assert events(records)[0]["actor_id"] == "hero_razzle"
