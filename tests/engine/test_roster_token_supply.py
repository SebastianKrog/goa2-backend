"""Roster-scoped supply at setup and after restoring older checkpoints."""
import ast
from pathlib import Path

import pytest

from goa2.domain.hex import Hex
from goa2.domain.models import Token, TokenType
from goa2.domain.models.base import Turret
from goa2.domain.models.token import HERO_TOKEN_TYPES, TOKEN_SUPPLY
from goa2.domain.state import GameState
from goa2.engine.setup import GameSetup

MAP = "src/goa2/data/maps/forgotten_island.json"


@pytest.mark.parametrize("hero,required", HERO_TOKEN_TYPES.items())
def test_every_token_hero_receives_full_future_supply(hero, required):
    state = GameSetup.create_game(MAP, [hero], ["Arien"], seed=1)
    assert set(state.token_pool) == required
    assert len(state.misc_entities) == sum(TOKEN_SUPPLY[kind] for kind in required)
    for kind in required:
        assert len(state.token_pool[kind]) == TOKEN_SUPPLY[kind]
    restored = GameState.model_validate_json(state.model_dump_json())
    assert set(restored.token_pool) == required
    for tokens in restored.token_pool.values():
        for token in tokens:
            assert restored.misc_entities[token.id] is token


def test_supply_catalog_covers_effect_modules():
    # Catch new token-capable heroes/types missing from roster supply metadata.
    for path in Path("src/goa2/scripts").glob("*_effects.py"):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        used = {
            TokenType[node.attr] for node in ast.walk(tree)
            if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name)
            and node.value.id == "TokenType"
        }
        declared = next((types for hero, types in HERO_TOKEN_TYPES.items()
                         if hero.lower() == path.stem.removesuffix("_effects")), frozenset())
        assert used <= declared, (path.name, used - declared)
    assert set().union(*HERO_TOKEN_TYPES.values()) == set(TokenType)


def test_restore_prunes_unused_inventory_but_preserves_live_scenario_entities():
    state = GameSetup.create_game(MAP, ["Arien"], ["Wasp"], seed=1)
    unused = Token(id="unused", name="Ice", token_type=TokenType.ICE)
    deployed = Token(id="deployed", name="Mine", token_type=TokenType.MINE_BLAST)
    reserve = Token(id="reserve", name="Mine", token_type=TokenType.MINE_BLAST)
    owned = Token(id="owned", name="Familiar", token_type=TokenType.FAMILIAR,
                  owner_id="hero_arien")
    turret = Turret(id="turret", name="Turret", owner_id="hero_arien")
    state.token_pool = {TokenType.ICE: [unused], TokenType.MINE_BLAST: [deployed, reserve],
                        TokenType.FAMILIAR: [owned]}
    state.misc_entities = {entity.id: entity for entity in [unused, deployed, reserve, owned, turret]}
    state.entity_locations[deployed.id] = Hex(q=0, r=0, s=0)
    counter = state.next_entity_id
    restored = GameState.model_validate_json(state.model_dump_json())
    assert set(restored.token_pool) == {TokenType.MINE_BLAST, TokenType.FAMILIAR}
    assert set(restored.misc_entities) == {"deployed", "reserve", "owned", "turret"}
    assert restored.next_entity_id == counter
    assert restored.entity_locations[deployed.id] == state.entity_locations[deployed.id]
    assert restored.misc_entities[deployed.id] is restored.token_pool[TokenType.MINE_BLAST][0]


def test_pruning_preserves_historical_entity_numbering():
    state = GameSetup.create_game(MAP, ["Min"], ["Gydion"], seed=1)
    # Former full allocation: smoke 1, grenade 2, four mines 3..6,
    # zombie 7..10, pyro 11, barrier 12..14, ice 15..17, totem 18,
    # tree 19..21, rock 22..24, magma 25..28, glitch 29..31,
    # illusion 32..34, familiar 35. Minions follow the same counter.
    assert state.token_pool[TokenType.SMOKE_BOMB][0].id == "smoke_bomb_1"
    assert state.token_pool[TokenType.FAMILIAR][0].id == "familiar_35"
    assert {token.id for kind in (TokenType.MINE_BLAST, TokenType.MINE_DUD)
            for token in state.token_pool[kind]} == {"mine_3", "mine_4", "mine_5", "mine_6"}
