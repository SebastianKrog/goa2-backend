import pytest

from arena_reports import load_report, save_report
from oracle import digest


def test_reports_share_a_start_and_reopen_without_the_scorer(tmp_path):
    envelope = {"state": {"example": 1}}
    one = {"trace": [{"legal_choices_sha256": "a" * 64, "choice": "SKIP"}]}
    two = {"trace": [{"legal_choices_sha256": "a" * 64, "choice": "HOLD"}]}
    first = save_report(tmp_path, envelope, one)
    second = save_report(tmp_path, envelope, two)
    assert first != second
    assert list((tmp_path / "starts").iterdir()) == [tmp_path / "starts" / f"{digest(envelope)}.json"]
    assert load_report(tmp_path, first) == (
        dict(one, schema_version=1, start_sha256=digest(envelope)), envelope,
    )
    assert save_report(tmp_path, envelope, one) == first


def test_invalid_report_and_missing_or_damaged_start(tmp_path):
    envelope = {"state": {}}
    report_id = save_report(tmp_path, envelope, {"trace": []})
    report = tmp_path / "reports" / f"{report_id}.json"
    data = report.read_bytes()
    report.write_bytes(data[:-1])
    with pytest.raises(ValueError, match="reports integrity"):
        load_report(tmp_path, report_id)
    report.write_bytes(data)

    start = tmp_path / "starts" / f"{digest(envelope)}.json"
    start.unlink()
    with pytest.raises(FileNotFoundError):
        load_report(tmp_path, report_id)
    start.write_text("{}")
    with pytest.raises(ValueError, match="starts integrity"):
        load_report(tmp_path, report_id)
    with pytest.raises(ValueError, match="invalid arena SHA-256"):
        load_report(tmp_path, "../escape")


def test_restore_python_start_rehydrates_steps_and_physical_context_aliases():
    from goa2.domain.hex import Hex
    from goa2.domain.models import Card, GamePhase
    from goa2.engine.steps.phases import RestoreActionTypeStep

    from arena_verify import restore_python_start
    from model_traces import setup
    from shared_objects import export_state_with_shared_objects

    state = setup(["Arien", "Hanu"], ["Brogan", "Tigerclaw"])
    state.phase = GamePhase.RESOLUTION
    stack_step = RestoreActionTypeStep(step_id="arena/restore-stack")
    stored_step = RestoreActionTypeStep(step_id="arena/restore-context")
    state.execution_stack = [stack_step]
    hero = state.get_hero("hero_arien")
    card = hero.hand[0]
    isolated_card = card.model_copy(deep=True)
    isolated_card.initiative = card.initiative
    assert isolated_card.model_dump() == card.model_dump()
    assert isolated_card != card
    different_card = card.model_copy(deep=True)
    different_card.metadata = {"different": 1}
    hero.hand.extend([isolated_card, different_card])
    state.execution_context.update(
        stack_step=stack_step, stored_step=stored_step, deferred_steps=[stored_step],
        card_alias=card, isolated_card=isolated_card,
        positions=state.entity_locations, context_hex=Hex(q=3, r=0, s=-3),
    )

    envelope, _ = export_state_with_shared_objects(state)
    restored, _ = restore_python_start(envelope)
    context = restored.execution_context
    assert isinstance(restored.execution_stack[0], RestoreActionTypeStep)
    assert restored.execution_stack[0] is context["stack_step"]
    assert isinstance(context["stored_step"], RestoreActionTypeStep)
    assert context["stored_step"].step_id == stored_step.step_id
    assert context["deferred_steps"][0] is context["stored_step"]
    assert context["card_alias"] is restored.get_hero("hero_arien").hand[0]
    assert isinstance(context["isolated_card"], Card)
    assert context["isolated_card"] != context["card_alias"]
    restored_hand = restored.get_hero("hero_arien").hand
    assert context["isolated_card"] is restored_hand[-2]
    assert restored_hand[-1].metadata == {"different": 1}
    assert restored_hand[-1] is not context["card_alias"]
    assert context["positions"] is restored.entity_locations
    assert all(isinstance(position, Hex) for position in context["positions"].values())
    assert isinstance(context["context_hex"], Hex)
