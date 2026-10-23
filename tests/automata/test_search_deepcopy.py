"""Bulk model copying must retain custom hooks, cycles and mutable aliases."""

from pydantic import BaseModel, ConfigDict, Field, PrivateAttr

from automata.runtime.deepcopy import search_deepcopy


class Payload(BaseModel):
    model_config = ConfigDict(extra="allow")
    values: list = Field(default_factory=list)
    _private: dict = PrivateAttr(default_factory=dict)


def test_mutable_model_fields_extras_privates_and_cycles_stay_independent():
    shared = [1, {"nested": [2]}]
    model = Payload(values=shared)
    model.extra_value = model.values
    model._private["shared"] = model.values
    model.values.append(model)
    copied = search_deepcopy(model, {})
    assert copied is not model
    assert copied.values is copied.extra_value is copied._private["shared"]
    assert copied.values[-1] is copied
    copied.values[1]["nested"].append(3)
    assert model.values[1]["nested"] == [2]
    assert copied.model_fields_set == model.model_fields_set
    copied.__pydantic_fields_set__.add("probe")
    assert "probe" not in model.model_fields_set


def test_custom_copy_hook_is_used_and_memo_preserves_aliases():
    class Custom(Payload):
        def __deepcopy__(self, memo=None):
            result = super().__deepcopy__(memo)
            result.values.append("custom")
            return result

    model = Custom(values=[1])
    copied = search_deepcopy([model, model], {})
    assert copied[0] is copied[1]
    assert copied[0].values == [1, "custom"]
    assert model.values == [1]


def test_tuple_container_aliases_and_cycles_follow_python_deepcopy():
    items = []
    linked = (items,)
    items.append(linked)
    copied = search_deepcopy({"a": linked, "b": linked}, {})
    assert copied["a"] is copied["b"]
    assert copied["a"][0][0] is copied["a"]
    assert copied["a"][0] is not items
