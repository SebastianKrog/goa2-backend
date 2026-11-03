"""Frozen marker contracts: physical identity, immunity, expiry, substitution."""

import pytest
from live_token_traces import scenes
from object_execution_traces import reference


@pytest.fixture(scope="module")
def token_scenes():
    return {name: (state, operations) for name, state, operations in scenes()}


def test_placement_updates_the_actual_physical_supply_only(token_scenes):
    for copied in (False, True):
        for immune in (False, True):
            state, operations = token_scenes[f"token-query-{copied}-{immune}"]
            reference(state, operations)
            ctx = state.execution_context
            token, supply = ctx["token_alias"], ctx["pool_alias"]
            assert (token is not supply) == copied
            assert supply.owner_id == "hero_wuk"
            assert supply.is_immune_to_enemy_actions == immune
            assert token.owner_id == ("hero_brogan" if copied else "hero_wuk")
            assert token.is_immune_to_enemy_actions == (False if copied else immune)
            assert ctx["own_units"] == [str(token.id)]
            assert ctx["enemy_units"] == ([] if immune and not copied else [str(token.id)])


def test_remove_expires_token_links_without_running_finishing_steps(token_scenes):
    for variant in ("remove", "relocate", "offboard", "overflow"):
        state, operations = token_scenes["token-effects-" + variant]
        reference(state, operations)
        ctx = state.execution_context
        removed = variant in {"remove", "overflow"}
        assert len(state.active_effects) == (1 if removed else 4)
        assert (state.active_effects is not ctx["old_effects"]) == removed
        assert len(ctx["old_effects"]) == 4
        assert "forbidden" not in ctx
        assert ctx["card_alias"].is_active
        assert state.entity_locations is ctx["locations_alias"]


def test_substitution_is_actor_scoped_and_retry_preserves_source_quirk(token_scenes):
    for overflow in (False, True):
        for applies in (False, True):
            state, operations = token_scenes[f"token-substitution-{overflow}-{applies}"]
            reference(state, operations)
            token = state.misc_entities[state.execution_context["placed"]]
            assert token.token_type.value == ("illusion" if applies else "totem")
            # Overflow authors a fresh PlaceTokenStep for the effective kind;
            # its retry retains the authored immunity, unlike direct substitution.
            assert token.is_immune_to_enemy_actions == (not applies or overflow)
            assert state.execution_stack == []


def test_falsy_owner_values_do_not_fall_back_to_actor(token_scenes):
    for label, owner in (("empty", ""), ("false", "False"), ("zero", "0")):
        state, operations = token_scenes["token-owner-" + label]
        records = reference(state, operations)
        assert state.execution_context["token_alias"].owner_id == owner
        events = [event for record in records for event in record["events"]]
        assert len(events) == 1
        assert events[0]["actor_id"] is None
