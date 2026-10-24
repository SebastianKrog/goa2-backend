"""Offline boundary tests; no neural dependencies or running server."""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))
from state_codec import InstanceEncoder, materialize, reject_reserved

from goa2.domain.models import Card, Token, TokenType


def test_duplicate_printed_cards_preserve_physical_identity_and_overrides():
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.setup import GameSetup  # noqa: F401 - registers hero data

    card = HeroRegistry.get("Arien").deck[0]
    assert isinstance(card, Card)
    duplicate = card.model_copy(deep=True)
    scenario = card.model_copy(deep=True)
    scenario.range_value = 17
    encoder = InstanceEncoder()
    first, alias, separate, overridden = [
        encoder.pack_card(value) for value in (card, card, duplicate, scenario)
    ]
    assert first == alias
    assert first != separate
    assert (
        encoder.instances[first["$goa2_instance"]]["definition"]
        == encoder.instances[separate["$goa2_instance"]]["definition"]
    )
    assert (
        encoder.instances[first["$goa2_instance"]]["definition"]
        != encoder.instances[overridden["$goa2_instance"]]["definition"]
    )
    assert len(encoder.instances) == 3
    assert len(encoder.definitions) == 2


def test_tokens_preserve_aliases_without_guessing_by_entity_id():
    token = Token(id="totem_1", name="Totem", token_type=TokenType.TOTEM)
    duplicate = token.model_copy(deep=True)
    encoder = InstanceEncoder()
    first = encoder.pack_token(token)
    assert encoder.pack_token(token) == first
    assert encoder.pack_token(duplicate) != first
    assert len(encoder.definitions) == 1
    envelope = {
        "definitions": encoder.definitions,
        "instances": encoder.instances,
        "state": [first, first],
    }
    assert materialize(envelope) == [token.model_dump(mode="json")] * 2


def test_reserved_keys_rejected_in_nested_scenario_metadata():
    with pytest.raises(ValueError, match="reserved boundary key"):
        reject_reserved({"metadata": [{"$goa2_instance": 0}]})


def test_definition_interning_preserves_secondary_action_order():
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import ActionType
    from goa2.engine.setup import GameSetup  # noqa: F401 - loads data

    card = HeroRegistry.get("Arien").deck[0]
    card.secondary_actions = {ActionType.HOLD: 0, ActionType.MOVEMENT: 2}
    other = card.model_copy(deep=True)
    other.secondary_actions = dict(reversed(list(card.secondary_actions.items())))
    encoder = InstanceEncoder()
    first = encoder.pack_card(card)
    second = encoder.pack_card(other)
    assert (
        encoder.instances[first["$goa2_instance"]]["definition"]
        != encoder.instances[second["$goa2_instance"]]["definition"]
    )


def test_export_sidecar_preserves_real_objects_for_physical_instance_oracles():
    from state_codec import export_state_with_instances

    from goa2.engine.setup import GameSetup

    state = GameSetup.create_game(
        "src/goa2/data/maps/forgotten_island.json", ["Arien"], ["Hanu"], seed=29
    )
    envelope, objects = export_state_with_instances(state)
    hero = state.get_hero("hero_arien")
    encoded = envelope["state"]["teams"]["RED"]["heroes"][0]
    index = encoded["hand"][0]["$goa2_instance"]
    assert objects[index] is hero.hand[0]
    assert any(reference["$goa2_instance"] == index for reference in encoded["deck"])
