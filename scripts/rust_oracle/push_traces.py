"""Real-handler push, vector and target-filter transitions on registered units."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.domain.board import Board
    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase, TargetType, Token, TokenType
    from goa2.domain.tile import Tile
    from goa2.engine.filters import (
        AdjacencyFilter,
        OrFilter,
        SpaceBehindEmptyFilter,
        TeamFilter,
        UnitTypeFilter,
    )
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import ComputeHexStep, PushUnitStep, SelectStep, SetContextFlagStep

    result = []

    def build():
        state = setup(["Arien", "Hanu"], ["Brogan", "Razzle"])
        piece = state.get_piece_ids("hero_razzle")[0]
        for unit in list(state.entity_locations):
            state.remove_entity(unit)
        coords = [Hex(q=q, r=-q, s=0) for q in range(-4, 7)]
        state.board = Board(map_id="push_fixture", tiles={h: Tile(hex=h) for h in coords})
        state.place_entity("hero_arien", coords[4])
        state.place_entity("hero_brogan", coords[5])
        state.place_entity("hero_hanu", coords[3])
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        state.execution_context = {
            "victim": "hero_brogan",
            "distance": 3,
            "source": "hero_hanu",
            "collision": "old",
            "push_victim_id": "old",
            "computed": "old",
        }
        return state, piece

    def add(name, state, steps, ops=None):
        steps = [*steps, SetContextFlagStep(key="push_done", value=True)]
        for i, step in enumerate(steps):
            step.step_id = f"fixture/push/{name}/{i}"
        push_steps(state, steps)
        ops = [advance] if ops is None else ops
        result.append(
            {
                "name": f"push-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

    variants = [
        "ordinary",
        "zero",
        "negative",
        "edge",
        "terrain",
        "unit-wall",
        "ignore-wall",
        "trim-wall",
        "literal-wins",
        "source-key",
        "source-literal-wins",
        "missing-source-key",
        "no-actor",
        "no-actor-explicit",
        "missing-victim",
        "offboard",
        "same-source",
        "off-axis",
        "unknown",
        "unknown-optional",
        "piece",
        "token",
        "token-zero",
        "integer-string",
        "float",
        "boolean",
        "null-distance",
        "inactive",
        "atomic",
    ]
    for variant in variants:
        state, piece = build()
        step = PushUnitStep(
            target_key="victim",
            distance=3,
            distance_key="distance",
            collision_output_key="collision",
        )
        if variant == "zero":
            state.execution_context["distance"] = 0
        if variant == "negative":
            state.execution_context["distance"] = -2
        if variant == "edge":
            state.execution_context["distance"] = 20
        if variant in ("terrain", "ignore-wall", "trim-wall"):
            state.board.tiles[Hex(q=2, r=-2, s=0)].is_terrain = True
            step.ignore_obstacles = variant != "terrain"
            if variant == "trim-wall":
                state.execution_context["distance"] = 1
        if variant == "unit-wall":
            state.move_unit("hero_hanu", Hex(q=2, r=-2, s=0))
        if variant == "literal-wins":
            step.target_id = "hero_brogan"
            state.execution_context["victim"] = True
        if variant == "source-key":
            step.source_key = "source"
        if variant == "source-literal-wins":
            step.source_hex = Hex(q=0, r=0, s=0)
            step.source_key = "source"
            state.execution_context["source"] = True
        if variant == "missing-source-key":
            step.source_key = "absent"
        if variant.startswith("no-actor"):
            state.current_actor_id = None
            if variant == "no-actor-explicit":
                step.source_hex = Hex(q=0, r=0, s=0)
        if variant == "missing-victim":
            state.execution_context.pop("victim")
        if variant == "offboard":
            state.remove_entity("hero_brogan")
        if variant == "same-source":
            step.source_hex = Hex(q=1, r=-1, s=0)
        if variant == "off-axis":
            hex = Hex(q=2, r=-1, s=-1)
            state.board.tiles[hex] = Tile(hex=hex)
            state.move_unit("hero_brogan", hex)
        if variant.startswith("unknown"):
            state.remove_entity("hero_brogan")
            state.place_entity("ghost", Hex(q=1, r=-1, s=0))
            state.execution_context["victim"] = "ghost"
            step.is_mandatory = variant != "unknown-optional"
        if variant == "piece":
            state.remove_entity("hero_brogan")
            state.place_entity(piece, Hex(q=1, r=-1, s=0))
            state.execution_context["victim"] = piece
        if variant.startswith("token"):
            state.remove_entity("hero_brogan")
            token = Token(
                id="push_token", name="Token", token_type=TokenType.TOTEM, owner_id="hero_arien"
            )
            state.misc_entities[token.id] = token
            state.place_entity(token.id, Hex(q=1, r=-1, s=0))
            state.execution_context["victim"] = token.id
            if variant == "token-zero":
                state.execution_context["distance"] = 0
        if variant == "integer-string":
            state.execution_context["distance"] = " +2 "
        if variant == "float":
            state.execution_context["distance"] = 2.9
        if variant == "boolean":
            state.execution_context["distance"] = False
        if variant == "null-distance":
            state.execution_context["distance"] = None
        if variant == "inactive":
            step.active_if_key = "enabled"
        ops = [{"kind": "advance_prefix", "steps": 1}, advance] if variant == "atomic" else None
        add(variant, state, [step], ops)
    for variant in [
        "ordinary",
        "origin-key",
        "missing-origin",
        "missing-target",
        "zero-scale",
        "negative-scale",
        "same",
        "far",
    ]:
        state, piece = build()
        step = ComputeHexStep(target_key="victim", output_key="computed")
        if variant == "origin-key":
            step.origin_key = "source"
        if variant == "missing-origin":
            step.origin_key = "absent"
        if variant == "missing-target":
            step.target_key = "absent"
        if variant == "zero-scale":
            step.scale = 0
        if variant == "negative-scale":
            step.scale = -2
        if variant == "same":
            state.execution_context["victim"] = "hero_arien"
        if variant == "far":
            state.move_unit("hero_brogan", Hex(q=2, r=-2, s=0))
        add(f"compute-{variant}", state, [step])
    for tags in [[], ["ENEMY"], ["FRIENDLY"], ["HERO"], ["MINION"], ["ENEMY", "HERO"]]:
        state, piece = build()
        add(
            "adjacency-" + ("-".join(tags) or "any"),
            state,
            [
                SelectStep(
                    target_type=TargetType.HEX,
                    prompt="Adjacent spaces",
                    output_key="selected",
                    filters=[AdjacencyFilter(target_tags=tags)],
                    is_mandatory=False,
                )
            ],
        )
    for variant in ["adjacent", "far", "blocked", "origin-key", "missing-origin"]:
        state, piece = build()
        if variant == "far":
            state.move_unit("hero_brogan", Hex(q=2, r=-2, s=0))
        if variant == "blocked":
            state.board.tiles[Hex(q=2, r=-2, s=0)].is_terrain = True
        filter = SpaceBehindEmptyFilter(origin_id="hero_arien")
        if variant == "origin-key":
            filter.origin_id = None
            filter.origin_key = "source"
        if variant == "missing-origin":
            filter.origin_id = "unknown"
        add(
            f"behind-{variant}",
            state,
            [
                SelectStep(
                    target_type=TargetType.UNIT,
                    prompt="Behind is empty",
                    output_key="selected",
                    filters=[filter],
                    is_mandatory=False,
                )
            ],
        )
    state, piece = build()
    add(
        "or-unit-filter",
        state,
        [
            SelectStep(
                target_type=TargetType.UNIT,
                prompt="OR targets",
                output_key="selected",
                filters=[
                    OrFilter(
                        filters=[
                            TeamFilter(relation="ENEMY"),
                            OrFilter(filters=[UnitTypeFilter(unit_type="MINION")]),
                        ]
                    )
                ],
                is_mandatory=False,
            )
        ],
    )
    return result
