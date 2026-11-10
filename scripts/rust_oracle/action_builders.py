"""Freeze public action-step constructors with nested object identities."""

from __future__ import annotations

from fixture_cache import write_text

import json

from oracle import ROOT, digest, verify_sources

OWNER = "template-passive-owner"
CARD = "template-action-card"
VALUE = 62003
RANGE = 62007
KEY = "template-action-key"
DATA = "template-action-value"
LOG = "template-action-log"


def catalog():
    from model_traces import setup
    from shared_objects import capture

    from goa2.domain.models import StatType, TargetType
    from goa2.engine.filters import ExcludeIdentityFilter
    from goa2.engine.filters_hex import FastTravelDestinationFilter
    from goa2.engine.steps import (
        AttackSequenceStep,
        CheckPassiveAbilitiesStep,
        ChooseActingPieceStep,
        FastTravelSequenceStep,
        FastTravelUnitStep,
        LogMessageStep,
        MoveSequenceStep,
        ResolveCardTextStep,
        ResolvePreActionDiscardStep,
        ResolvePreActionMovementStep,
        RestoreActionContextStep,
        SelectStep,
        SetContextFlagStep,
    )
    from goa2.engine.steps.cards import _clear_steps

    state = setup()
    hero = state.get_hero("hero_arien")
    # Clear construction checks board presence, unlike other constructors.
    point = next(iter(state.board.tiles))
    state.place_entity(hero.id, point)
    groups = {
        "restore_context": [RestoreActionContextStep()],
        "restore_other_context": [RestoreActionContextStep(other_hero_action=True)],
        "piece": [ChooseActingPieceStep(hero_id=OWNER)],
        "passive": [CheckPassiveAbilitiesStep(trigger=DATA)],
        "pre_move": [ResolvePreActionMovementStep(hero_id=OWNER)],
        "pre_discard": [ResolvePreActionDiscardStep(hero_id=OWNER)],
        "text": [ResolveCardTextStep(hero_id=OWNER, card_id=CARD)],
        "movement": [
            MoveSequenceStep(unit_id=OWNER, range_val=VALUE, range_stat_type=StatType.MOVEMENT)
        ],
        "travel": [FastTravelSequenceStep(unit_id=OWNER, source_card_id=CARD)],
        "travel_unit": [FastTravelUnitStep(unit_id=OWNER, source_card_id=CARD, is_mandatory=False)],
        "travel_select": [
            SelectStep(
                target_type=TargetType.HEX,
                prompt="Select Fast Travel Destination",
                filters=[FastTravelDestinationFilter(unit_id=OWNER, source_card_id=CARD)],
                is_mandatory=False,
            )
        ],
        "attack": [
            AttackSequenceStep(
                damage=VALUE,
                range_val=RANGE,
                damage_stat_type=StatType.ATTACK,
                range_stat_type=StatType.RANGE,
            )
        ],
        "log": [LogMessageStep(message=LOG)],
        "context": [SetContextFlagStep(key=KEY, value=DATA)],
        "clear": _clear_steps(state, str(hero.id)),
    }

    def labels(value):
        if isinstance(value, dict):
            return {
                key: "template" if key == "step_id" else labels(child)
                for key, child in value.items()
            }
        if isinstance(value, list):
            return [labels(child) for child in value]
        return value

    recipes = {}
    for name, steps in groups.items():
        rows = []
        for step in steps:
            fields = step.model_dump(mode="json")
            objects, _ = capture(step, fields)
            rows.append({"fields": labels(fields), "objects": objects})
        recipes[name] = rows
    exclusion = ExcludeIdentityFilter(exclude_self=False, exclude_keys=[KEY]).model_dump(
        mode="json"
    )
    source = {
        "schema_version": 1,
        "source_sha256": verify_sources()["source_sha256"],
        "recipes": recipes,
        "exclusion_filter": exclusion,
        "operands": {
            "owner": OWNER,
            "card": CARD,
            "value": VALUE,
            "range": RANGE,
            "key": KEY,
            "data": DATA,
            "log": LOG,
        },
    }
    return {**source, "catalog_sha256": digest(source)}


if __name__ == "__main__":
    payload = catalog()
    write_text(ROOT / "rust/content/catalogs/action-builders.json", json.dumps(payload, indent=2) + "\n")
    print({"recipes": len(payload["recipes"]), "catalog_sha256": payload["catalog_sha256"]})
