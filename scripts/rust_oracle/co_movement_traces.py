"""Co-mover selection, unit/token sequencing, and token-walking source contracts."""

from __future__ import annotations


def build(variant="plain", *, token=False):
    from aura_traces import _board
    from model_traces import setup

    from goa2.domain.models import GamePhase, Token, TokenType

    state = setup(["Hanu", "Arien"], ["Min", "Brogan"])
    h = _board(state)
    state.phase = GamePhase.RESOLUTION
    state.current_actor_id = "hero_hanu"
    state.resolution_owner_id = "hero_hanu"
    anchor = "anchor-token" if token else "hero_hanu"
    if token:
        entity = Token(
            id=anchor,
            name="Anchor",
            token_type=TokenType.TREE,
            owner_id="hero_hanu",
            is_passable=False,
        )
        state.misc_entities[anchor] = entity
        state.token_pool.setdefault(entity.token_type, []).append(entity)
    state.place_entity(anchor, h(-2, 0))
    partner = "hero_arien"
    state.place_entity(partner, h(-1, 0))
    destination = h(1, 0)
    if variant == "parallel":
        state.place_entity(partner, h(-2, 1))
    elif variant == "behind":
        state.place_entity(partner, h(-3, 0))
    elif variant == "same-location":
        state.entity_locations[partner] = h(-2, 0)
    elif variant == "stationary":
        destination = h(-2, 0)
    elif variant == "nonstraight":
        destination = h(0, 1)
    elif variant == "offboard-destination":
        destination = h(4, 0)
    elif variant == "anchor-offboard":
        state.remove_entity(anchor)
    elif variant == "partner-offboard":
        state.remove_entity(partner)
    elif variant == "landing-occupied":
        state.place_entity("hero_brogan", h(1, 0))
    elif variant == "partner-landing-occupied":
        state.place_entity("hero_brogan", h(2, 0))
    elif variant == "landing-terrain":
        state.board.tiles[h(1, 0)].is_terrain = True
    elif variant == "partner-landing-terrain":
        state.board.tiles[h(2, 0)].is_terrain = True
    elif variant == "path-terrain":
        state.board.tiles[h(0, 0)].is_terrain = True
    elif variant == "vacating-terrain":
        state.board.tiles[h(-1, 0)].is_terrain = True
    elif variant == "path-gap":
        del state.board.tiles[h(0, 0)]
    elif variant == "path-occupied":
        state.place_entity("hero_brogan", h(0, 0))
    elif variant in ("mine-dud", "mine-blast", "passable-token"):
        kind = (
            TokenType.MINE_BLAST
            if variant == "mine-blast"
            else TokenType.MINE_DUD if variant == "mine-dud" else TokenType.TREE
        )
        entity = Token(
            id="path-token",
            name="Path token",
            token_type=kind,
            owner_id="hero_min",
            is_passable=True,
        )
        state.misc_entities[entity.id] = entity
        state.token_pool.setdefault(kind, []).append(entity)
        state.place_entity(entity.id, h(0, 0))
    elif variant == "no-actor":
        state.current_actor_id = None
    state.execution_context = {
        "anchor": anchor,
        "partner": partner,
        "anchor_dest": destination.model_dump(),
        "co_drag_partner_dest": h(4, -4),
    }
    state.execution_context.update(
        locations_alias=state.entity_locations, effects_alias=state.active_effects
    )
    state.execution_context["old-partner-dest"] = state.execution_context["co_drag_partner_dest"]
    state.execution_context["destination-alias"] = state.execution_context["anchor_dest"]
    if variant == "hex-destination":
        state.execution_context["anchor_dest"] = destination
    elif variant == "missing-anchor":
        state.execution_context.pop("anchor")
    elif variant == "missing-partner":
        state.execution_context.pop("partner")
    elif variant == "missing-destination":
        state.execution_context.pop("anchor_dest")
    elif variant == "false-anchor":
        state.execution_context["anchor"] = False
    elif variant == "unknown-anchor":
        state.execution_context["anchor"] = "unknown"
    return state, h


def _scene(name, state, steps, mode="first"):
    from object_execution_traces import live_choice_operations

    from goa2.engine.handler import push_steps
    from goa2.engine.steps import SetContextFlagStep

    steps = [*steps, SetContextFlagStep(key="after", value=True)]
    for index, step in enumerate(steps):
        step.step_id = f"fixture/co-movement/{name}/{index}"
    push_steps(state, steps)
    return "co-movement-" + name, state, live_choice_operations(state, mode, stop_types=())


def scenes():
    from goa2.domain.models import TargetType
    from goa2.engine.filters_geometry import CoMoverValidHexFilter
    from goa2.engine.steps import CoDirectionalDragStep, CountStep, SelectStep
    from goa2.engine.steps.markers import MoveTokenStep

    variants = [
        "plain",
        "parallel",
        "behind",
        "same-location",
        "stationary",
        "nonstraight",
        "offboard-destination",
        "anchor-offboard",
        "partner-offboard",
        "landing-occupied",
        "partner-landing-occupied",
        "landing-terrain",
        "partner-landing-terrain",
        "path-terrain",
        "vacating-terrain",
        "path-gap",
        "path-occupied",
        "mine-dud",
        "mine-blast",
        "passable-token",
        "no-actor",
        "hex-destination",
        "missing-anchor",
        "missing-partner",
        "missing-destination",
        "false-anchor",
        "unknown-anchor",
    ]
    for token in (False, True):
        for through in (False, True):
            for variant in variants:
                state, _ = build(variant, token=token)
                predicate = CoMoverValidHexFilter(
                    anchor_key="anchor", partner_key="partner", ignore_path_obstacles=through
                )
                # Unit candidates must fail even if their positions are valid hex destinations.
                assert not predicate.apply("hero_arien", state, state.execution_context)
                assert not predicate.apply(
                    {"q": 1, "r": 0, "s": -1}, state, state.execution_context
                )
                yield _scene(
                    f"filter-{token}-{through}-{variant}",
                    state,
                    [
                        CountStep(
                            target_type=TargetType.HEX, filters=[predicate], output_key="choices"
                        ),
                        SelectStep(
                            target_type=TargetType.HEX,
                            prompt="Co-mover destination",
                            filters=[predicate],
                            output_key="selected",
                            is_mandatory=False,
                        ),
                    ],
                )
                if variant not in ("nonstraight", "same-location"):
                    state, _ = build(variant, token=token)
                    yield _scene(
                        f"drag-{token}-{through}-{variant}",
                        state,
                        [
                            CoDirectionalDragStep(
                                anchor_key="anchor",
                                partner_key="partner",
                                anchor_is_token=token,
                                ignore_path_obstacles=through,
                            )
                        ],
                    )
    for variant in [
        *variants,
        "active-gate",
        "skip-gate",
        "negative-range",
        "zero-range",
        "short-range",
        "non-token",
    ]:
        for straight in (False, True):
            state, _ = build(variant, token=True)
            step = MoveTokenStep(
                token_key="anchor",
                destination_key="anchor_dest",
                range_val=4,
                force_straight_line=straight,
            )
            if variant == "active-gate":
                step.active_if_key = "missing"
            elif variant == "skip-gate":
                step.skip_if_key = "anchor"
            elif variant.endswith("range"):
                step.range_val = {"negative-range": -1, "zero-range": 0, "short-range": 1}[variant]
            elif variant == "non-token":
                state.execution_context["anchor"] = "hero_arien"
            yield _scene(f"token-walk-{straight}-{variant}", state, [step])

    for token in (False, True):
        for variant in ("shared-destination-key", "same-identity", "custom-keys"):
            state, _ = build(token=token)
            step = CoDirectionalDragStep(
                anchor_key="anchor", partner_key="partner", anchor_is_token=token
            )
            if variant == "shared-destination-key":
                step.partner_dest_key = "anchor_dest"
            elif variant == "same-identity":
                state.execution_context["partner"] = state.execution_context["anchor"]
            else:
                for before, after in (
                    ("anchor", "custom-anchor"),
                    ("partner", "custom-partner"),
                    ("anchor_dest", "custom-destination"),
                ):
                    state.execution_context[after] = state.execution_context.pop(before)
                step.anchor_key = "custom-anchor"
                step.partner_key = "custom-partner"
                step.anchor_dest_key = "custom-destination"
                step.partner_dest_key = "custom-partner-destination"
            yield _scene(f"drag-alias-{token}-{variant}", state, [step])
