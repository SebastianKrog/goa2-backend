from arena_profiles import MAPS, generated_starts


def test_recipe_is_repeatable_and_holdout_has_a_separate_stream():
    heroes = ["A", "B", "C", "D", "E", "F"]
    tuning = list(generated_starts("extend", 17, 12, heroes))
    held_out = list(generated_starts("confirm", 17, 12, heroes))
    assert list(generated_starts("extend", 17, 12, heroes)) == tuning
    assert {start[1] for start in tuning}.isdisjoint(start[1] for start in held_out)
    assert tuple(start[0] for start in held_out[:6]) == MAPS
    assert all(len(set(red + blue)) == 4 for _, _, red, blue in held_out)
