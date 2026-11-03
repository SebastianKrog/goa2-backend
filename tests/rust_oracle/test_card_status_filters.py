"""Source contracts for hero current-card and resolved-slot filtering."""

from card_status_filter_traces import scenes


def test_card_status_and_slot_contracts_cover_all_source_scenarios():
    rows = list(scenes())
    assert len(rows) == 37
    assert len({name for name, _, _ in rows}) == 37
    assert sum(len(operations) for _, _, operations in rows) == 222
