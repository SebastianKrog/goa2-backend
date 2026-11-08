"""Arena legal-choice projection and canonical SHA-256 encoding."""

import hashlib
import json

CHOICE_ENCODING = "arena_choices_v1"


def canonical_choice_bytes(descriptor):
    def encode(value):
        return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)

    def normalize(value, field=None):
        if isinstance(value, dict):
            return {key: normalize(item, key) for key, item in value.items()}
        if isinstance(value, list):
            rows = [normalize(item) for item in value]
            if field == "choices":
                rows = [[color, tier, sorted(pair)] for color, tier, pair in rows]
            if field in {"players", "moves", "owners", "counts", "available", "choices"}:
                by_wire = {encode(item): item for item in rows}
                return [by_wire[key] for key in sorted(by_wire)]
            return rows
        return value

    return encode([CHOICE_ENCODING, normalize(descriptor)]).encode("utf-8")


def decision_hash(descriptor):
    return hashlib.sha256(canonical_choice_bytes(descriptor)).hexdigest()


def python_legal_choices(state, prompt):
    # Keep the scoring driver independent of the Python engine.
    from automata.runtime.driver import eligible_hero_ids_for_request
    from goa2.domain.input import selection_value
    from goa2.domain.models import SpellCard
    from goa2.engine.phases import hero_can_play_two_cards, planning_open_for_second_card
    from goa2.engine.steps import MultiSelectStep
    from goa2.engine.steps.movement import MinePathChoiceStep

    if prompt is None:
        players = []
        for team in state.teams.values():
            for hero in team.heroes:
                has_committed = hero.id in state.pending_inputs
                second_slot = planning_open_for_second_card(state, hero.id)
                if has_committed and not second_slot:
                    continue
                cards = [card.id for card in hero.hand if not isinstance(card, SpellCard)]
                if len(cards) != len(set(cards)):
                    raise ValueError("ambiguous printed card in arena planning")
                moves = [[card, None] for card in cards]
                if second_slot or not hero.hand:
                    moves.append([None, None])
                if not second_slot and hero_can_play_two_cards(hero):
                    moves.extend([first, second] for first in cards for second in cards if first != second)
                players.append({"player": hero.id, "second_slot": second_slot, "moves": moves})
        return {"kind": "planning", "players": players}

    request = prompt.request_type.value
    if request == "UPGRADE_PHASE":
        players = []
        for player, row in prompt.context["players"].items():
            if not row["options"]:
                continue
            hero = state.get_hero(player)

            def upgrade_cards(cards, *, deck=False):
                return [
                    [card.color.value, card.tier.value, card.id]
                    for card in cards
                    if card.color is not None and card.color.value in {"RED", "BLUE", "GREEN"}
                    and card.tier.value in {"I", "II", "III"}
                    and (not deck or (card.tier.value != "I" and card.state.value == "DECK"))
                ]

            players.append({
                "player": player,
                "remaining": row["remaining"],
                "choices": [[entry["color"], entry["tier"], entry["pair"]] for entry in row["options"]],
                "hand": upgrade_cards(hero.hand),
                "available": upgrade_cards(hero.deck, deck=True),
            })
        return {"kind": "upgrades", "players": players}

    owners = eligible_hero_ids_for_request(state, prompt)
    if prompt.decision_context and prompt.decision_context.purpose == "rune_inscription":
        return {"kind": "runes", "owners": owners, "moves": [option.id for option in prompt.options],
                "counts": prompt.context["selection_counts"]}

    step = state.execution_stack[-1]
    if isinstance(step, MinePathChoiceStep):
        moves = [[option.id, option.metadata["metadata"]["path"], option.metadata["metadata"]["mine_hexes"]]
                 for option in prompt.options]
        return {"kind": "mines", "owners": owners, "moves": moves}

    moves = [selection_value(option) for option in prompt.options]
    multi = isinstance(step, MultiSelectStep)
    if prompt.can_skip:
        moves.append("SKIP")
        if multi:
            moves.append("DONE")
    return {"kind": "input", "request": request, "multi": multi, "owners": owners, "moves": moves}
