from arena_choices import decision_hash


def test_choice_hash_ignores_option_order_but_detects_unselected_option_changes():
    decision = {
        "kind": "input",
        "request": "SELECT_HEX",
        "multi": False,
        "owners": ["hero_arien", "hero_hanu"],
        "moves": [{"q": 0, "r": 1, "s": -1}, {"q": 1, "r": 0, "s": -1}, "SKIP"],
    }
    reordered = dict(decision, moves=list(reversed(decision["moves"])),
                     owners=list(reversed(decision["owners"])))
    changed = dict(decision, moves=[decision["moves"][0], {"q": 2, "r": 0, "s": -2}, "SKIP"])
    assert decision_hash(decision) == decision_hash(reordered)
    assert decision_hash(decision) != decision_hash(changed)


def test_choice_hash_covers_ordered_commitments_and_batch_constraints():
    first = {"kind": "planning", "players": [{"player": "hero_emmitt", "second_slot": False,
                                            "moves": [["a", "b"]]}]}
    second = {"kind": "planning", "players": [{"player": "hero_emmitt", "second_slot": False,
                                             "moves": [["b", "a"]]}]}
    assert decision_hash(first) != decision_hash(second)
    runes = {"kind": "runes", "owners": ["hero_snorri"],
             "moves": ["axe", "bird", "horn"], "counts": [1, 2, 3]}
    assert decision_hash(runes) != decision_hash(dict(runes, counts=[1, 3]))
