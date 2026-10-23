"""Search-private deepcopy with bulk scalar copying for ordinary Pydantic models.

Custom copy hooks and unfamiliar objects retain Python deepcopy semantics.
Mutable containers are memoized before recursion, preserving aliases and cycles.
"""

import copy
from enum import Enum
from functools import cache

from pydantic import BaseModel
from pydantic_core import PydanticUndefined

_ATOMIC = frozenset(
    cls for cls, copier in copy._deepcopy_dispatch.items() if copier is copy._deepcopy_atomic
)
_MISSING = object()


@cache
def _model_copy_is_standard(cls):
    return issubclass(cls, BaseModel) and cls.__deepcopy__ is BaseModel.__deepcopy__


def search_deepcopy(value, memo):
    cls = type(value)
    if cls in _ATOMIC or isinstance(value, Enum):
        return value
    identity = id(value)
    existing = memo.get(identity, _MISSING)
    if existing is not _MISSING:
        return existing
    if cls is list:
        result = []
        memo[identity] = result
        result.extend(search_deepcopy(item, memo) for item in value)
        return result
    if cls is dict:
        result = {}
        memo[identity] = result
        for key, item in value.items():
            copied_key = key if type(key) in _ATOMIC else search_deepcopy(key, memo)
            result[copied_key] = item if type(item) in _ATOMIC else search_deepcopy(item, memo)
        return result
    if cls is set:
        result = set()
        memo[identity] = result
        result.update(search_deepcopy(item, memo) for item in value)
        return result
    if _model_copy_is_standard(cls):
        result = cls.__new__(cls)
        memo[identity] = result
        object.__setattr__(result, "__dict__", search_deepcopy(value.__dict__, memo))
        object.__setattr__(
            result, "__pydantic_extra__", search_deepcopy(value.__pydantic_extra__, memo)
        )
        object.__setattr__(result, "__pydantic_fields_set__", value.__pydantic_fields_set__.copy())
        private = getattr(value, "__pydantic_private__", None)
        if private is not None:
            private = {key: item for key, item in private.items() if item is not PydanticUndefined}
        object.__setattr__(result, "__pydantic_private__", search_deepcopy(private, memo))
        return result
    return copy.deepcopy(value, memo)
