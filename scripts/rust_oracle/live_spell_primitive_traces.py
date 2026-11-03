"""Reusable spell rule predicates and both minion-respawn input protocols.

Raw setup exposes collection holes, aliases, live filter models, and respawn
continuation data independently of spell action dispatch.
"""

from __future__ import annotations


def query_scenes():
    from live_defeat_traces import retain
    from live_spell_body_traces import base

    from goa2.domain.models import CardContainerType, TargetType, TokenType
    from goa2.engine.filters_cards import CardsInContainerFilter
    from goa2.engine.filters_composite import CountMatchFilter, OrFilter
    from goa2.engine.filters_hex import RangeFilter, TerrainFilter
    from goa2.engine.filters_units import TeamFilter, UnitTypeFilter
    from goa2.engine.steps import CheckUnitFiltersStep, CountStep

    for container in CardContainerType:
        for visible in (False, True):
            for target in ("hero_gydion", "minion", "absent"):
                state, hero, _ = base("power_word_kill")
                hero.hand[0].is_facedown = True
                hero.played_cards[:] = [None, hero.hand[0], hero.hand[1], hero.hand[1]]
                if target == "minion":
                    target = state.teams["BLUE"].minions[0].id
                state.execution_context.update(
                    predicate_target=target,
                    predicate_result="retained",
                    played_alias=hero.played_cards,
                )
                predicate = CardsInContainerFilter(
                    container=container, min_cards=0, max_cards=2, exclude_facedown=visible
                )
                state.execution_context["predicate_alias"] = predicate
                step = CheckUnitFiltersStep(
                    unit_key="predicate_target", filters=[predicate], output_key="predicate_result"
                )
                ops, _ = retain(state, [step])
                yield f"spell-primitive-cards-{container.value}-{visible}-{target}", state, ops
    for kind in ("empty", "kind", "terrain", "or", "missing", "falsey", "skip"):
        state, _, _ = base("disintegrate")
        filters = [] if kind == "empty" else [UnitTypeFilter(unit_type="TOKEN")]
        if kind == "terrain":
            filters = [TerrainFilter(is_terrain=True)]
        if kind == "or":
            filters = [
                OrFilter(
                    filters=[UnitTypeFilter(unit_type="TOKEN"), TerrainFilter(is_terrain=True)]
                )
            ]
        state.execution_context.update(
            predicate_target=0 if kind == "falsey" else "absent", predicate_result="retained"
        )
        if kind == "missing":
            state.execution_context.pop("predicate_target")
        step = CheckUnitFiltersStep(
            unit_key="predicate_target", filters=filters, output_key="predicate_result"
        )
        if kind == "skip":
            step.skip_if_key = "predicate_result"
        ops, _ = retain(state, [step])
        yield "spell-primitive-check-" + kind, state, ops
    for target_type in ("UNIT", "HEX"):
        for variant in ("empty", "neighbor", "token"):
            for tokens in (False, True):
                from goa2.domain.hex import Hex

                state, hero, _ = base("fireball")
                if variant == "neighbor":
                    state.place_entity("hero_hanu", Hex(q=2, r=-1, s=-1))
                sub = [
                    TeamFilter(relation="FRIENDLY"),
                    RangeFilter(
                        min_range=1, max_range=1, origin_hex_key=CountMatchFilter.ORIGIN_HEX_KEY
                    ),
                ]
                if variant == "token":
                    token = state.token_pool[TokenType.FAMILIAR][0]
                    token.owner_id = hero.id
                    state.place_entity(token.id, Hex(q=2, r=1, s=-3))
                    sub[0] = UnitTypeFilter(unit_type="TOKEN")
                predicate = CountMatchFilter(
                    sub_filters=sub, min_count=1, max_count=1, include_tokens=tokens
                )
                state.execution_context.update(
                    predicate_target="hero_tigerclaw",
                    predicate_result="retained",
                    predicate_alias=predicate,
                    _cmf_origin_hex={"retained": True},
                )
                if target_type == "UNIT":
                    step = CheckUnitFiltersStep(
                        unit_key="predicate_target",
                        filters=[predicate],
                        output_key="predicate_result",
                    )
                else:
                    step = CountStep(
                        target_type=TargetType.HEX,
                        filters=[predicate],
                        output_key="predicate_result",
                    )
                ops, _ = retain(state, [step])
                yield f"spell-primitive-count-{target_type}-{variant}-{tokens}", state, ops


def respawn_scenes():
    from live_defeat_traces import retain
    from live_spell_body_traces import base

    from goa2.domain.input import selection_value
    from goa2.domain.models import MinionType
    from goa2.engine.steps import RespawnMinionAtHexStep

    for variant in (
        "ordinary",
        "last",
        "invalid",
        "invalid-hex",
        "bad-second",
        "stale-hex",
        "stale-occupied",
        "legacy",
        "legacy-invalid",
        "legacy-missing",
        "off-zone",
        "one-type",
        "full-zone",
    ):
        state, _hero, _ = base("create_undead", variant)
        legacy = variant.startswith("legacy")
        step = RespawnMinionAtHexStep(
            team="RED", lane_bound=not legacy, unit_key="respawn_unit" if legacy else None
        )
        minion = next(m for m in state.teams["RED"].minions if m.type == MinionType.MELEE)
        state.execution_context["respawn_unit"] = (
            "absent" if variant == "legacy-missing" else minion.id
        )
        if variant.startswith("stale-"):
            state.execution_context["lane_respawn_hex"] = {"q": 0, "r": 0, "s": 0}
            step.pending_input = {
                "selection": minion.id if variant == "stale-occupied" else "absent"
            }
        if variant == "off-zone":
            state.board.zones = {}
            state.board.lanes = {}
            state.battle_zones = {}
            for tile in state.board.tiles.values():
                tile.zone_id = None
        state.execution_context["respawn_alias"] = step
        bad = False

        def select(request, _mode, variant=variant):
            nonlocal bad
            if not bad and variant in ("invalid", "invalid-hex", "legacy-invalid"):
                bad = True
                return {"q": 99, "r": 0, "s": -99} if variant == "invalid-hex" else "not-offered"
            if variant == "bad-second" and request.request_type.value == "SELECT_OPTION":
                return "not-offered"
            return selection_value(request.options[-1] if variant == "last" else request.options[0])

        ops, _ = retain(state, [step], select=select)
        yield "spell-primitive-respawn-" + variant, state, ops


def scenes():
    yield from query_scenes()
    yield from respawn_scenes()
    yield from tide_scenes()
    yield from null_origin_scenes()


def tide_scenes():
    from live_defeat_traces import retain
    from live_spell_body_traces import base

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.filters_hex import BattleZoneFilter, SpawnPointTeamFilter
    from goa2.engine.steps import RespawnMinionAtHexStep

    for spawns in (False, True):
        state, hero, _ = base("create_undead")
        hero.level = 8
        hero.ultimate_card = HeroRegistry.get("Dodger").ultimate_card.model_copy(deep=True)
        filters = [BattleZoneFilter()]
        if spawns:
            filters.append(SpawnPointTeamFilter(relation="FRIENDLY"))
        step = RespawnMinionAtHexStep(
            team="RED",
            lane_bound=True,
            hex_filters=filters,
            pending_input={"selection": {"q": 99, "r": 0, "s": -99}},
        )
        ops, _ = retain(state, [step])
        yield f"spell-primitive-respawn-tide-{spawns}", state, ops


def null_origin_scenes():
    """CountMatch's finally block deletes an explicit null, only if evaluated."""
    from live_defeat_traces import retain
    from live_spell_body_traces import base

    from goa2.domain.models import TargetType
    from goa2.engine.filters_composite import CountMatchFilter
    from goa2.engine.filters_units import TeamFilter
    from goa2.engine.steps import CheckUnitFiltersStep, CountStep, SelectStep

    for mode in ("unit", "hex", "select", "missing", "skip", "output"):
        state, _, _ = base("fireball")
        state.execution_context.update(
            _cmf_origin_hex=None, predicate_target="hero_gydion", predicate_result="retained"
        )
        predicate = CountMatchFilter(sub_filters=[TeamFilter(relation="FRIENDLY")], min_count=1)
        output = "_cmf_origin_hex" if mode == "output" else "predicate_result"
        if mode == "hex":
            step = CountStep(target_type=TargetType.HEX, filters=[predicate], output_key=output)
        elif mode == "select":
            step = SelectStep(
                target_type=TargetType.UNIT,
                filters=[predicate],
                output_key=output,
                prompt="Select matching unit",
            )
        else:
            step = CheckUnitFiltersStep(
                unit_key="predicate_target", filters=[predicate], output_key=output
            )
        if mode == "missing":
            state.execution_context["predicate_target"] = "absent"
        if mode == "skip":
            step.skip_if_key = "predicate_result"
        ops, _ = retain(state, [step])
        yield "spell-primitive-count-null-origin-" + mode, state, ops
