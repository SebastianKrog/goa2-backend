"""Frozen retrieval equality, expiry, alias, and complete card-body contracts."""

from live_retrieval_traces import scenes
from object_execution_traces import reference


def test_retrieval_uses_post_state_model_equality_and_private_revisions():
    wanted = {
        "retrieve-equal-discard",
        "retrieve-equal-current",
        "retrieve-revision-current",
        "retrieve-metadata-current",
    }
    checked = set()
    for name, state, operations in scenes():
        if name not in wanted:
            continue
        hero = state.get_hero("hero_arien")
        target = state.execution_context["target"]
        clone = state.execution_context["clone"]
        reference(state, operations)
        assert any(c is target for c in hero.hand)
        if name == "retrieve-equal-discard":
            assert hero.discard_pile == []
        elif name == "retrieve-revision-current":
            assert hero.current_turn_card is clone
        else:
            assert hero.current_turn_card is None
        assert target is not clone
        checked.add(name)
        if checked == wanted:
            break
    assert checked == wanted


def test_retrieve_all_preserves_snapshot_order_and_premature_expiry():
    checked = 0
    for name, state, operations in scenes():
        if name not in {"retrieve-all-printed", "retrieve-all-effects"}:
            continue
        hero = state.get_hero("hero_arien")
        original = list(hero.discard_pile)
        old_effects = state.execution_context["old_effects"]
        records = reference(state, operations)
        assert all(a is b for a, b in zip(hero.hand[-2:], original, strict=True))
        assert hero.discard_pile == []
        events = [e for record in records for e in record["events"]]
        assert [e["metadata"]["card_id"] for e in events] == [c.id for c in original]
        assert "forbidden" not in state.execution_context
        assert state.active_effects is not old_effects
        if name == "retrieve-all-printed":
            assert [c.is_active for c in original] == [False, True]
        else:
            assert len(old_effects) == 3 and len(state.active_effects) == 1
        checked += 1
    assert checked == 2


def test_shared_role_lists_keep_identity_after_retrieval():
    checked = 0
    for name, state, operations in scenes():
        if name not in {"retrieve-shared-discard-played", "retrieve-shared-hand-discard"}:
            continue
        hero = state.get_hero("hero_arien")
        other = next(h for team in state.teams.values() for h in team.heroes if h.id != hero.id)
        reference(state, operations)
        if name.endswith("discard-played"):
            assert other.played_cards is hero.discard_pile and hero.discard_pile == []
        else:
            assert other.discard_pile is hero.hand
        checked += 1
    assert checked == 2


def test_source_built_query_retrieval_bodies_complete_when_enabled_or_skipped():
    checked = 0
    for name, state, operations in scenes():
        if not name.startswith("retrieve-card-"):
            continue
        hero = state.get_hero(state.current_actor_id)
        returned = hero.discard_pile[0]
        enabled = "-True-" in name
        records = reference(state, operations)
        assert state.execution_stack == []
        events = [
            e for record in records for e in record["events"] if e["event_type"] == "CARD_RETRIEVED"
        ]
        assert len(events) == int(enabled)
        if enabled:
            assert any(c is returned for c in hero.hand)
        else:
            assert hero.discard_pile[0] is returned
        checked += 1
    assert checked == 18
