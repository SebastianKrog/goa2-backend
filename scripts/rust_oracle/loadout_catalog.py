"""Export static upgrade provenance and differential cases for Rust search."""

import json

from model_traces import setup
from oracle import ROOT, verify_sources

from goa2.data.heroes.registry import HeroRegistry
from goa2.domain.card_knowledge import (
    enumerate_static_loadout_hypotheses, has_standard_loadout_provenance,
)
from goa2.domain.models import StatType


def export():
    source = verify_sources()
    setup()
    heroes = {}
    for name in sorted(HeroRegistry.list_heroes(include_playtest=True)):
        hero = HeroRegistry.get(name)
        heroes[name] = {
            "standard": has_standard_loadout_provenance(hero),
            "starting": sorted(
                str(card.id) for card in hero.deck
                if card.tier.value in {"UNTIERED", "I"} and not card.starts_in_deck
            ),
            "cards": [
                dict(id=str(card.id), color=card.color.value, tier={"I": 1, "II": 2, "III": 3}[card.tier.value],
                     item=card.item.value if card.item else None, starts_in_deck=card.starts_in_deck)
                for card in hero.deck
                if card.color and card.color.value in {"RED", "BLUE", "GREEN"}
                and card.tier.value in {"I", "II", "III"}
            ],
        }
        if name == "Takahide":
            heroes[name]["cyclic_golds"] = sorted(
                str(card.id) for card in hero.deck if card.color and card.color.value == "GOLD"
            )
    cases = []
    for name in ("Arien", "Brogan", "Hanu", "Gydion"):
        hero = HeroRegistry.get(name)
        unfiltered = enumerate_static_loadout_hypotheses(hero, 5)
        reveal = unfiltered[0].active_card_ids[-1]
        for level, items, reveals, extra in [
            (1, None, [], False), (5, None, [reveal], False), (8, None, [], False),
            (3, {StatType.ATTACK: 2}, [], False),
            (3, {StatType.ATTACK: 2, StatType.DEFENSE: 2}, [], True),
        ]:
            result = enumerate_static_loadout_hypotheses(
                hero, level, public_items=items, revealed_card_ids=tuple(reveals),
                allow_extra_public_items=extra,
            )
            cases.append(dict(hero=name, level=level, items=items, reveals=reveals, extra=extra,
                              expected=[row.model_dump(mode="json") for row in result]))
    out = ROOT / "rust/content/catalogs/loadout-provenance.json"
    out.write_text(json.dumps(dict(source_sha256=source["source_sha256"], heroes=heroes, cases=cases),
                              separators=(",", ":")) + "\n", encoding="utf-8")


if __name__ == "__main__":
    export()
