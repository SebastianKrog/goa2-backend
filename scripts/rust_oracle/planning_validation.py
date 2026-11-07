"""Validate a scoped Rust planning response against an authoritative Python state."""

from automata.runtime.legality import DecisionMove, check_sequences
from goa2.domain.models import GamePhase
from goa2.domain.types import HeroID
from goa2.engine.phases import hero_can_play_two_cards, planning_open_for_second_card
from state_codec import REFERENCE_KEY


def validate_commitments(
    state, envelope, response, *, player_id, controlled_ids, searched_revision, live_revision,
    require_score=True,
):
    """Return the whole validated decision prefix, or reject it before submission.

    The caller supplies trusted ownership and revisions and holds the live state
    stable from validation through submission. The envelope is the original
    request's physical card table.
    """
    if searched_revision != live_revision or state.phase != GamePhase.PLANNING:
        raise ValueError("stale planning result")
    if (
        not isinstance(response, dict)
        or "score" not in response
        or not isinstance(response.get("commitments"), list)
    ):
        raise ValueError("malformed planning result")
    score = response["score"]
    shallow_score = response.get("shallow_score")
    if (
        (score is not None and type(score) is not int)
        or (shallow_score is not None and type(shallow_score) is not int)
        or (score is not None and shallow_score is not None)
    ):
        raise ValueError("malformed planning result")
    if require_score and score is None and shallow_score is None:
        raise ValueError("planning result has no completed score")
    player = state.get_hero(HeroID(player_id))
    if player is None or player.id != player_id or player_id not in controlled_ids:
        raise ValueError("planning player is not a controlled bot")

    source_hands = {
        hero["id"]: {card[REFERENCE_KEY] for card in hero["hand"]}
        for team in envelope["state"]["teams"].values()
        for hero in team["heroes"]
    }
    moves = []
    seen = set()
    for row in response["commitments"]:
        if (
            not isinstance(row, dict)
            or not isinstance(row.get("player_id"), str)
            or not {"first_card", "second_card"} <= row.keys()
        ):
            raise ValueError("malformed planning commitment")
        hero_id = row["player_id"]
        hero = state.get_hero(HeroID(hero_id))
        if (
            hero_id in seen or hero_id not in controlled_ids
            or hero is None or hero.id != hero_id or hero.team != player.team
        ):
            raise ValueError("unexpected hero in planning result")
        seen.add(hero_id)
        second_slot = planning_open_for_second_card(state, hero.id)
        if hero.id in state.pending_inputs and not second_slot:
            raise ValueError("planning slot is closed")

        first, second = row["first_card"], row["second_card"]
        if second is not None and (first is None or second_slot or first == second):
            raise ValueError("unexpected second planning card")
        if first is None:
            kind = "FINISH_PLANNING" if second_slot else "PASS"
            moves.append(DecisionMove(kind, hero_id))
            continue

        for index in (first, second):
            if index is None:
                continue
            if type(index) is not int or not 0 <= index < len(envelope["instances"]):
                raise ValueError("invalid physical card index")
            if index not in source_hands.get(hero_id, ()):
                raise ValueError("physical card was outside the searched hand")
            instance = envelope["instances"][index]
            definition = envelope["definitions"][instance["definition"]]
            if definition["kind"] != "card":
                raise ValueError("physical instance is not a planning card")
            card_id = definition["fields"]["id"]
            if sum(card.id == card_id for card in hero.hand) != 1:
                raise ValueError("planning card does not identify one live card in hand")
            moves.append(DecisionMove("CARD", hero_id, card_id))

        if second is None and not second_slot and len(hero.hand) > 1 and hero_can_play_two_cards(hero):
            moves.append(DecisionMove("FINISH_PLANNING", hero_id))

    open_slots = {
        hero_id for hero_id in controlled_ids
        if hero_id not in state.pending_inputs
        or planning_open_for_second_card(state, HeroID(hero_id))
    }
    if seen != open_slots:
        raise ValueError("planning result does not cover the controlled open slots")

    if not moves or check_sequences(state, [moves]).results[0].legal is not True:
        raise ValueError("Python rejected the planning sequence")
    return tuple(moves)
