"""Freeze reviewed primary builders with physical nested-object bindings.

The list is deliberately reviewed rather than inferred from a successful call.
Pure recipes are separate from named stateful variants, whose selection requires
an explicit native runtime port.
"""

from __future__ import annotations

import json

from fixture_cache import write_text
from oracle import ROOT, digest, verify_sources
from primary_templates import (
    CHARGE_ATTACKS,
    CHARGE_PRIMARY_PROBE,
    EFFECTS,
    LIVE_ONLY_EFFECTS,
    PRIMARY,
    RADIUS,
    RANGE,
    bind_charge_damage,
)

OWNER = "template-passive-owner"
CARD = "template-primary-card"


def freeze_steps(steps):
    from shared_objects import capture

    wires = [step.model_dump(mode="json") for step in steps]
    bindings, _ = capture(steps, wires)
    # Link cross-root model/container identities after grouped allocation.
    # Shared executable steps need a grouped generation-counter policy first.
    imports = {}
    for binding in bindings:
        paths = [path for path in binding["paths"] if path]
        roots = {path[0] for path in paths}
        if len(roots) <= 1:
            continue
        assert binding["kind"] != "step", "cross-root executable aliases need grouped IDs"
        original = paths[0]
        for path in paths[1:]:
            if path[0] != original[0]:
                imports.setdefault(path[0], []).append(
                    {"path": path[1:], "back": path[0] - original[0], "original": original[1:]}
                )

    def labels(value):
        if isinstance(value, dict):
            return {
                key: "template" if key == "step_id" else labels(child)
                for key, child in value.items()
            }
        if isinstance(value, list):
            return [labels(child) for child in value]
        return value

    rows = []
    for step, wire in zip(steps, wires, strict=True):
        bindings, _ = capture(step, wire)
        row = {"fields": labels(wire), "objects": bindings}
        if len(rows) in imports:
            row["imports"] = imports[len(rows)]
        rows.append(row)
    return rows


def raw_binding_paths(bound, raw):
    """Record only differences observed between both real source APIs.

    Runtime can undo attribution without duplicating instructions or guessing
    which nested fields the source's generic binder visits. Physical model
    topology must be identical, including aliases, for this patch to be valid.
    """
    assert len(bound) == len(raw), "raw primary root shape changed"
    result = []
    for ordinary, unbound in zip(bound, raw, strict=True):
        assert ordinary["objects"] == unbound["objects"], "raw primary aliases changed"
        assert ordinary.get("imports") == unbound.get(
            "imports"
        ), "raw primary cross-root aliases changed"
        paths = []

        def compare(left, right, path, paths=paths):
            if left == right:
                return
            if isinstance(left, dict) and isinstance(right, dict):
                assert left.keys() == right.keys(), "raw primary fields changed"
                for key in left:
                    compare(left[key], right[key], [*path, key])
            elif isinstance(left, list) and isinstance(right, list):
                assert len(left) == len(right), "raw primary list shape changed"
                for index, (a, b) in enumerate(zip(left, right, strict=True)):
                    compare(a, b, [*path, index])
            else:
                assert path[-1] == "source_card_id" and left == CARD and right is None, (
                    "raw primary difference needs an explicit native port",
                    path,
                    left,
                    right,
                )
                paths.append(path)

        compare(ordinary["fields"], unbound["fields"], [])
        result.append(paths)
    return result


def bounty_movement_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import Marker, MarkerType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from primary_templates import BAIN_BOUNTY_MOVEMENT

    marked_state = state.model_copy(update={
        "markers": {**state.markers, MarkerType.BOUNTY: Marker(
            type=MarkerType.BOUNTY, target_id="hero_brogan", source_id="hero_bain",
        )},
    })
    hero = HeroRegistry.get("Bain").model_copy(update={"id": OWNER})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for effect in BAIN_BOUNTY_MOVEMENT:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        base, = build(state, hero, card, stats)
        marked, = build(marked_state, hero, card, stats)
        bonus = marked.range_val - base.range_val
        assert base.range_val == PRIMARY and bonus > 0
        assert freeze_steps([marked.model_copy(update={"range_val": PRIMARY})]) == freeze_steps([base])
        result[effect] = {
            "predicate": "bounty_movement",
            "bonus": bonus,
            "variants": {"base": freeze_steps([base])},
        }
    return result


def swift_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import CardState
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("Swift").model_copy(update={"id": OWNER})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)

    def freeze(effect, performer, **updates):
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False, **updates}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        return freeze_steps(build(state, performer, card, stats))

    previous = next(card for card in hero.deck if card.effect_id == "bounce").model_copy(
        update={"id": CARD, "state": CardState.RESOLVED}
    )
    with_previous = hero.model_copy(update={"played_cards": [previous, None]})
    return {
        "bounce": {
            "predicate": "resolved_or_reperformed",
            "variants": {
                "first": freeze("bounce", hero, state=CardState.UNRESOLVED),
                "repeat": freeze("bounce", hero, state=CardState.RESOLVED),
            },
        },
        "reload": {
            "predicate": "rightmost_played_card",
            "variants": {
                "empty": freeze("reload", hero),
                "present": freeze("reload", with_previous),
            },
        },
    }


def rowenna_recipes(*, raw=False):
    from goa2.domain.models import StatType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from model_traces import setup
    from primary_templates import ROWENNA_LANCE_ATTACKS

    state = setup(["Rowenna", "Hanu"], ["Brogan", "Tigerclaw"])
    hero = state.get_hero("hero_rowenna")
    hero.id = OWNER
    hero.items[StatType.RANGE] = RANGE - 2
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for effect in ROWENNA_LANCE_ATTACKS:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        variants = {}
        for name, level in (("melee", 7), ("lance", 8)):
            hero.level = level
            variants[name] = freeze_steps(build(state, hero, card, stats))
        result[effect] = {"predicate": "rowenna_lance", "variants": variants}
    return result


def dodger_respawn_recipes(*, raw=False):
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from model_traces import setup

    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for team, rosters in (
        ("RED", (["Dodger", "Hanu"], ["Brogan", "Tigerclaw"])),
        ("BLUE", (["Brogan", "Tigerclaw"], ["Dodger", "Hanu"])),
    ):
        state = setup(*rosters)
        hero = state.get_hero("hero_dodger")
        hero.id = OWNER
        state.remove_entity(state.teams[team].minions[0].id)
        for effect in ("necromancy", "necromastery"):
            card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
                update={"id": CARD, "is_facedown": False}
            )
            impl = CardEffectRegistry.get(effect)
            build = impl.build_steps if raw else impl.get_steps_with_stats
            result.setdefault(effect, {"predicate": "dodger_respawn", "variants": {}})["variants"][team] = (
                freeze_steps(build(state, hero, card, stats))
            )
    return result


def dodger_ritual_recipes(*, raw=False):
    from aura_traces import _board
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from model_traces import setup

    state = setup(["Dodger", "Hanu"], ["Brogan", "Tigerclaw"])
    h = _board(state)
    hero = state.get_hero("hero_dodger")
    hero.id = OWNER
    hero.level = 8
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for effect in ("dark_ritual", "darker_ritual", "darkest_ritual"):
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        hero.level = 8
        state.place_entity(hero.id, h(0, 0))
        enough = freeze_steps(build(state, hero, card, stats))
        state.remove_entity(hero.id)
        result[effect] = {
            "predicate": "dodger_ritual",
            "variants": {"enough": enough, "short": freeze_steps(build(state, hero, card, stats))},
        }
        if effect == "darkest_ritual":
            variants = result[effect]["variants"]
            result[effect]["predicate"] = "dodger_darkest_ritual"
            variants["short_unlocked"] = variants["short"]
            variants["enough_unlocked"] = variants["enough"]
            hero.level = 7
            variants["short"] = freeze_steps(build(state, hero, card, stats))
            state.place_entity(hero.id, h(0, 0))
            from unittest.mock import patch
            # Probe the successful count independently of the level gate.
            with patch("goa2.scripts.dodger_effects._count_empty_spawn_points", return_value=2):
                variants["enough"] = freeze_steps(build(state, hero, card, stats))
            state.remove_entity(hero.id)
    return result


def dread_razor_recipes(state, *, raw=False):
    from unittest.mock import patch
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("Dodger").model_copy(update={"id": OWNER})
    card = next(card for card in hero.deck if card.effect_id == "dread_razor").model_copy(
        update={"id": CARD, "is_facedown": False}
    )
    impl = CardEffectRegistry.get("dread_razor")
    build = impl.build_steps if raw else impl.get_steps_with_stats
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    variants = {"adjacent_only": freeze_steps(build(state, hero, card, stats))}
    point = next(iter(state.board.tiles))
    positioned = state.model_copy(update={
        "entity_locations": {**state.entity_locations, OWNER: point},
    })
    with patch("goa2.scripts.dodger_effects._is_adjacent_to_empty_spawn_in_battle_zone", return_value=True):
        variants["choice"] = freeze_steps(build(positioned, hero, card, stats))
    assert len(variants["choice"]) == 5
    return {"dread_razor": {"predicate": "dodger_dread_razor", "variants": variants}}


def death_trap_recipes(*, raw=False):
    from unittest.mock import patch
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from model_traces import setup

    state = setup(["Dodger", "Brogan"], ["Hanu", "Tigerclaw"])
    hero = state.get_hero("hero_dodger")
    excluded = state.get_hero("hero_tigerclaw")
    operand = "template-death-trap-excluded"
    for unit, replacement in ((hero, OWNER), (excluded, operand)):
        point = state.entity_locations[unit.id]
        state.remove_entity(unit.id)
        unit.id = replacement
        state.place_entity(replacement, point)
    card = next(card for card in hero.deck if card.effect_id == "death_trap").model_copy(
        update={"id": CARD, "is_facedown": False}
    )
    impl = CardEffectRegistry.get("death_trap")
    build = impl.build_steps if raw else impl.get_steps_with_stats
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    target = state.entity_locations[operand]
    predicate = "goa2.scripts.dodger_effects._is_adjacent_to_empty_spawn_in_battle_zone"
    with patch(predicate, side_effect=lambda _state, point, _owner: point != target):
        excluded_rows = freeze_steps(build(state, hero, card, stats))
    with patch(predicate, return_value=True):
        all_rows = freeze_steps(build(state, hero, card, stats))
    assert len(excluded_rows) == 3 and len(all_rows) == 2
    return {"death_trap": {
        "predicate": "dodger_death_trap",
        "operand": operand,
        "variants": {"all": all_rows, "excluded": excluded_rows},
    }}


def tali_winter_weapon_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import CardState
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from primary_templates import TALI_WINTER_WEAPONS

    hero = HeroRegistry.get("Tali").model_copy(update={"id": OWNER})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for effect in TALI_WINTER_WEAPONS:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False, "state": CardState.DISCARD}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        base, = build(state, hero, card, stats)
        card.state = CardState.UNRESOLVED
        boosted, = build(state, hero, card, stats)
        bonus = boosted.damage - base.damage
        assert base.damage == PRIMARY and bonus > 0
        assert freeze_steps([boosted.model_copy(update={"damage": PRIMARY})]) == freeze_steps([base])
        result[effect] = {
            "predicate": "tali_winter_weapon", "bonus": bonus,
            "variants": {"base": freeze_steps([base])},
        }
    return result


def tali_discard_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import CardState
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("Tali").model_copy(update={"id": OWNER})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for effect in ("guardian_spirit", "warrior_spirit", "spirit_wolf", "spirit_bear", "blizzard"):
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": "blizzard" if effect == "blizzard" else CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        variants = {}
        for name, lifecycle in (("played", CardState.UNRESOLVED), ("discarded", CardState.DISCARD)):
            card.state = lifecycle
            rows = freeze_steps(build(state, hero, card, stats))
            if effect == "blizzard":
                def bind_direction_keys(value):
                    if isinstance(value, dict):
                        return {key: bind_direction_keys(child) for key, child in value.items()}
                    if isinstance(value, list):
                        return [bind_direction_keys(child) for child in value]
                    if value in ("blizzard", "blizzard_direction", "blizzard_repeat_direction"):
                        return CARD + value.removeprefix("blizzard")
                    return value
                rows = bind_direction_keys(rows)
            variants[name] = rows
        result[effect] = {"predicate": "card_in_discard", "variants": variants}
    return result


def ursafar_rage_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from primary_templates import URSAFAR_RAGE_RECIPES

    hero = HeroRegistry.get("Ursafar").model_copy(update={"id": OWNER})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for effect in URSAFAR_RAGE_RECIPES:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        variants = {}
        bonus = 0
        for name, level in (("calm", 7), ("enraged", 8)):
            performer = hero.model_copy(update={"level": level})
            if effect == "angry_roar" and name == "enraged":
                from unittest.mock import patch

                with patch.object(type(impl), "_get_active_effect_card_ids", return_value=[CARD]):
                    steps = build(state, performer, card, stats)
            else:
                steps = build(state, performer, card, stats)
            if effect in ("cold_ire", "eyes_of_flame"):
                assert steps[0].type.value == "move_sequence"
                if name == "enraged":
                    bonus = steps[0].range_val - PRIMARY
                    assert bonus > 0
                    steps[0].range_val = PRIMARY
                else:
                    assert steps[0].range_val == PRIMARY
            variants[name] = freeze_steps(steps)
        result[effect] = {"predicate": "ursafar_enraged", "variants": variants}
        if bonus:
            assert variants["calm"] == variants["enraged"]
            result[effect]["primary_bonus"] = bonus
    return result


def ignatia_coin_recipes(state, *, raw=False):
    from combat_effect_traces import effect as make_effect
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import EffectType
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats
    from primary_templates import IGNATIA_COIN_RECIPES

    hero = HeroRegistry.get("Ignatia").model_copy(update={"id": OWNER})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    equilibrium = make_effect(state, "template-equilibrium", EffectType.EQUILIBRIUM, source=OWNER)
    result = {}
    for effect in IGNATIA_COIN_RECIPES:
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        variants = {}
        for branch in ("blue", "orange", "equilibrium"):
            world = state.model_copy(update={
                "tie_breaker_team": "RED" if branch == "orange" else "BLUE",
                "active_effects": [equilibrium] if branch == "equilibrium" else [],
            })
            for level in (7, 8):
                performer = hero.model_copy(update={"level": level})
                key = branch if level == 7 else f"{branch}_ultimate"
                variants[key] = freeze_steps(build(world, performer, card, stats))
        result[effect] = {"predicate": "ignatia_coin", "variants": variants}
    return result


def widget_skill_recipe(state, *, raw=False):
    from unittest.mock import patch

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("Widget").model_copy(update={"id": OWNER})
    card = next(card for card in hero.deck if card.effect_id == "fight_as_one").model_copy(
        update={"id": CARD, "is_facedown": False}
    )
    impl = CardEffectRegistry.get("fight_as_one")
    build = impl.build_steps if raw else impl.get_steps_with_stats
    with patch("goa2.scripts.widget_effects._performable_faceup_skill_card_ids", return_value=[CARD]):
        steps = build(state, hero, card, CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS))
    return {"fight_as_one": {"predicate": "faceup_skills", "variants": {"body": freeze_steps(steps)}}}


def nebkher_topology_recipes(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.hex import Hex
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("NebKher").model_copy(update={"id": OWNER})
    world = state.model_copy(update={"entity_locations": {OWNER: Hex(q=0, r=0, s=0)}})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    recipes = {}
    for effect in ("crack_in_reality", "shift_reality"):
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        steps = build(world, hero, card, stats)
        assert len(steps) == 7
        recipes[effect] = {"predicate": "topology_anchor", "variants": {"body": freeze_steps(steps)}}
    return recipes


def razzle_swap_recipes(state, *, raw=False):
    from unittest.mock import patch

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("Razzle").model_copy(update={"id": OWNER})
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=RADIUS)
    result = {}
    for effect in ("theatrics", "spectacle", "alleyoop", "group_performance", "team_spirit"):
        card = next(card for card in hero.deck if card.effect_id == effect).model_copy(
            update={"id": CARD, "is_facedown": False}
        )
        impl = CardEffectRegistry.get(effect)
        build = impl.build_steps if raw else impl.get_steps_with_stats
        with patch("goa2.scripts.razzle_effects._acting_piece", return_value="template-razzle-piece"):
            steps = build(state, hero, card, stats)
        result[effect] = {"predicate": "acting_piece", "variants": {"body": freeze_steps(steps)}}
    return result


def stunt_doubles_recipe(state, *, raw=False):
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    hero = HeroRegistry.get("Razzle").model_copy(update={"id": OWNER})
    card = next(card for card in hero.deck if card.effect_id == "stunt_doubles").model_copy(
        update={"id": CARD, "is_facedown": False}
    )
    impl = CardEffectRegistry.get("stunt_doubles")
    build = impl.build_steps if raw else impl.get_steps_with_stats
    # The existing radius-or-one operand is shared with other primary recipes.
    stats = CardStats(primary_value=PRIMARY, range=RANGE, radius=1234567900)
    variants = {}
    for name, level in (("without_ultimate", 7), ("with_ultimate", 8)):
        performer = hero.model_copy(update={"level": level})
        variants[name] = freeze_steps(build(state, performer, card, stats))
    return {"stunt_doubles": {
        "predicate": "has_level_eight_ultimate", "required_ultimate": "twin_strike",
        "variants": variants,
    }}


def catalog():
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffect, CardEffectRegistry, bind_effect_cards
    from goa2.engine.stats import CardStats

    state = setup()
    recipes, ranged_steps, radius_fallbacks, raw_bindings = {}, {}, {}, {}
    range_fallbacks = {}
    from primary_radius import bind_radius_fallbacks, bind_range_fallbacks, merge_operand_bindings

    methods = {
        name: type(effect).build_steps.__qualname__
        for name, effect in sorted(CardEffectRegistry._effects.items())
    }

    for effect, name in {**EFFECTS, **LIVE_ONLY_EFFECTS}.items():
        primary = CHARGE_PRIMARY_PROBE if effect in CHARGE_ATTACKS else PRIMARY
        hero = HeroRegistry.get(name).model_copy(update={"id": OWNER})
        card = next(
            card
            for card in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
            if card.effect_id == effect
        )
        printed_sensitive = effect in {
            "illusionary_force", "twist_fate", "fleeting_image", "multiple_projections",
            "phantasmal_warrior",
        }
        card = card.model_copy(update={
            "id": card.id if printed_sensitive else CARD, "is_facedown": False,
        })
        stats = CardStats(primary_value=primary, range=RANGE, radius=RADIUS)
        impl = CardEffectRegistry.get(effect)
        def build_bound(card, stats):
            if printed_sensitive:
                return bind_effect_cards(impl.build_steps(state, hero, card, stats), CARD)
            return impl.get_steps_with_stats(state, hero, card, stats)

        recipes[effect] = freeze_steps(build_bound(card, stats))
        raw_bindings[effect] = raw_binding_paths(
            recipes[effect], freeze_steps(impl.build_steps(state, hero, card, stats))
        )
        variants = []
        # These builders require an explicit radius, including in probe states.
        radius_probes = (0, 0) if effect in {
            "awaken", "crawling_dead", "walking_dead", "braains",
            "airborne_attack", "airborne_assault",
        } else (None, 0)
        for radius in radius_probes:
            variant = freeze_steps(
                build_bound(
                    card, CardStats(primary_value=primary, range=RANGE, radius=radius)
                )
            )
            assert len(variant) == len(recipes[effect])
            for present, changed in zip(recipes[effect], variant, strict=True):
                assert present["objects"] == changed["objects"]
            variants.append([row["fields"] for row in variant])
        fields = bind_radius_fallbacks(
            [row["fields"] for row in recipes[effect]],
            variants[0],
            variants[1],
            RADIUS,
            radius_fallbacks,
        )
        zero = freeze_steps(
            build_bound(
                card, CardStats(primary_value=primary, range=0, radius=RADIUS)
            )
        )
        for original, changed in zip(recipes[effect], zero, strict=True):
            assert (
                original["objects"] == changed["objects"]
            ), "range-dependent aliases need a native port"
            assert original.get("imports") == changed.get(
                "imports"
            ), "range-dependent cross-root aliases need a native port"
        range_fields = bind_range_fallbacks(
            [row["fields"] for row in recipes[effect]],
            [row["fields"] for row in zero],
            RANGE,
            range_fallbacks,
        )
        for row, bound_fields, range_bound in zip(
            recipes[effect], fields, range_fields, strict=True
        ):
            row["fields"] = merge_operand_bindings(row["fields"], bound_fields, range_bound, RANGE)
        variants = []
        for ranged in (False, True):
            changed = card.model_copy(update={"is_ranged": ranged})
            variants.append(freeze_steps(build_bound(changed, stats)))
        assert len(variants[0]) == len(variants[1])
        changed = []
        for index, (a, b) in enumerate(zip(*variants, strict=True)):
            if a != b:
                assert a["objects"] == b["objects"]
                assert a["fields"]["type"] == b["fields"]["type"] == "attack_sequence"
                assert {**a["fields"], "is_ranged": False} == {**b["fields"], "is_ranged": False}
                changed.append(index)
        ranged_steps[effect] = changed
        bind_charge_damage(effect, [row["fields"] for row in recipes[effect]])
    # Primitive fallback construction is frozen separately from registered empty bodies.
    from goa2.engine.steps import AttackSequenceStep, LogMessageStep, MoveSequenceStep

    fallback = {
        "movement": freeze_steps([MoveSequenceStep(unit_id=OWNER, range_val=PRIMARY)]),
        "attack": freeze_steps([AttackSequenceStep(damage=PRIMARY, range_val=RANGE)]),
        "defense": freeze_steps([LogMessageStep(message="template-primary-defense-log")]),
        "skill": freeze_steps([LogMessageStep(message="template-primary-skill-log")]),
    }
    from min_builders import stateful_recipes as min_stateful_recipes
    from misa_builders import stateful_recipes

    from snorri_builders import stateful_recipes as snorri_stateful_recipes
    from whisper_builders import stateful_recipes as whisper_stateful_recipes

    from takahide_builders import stateful_recipes as takahide_stateful_recipes

    from emmitt_builders import stateful_recipes as emmitt_stateful_recipes

    from mortimer_builders import stateful_recipes as mortimer_stateful_recipes

    stateful = {
        **stunt_doubles_recipe(state),
        **razzle_swap_recipes(state),
        **nebkher_topology_recipes(state),
        **widget_skill_recipe(state),
        **ignatia_coin_recipes(state),
        **ursafar_rage_recipes(state),
        **tali_discard_recipes(state),
        **tali_winter_weapon_recipes(state),
        **death_trap_recipes(),
        **dread_razor_recipes(state),
        **dodger_ritual_recipes(),
        **dodger_respawn_recipes(),
        **rowenna_recipes(),
        **swift_recipes(state),
        **bounty_movement_recipes(state),
        **mortimer_stateful_recipes(state),
        **emmitt_stateful_recipes(raw=False),
        **takahide_stateful_recipes(state),
        **stateful_recipes(state), **min_stateful_recipes(state), **snorri_stateful_recipes(state),
        **whisper_stateful_recipes(state),
    }
    raw_stateful = {
        **stunt_doubles_recipe(state, raw=True),
        **razzle_swap_recipes(state, raw=True),
        **nebkher_topology_recipes(state, raw=True),
        **widget_skill_recipe(state, raw=True),
        **ignatia_coin_recipes(state, raw=True),
        **ursafar_rage_recipes(state, raw=True),
        **tali_discard_recipes(state, raw=True),
        **tali_winter_weapon_recipes(state, raw=True),
        **death_trap_recipes(raw=True),
        **dread_razor_recipes(state, raw=True),
        **dodger_ritual_recipes(raw=True),
        **dodger_respawn_recipes(raw=True),
        **rowenna_recipes(raw=True),
        **swift_recipes(state, raw=True),
        **bounty_movement_recipes(state, raw=True),
        **mortimer_stateful_recipes(state, raw=True),
        **emmitt_stateful_recipes(raw=True),
        **takahide_stateful_recipes(state, raw=True),
        **stateful_recipes(state, raw=True),
        **min_stateful_recipes(state, raw=True),
        **snorri_stateful_recipes(state, raw=True),
        **whisper_stateful_recipes(state, raw=True),
    }
    for effect, body in stateful.items():
        body["raw_card_bindings"] = {
            name: raw_binding_paths(rows, raw_stateful[effect]["variants"][name])
            for name, rows in body["variants"].items()
        }
    source = {
        "schema_version": 3,
        "source_sha256": verify_sources()["source_sha256"],
        "methods": methods,
        "spell_effects": sorted(CardEffectRegistry._spell_effects),
        "recipes": recipes,
        "stateful_recipes": stateful,
        "raw_card_bindings": raw_bindings,
        "ranged_steps": ranged_steps,
        "radius_fallbacks": radius_fallbacks,
        "range_fallbacks": range_fallbacks,
        "fallback": fallback,
        "operands": {
            "owner": OWNER,
            "card": CARD,
            "primary": PRIMARY,
            "range": RANGE,
            "radius": RADIUS,
        },
        "default_method": CardEffect.build_steps.__qualname__,
    }
    return {**source, "catalog_sha256": digest(source)}


def export():
    payload = catalog()
    write_text(
        ROOT / "rust/tests/fixtures/primary-builders.json", json.dumps(payload, indent=2) + "\n"
    )
    print(
        {
            "effects": len(payload["recipes"]),
            "methods": len(payload["methods"]),
            "catalog_sha256": payload["catalog_sha256"],
        }
    )


if __name__ == "__main__":
    export()
