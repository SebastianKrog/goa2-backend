"""Complete spell actions and explicit state-dependent branch fixtures.

Raw setup retains physical spell/hero/effect/container aliases and source step
boundaries. It deliberately uses the live CastSpell/PerformCardAction paths.
"""

from __future__ import annotations

from spell_builders import EFFECTS


def base(effect, variant="ordinary"):
    from live_spellbook_traces import base as spellbook

    from goa2.domain.hex import Hex
    from goa2.domain.models import CardState, MinionType, StatType, TokenType

    state, hero = spellbook()

    def h(q, r):
        return Hex(q=q, r=r, s=-q - r)

    card = next(c for c in hero.spells if c.effect_id == effect)
    state.place_entity("hero_arien", h(1, 0))
    state.place_entity("hero_tigerclaw", h(2, 0))
    foe = next(m for m in state.teams["BLUE"].minions if m.type == MinionType.MELEE)
    state.place_entity(foe.id, h(1, -1))
    friend = next(m for m in state.teams["RED"].minions if m.type == MinionType.MELEE)
    state.place_entity(friend.id, h(0, -1))
    for count, label in ((2, "outside-two"), (3, "outside-three"), (7, "outside-many")):
        if variant == label:
            for other in [c for c in hero.spells if c is not card][:count]:
                other.state = CardState.OUTSIDE_SPELLBOOK
                other.is_facedown = False
    if variant in ("wish-count-two", "wish-no-other", "wish-fizzle"):
        hero.wish_cast_count = 2
        if variant != "wish-count-two":
            for other in hero.spells:
                if other.id != "wish" and not (
                    variant == "wish-fizzle" and other.id == "magic_missile"
                ):
                    other.state = CardState.OUTSIDE_SPELLBOOK
                    other.is_facedown = False
        if variant == "wish-fizzle":
            for enemy in [*state.teams["BLUE"].heroes, *state.teams["BLUE"].minions]:
                state.remove_entity(enemy.id)
    if variant == "negative-attack":
        hero.items = {StatType.ATTACK: -12}
    if variant == "radius-none":
        card.radius_value = None
    if variant == "radius-zero":
        card.radius_value = 0
    if variant == "radius-negative":
        card.radius_value = -1
    if effect == "vampiric_touch":
        spare = hero.hand[0]
        hero.hand.remove(spare)
        hero.discard_pile.append(spare)
    if effect == "energy_drain":
        victim = state.get_hero("hero_arien")
        victim.hand[:] = [c for c in victim.deck if c.tier.value not in ("I", "UNTIERED")][:2]
        state.teams["RED"].life_counters = 2
    if effect == "power_word_kill" and variant != "has-hand":
        state.get_hero("hero_arien").hand.clear()
    if variant == "friendly-neighbor":
        state.place_entity("hero_hanu", h(2, -1))
    if variant == "token":
        token = state.token_pool[TokenType.FAMILIAR][0]
        token.owner_id = hero.id
        state.place_entity(token.id, h(-1, 0))
        state.execution_context["spell_token_alias"] = token
    if effect == "sunburst":
        from goa2.engine.stats import compute_card_stats

        count = sum(c.state == CardState.OUTSIDE_SPELLBOOK and c.id != card.id for c in hero.spells)
        distance = (compute_card_stats(state, hero.id, card).range or 0) + count
        if 0 < distance <= 4:
            state.place_entity("hero_tigerclaw", h(distance, 0))
    if effect == "create_undead":
        from goa2.domain.board import Zone
        from goa2.domain.models.spawn import SpawnPoint, SpawnType

        state.remove_entity(friend.id)
        state.place_entity("hero_arien", h(-3, 1))
        state.place_entity("hero_tigerclaw", h(3, -1))
        spawns = [
            SpawnPoint(
                location=h(1, 0), team="RED", type=SpawnType.MINION, minion_type=MinionType.MELEE
            ),
            SpawnPoint(
                location=h(2, 0), team="RED", type=SpawnType.MINION, minion_type=MinionType.RANGED
            ),
        ]
        zone = Zone(id="spell-zone", hexes=set(state.board.tiles), spawn_points=spawns)
        state.board.zones = {zone.id: zone}
        state.board.lanes = {"lane_1": [zone.id]}
        state.board.spawn_points = spawns
        for tile in state.board.tiles.values():
            tile.zone_id = zone.id
        for spawn in spawns:
            state.board.tiles[spawn.location].spawn_point = spawn
        state.battle_zones = {"lane_1": zone.id}
        if variant == "one-type":
            for minion in state.teams["RED"].minions:
                if minion.type == MinionType.RANGED:
                    minion.lane_id = "other-lane"
        if variant == "full-zone":
            for kind, point in ((MinionType.MELEE, h(-2, 0)), (MinionType.RANGED, h(-2, 1))):
                minion = next(m for m in state.teams["RED"].minions if m.type == kind)
                state.place_entity(minion.id, point)
        if variant == "no-zone":
            state.battle_zones = {}
        if variant == "no-team":
            hero.team = None
    state.execution_context.update(
        spell_alias=card,
        hand_alias=hero.hand,
        discard_alias=hero.discard_pile,
        locations_alias=state.entity_locations,
        token_pool_alias=state.token_pool,
        _cmf_origin_hex={"retained": True},
    )
    return state, hero, card


def action_scene(effect, entry="cast", mode="first", variant="ordinary", caster=None):
    from live_defeat_traces import operations

    from goa2.domain.input import selection_value
    from goa2.domain.models import CardState
    from goa2.engine.steps import CastSpellStep, PerformCardActionStep, SetContextFlagStep

    state, hero, card = base(effect, variant)
    if caster is not None:
        state.current_actor_id = caster
        if effect == "magic_missile":
            from goa2.domain.hex import Hex

            state.place_entity("hero_gydion", Hex(q=-1, r=0, s=1))
        if effect == "create_undead":
            from goa2.domain.hex import Hex
            from goa2.domain.models import MinionType
            from goa2.domain.models.spawn import SpawnPoint, SpawnType

            point = Hex(q=-1, r=1, s=0)
            spawn = SpawnPoint(
                location=point, team="BLUE", type=SpawnType.MINION, minion_type=MinionType.MELEE
            )
            state.board.spawn_points.append(spawn)
            state.board.zones["spell-zone"].spawn_points.append(spawn)
            state.board.tiles[point].spawn_point = spawn
            for minion in state.teams["BLUE"].minions:
                state.remove_entity(minion.id)
    if entry == "cast":
        step = CastSpellStep(allowed_spell_ids=[card.id], caster_id=caster or hero.id)
    else:
        card.state = CardState.OUTSIDE_SPELLBOOK
        card.is_facedown = False
        state.execution_context.update(
            spell_owner_id=hero.id, spell_caster_id=caster or hero.id, cast_spell_id=card.id
        )
        step = PerformCardActionStep(
            card_key="cast_spell_id",
            card_owner_key="spell_owner_id",
            hero_id=caster or hero.id,
            suppress_after_resolve_card=True,
        )
    step.step_id = "fixture/spell-body"
    tail = SetContextFlagStep(step_id="fixture/spell-body-tail", key="spell_body_tail", value=True)
    state.execution_stack = [tail, step]
    state.execution_context["spell_body_step"] = step
    bad = False

    def select(request, _mode):
        nonlocal bad
        if mode == "invalid" and not bad:
            bad = True
            return "not-offered"
        kind = request.request_type.value
        if kind == "SELECT_CARD" and request.prompt == "Choose a prepared spell to cast":
            if any(o.id == card.id for o in request.options):
                return card.id
            for wanted in ("expeditious_retreat", "magic_missile"):
                if any(o.id == wanted for o in request.options):
                    return wanted
        if kind == "CHOOSE_ACTION":
            primary = (
                "SKILL"
                if card.primary_action.value == "DEFENSE_SKILL"
                else card.primary_action.value
            )
            if effect == "wish" and not any(o.id == primary for o in request.options):
                primary = request.options[0].id
            return primary
        if kind == "SELECT_CARD_OR_PASS":
            return selection_value(request.options[0]) if variant == "defend" else "PASS"
        if mode == "skip" and request.can_skip:
            return "SKIP"
        if kind in ("SELECT_UNIT", "SELECT_UNIT_OR_TOKEN"):
            if variant == "token":
                for option in request.options:
                    if "familiar" in option.id:
                        return option.id
            for wanted in ("hero_arien", "hero_tigerclaw"):
                if any(o.id == wanted for o in request.options):
                    return wanted
        return selection_value(request.options[-1] if mode == "last" else request.options[0])

    ops, final = operations(state, select=select)
    return state, [*ops, {"kind": "advance_prefix", "steps": 0}], final


def scenes():
    for effect in EFFECTS:
        for entry in ("cast", "copy"):
            for mode in ("first", "last", "skip", "invalid"):
                state, ops, _ = action_scene(effect, entry, mode)
                yield f"spell-body-{effect}-{entry}-{mode}", state, ops
    variants = {
        "shocking_grasp": ["defend", "negative-attack"],
        "magic_missile": ["negative-attack"],
        "fireball": ["friendly-neighbor", "negative-attack"],
        "create_undead": ["one-type", "full-zone", "no-zone", "no-team"],
        "midas_touch": ["outside-two", "outside-three", "outside-many"],
        "dimension_door": ["outside-two", "radius-none", "radius-zero", "radius-negative"],
        "sunburst": ["outside-two", "outside-three", "negative-attack"],
        "disintegrate": ["token"],
        "banishment": ["token"],
        "polymorph": ["token"],
        "power_word_kill": ["has-hand"],
    }
    for effect, items in variants.items():
        for variant in items:
            for mode in ("first", "skip"):
                state, ops, _ = action_scene(effect, "cast", mode, variant)
                yield f"spell-body-{effect}-{variant}-{mode}", state, ops


def foreign_scenes():
    for effect in ("magic_missile", "shield", "create_undead", "midas_touch", "wish"):
        for entry in ("cast", "copy"):
            variant = "outside-three" if effect == "midas_touch" else "ordinary"
            state, ops, _ = action_scene(effect, entry, "first", variant, caster="hero_arien")
            yield f"spell-body-{effect}-foreign-{entry}", state, ops


def consumer_scenes():
    from live_defeat_traces import operations, retain

    from goa2.domain.input import selection_value
    from goa2.domain.models import ActionType, CardState
    from goa2.engine.filters_units import ContextIdsFilter, ImmunityFilter
    from goa2.engine.steps import CastSpellStep, CountStep, SetContextFlagStep

    for effect in ("shield", "invulnerability"):
        for basic in (False, True):
            _, _, state = action_scene(effect)
            attacker = state.get_hero("hero_arien")
            card = next(c for c in attacker.deck if c.is_basic == basic)
            card.is_facedown = False
            state.current_actor_id = attacker.id
            state.execution_context.update(
                current_action_type=ActionType.ATTACK,
                performing_card_id=card.id,
                performing_card_owner_id=attacker.id,
                protected=["hero_gydion"],
            )
            step = CountStep(
                filters=[ContextIdsFilter(ids_key="protected"), ImmunityFilter()],
                output_key="spell_attack_targets",
            )
            ops, _ = retain(state, [step])
            yield f"spell-body-{effect}-consumer-{basic}", state, ops
    _, _, state = action_scene("find_familiar")
    owner = state.get_hero("hero_gydion")
    card = next(c for c in owner.spells if c.id == "find_familiar")
    card.state = CardState.SPELLBOOK
    card.is_facedown = True
    step = CastSpellStep(
        allowed_spell_ids=[card.id], caster_id=owner.id, step_id="fixture/familiar-recast"
    )
    state.execution_stack = [
        SetContextFlagStep(
            key="familiar_recast_done", value=True, step_id="fixture/familiar-recast-tail"
        ),
        step,
    ]

    def select(request, _mode):
        if request.prompt == "Choose a prepared spell to cast":
            return card.id
        if request.request_type.value == "CHOOSE_ACTION":
            return "SKILL"
        if request.can_skip:
            return "SKIP"
        return selection_value(request.options[-1])

    ops, _ = operations(state, select=select)
    yield "spell-body-find_familiar-recast", state, [*ops, {"kind": "advance_prefix", "steps": 0}]


def extra_scenes():
    yield from foreign_scenes()
    yield from consumer_scenes()
    for variant in ("wish-count-two", "wish-no-other", "wish-fizzle"):
        state, ops, _ = action_scene("wish", variant=variant)
        yield "spell-body-wish-" + variant, state, ops
