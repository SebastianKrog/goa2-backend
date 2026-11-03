"""Frozen token batch feasibility, steering, opt-in, and scratch-context traces."""

from __future__ import annotations


def scenes():
    from live_passive_traces import build
    from object_execution_traces import live_choice_operations

    from goa2.domain.hex import Hex
    from goa2.domain.models import TargetType, TokenType
    from goa2.engine.filters import FarthestEmptyAdjacentFilter, ObstacleFilter, RangeFilter
    from goa2.engine.filters_hex import HexBatchCompletableFilter
    from goa2.engine.steps import PlaceTokenBatchStep, SelectStep, SetContextFlagStep

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def retain(state, step, mode="first"):
        tail = SetContextFlagStep(step_id="fixture/batch/tail", key="tail", value=True)
        state.execution_stack = [tail, step]
        state.execution_context.update(
            batch_alias=step,
            slots_alias=getattr(step, "slot_filters", None),
            old_effects=state.active_effects,
            locations_alias=state.entity_locations,
        )
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    for mode in ("first", "last", "invalid"):
        for variant in (
            "plain",
            "optin",
            "owner",
            "no-actor",
            "zero",
            "negative",
            "oversupply",
            "bad-slots",
            "impossible",
            "mandatory-impossible",
            "mandatory-oversupply",
            "skip",
            "active",
            "empty-prefix",
            "empty-flag",
            "empty-prompt",
        ):
            state, _hero = build("Mrak")
            step = PlaceTokenBatchStep(
                step_id="fixture/batch/main",
                token_type=TokenType.ROCK,
                count=2,
                key_prefix="b",
                placed_flag_key="placed",
            )
            if variant in {"impossible", "oversupply", "negative", "bad-slots"}:
                step.is_mandatory = False
            state.execution_context.update(only=h(1), skip=False)
            if variant == "optin":
                step.opt_in_prompt = "Place rocks?"
            if variant == "owner":
                step.owner_id = "hero_brogan"
            if variant == "no-actor":
                state.current_actor_id = None
            if variant == "zero":
                step.count = 0
                step.opt_in_prompt = "Place none?"
            if variant == "negative":
                step.count = -1
            if variant in {"oversupply", "mandatory-oversupply"}:
                step.count = len(state.token_pool[TokenType.ROCK]) + 1
            if variant == "bad-slots":
                step.slot_filters = [[RangeFilter(max_range=1)]]
            if variant in {"impossible", "mandatory-impossible"}:
                step.slot_filters = [[RangeFilter(max_range=0, origin_hex_key="only")]] * 2
                step.opt_in_prompt = "Must never appear"
            if variant.startswith("mandatory-"):
                step.is_mandatory = True
            if variant == "skip":
                step.skip_if_key = "skip"
            if variant == "active":
                step.active_if_key = "missing"
            if variant == "empty-prefix":
                step.key_prefix = ""
            if variant == "empty-flag":
                step.placed_flag_key = ""
            if variant == "empty-prompt":
                step.opt_in_prompt = ""
            yield f"token-batch-{variant}-{mode}", state, retain(state, step, mode)

    for mode in ("first", "last", "invalid"):
        for variant in (
            "reserve-last",
            "remove-needed",
            "all-removals",
            "farthest",
            "farthest-removals",
            "occupied-infeasible",
        ):
            state, _hero = build("Mrak")
            pool = state.token_pool[TokenType.ROCK]
            count = 3 if variant in {"all-removals", "farthest", "farthest-removals"} else 2
            state.execution_context.update(one=h(1), two=h(2), three=h(3), anchor="hero_hanu")
            step = PlaceTokenBatchStep(
                step_id="fixture/batch/steer",
                token_type=TokenType.ROCK,
                count=count,
                key_prefix="b",
                placed_flag_key="placed",
            )
            if variant == "reserve-last":
                step.slot_filters = [
                    [RangeFilter(max_range=2)],
                    [RangeFilter(max_range=0, origin_hex_key="one")],
                ]
            elif variant in {"farthest", "farthest-removals"}:
                state.place_entity("hero_hanu", h(2))
                step.slot_filters = [
                    [
                        FarthestEmptyAdjacentFilter(
                            anchor_key="anchor", occupied_hex_keys=[f"b_hex_{j}" for j in range(i)]
                        )
                    ]
                    for i in range(count)
                ]
            else:
                step.slot_filters = [
                    [RangeFilter(max_range=0, origin_hex_key=key)]
                    for key in ("one", "two", "three")[:count]
                ]
            if variant == "remove-needed":
                state.place_entity(pool[0].id, h(1))
                state.place_entity(pool[1].id, h(3))
            if variant in {"all-removals", "farthest-removals"}:
                points = (
                    [h(1), h(3), h(2, -1)] if variant == "farthest-removals" else [h(1), h(2), h(3)]
                )
                for token, point in zip(pool, points, strict=True):
                    state.place_entity(token.id, point)
            if variant == "occupied-infeasible":
                state.place_entity("hero_hanu", h(1))
            yield f"token-batch-{variant}-{mode}", state, retain(state, step, mode)

    for variant in (
        "bad-index",
        "duplicate-prior",
        "missing-prior",
        "removal-budget-zero",
        "removal-budget-one",
        "free-terrain",
        "farthest-hints",
    ):
        state, _ = build("Mrak")
        state.execution_context.update(one=h(1), two=h(2), b_hex_0=h(1), anchor="hero_hanu")
        filters = [
            [RangeFilter(max_range=0, origin_hex_key="one"), ObstacleFilter()],
            [RangeFilter(max_range=0, origin_hex_key="two"), ObstacleFilter()],
        ]
        rule = HexBatchCompletableFilter(
            slot_index=1, slot_keys=["b_hex_0", "b_hex_1"], slot_filters=filters
        )
        if variant == "bad-index":
            rule.slot_index = -1
        if variant == "duplicate-prior":
            state.execution_context["b_hex_0"] = h(2)
        if variant == "missing-prior":
            state.execution_context.pop("b_hex_0")
        if variant.startswith("removal-budget"):
            rule.removable_hexes = [h(1), h(2)]
            rule.removal_budget = 0 if variant.endswith("zero") else 1
        if variant == "free-terrain":
            state.board.get_tile(h(2)).is_terrain = True
            rule.free_hexes = [h(2)]
        if variant == "farthest-hints":
            state.place_entity("hero_hanu", h(2))
            rule.slot_index = 0
            rule.slot_filters = [
                [FarthestEmptyAdjacentFilter(anchor_key="anchor", occupied_hex_keys=[])],
                [FarthestEmptyAdjacentFilter(anchor_key="anchor", occupied_hex_keys=["b_hex_0"])],
            ]
        step = SelectStep(
            step_id="fixture/batch/filter",
            target_type=TargetType.HEX,
            prompt="Batch predicate",
            output_key="answer",
            filters=[rule],
        )
        yield "token-batch-filter-" + variant, state, retain(state, step)

    # Isolate each source-authored batch boundary. Other attack/swap/ultimate
    # consumers remain separate implementation tasks; get_steps retains binding.
    from goa2.domain.models import CardState
    from goa2.engine.effects import CardEffectRegistry

    for effect_id in ("stone_grip", "flashback", "deja_vu", "unstable_timeline"):
        for full in (False, True):
            for mode in ("first", "last", "invalid"):
                state, hero = build("Mrak" if effect_id == "stone_grip" else "Emmitt")
                card = next(
                    card for card in [*hero.hand, *hero.deck] if card.effect_id == effect_id
                )
                card.state = CardState.UNRESOLVED
                card.is_facedown = False
                hero.current_turn_card = card
                state.place_entity("hero_brogan", h(2))
                state.execution_context["grip_hero"] = "hero_brogan"
                step = next(
                    step
                    for step in CardEffectRegistry.get(effect_id).get_steps(state, hero, card)
                    if isinstance(step, PlaceTokenBatchStep)
                )
                step.step_id = "fixture/batch/card"
                if full:
                    points = [
                        point
                        for point in state.board.tiles
                        if not state.board.get_tile(point).is_occupied
                    ]
                    for token, point in zip(
                        state.token_pool[step.token_type], points, strict=False
                    ):
                        state.place_entity(token.id, point)
                yield f"token-batch-card-{effect_id}-{full}-{mode}", state, retain(
                    state, step, mode
                )


def export(path):
    from object_execution_traces import reference
    from oracle import verify_sources
    from shared_objects import export_state_with_shared_objects
    from stack_traces import write_corpus_stream

    manifest = verify_sources()

    def cases():
        for name, state, operations in scenes():
            envelope, _ = export_state_with_shared_objects(state)
            yield {
                "name": "object-execution-" + name,
                "envelope": envelope,
                "operations": operations,
                "expected": reference(state, operations),
            }

    return write_corpus_stream(
        path,
        {
            "schema_version": 1,
            "oracle_revision": manifest["oracle_revision"],
            "source_sha256": manifest["source_sha256"],
        },
        cases(),
    )
