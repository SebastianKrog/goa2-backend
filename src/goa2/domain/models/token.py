from __future__ import annotations

from collections.abc import Iterable
from typing import Literal

from goa2.domain.models.base import BoardEntity
from goa2.domain.models.enums import TokenType
from goa2.domain.types import HeroID

TOKEN_SUPPLY: dict[TokenType, int] = {
    TokenType.SMOKE_BOMB: 1,
    TokenType.GRENADE: 1,
    TokenType.MINE_BLAST: 2,
    TokenType.MINE_DUD: 2,
    TokenType.ZOMBIE: 4,
    TokenType.PYRO: 1,
    TokenType.BARRIER: 3,
    TokenType.ICE: 3,
    TokenType.TOTEM: 1,
    TokenType.TREE: 3,
    TokenType.ROCK: 3,
    TokenType.MAGMA: 4,
    TokenType.GLITCH: 3,
    TokenType.ILLUSION: 3,
    TokenType.FAMILIAR: 1,
}


# Include every upgrade/spell, not just the hero's current hand or level.
HERO_TOKEN_TYPES: dict[str, frozenset[TokenType]] = {
    "Min": frozenset(
        {TokenType.SMOKE_BOMB, TokenType.GRENADE, TokenType.MINE_BLAST, TokenType.MINE_DUD}
    ),
    "Mortimer": frozenset({TokenType.ZOMBIE}),
    "Widget": frozenset({TokenType.PYRO}),
    "Trinkets": frozenset({TokenType.BARRIER}),
    "Tali": frozenset({TokenType.ICE, TokenType.TOTEM}),
    "Wuk": frozenset({TokenType.TREE}),
    "Mrak": frozenset({TokenType.ROCK}),
    "Ignatia": frozenset({TokenType.MAGMA}),
    "Emmitt": frozenset({TokenType.GLITCH}),
    "NebKher": frozenset({TokenType.ILLUSION}),
    "Gydion": frozenset({TokenType.FAMILIAR}),
}


def roster_token_types(hero_names: Iterable[str]) -> set[TokenType]:
    """Supply needed for all future actions available to this roster."""
    return {kind for name in hero_names for kind in HERO_TOKEN_TYPES.get(name, ())}


class Token(BoardEntity):
    """
    A BoardEntity that represents an object on the board that is NOT a Unit.
    Inherits `id` and `name` from GameEntity/BoardEntity.
    """

    entity_kind: Literal["token"] = "token"
    token_type: TokenType
    owner_id: HeroID | None = None
    is_passable: bool = False
    is_facedown: bool = False
    persists_end_of_round: bool = False
    is_immune_to_enemy_actions: bool = False
