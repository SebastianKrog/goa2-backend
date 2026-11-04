"""Raw builder metadata is audited against both frozen Python source APIs."""

from copy import deepcopy

import pytest
from primary_builders import CARD, catalog, raw_binding_paths


def apply(rows, paths):
    result = deepcopy(rows)
    for row, root_paths in zip(result, paths, strict=True):
        for path in root_paths:
            value = row["fields"]
            for key in path[:-1]:
                value = value[key]
            assert value[path[-1]] == CARD
            value[path[-1]] = None
    return result


def test_raw_bindings_cover_all_reviewed_pure_and_stateful_builders():
    data = catalog()
    assert set(data["raw_card_bindings"]) == set(data["recipes"])
    assert len(data["raw_card_bindings"]) == 447
    for effect, rows in data["recipes"].items():
        paths = data["raw_card_bindings"][effect]
        raw = apply(rows, paths)
        assert raw_binding_paths(rows, raw) == paths
    assert len(data["stateful_recipes"]) == 101
    for body in data["stateful_recipes"].values():
        for name, rows in body["variants"].items():
            paths = body["raw_card_bindings"][name]
            assert raw_binding_paths(rows, apply(rows, paths)) == paths


def test_raw_binding_audit_rejects_body_and_physical_alias_drift():
    row = {"fields": {"type": "create_effect", "source_card_id": CARD}, "objects": []}
    with pytest.raises(AssertionError, match="explicit native port"):
        raw_binding_paths([row], [{**row, "fields": {**row["fields"], "type": "log"}}])
    with pytest.raises(AssertionError, match="aliases changed"):
        raw_binding_paths([row], [{**row, "objects": [{"kind": "model"}]}])


def test_raw_attribution_paths_are_audited_from_actual_source_fields():
    data = catalog()
    paths = data["raw_card_bindings"]["death_grenade"]
    assert paths == [[], [], [["source_card_id"]]]
    assert any(data["raw_card_bindings"]["inner_strength"])


def test_flurry_is_raw_while_cloak_attributes_effects_to_the_repeated_card():
    from live_dynamic_passive_traces import scenes
    from object_execution_traces import reference

    wanted = {
        "dynamic-passive-flurry_of_blows-inner_strength-ordinary": None,
        "dynamic-passive-cloak_and_daggers-inner_strength-ordinary": "repeat-source",
    }
    for name, state, ops in scenes(wanted):
        if name not in wanted:
            continue
        reference(state, ops)
        effects = [s for s in state.execution_stack if s.type.value == "create_effect"]
        assert effects and all(s.source_card_id == wanted[name] for s in effects)
        assert state.execution_context["current_card_id"] == "outer-action"
        del wanted[name]
    assert not wanted


def test_complete_repeats_reach_an_actual_attack_and_preserve_outer_card():
    from live_dynamic_passive_traces import scenes
    from object_execution_traces import reference

    wanted = {
        "dynamic-passive-flurry_of_blows-crane_stance-complete",
        "dynamic-passive-cloak_and_daggers-crane_stance-complete",
    }
    for name, state, ops in scenes(wanted):
        if name not in wanted:
            continue
        records = reference(state, ops)
        assert any(e["event_type"] == "COMBAT_RESOLVED" for row in records for e in row["events"])
        assert state.execution_context["current_card_id"] == "outer-action"
        assert not state.execution_context.get("is_flurry_repeat")
        wanted.remove(name)
    assert not wanted


def test_flurry_keeps_negative_attack_values_and_exact_root_exclusion_defaults():
    from live_dynamic_passive_traces import scenes
    from object_execution_traces import reference

    name = "dynamic-passive-flurry_of_blows-crane_stance-negative-attack"
    _, state, ops = next(scenes({name}))
    reference(state, ops)
    attack = next(s for s in state.execution_stack if s.type.value == "attack_sequence")
    assert attack.damage < 0
    assert attack.is_mandatory
    exclusion = attack.target_filters[-1]
    assert exclusion.exclude_keys == ["last_flurry_target"] and exclusion.exclude_self


def test_explicit_registry_override_keeps_physical_card_spell_access():
    from live_dynamic_passive_traces import scenes
    from object_execution_traces import reference

    name = "dynamic-passive-flurry_of_blows-cantrip-registry-override"
    _, state, ops = next(scenes({name}))
    reference(state, ops)
    cast = next(s for s in state.execution_stack if s.type.value == "cast_spell")
    assert cast.allowed_spell_ids == []
