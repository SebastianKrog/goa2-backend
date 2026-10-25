"""Source identity contracts needed by stored finishing steps and snapshots."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))


def test_equal_step_values_and_ids_do_not_erase_independent_physical_identity():
    from object_arena_traces import scenes

    checked = set()
    for name, state, envelope, _objects, _operations in scenes():
        if name not in ["step", "independent-steps"]:
            continue
        assert envelope["state"]["execution_stack"][0] == envelope["state"]["execution_stack"][1]
        bindings = [binding for binding in envelope["object_bindings"] if binding["kind"] == "step"]
        assert len(bindings) == (1 if name == "step" else 2)
        assert (state.execution_stack[0] is state.execution_stack[1]) == (name == "step")
        assert state.execution_stack[0] is state.active_effects[0].finishing_steps[0]
        checked.add(name)
    assert checked == {"step", "independent-steps"}


def test_shallow_snapshots_share_mutable_children_but_retain_prior_scalars():
    from object_arena_traces import reference, scenes

    _name, state, _envelope, objects, operations = next(
        scene for scene in scenes() if scene[0] == "snapshot"
    )
    records = reference(state, objects, operations[:3])
    context = records[-1]["semantic_state"]["execution_context"]
    assert context["scalar"] == 2 and context["old"]["scalar"] == 1
    assert context["items"] == context["old"]["items"] == [1, 2, 3]
    assert state.execution_context["items"] is state.execution_context["old"]["items"]
    paths = [binding["paths"] for binding in records[-1]["object_bindings"]]
    assert [["execution_context", "items"], ["execution_context", "old", "items"]] in paths
