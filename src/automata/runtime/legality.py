"""Read-only batch and sequence legality checks using actual engine decisions.

A batch indexes one live prompt once. Sequence batches execute a prefix trie,
so shared prefixes run once and branches receive independent mutable worlds.
No bot silently supplies missing decisions or chooses an opponent's action.
"""

from __future__ import annotations

import copy
from contextlib import suppress
from dataclasses import dataclass, field
from typing import Any, Literal

from goa2.domain.hex import Hex
from goa2.domain.input import (
    DONE,
    SKIP,
    InputRequestType,
    InputResponse,
    parse_hex_selection,
    selection_value,
)
from goa2.domain.models import GamePhase
from goa2.domain.models.spell import SpellCard
from goa2.domain.types import HeroID
from goa2.engine.session import GameSession
from goa2.engine.steps.base import GameStep
from goa2.engine.steps.selection import MultiSelectStep

from .clone import clone_state


class _UnverifiableInput(Exception):
    """Only this condition produces an unknown legality verdict."""


@dataclass(frozen=True)
class DecisionMove:
    kind: Literal["CARD", "INPUT", "PASS", "FINISH_PLANNING"]
    player_id: str
    selection: Any = None
    # Omit when constructing hypothetical future decisions; explicit IDs remain strict.
    request_id: str | None = None


@dataclass(frozen=True)
class LegalityResult:
    legal: bool | None
    checked: int
    failed_at: int | None = None
    reason: str | None = None


@dataclass(frozen=True)
class LegalityBatch:
    results: tuple[LegalityResult, ...]
    applied_decisions: int
    state_clones: int


def _key(value):
    if isinstance(value, Hex):
        return ("hex", value.q, value.r, value.s)
    if isinstance(value, dict):
        return ("dict", frozenset((_key(k), _key(v)) for k, v in value.items()))
    if isinstance(value, (tuple, list)):
        return (type(value), tuple(_key(v) for v in value))
    if isinstance(value, (str, int, float, bool)) or value is None:
        return (type(value), value)
    raise TypeError(f"Unsupported selection value: {type(value).__name__}")


@dataclass
class _Cursor:
    session: GameSession
    request: Any = None
    options: set | None = None

    def fork(self):
        session = GameSession(clone_state(self.session.state), create_rollback_snapshots=False)
        session._last_phase = self.session._last_phase
        session._rollback_actor_id = self.session._rollback_actor_id
        return _Cursor(session, self.request, self.options)

    def ready(self):
        # Automatic rule steps run, but every actual player prompt stops us.
        for _ in range(1000):
            state = self.session.state
            if self.request is not None or state.phase in (GamePhase.PLANNING, GamePhase.GAME_OVER):
                return
            before = (
                state.phase,
                state.round,
                state.turn,
                state.current_actor_id,
                len(state.execution_stack),
            )
            result = self.session.advance()
            self.request = result.input_request
            after = (
                state.phase,
                state.round,
                state.turn,
                state.current_actor_id,
                len(state.execution_stack),
            )
            if self.request is None and before == after:
                return  # An action boundary can have no further player decision.
        raise RuntimeError("Decision preparation exceeded the engine advancement limit")

    def selection_key(self, selection):
        if self.request.request_type in (
            InputRequestType.SELECT_HEX,
            InputRequestType.MOVEMENT_HEX,
            InputRequestType.FAST_TRAVEL_DESTINATION,
            InputRequestType.CHOOSE_RESPAWN_HEX,
        ):
            parsed = parse_hex_selection(selection)
            if parsed is not None:
                return _key(parsed)
        if self.request.request_type is InputRequestType.SELECT_NUMBER:
            with suppress(TypeError, ValueError, OverflowError):
                selection = int(selection)
        return _key(selection)

    def check_input(self, move):
        request, state = self.request, self.session.state
        if request is None or state.phase is GamePhase.PLANNING:
            raise ValueError("No input decision is pending")
        if move.request_id is not None and move.request_id != request.id:
            raise ValueError("Stale input request ID")
        addressed = move.player_id == request.player_id
        if request.player_id.startswith("team:"):
            hero = state.get_hero(HeroID(move.player_id))
            addressed = hero is not None and f"team:{hero.team}" == request.player_id
        elif request.player_id == "simultaneous":
            addressed = (
                move.player_id in request.context.get("players", {})
                and isinstance(move.selection, dict)
                and move.selection.get("hero_id") == move.player_id
            )
        if not addressed:
            raise ValueError("Input is addressed to another player")
        if not state.execution_stack:
            raise ValueError("No pending step to receive input")
        step = state.execution_stack[-1]
        # Structured batches (runes/upgrades) can be legal even when their
        # aggregate payload is not one of the individually displayed options.
        if type(step).validate_input is not GameStep.validate_input:
            step.validate_input(state, copy.deepcopy(move.selection))
            return
        if request.can_skip and (
            move.selection == SKIP or (isinstance(step, MultiSelectStep) and move.selection == DONE)
        ):
            return
        if request.options:
            if self.options is None:
                self.options = {
                    self.selection_key(selection_value(option)) for option in request.options
                }
                if request.can_skip:
                    self.options.add(self.selection_key(SKIP))
                    if isinstance(step, MultiSelectStep):
                        self.options.add(self.selection_key(DONE))
            if self.selection_key(move.selection) not in self.options:
                raise ValueError("Selection is not offered by the current decision")
        elif type(step).validate_input is GameStep.validate_input:
            # Absence of an enumerated option is not proof that a custom payload
            # is illegal. Never claim a definitive result without an engine check.
            raise _UnverifiableInput("This unenumerated input has no validation hook")
        step.validate_input(state, copy.deepcopy(move.selection))

    def planning(self, move):
        state = self.session.state
        if state.phase is not GamePhase.PLANNING:
            raise ValueError("Planning decision outside planning phase")
        hero = state.get_hero(HeroID(move.player_id))
        if hero is None or str(hero.id) != move.player_id:
            raise ValueError("Unknown planning player")
        if move.request_id is not None:
            raise ValueError("Planning decisions do not have input request IDs")
        if move.kind == "CARD":
            card = next((c for c in hero.hand if c.id == move.selection), None)
            if card is None or isinstance(card, SpellCard):
                raise ValueError("Card is not a playable card in this player's hand")
            result = self.session.commit_card(HeroID(hero.id), card)
        elif move.kind == "PASS":
            result = self.session.pass_turn(HeroID(hero.id))
        elif move.kind == "FINISH_PLANNING":
            result = self.session.finish_planning(HeroID(hero.id))
        else:
            raise ValueError(f"Unknown decision kind: {move.kind}")
        self.request, self.options = result.input_request, None

    def apply(self, move):
        if move.kind == "INPUT":
            self.check_input(move)
            result = self.session.advance(
                InputResponse(request_id=self.request.id, selection=copy.deepcopy(move.selection))
            )
            self.request, self.options = result.input_request, None
        else:
            self.planning(move)
        self.ready()


def _root(state):
    cursor = _Cursor(GameSession(clone_state(state), create_rollback_snapshots=False))
    cursor.ready()
    return cursor


def check_moves(state, moves) -> LegalityBatch:
    """Check independent submissions at one state; never apply one to the next.

    Input options are indexed once. Planning uses a throwaway engine session
    for each distinct candidate, preserving all planning rules without a second
    implementation. ``legal=None`` denotes an unenumerated custom input for
    which the engine supplies no validation hook.
    """
    cursor, results, cache = _root(state), [], {}
    clones, applied = 1, 0
    for move in moves:
        key = (move.kind, move.player_id, move.request_id, _key(move.selection))
        result = cache.get(key)
        if result is None:
            try:
                if move.kind == "INPUT":
                    cursor.check_input(move)
                else:
                    trial = cursor.fork()
                    clones += 1
                    trial.planning(move)
                    applied += 1
                result = LegalityResult(True, 1)
            except ValueError as error:
                result = LegalityResult(False, 0, 0, str(error))
            except _UnverifiableInput as error:
                result = LegalityResult(None, 0, 0, str(error))
            cache[key] = result
        results.append(result)
    return LegalityBatch(tuple(results), applied, clones)


@dataclass
class _Prefix:
    move: DecisionMove | None = None
    children: dict = field(default_factory=dict)
    outputs: list[int] = field(default_factory=list)


def check_sequences(state, sequences) -> LegalityBatch:
    """Validate complete decision prefixes, executing each shared prefix once.

    A legal result means every submitted decision is legal; ending at a prompt
    is allowed. Terminal input submissions need no simulation of consequences.
    Missing decisions are never invented. Errors identify the first
    failing zero-based index. Calls are independent, including after mutations
    to the source state. Request IDs omitted by a move bind to its actual prompt.
    """
    root, results = _Prefix(), []
    for sequence in sequences:
        index = len(results)
        results.append(None)
        node = root
        for move in sequence:
            key = (move.kind, move.player_id, move.request_id, _key(move.selection))
            child = node.children.get(key)
            if child is None:
                child = node.children[key] = _Prefix(move)
            node = child
        node.outputs.append(index)
    applied, clones = 0, 1
    pending = [(root, _root(state), 0, None)]
    while pending:
        node, cursor, depth, failure = pending.pop()
        if failure is None and node.move is not None:
            try:
                cursor.apply(node.move)
                applied += 1
            except ValueError as error:
                failure = LegalityResult(False, depth - 1, depth - 1, str(error))
            except _UnverifiableInput as error:
                failure = LegalityResult(None, depth - 1, depth - 1, str(error))
        for index in node.outputs:
            results[index] = failure or LegalityResult(True, depth)
        children = []
        for child in node.children.values():
            # At the end of a proposed sequence, accepting an offered input is
            # sufficient. Do not execute its consequences just to prove legality.
            if child.move.kind == "INPUT" and not child.children:
                verdict = failure
                if verdict is None:
                    try:
                        cursor.check_input(child.move)
                        verdict = LegalityResult(True, depth + 1)
                    except ValueError as error:
                        verdict = LegalityResult(False, depth, depth, str(error))
                    except _UnverifiableInput as error:
                        verdict = LegalityResult(None, depth, depth, str(error))
                for index in child.outputs:
                    results[index] = verdict
            else:
                children.append(child)
        for index, child in enumerate(children):
            branch = cursor
            if failure is None and index < len(children) - 1:
                branch = cursor.fork()
                clones += 1
            pending.append((child, branch, depth + 1, failure))
    return LegalityBatch(tuple(results), applied, clones)
