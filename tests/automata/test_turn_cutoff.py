"""Game-turn horizons count full turns, not input prompts or full rounds."""
from types import SimpleNamespace

import pytest
from automata.decision import DecisionDescriptor
from automata.search.config import SearchConfig
from automata.search.contracts import CutoffUnit, LeafEvaluation, LeafMode, SearchContext
from automata.search.ismcts.engine import _rollout
from goa2.domain.models import TeamColor
import automata.search.ismcts.engine as engine


@pytest.mark.parametrize("start_round,start_turn,origin,expected_calls,expected_clock", [
    (1,1,None,4,(1,3)),
    (1,4,None,4,(2,2)),
    # The selected root edge already ended the originating turn.
    (2,1,(1,4),2,(2,2)),
])
def test_two_turn_cutoff_counts_multiple_inputs_and_round_rollover(
    monkeypatch,start_round,start_turn,origin,expected_calls,expected_clock
):
    decision = DecisionDescriptor("CARD")
    context = SearchContext("hero",TeamColor.RED,"hero",decision)
    state = SimpleNamespace(round=start_round,turn=start_turn)
    calls = []
    def apply_ours(_decision,key):
        calls.append(key)
        if len(calls) % 2 == 0:
            if state.turn == 4:
                state.round += 1
                state.turn = 1
            else:
                state.turn += 1
        return decision
    sim = SimpleNamespace(state=state,our_team=TeamColor.RED,apply_ours=apply_ours)
    monkeypatch.setattr(engine,"legal_keys",lambda _: ["action"])
    monkeypatch.setattr(engine,"_decision_owner_id",lambda *_: "hero")
    policy = SimpleNamespace(choose=lambda *_: "action")
    evaluated = []
    def evaluate(_context,leaf):
        evaluated.append((leaf.round,leaf.turn))
        return LeafEvaluation(value=0.25)
    cfg = SearchConfig(cutoff_unit=CutoffUnit.TURNS,cutoff_limit=2,leaf_mode=LeafMode.BOUNDED_CONTINUATION)
    reward = _rollout(sim,decision,cfg,policy,SimpleNamespace(evaluate=evaluate),context,turn_origin=origin)
    assert len(calls) == expected_calls
    assert evaluated == [expected_clock]
    assert reward == pytest.approx(0.625)
