"""Generate recipient-scoped frontend view cases with the pinned Python oracle."""

import argparse
import gzip
import os
from pathlib import Path

from model_traces import setup
from oracle import ROOT
from shared_objects import export_state_with_shared_objects
from state_codec import verified_instance_exporter


def cases():
    from goa2.bootstrap import register_all_effects
    from goa2.domain.input import InputResponse
    from goa2.domain.models import CardState, GamePhase, StatType, TokenType
    from goa2.domain.views import build_view
    from goa2.engine.session import GameSession
    from goa2.engine.setup import GameSetup
    from goa2.engine.steps import ResolveUpgradesStep
    from goa2.server.visibility import (
        awaiting_input_hero_ids,
        events_for_viewer,
        input_request_for_viewer,
    )

    register_all_effects()
    exporter = verified_instance_exporter()

    def project(name, state, request=None, events=()):
        envelope, _ = export_state_with_shared_objects(state, instance_exporter=exporter)
        viewers = [None, *(hero.id for team in state.teams.values() for hero in team.heroes)]
        return {
            "name": name,
            "envelope": envelope,
            "events": list(events),
            "projections": [
                {
                    "viewer": viewer,
                    "expected": {
                        "view": build_view(state, viewer),
                        "input_request": input_request_for_viewer(request, state, viewer),
                        "awaiting_input": awaiting_input_hero_ids(request, state),
                        "winner": state.individual_winner_id or state.winner,
                        "hero_names": {},
                    },
                    "expected_events": events_for_viewer(events, state, viewer),
                }
                for viewer in viewers
            ],
        }

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(path, ["Arien", "Min"], ["Razzle", "Gydion"], seed=29)
        yield project(f"opening-{path.stem}", state)

    state = setup(["Arien", "Hanu"], ["Brogan", "Tigerclaw"])
    session = GameSession(state)
    for team in state.teams.values():
        for hero in team.heroes:
            result = session.commit_card(hero.id, hero.hand[0])
            if hero.id == "hero_arien":
                yield project("one-private-commitment", state)
    request = result.input_request
    assert request is not None
    yield project("first-resolution-prompt", state, request)
    state = setup(["Arien", "Hanu"], ["Brogan", "Tigerclaw"])
    session = GameSession(state)
    for team in state.teams.values():
        for hero in team.heroes:
            if hero.id == "hero_arien":
                card = next(card for card in hero.hand if card.id == "noble_blade")
            else:
                card = min(hero.hand, key=lambda card: card.initiative)
            result = session.commit_card(hero.id, card)
    request = result.input_request
    assert request is not None and request.request_type.value == "CHOOSE_ACTION"
    assert "MOVEMENT" in [option.id for option in request.options]
    result = session.advance(InputResponse(request_id=request.id, selection="MOVEMENT"))
    assert result.input_request is not None and result.input_request.request_type.value == "SELECT_HEX"
    yield project("hex-selection", state, result.input_request)

    state = setup(["Min", "Takahide"], ["Gydion", "Razzle"])
    min_hero = state.get_hero("hero_min")
    takahide = state.get_hero("hero_takahide")
    gydion = state.get_hero("hero_gydion")
    mine = state.token_pool[TokenType.MINE_BLAST][0]
    mine.owner_id = min_hero.id
    loc = next(h for h, tile in state.board.tiles.items()
               if not tile.is_obstacle and tile.zone_id is not None)
    state.place_entity(mine.id, loc)
    gydion.spells[0].state = CardState.SPELLBOOK
    hidden = takahide.hand[0]
    takahide.discard_card(hidden)
    hidden.is_facedown = True
    shown = takahide.hand[0]
    guessing = takahide.hand[1]
    state.card_guess = {"guesser_id": min_hero.id, "attempts": [{
        "attempt": 1, "victim_id": takahide.id, "card_id": guessing.id,
        "guessed_color": None, "actual_color": None, "correct": None,
    }]}
    state.card_reveal = {
        "revealer_id": min_hero.id, "target_unit_id": takahide.id, "owner_id": takahide.id,
        "card_id": shown.id, "tier_value": 1,
    }
    events = [
        {"event_type": "TOKEN_PLACED", "target_id": mine.id, "metadata": {"token_type": mine.token_type}},
        {"event_type": "MINE_TRIGGERED", "target_id": mine.id, "metadata": {"token_type": mine.token_type}},
        {"event_type": "EFFECT_CREATED", "metadata": {
            "card_id": hidden.id, "extra": {"card_names": [hidden.name, "Visible"]},
            "spell_ids": [gydion.spells[0].id],
        }},
        {"event_type": "CARD_REVEALED", "metadata": {"card_id": shown.id}},
    ]
    yield project("concealed-card-spell-and-mine", state, events=events)
    state.card_guess["attempts"][0].update(
        guessed_color="BLUE", actual_color=guessing.color, correct=guessing.color == "BLUE"
    )
    yield project("publicly-flipped-guess", state)

    state = setup(["Arien", "Hanu"], ["Brogan", "Tigerclaw"])
    state.phase = GamePhase.LEVEL_UP
    state.pending_upgrades = {"hero_arien": 1, "hero_brogan": 2}
    state.upgrade_reveal_snapshot = {"hero_arien": {StatType.ATTACK: 0},
                                     "hero_brogan": {StatType.ATTACK: 0}}
    state.execution_stack.append(ResolveUpgradesStep())
    session = GameSession(state)
    result = session.advance()
    request = result.input_request
    assert request is not None
    yield project("simultaneous-upgrade", state, request)
    chosen = request.context["players"]["hero_arien"]["options"][0]["pair"][0]
    result = session.advance(InputResponse(
        request_id=request.id, selection={"hero_id": "hero_arien", "card_id": chosen}
    ))
    yield project("upgrade-awaits-opponent", state, result.input_request)


def main():
    from pydantic_core import to_json

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if os.environ.get("PYTHONHASHSEED") != "0":
        parser.error("run with PYTHONHASHSEED=0")
    with args.output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as output:
            count = 0
            for case in cases():
                output.write(to_json(case) + b"\n")
                count += 1
    print(f"Exported {count} frontend visibility cases")


if __name__ == "__main__":
    main()
