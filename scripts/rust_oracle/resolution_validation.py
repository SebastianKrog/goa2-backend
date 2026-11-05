"""Bind a native recommendation to the live Python prompt before submission."""

from automata.runtime.driver import eligible_hero_ids_for_request
from automata.runtime.legality import DecisionMove, check_moves
from goa2.domain.input import InputRequestType


def validate_resolution(
    state, live_request, response, *, player_id, controlled_ids, searched_revision, live_revision,
):
    if searched_revision != live_revision or live_request is None:
        raise ValueError("stale resolution result")
    decision = response["decision"]
    if decision["kind"] != "input" or decision["request_id"] != live_request.id:
        raise ValueError("resolution result does not match the live prompt")
    if (
        decision["player_id"] != player_id
        or player_id not in controlled_ids
        or player_id not in eligible_hero_ids_for_request(state, live_request)
    ):
        raise ValueError("resolution player is not an eligible bot")
    if live_request.request_type == InputRequestType.UPGRADE_PHASE:
        player = live_request.context.get("players", {}).get(player_id, {})
        if player.get("remaining", 0) <= 0:
            raise ValueError("resolution player has no pending upgrades")
    if response["score"] is None:
        raise ValueError("resolution result has no completed score")

    move = DecisionMove("INPUT", player_id, decision["selection"], live_request.id)
    if check_moves(state, [move]).results[0].legal is not True:
        raise ValueError("Python rejected the resolution move")
    return move
