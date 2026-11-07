"""Bind a native recommendation to the live Python prompt before submission."""

from automata.runtime.driver import eligible_hero_ids_for_request
from automata.runtime.legality import DecisionMove, check_moves
from goa2.domain.input import InputRequestType
from goa2.engine.steps.movement import MinePathChoiceStep


def is_mine_path_request(state, request):
    return (
        request.request_type == InputRequestType.SELECT_OPTION
        and bool(state.execution_stack)
        and isinstance(state.execution_stack[-1], MinePathChoiceStep)
    )


def validate_resolution(
    state, live_request, response, *, player_id, controlled_ids, searched_revision, live_revision,
    require_score=True,
):
    if searched_revision != live_revision or live_request is None:
        raise ValueError("stale resolution result")
    if (
        not isinstance(response, dict)
        or "score" not in response
        or (response["score"] is not None and type(response["score"]) is not int)
        or not isinstance(response.get("decision"), dict)
    ):
        raise ValueError("malformed resolution result")
    decision = response["decision"]
    if not {"kind", "request_id", "player_id", "selection"} <= decision.keys():
        raise ValueError("malformed resolution decision")
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
    if require_score and response["score"] is None:
        raise ValueError("resolution result has no completed score")

    move = DecisionMove("INPUT", player_id, decision["selection"], live_request.id)
    if is_mine_path_request(state, live_request):
        # The generic helper converts numeric option IDs to integers. The
        # issued MinePathChoice options require their exact string IDs.
        legal = any(move.selection == option.id for option in live_request.options)
    else:
        legal = check_moves(state, [move]).results[0].legal
    if legal is not True:
        raise ValueError("Python rejected the resolution move")
    return move
