"""Frozen Min contracts use real source flows and retained physical aliases."""

from live_min_primary_traces import MINES, STANCES, action_scene, after_scenes, item_scenes
from object_execution_traces import reference


def run(effect, entry="text", mode="first", variant="ordinary"):
    state, ops, _ = action_scene(effect, entry, mode, variant)
    records = reference(state, ops)
    return state, [e for row in records for e in row["events"]]


def direct(generator, name):
    for case, state, ops in generator():
        if case == name:
            records = reference(state, ops)
            return state, [e for row in records for e in row["events"]]
    raise AssertionError(name)


def test_min_catalog_contains_pure_and_live_retired_card_builders():
    from primary_builders import catalog
    from primary_templates import MIN_EFFECTS, MIN_STATEFUL_EFFECTS

    data = catalog()
    assert set(MIN_EFFECTS) - set(data["recipes"]) == set(MIN_STATEFUL_EFFECTS)
    body = data["stateful_recipes"]["perfect_self"]
    assert body["predicate"] == "min_has_retired_tier_two"
    assert len(body["variants"]["without_retired_tier_two"]) == 1
    assert len(body["variants"]["with_retired_tier_two"]) == 6


def test_mine_placements_are_distinct_and_keep_exact_blast_dud_counts():
    from goa2.domain.models import TokenType

    for effect, counts in zip(MINES, ((1, 1), (1, 2), (2, 1)), strict=True):
        state, events = run(effect)
        points = []
        for kind, expected in zip((TokenType.MINE_BLAST, TokenType.MINE_DUD), counts, strict=True):
            placed = [
                token
                for token in state.token_pool[kind]
                if state.get_position(token.id) is not None
            ]
            assert len(placed) == expected
            points.extend(state.get_position(token.id) for token in placed)
        assert len(set(points)) == sum(counts)
        assert sum(e["event_type"] == "TOKEN_PLACED" for e in events) == sum(counts)


def test_smoke_bomb_effect_is_attached_to_the_actual_placed_token():
    state, _ = run("smoke_bomb")
    token = state.execution_context["smoke_bomb_token_id"]
    effect = state.active_effects[-1]
    assert effect.effect_type.value == "los_blocker"
    assert effect.scope.origin_id == token and effect.token_id == token
    assert effect.source_card_id is None


def test_stances_attack_then_push_and_keep_fixed_post_attack_movement():
    for effect in ("crane_stance", "tiger_stance", "dragon_stance"):
        _, events = run(effect)
        assert any(e["event_type"] == "COMBAT_RESOLVED" for e in events)
        assert any(e["event_type"] == "UNIT_PUSHED" for e in events)
    for effect, distance in (("tiger_stance", 1), ("dragon_stance", 2)):
        _, events = run(effect, variant="survives")
        moves = [e for e in events if e["event_type"] == "UNIT_MOVED"]
        assert moves and all(e["metadata"]["range"] == distance for e in moves)


def test_smoke_swap_preserves_the_token_and_cobra_relocates_it():
    from goa2.domain.hex import Hex

    for effect in ("viper_stance", "cobra_stance"):
        state, events = run(effect)
        token = state.execution_context["min_smoke_alias"]
        assert state.get_position("hero_min") == Hex(q=2, r=0, s=-2)
        assert (state.get_position(token.id) == Hex(q=0, r=0, s=0)) == (effect == "viper_stance")
        assert sum(e["event_type"] == "UNITS_SWAPPED" for e in events) == 1


def test_grenades_detonate_at_turn_expiry_and_remove_token_and_effect():
    from goa2.domain.models import TokenType

    for effect, victims in (("death_grenade", 1), ("holy_death_grenade", 2)):
        state, events = run(effect)
        assert state.turn == 2 and state.phase.value == "PLANNING"
        assert len(state.get_hero("hero_brogan").hand) == 4
        assert len(state.get_hero("hero_tigerclaw").hand) == (4 if victims == 2 else 5)
        assert not state.active_effects
        assert all(state.get_position(t.id) is None for t in state.token_pool[TokenType.GRENADE])
        assert any(e["event_type"] == "TOKEN_REMOVED" for e in events)
        _, events = run(effect, variant="empty-hands")
        assert sum(e["event_type"] == "UNIT_DEFEATED" for e in events) == victims


def test_inner_strength_doubles_positive_item_bonuses_and_keeps_negative_values():
    from goa2.domain.models import StatType
    from goa2.engine.stats import compute_card_stats

    state, _ = run("inner_strength", variant="items")
    hero = state.get_hero("hero_min")
    red = next(c for c in hero.deck if c.effect_id == "crane_stance")
    from goa2.engine.effect_manager import EffectManager

    hero.resolve_current_card(turn_number=state.turn)
    EffectManager.activate_effects_by_card(state, "inner_strength")
    for bonus, applied in ((2, 4), (-2, -2), (0, 0)):
        hero.items[StatType.ATTACK] = bonus
        assert (
            compute_card_stats(state, hero.id, red).primary_value
            == red.primary_action_value + applied
        )


def test_perfect_self_options_double_convert_or_both():
    from goa2.domain.models import CardState

    for variant, doubled, converted in (
        ("ordinary", True, False),
        ("option-double", True, False),
        ("option-item", False, True),
        ("option-both", True, True),
    ):
        state, events = run("perfect_self", variant=variant)
        assert bool(state.active_effects) == doubled
        assert any(e["event_type"] == "ITEM_GAINED" for e in events) == converted
        if converted:
            card = state.execution_context["min_retired_alias"]
            assert card.state == CardState.ITEM
            assert state.get_hero("hero_min").items[card.item] == 1


def test_copied_perfect_self_uses_the_performers_retired_deck_and_items():
    from goa2.domain.models import CardState

    state, events = run("perfect_self", "copy", variant="foreign-retired")
    card = state.execution_context["min_retired_alias"]
    assert card.state == CardState.ITEM and card in state.get_hero("hero_wasp").deck
    assert not state.get_hero("hero_min").items
    assert any(e["event_type"] == "ITEM_GAINED" and e["actor_id"] == "hero_wasp" for e in events)


def test_fast_as_lightning_reuses_red_text_without_replacing_outer_card_context():
    state, events = run("fast_as_lightning")
    assert state.execution_context["current_card_id"] == "fast_as_lightning"
    assert sum(e["event_type"] == "COMBAT_RESOLVED" for e in events) == 1
    assert any(e["event_type"] == "UNIT_PUSHED" for e in events)
    _, events = run("fast_as_lightning", variant="no-red")
    assert not any(e["event_type"] == "UNIT_PUSHED" for e in events)


def test_copied_fast_as_lightning_looks_up_the_performers_own_red_card():
    state, events = run("fast_as_lightning", "copy", variant="foreign-red")
    assert state.execution_context["foreign_red_alias"].id == "electrocute"
    assert len(state.get_hero("hero_brogan").hand) == 4
    assert not any(e["event_type"] == "UNIT_PUSHED" for e in events)


def test_deferred_custom_target_is_the_actual_defender_and_second_attack_is_distinct():
    state, events = run("fast_as_lightning", variant="after-custom-target")
    assert state.execution_context["thunder_target_1"] == "hero_brogan"
    attacks = [e for e in events if e["event_type"] == "COMBAT_RESOLVED"]
    assert len(attacks) == 2 and attacks[0]["target_id"] != attacks[1]["target_id"]


def test_late_discard_is_found_when_the_deferred_step_executes():
    state, events = direct(after_scenes, "min-after-late-discard")
    assert state.execution_context["after_red_alias"] in state.get_hero("hero_min").discard_pile
    assert any(e["event_type"] == "UNIT_PUSHED" for e in events)


def test_played_precedence_and_facedown_mask_are_applied_before_red_effect_lookup():
    _, events = direct(after_scenes, "min-after-unregistered-first")
    assert not any(e["event_type"] in ("UNIT_MOVED", "UNIT_PUSHED") for e in events)
    _, events = direct(after_scenes, "min-after-facedown-first")
    assert any(e["event_type"] == "UNIT_PUSHED" for e in events)


def test_conversion_clears_initiative_and_preserves_other_flags_and_identity():
    from goa2.domain.models import CardState

    state, events = direct(item_scenes, "min-item-flags")
    card = state.execution_context["converted_card_alias"]
    assert card.state == CardState.ITEM and card.initiative_override is None
    assert card.enraged_active_override and card.passive_uses_this_turn == 2
    assert len(events) == 1 and events[0]["event_type"] == "ITEM_GAINED"


def test_conversion_updates_shared_item_dictionary_and_chooses_first_duplicate_id():
    from goa2.domain.models import CardState, StatType

    state, _ = direct(item_scenes, "min-item-shared-items")
    assert state.get_hero("hero_min").items is state.get_hero("hero_wasp").items
    assert state.get_hero("hero_wasp").items[StatType.DEFENSE] == 1
    state, _ = direct(item_scenes, "min-item-duplicate-card")
    assert state.execution_context["first_convert_alias"].state == CardState.ITEM
    assert state.execution_context["converted_card_alias"].state != CardState.ITEM
    assert state.get_hero("hero_min").items[StatType.INITIATIVE] == 1


def test_repeated_conversion_is_not_restricted_to_retired_cards():
    state, events = direct(item_scenes, "min-item-same-step")
    assert (
        state.get_hero("hero_min").items[state.execution_context["converted_card_alias"].item] == 2
    )
    assert sum(e["event_type"] == "ITEM_GAINED" for e in events) == 2


def test_negative_min_attacks_are_legal_and_unclamped():
    for effect in (*STANCES, "fast_as_lightning"):
        _, events = run(effect, variant="negative-attack")
        attacks = [e for e in events if e["event_type"] == "COMBAT_RESOLVED"]
        assert attacks and attacks[0]["metadata"]["attack_value"] < 0


def test_perfect_self_reperformance_rechecks_retired_cards_after_conversion():
    from live_min_primary_traces import reperform_scenes

    state, events = direct(reperform_scenes, "min-primary-perfect_self-reperform-after-convert")
    card = state.execution_context["min_retired_alias"]
    assert state.get_hero("hero_min").items[card.item] == 1
    assert len(state.active_effects) == 1
    assert sum(e["event_type"] == "ITEM_GAINED" for e in events) == 1
    assert state.execution_context["min_repeat_tail"] is True
