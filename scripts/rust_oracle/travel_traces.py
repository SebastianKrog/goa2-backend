"""Real handler Fast Travel traces, including relocation and fresh validation."""

from __future__ import annotations

from oracle import ROOT
from state_codec import export_state


def cases(advance, answer):
    from stack_traces import reference_trace

    from goa2.domain.board import Board, Zone
    from goa2.domain.hex import Hex
    from goa2.domain.models import (
        GamePhase,
        Hero,
        Minion,
        MinionType,
        TargetType,
        Team,
        TeamColor,
        Token,
        TokenType,
    )
    from goa2.domain.models.unit import HeroPiece
    from goa2.domain.state import GameState
    from goa2.domain.tile import Tile
    from goa2.engine.filters_hex import FastTravelDestinationFilter
    from goa2.engine.handler import process_stack, push_steps
    from goa2.engine.setup import GameSetup
    from goa2.engine.steps import (
        ChooseActingPieceStep,
        FastTravelSequenceStep,
        FastTravelUnitStep,
        MoveUnitStep,
        SelectStep,
        SetContextFlagStep,
    )

    result = []

    def add(name, state, steps, operations):
        state.phase = GamePhase.RESOLUTION
        for i, step in enumerate(steps):
            step.step_id = f"fixture/travel/{name}/{i}"
        push_steps(state, steps)
        envelope = export_state(state)
        result.append(
            {
                "name": f"travel-{name}",
                "envelope": envelope,
                "operations": operations,
                "expected": reference_trace(state, operations),
            }
        )

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        for change in (False, True):
            state = GameSetup.create_game(
                str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
            )
            state.current_actor_id = "hero_arien"
            accepts = FastTravelDestinationFilter(
                unit_id="hero_arien", require_zone_change=change
            ).prepare(state, state.execution_context)
            options = [h for h in state.board.tiles if accepts(h)]
            operations = (
                [advance, answer(options[0].model_dump(mode="json")), advance]
                if options
                else [advance]
            )
            add(
                f"{path.stem}-change-{change}",
                state,
                [FastTravelSequenceStep(unit_id="hero_arien", require_zone_change=change)],
                operations,
            )

    def h(q, r, s):
        return Hex(q=q, r=r, s=s)

    destination = h(2, -1, -1)

    def build():
        coords = [h(q, r, -q - r) for q in range(-3, 4) for r in range(-3, 4) if abs(q + r) <= 3]

        def zid(hex):
            return "A" if hex.q <= 0 else "B" if hex.q <= 2 else "C"

        zones = {
            key: Zone(id=key, hexes={hex for hex in coords if zid(hex) == key}, neighbors=neighbors)
            for key, neighbors in [("A", ["B"]), ("B", ["A", "C"]), ("C", ["B"])]
        }
        tiles = {hex: Tile(hex=hex, zone_id=zid(hex), is_terrain=hex.q == 1) for hex in coords}
        red = Hero(id="hero_arien", name="Arien", team=TeamColor.RED, deck=[])
        blue = Hero(id="hero_hanu", name="Hanu", team=TeamColor.BLUE, deck=[])
        razzle = Hero(id="hero_razzle", name="Razzle", team=TeamColor.RED, deck=[], piece_supply=2)
        minion = Minion(id="friendly", name="Minion", team=TeamColor.RED, type=MinionType.MELEE)
        state = GameState(
            board=Board(zones=zones, tiles=tiles),
            teams={
                TeamColor.RED: Team(color=TeamColor.RED, heroes=[red, razzle], minions=[minion]),
                TeamColor.BLUE: Team(color=TeamColor.BLUE, heroes=[blue]),
            },
            phase=GamePhase.RESOLUTION,
            rules_version=9,
            current_actor_id=red.id,
        )
        for uid, loc in [(red.id, h(0, 0, 0)), (blue.id, h(3, -3, 0)), (minion.id, h(-2, 2, 0))]:
            state.place_entity(uid, loc)
        for i, hex in enumerate([h(-2, 1, 1), h(-1, 2, -1)], 1):
            piece = HeroPiece(
                id=f"hero_razzle_piece_{i}",
                name="Razzle piece",
                owner_hero_id=razzle.id,
                team=TeamColor.RED,
            )
            state.misc_entities[piece.id] = piece
            state.place_entity(piece.id, hex)
        mine = Token(
            id="mine",
            name="Mine",
            token_type=TokenType.MINE_BLAST,
            owner_id=blue.id,
            is_passable=True,
        )
        state.misc_entities[mine.id] = mine
        state.place_entity(mine.id, h(-1, 1, 0))
        return state

    for variant in (
        "sequence",
        "require-change",
        "preselected",
        "skip",
        "preselected-skip",
        "no-actor",
        "offboard",
        "no-team",
        "unsafe-start",
        "unsafe-destination",
        "token-mover",
        "minion-mover",
        "piece-bound",
        "choose-piece",
        "missing-tile-zone",
        "direct",
        "direct-same-zone",
        "direct-current",
        "direct-terrain",
        "direct-mine",
        "direct-optional",
        "direct-skip",
        "direct-bad-value",
        "direct-no-destination",
        "tile-zone-override",
        "offmap",
        "revalidate",
        "restored",
    ):
        state = build()
        step = FastTravelSequenceStep(source_card_id="source-card")
        operations = [advance, answer(destination.model_dump(mode="json")), advance]
        if variant == "require-change":
            step.require_zone_change = True
        elif variant == "preselected":
            state.execution_context["target_hex"] = destination.model_dump(mode="json")
            operations = [advance]
        elif variant == "skip":
            operations = [advance, answer("SKIP"), advance]
        elif variant == "preselected-skip":
            state.execution_context["target_hex"] = "SKIP"
            operations = [advance]
        elif variant == "no-actor":
            state.current_actor_id = None
            operations = [advance]
        elif variant == "offboard":
            state.remove_entity("hero_arien")
            operations = [advance]
        elif variant == "no-team":
            state.get_hero("hero_arien").team = None
            operations = [advance]
        elif variant == "unsafe-start":
            state.move_unit("hero_hanu", h(-1, 0, 1))
            operations = [advance]
        elif variant == "unsafe-destination":
            state.move_unit("hero_hanu", h(2, 0, -2))
            step.require_zone_change = True
            operations = [advance]
        elif variant == "token-mover":
            step.unit_id = "mine"
            operations = [advance]
        elif variant == "minion-mover":
            step.unit_id = "friendly"
        elif variant == "piece-bound":
            step.unit_id = "hero_razzle"
            state.acting_piece_id = "hero_razzle_piece_1"
        elif variant == "missing-tile-zone":
            state.board.tiles[h(0, 0, 0)].zone_id = None
            operations = [advance]
        elif variant.startswith("direct") or variant in ("tile-zone-override", "offmap"):
            step = FastTravelUnitStep(unit_id="hero_arien", target_hex_arg=destination)
            operations = [advance]
            if variant == "direct-same-zone":
                step.require_zone_change = True
                step.target_hex_arg = h(-1, -1, 2)
            elif variant == "direct-current":
                step.target_hex_arg = h(0, 0, 0)
            elif variant == "direct-terrain":
                step.target_hex_arg = h(1, 0, -1)
            elif variant in ("direct-mine", "direct-optional"):
                step.target_hex_arg = h(-1, 1, 0)
                step.is_mandatory = variant != "direct-optional"
            elif variant in ("direct-skip", "direct-bad-value", "direct-no-destination"):
                step.target_hex_arg = None
                if variant != "direct-no-destination":
                    state.execution_context["target_hex"] = (
                        "SKIP" if variant == "direct-skip" else "bad value"
                    )
            elif variant == "tile-zone-override":
                state.board.tiles[destination].zone_id = "C"
            elif variant == "offmap":
                step.target_hex_arg = h(100, -100, 0)
                state.board.zones["B"].hexes.add(step.target_hex_arg)
        steps = [step, SetContextFlagStep(key="continued")]
        if variant == "choose-piece":
            state.current_actor_id = "hero_razzle"
            steps = [
                ChooseActingPieceStep(hero_id="hero_razzle"),
                FastTravelSequenceStep(unit_id="hero_razzle"),
                SetContextFlagStep(key="continued"),
            ]
            operations = [
                advance,
                answer("hero_razzle_piece_2"),
                advance,
                answer(destination.model_dump(mode="json")),
                advance,
            ]
        elif variant == "sequence":
            operations = [
                advance,
                answer(h(-1, 1, 0).model_dump(mode="json")),
                advance,
                answer({"q": 1}),
                advance,
                answer(destination.model_dump(mode="json")),
                advance,
            ]
        elif variant == "revalidate":
            state.remove_entity("mine")
            steps = [
                SelectStep(
                    target_type=TargetType.HEX,
                    prompt="Choose travel",
                    filters=[FastTravelDestinationFilter(unit_id="hero_arien")],
                    output_key="target_hex",
                ),
                MoveUnitStep(unit_id="hero_hanu", target_hex_arg=h(2, -2, 0), range_val=1),
                FastTravelUnitStep(unit_id="hero_arien", is_mandatory=False),
                SetContextFlagStep(key="continued"),
            ]
        elif variant == "restored":
            push_steps(state, [FastTravelSequenceStep(unit_id="hero_arien")])
            request = process_stack(state).input_request
            assert request
            for i, child in enumerate(state.execution_stack):
                child.step_id = f"fixture/travel/restored/{i}"
                if child.pending_request_id:
                    child.pending_request_id = "fixture/travel-restored"
            envelope = export_state(state)
            operations = [
                answer(destination.model_dump(mode="json"), "fixture/travel-restored"),
                advance,
            ]
            result.append(
                {
                    "name": "travel-restored",
                    "envelope": envelope,
                    "operations": operations,
                    "expected": reference_trace(state, operations),
                }
            )
            continue
        add(variant, state, steps, operations)
    return result
