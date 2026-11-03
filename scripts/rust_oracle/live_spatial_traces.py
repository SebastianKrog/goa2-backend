"""Live hex predicates, key identity and movement reuse source scenario builders."""

from __future__ import annotations


def scenes(*, tail_only=False):
    import copy

    from live_passive_traces import build
    from movement_sequence_traces import drive, filter_states, sequence_scenes
    from object_execution_traces import live_choice_operations
    from oracle import ROOT

    from goa2.domain.board import Board
    from goa2.domain.models import GamePhase, TargetType
    from goa2.engine.filters_geometry import (
        InStraightLineFilter,
        NotInStraightLineFilter,
        StraightLinePathFilter,
    )
    from goa2.engine.filters_hex import (
        AdjacentSpawnPointFilter,
        FastTravelDestinationFilter,
        MovementPathFilter,
        ObstacleFilter,
        RangeFilter,
        SpawnPointFilter,
    )
    from goa2.engine.filters_units import AdjacencyFilter
    from goa2.engine.setup import GameSetup
    from goa2.engine.steps import MoveSequenceStep, SelectStep

    def retain(state, step, mode="first"):
        state.execution_stack = [step]
        state.execution_context.update(
            selector=step, pending_alias=step.pending_input, filters_alias=step.filters
        )
        operations = live_choice_operations(state, mode, stop_types=())
        operations.append({"kind": "advance_prefix", "steps": 0})
        return operations

    if not tail_only:
        for name, state, steps, mode in sequence_scenes():
            emitted = []
            drive(
                name,
                state,
                steps,
                mode=mode,
                emit=lambda n, s, o, emitted=emitted: emitted.append(
                    ("live-" + n, s, [*o, {"kind": "advance_prefix", "steps": 0}])
                ),
            )
            yield emitted.pop()

        # Every supported filter family and each typed context operand, across the
        # same effect/topology states as the ordinary spatial fixtures.
        for name, state, h, pieces in filter_states():
            source = copy.deepcopy(state)
            source.execution_context.update(origin=pieces[0], from_hex=h(1, 0), low=False, high=2)
            filters = [
                RangeFilter(origin_key="origin", min_range_key="low", max_range_key="high"),
                RangeFilter(origin_hex_key="from_hex", max_range=2),
                MovementPathFilter(unit_key="origin", range_val=2),
                ObstacleFilter(is_obstacle=False, exclude_id_key="origin"),
                InStraightLineFilter(origin_key="origin"),
                NotInStraightLineFilter(origin_key="origin"),
                StraightLinePathFilter(origin_key="origin"),
                SpawnPointFilter(has_spawn_point=True),
                AdjacentSpawnPointFilter(),
                AdjacencyFilter(target_tags=["ENEMY", "HERO"]),
                FastTravelDestinationFilter(unit_id="hero_arien"),
            ]
            for index, condition in enumerate(filters):
                # The full ordinary predicate matrix already checks numeric rules;
                # these live scenes check fresh fields/context and prompt identity.
                current = copy.deepcopy(source)
                step = SelectStep(
                    step_id="fixture/live-hex/filter",
                    target_type=TargetType.HEX,
                    prompt="Live hex",
                    output_key="chosen",
                    filters=[condition.model_copy(deep=True)],
                    is_mandatory=False,
                )
                yield f"hex-filter-{name}-{index}", current, retain(current, step, "last")

    for variant in (
        "tile-key",
        "private-key",
        "visible-key",
        "pending-frozen",
        "pending-dict",
        "pending-invalid",
        "optional-skip",
        "no-candidates",
        "unknown-owner",
        "piece-owner",
        "auto-dirty",
    ):
        state, hero = build("Razzle" if variant == "piece-owner" else "Arien")
        key = next(iter(state.board.tiles))
        tile = state.board.tiles[key]
        if variant in {"private-key", "visible-key"}:
            tile.hex = type(key).model_validate(key.model_dump())
            assert tile.hex == key and tile.hex is not key
        state.board = Board(tiles={} if variant == "no-candidates" else {key: tile})
        step = SelectStep(
            step_id="fixture/live-hex/identity",
            target_type=TargetType.HEX,
            prompt="Identity",
            output_key="chosen",
            auto_select_if_one=variant in {"tile-key", "private-key", "visible-key", "auto-dirty"},
            is_mandatory=variant != "optional-skip",
        )
        state.execution_context["chosen"] = "old"
        if variant == "visible-key":
            state.execution_context["key_alias"] = key
        if variant == "pending-frozen":
            step.pending_input = {"selection": key}
        if variant == "pending-dict":
            step.pending_input = {"selection": key.model_dump()}
        if variant in {"pending-invalid", "auto-dirty"}:
            step.pending_input = {"selection": {"q": 0}}
        if variant == "optional-skip":
            step.pending_input = {"selection": "SKIP"}
        if variant == "unknown-owner":
            step.override_player_id = "team:BLUE"
        if variant == "piece-owner":
            step.override_player_id = f"{hero.id}_piece_1"
        step.pending_request_id = "fixture/live-hex/old-request"
        yield "hex-identity-" + variant, state, retain(state, step)

    for filename in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        for mask in range(4):
            state = GameSetup.create_game(
                str(filename), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
            )
            state.phase = GamePhase.RESOLUTION
            state.current_actor_id = "hero_arien"
            start = next(
                key
                for key, tile in state.board.tiles.items()
                if not tile.is_terrain and key not in state.entity_locations.values()
            )
            state.place_entity("hero_arien", start)
            step = MoveSequenceStep(
                step_id="fixture/live-hex/map",
                unit_id="hero_arien",
                range_val=2,
                force_straight_line=bool(mask & 1),
                force_full_distance=bool(mask & 2),
                is_mandatory=False,
            )
            state.execution_stack = [step]
            operations = live_choice_operations(state, "last", stop_types=())
            operations.append({"kind": "advance_prefix", "steps": 0})
            yield f"live-map-movement-{filename.stem}-{mask}", state, operations

    for effect_id, name, trigger in [
        ("living_tsunami", "Arien", "before_attack"),
        ("wild_hunt", "Silverarrow", "before_action"),
        ("jinx", "Cordelia", "after_card_discard"),
    ]:
        from live_passive_traces import enable

        from goa2.engine.steps import OfferPassiveStep

        for mode in ("first", "last", "invalid"):
            state, hero = build(name)
            card = next(
                card
                for card in [*hero.deck, hero.ultimate_card]
                if card and card.effect_id == effect_id
            )
            from goa2.domain.models import CardState

            hero.level = 8
            card.state = CardState.RESOLVED
            card.is_facedown = False
            if card is not hero.ultimate_card:
                hero.played_cards = [card]
            enable(state, hero, card, effect_id)
            step = OfferPassiveStep(
                step_id="fixture/live-hex/passive",
                card_id=card.id,
                trigger=trigger,
                pending_input={"selection": "YES"},
            )
            state.execution_stack = [step]
            operations = live_choice_operations(state, mode, stop_types=())
            operations.append({"kind": "advance_prefix", "steps": 0})
            yield f"live-moving-passive-{effect_id}-{mode}", state, operations

    yield from fast_travel_scenes()


def fast_travel_scenes():
    from combat_effect_traces import effect
    from live_passive_traces import build

    from goa2.domain.board import Zone
    from goa2.domain.models import ActionType, TargetType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.filters_hex import FastTravelDestinationFilter
    from goa2.engine.steps import SelectStep

    for variant in (
        "source-gold",
        "source-silver",
        "source-facedown",
        "current-gold",
        "card-gold",
        "card-silver",
        "card-null",
        "source-unknown",
    ):
        state, hero = build("Arien")
        zone = Zone(id="safe", hexes=list(state.board.tiles), neighbors=[])
        state.board.zones = {"safe": zone}
        for tile in state.board.tiles.values():
            tile.zone_id = "safe"
        gold = next(c for c in hero.hand if c.id == "noble_blade")
        silver = next(c for c in hero.hand if c.id == "spell_break")
        restriction = effect(
            state,
            "travel-restriction",
            EffectType.REPEAT_PREVENTION,
            source=str(hero.id),
            restrictions=[ActionType.MOVEMENT],
            except_card_colors=[gold.color],
            blocks_self=True,
        )
        state.active_effects = [restriction]
        source = gold.id
        if variant in {"source-silver", "card-gold"}:
            source = silver.id
        if variant == "source-facedown":
            gold.is_facedown = True
        if variant == "source-unknown":
            source = "unknown"
        if variant == "current-gold":
            source = None
            state.execution_context["current_card_id"] = gold.id
        if variant.startswith("card-"):
            state.execution_context["card"] = {
                "card-gold": gold,
                "card-silver": silver,
                "card-null": None,
            }[variant]
        step = SelectStep(
            step_id="fixture/live-travel/exception",
            target_type=TargetType.HEX,
            prompt="Travel",
            output_key="chosen",
            is_mandatory=False,
            filters=[FastTravelDestinationFilter(unit_id=str(hero.id), source_card_id=source)],
        )
        state.execution_stack = [step]
        state.execution_context["selector"] = step
        yield "hex-travel-" + variant, state, [
            {"kind": "advance_prefix", "steps": 1},
            {"kind": "advance_prefix", "steps": 0},
        ]
