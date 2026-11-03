"""Source line/trail token continuations, including pre-placement removals."""

from __future__ import annotations


def scenes():
    from live_passive_traces import build
    from object_execution_traces import live_choice_operations

    from goa2.domain.hex import Hex
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokensInLineStep, PlaceTokenTrailStep

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def retain(state, step, mode="first"):
        state.execution_stack = [step]
        state.execution_context.update(
            path_step=step, old_effects=state.active_effects, locations_alias=state.entity_locations
        )
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    variants = (
        "plain",
        "dict",
        "same",
        "not-straight",
        "missing-ref",
        "null-ref",
        "no-actor",
        "origin-key",
        "missing-origin-key",
        "origin-literal",
        "unknown-origin",
        "offboard-origin",
        "literal-wins",
        "zero",
        "negative",
        "one",
        "edge",
        "terrain",
        "occupied",
        "no-output",
        "empty-output",
        "skip",
        "active",
    )
    for variant in variants:
        state, hero = build("Mrak")
        state.execution_context.update(
            line_direction_ref=h(1), origin="hero_mrak", skip=False, targets="old"
        )
        step = PlaceTokensInLineStep(
            step_id="fixture/token-path/line", token_type=TokenType.ROCK, output_key="targets"
        )
        if variant == "dict":
            state.execution_context["line_direction_ref"] = h(1).model_dump()
        if variant == "same":
            state.execution_context["line_direction_ref"] = h(0)
        if variant == "not-straight":
            state.execution_context["line_direction_ref"] = h(1, 2)
        if variant == "missing-ref":
            state.execution_context.pop("line_direction_ref")
        if variant == "null-ref":
            state.execution_context["line_direction_ref"] = None
        if variant == "no-actor":
            state.current_actor_id = None
        if variant in {"origin-key", "missing-origin-key", "literal-wins"}:
            step.origin_key = "origin"
        if variant == "missing-origin-key":
            state.execution_context.pop("origin")
        if variant in {"origin-literal", "literal-wins"}:
            step.origin_id = "hero_mrak"
        if variant == "literal-wins":
            state.execution_context["origin"] = "unknown"
        if variant == "unknown-origin":
            step.origin_id = "unknown"
        if variant == "offboard-origin":
            step.origin_id = "hero_hanu"
        if variant in {"zero", "negative", "one", "edge"}:
            step.count = {"zero": 0, "negative": -1, "one": 1, "edge": 9}[variant]
        if variant == "terrain":
            state.board.get_tile(h(1)).is_terrain = True
        if variant == "occupied":
            state.place_entity("hero_hanu", h(1))
        if variant == "no-output":
            step.output_key = None
        if variant == "empty-output":
            step.output_key = ""
        if variant == "skip":
            step.skip_if_key = "skip"
        if variant == "active":
            step.active_if_key = "absent"
        ops = retain(state, step)
        yield "token-line-" + variant, state, ops

    for i, reference in enumerate(h(0).neighbors()):
        state, _ = build("Mrak")
        state.execution_context["line_direction_ref"] = reference
        step = PlaceTokensInLineStep(
            step_id="fixture/token-path/direction", token_type=TokenType.ROCK
        )
        yield f"token-line-direction-{i}", state, retain(state, step)

    for variant in (
        "plain",
        "dict",
        "same",
        "not-straight",
        "missing-origin",
        "missing-dest",
        "null-origin",
        "null-dest",
        "excluded",
        "duplicate-exclusions",
        "all-excluded",
        "empty-exclusion-key",
        "occupied",
        "terrain",
        "offmap",
        "skip",
        "active",
    ):
        state, hero = build("Ignatia")
        state.place_entity(hero.id, h(3))
        state.execution_context.update(start=h(0), end=h(3), excluded=[h(1)], skip=False)
        step = PlaceTokenTrailStep(
            step_id="fixture/token-path/trail",
            token_type=TokenType.MAGMA,
            origin_hex_key="start",
            dest_key="end",
        )
        if variant == "dict":
            state.execution_context.update(start=h(0).model_dump(), end=h(3).model_dump())
        if variant == "same":
            state.execution_context["end"] = h(0)
        if variant == "not-straight":
            state.execution_context["end"] = h(1, 2)
        if variant == "missing-origin":
            state.execution_context.pop("start")
        if variant == "missing-dest":
            state.execution_context.pop("end")
        if variant == "null-origin":
            state.execution_context["start"] = None
        if variant == "null-dest":
            state.execution_context["end"] = None
        if variant in {"excluded", "duplicate-exclusions", "all-excluded", "empty-exclusion-key"}:
            step.excluded_hexes_key = "excluded"
        if variant == "duplicate-exclusions":
            state.execution_context["excluded"] = [h(1), h(1).model_dump()]
        if variant == "all-excluded":
            state.execution_context["excluded"] = [h(0), h(1), h(2)]
        if variant == "empty-exclusion-key":
            step.excluded_hexes_key = ""
        if variant == "occupied":
            state.place_entity("hero_hanu", h(1))
        if variant == "terrain":
            state.board.get_tile(h(1)).is_terrain = True
        if variant == "offmap":
            state.execution_context.update(start=h(8), end=h(10))
        if variant == "skip":
            step.skip_if_key = "skip"
        if variant == "active":
            step.active_if_key = "absent"
        yield "token-trail-" + variant, state, retain(state, step)

    for trail in (False, True):
        for mode in ("first", "last", "invalid"):
            for owner in (False, True):
                state, hero = build("Ignatia" if trail else "Mrak")
                kind = TokenType.MAGMA if trail else TokenType.ROCK
                pool = state.token_pool[kind]
                free = [t.hex for t in state.board.tiles.values() if not t.is_occupied]
                # Put one old token on the path: removing it changes which hexes
                # a line must recompute before placing its first new token.
                points = [h(1), *[point for point in free if point != h(1)]]
                for token, point in zip(pool, points, strict=False):
                    state.place_entity(token.id, point)
                state.execution_context.update(
                    start=h(0), end=h(3), line_direction_ref=h(1), owner=str(hero.id)
                )
                if trail:
                    state.place_entity(hero.id, h(3))
                    step = PlaceTokenTrailStep(
                        step_id="fixture/token-path/retry",
                        token_type=kind,
                        origin_hex_key="start",
                        dest_key="end",
                    )
                else:
                    step = PlaceTokensInLineStep(
                        step_id="fixture/token-path/retry", token_type=kind, output_key="targets"
                    )
                step.owner_id_key = "owner" if owner else None
                yield f"token-path-removal-{trail}-{owner}-{mode}", state, retain(state, step, mode)


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
