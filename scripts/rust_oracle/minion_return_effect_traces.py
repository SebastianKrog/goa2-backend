"""Frozen automatic minion return with effect-aware walking and placement.

Raw return steps isolate a phase dependency. Expectations come exclusively from
real frozen handler execution, including every generated instruction boundary.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from combat_effect_traces import effect
from displacement_effect_traces import states
from mine_traces import drive
from oracle import ROOT, digest, verify_sources


def cases():
    from goa2.domain.models.effect import EffectType
    from goa2.engine.steps import ReturnMinionToZoneStep, SetContextFlagStep

    result = []
    for variant, state, h, _, _minion in states():
        # Keep the registered roster, effects, cards and physical aliases. Move
        # the home zone off the displaced minion's start without fabricating IDs.
        state.board.zones["area"].hexes = {h(0, -2), h(1, -2), h(2, -2)}
        state.execution_context = {"kept": True}
        case, final = drive(
            f"return-effect-displacement-{variant}",
            state,
            [ReturnMinionToZoneStep(), SetContextFlagStep(key="returned", value=True)],
        )
        assert final.execution_context["returned"]
        result.append(case)

    from terrain_traces import states as terrain_states

    for variant, state, h, _, _minion in terrain_states():
        if variant.startswith(("base-", "restriction-")):
            continue
        state.board.zones["area"].hexes = {h(0, -2), h(1, -2), h(2, -2)}
        case, final = drive(
            f"return-effect-terrain-{variant}",
            state,
            [ReturnMinionToZoneStep(), SetContextFlagStep(key="returned", value=True)],
        )
        assert final.execution_context["returned"]
        result.append(case)

    from goa2.domain.models import Token, TokenType

    # A forced corridor proves that passable tokens are crossed rather than
    # merely present on a board where another equally short path avoids them.
    for passable in [False, True]:
        for token_kind in [TokenType.TOTEM, TokenType.MINE_DUD]:
            _, state, h, _, minion = next(states())
            start, middle, end = h(-2, 1), h(-1, 1), h(0, 1)
            for uid in list(state.entity_locations):
                state.remove_entity(uid)
            state.board.tiles = {p: state.board.tiles[p] for p in [start, middle, end]}
            state.board.zones["area"].hexes = {end}
            state.place_entity(minion.id, start)
            token = Token(
                id="return-corridor-token",
                name="Return corridor token",
                token_type=token_kind,
                owner_id="hero_arien",
                is_passable=passable,
                is_facedown=True,
            )
            state.misc_entities[token.id] = token
            state.token_pool.setdefault(token_kind, []).append(token)
            state.place_entity(token.id, middle)
            case, final = drive(
                f"return-effect-corridor-{token_kind.value}-{passable}",
                state,
                [ReturnMinionToZoneStep(), SetContextFlagStep(key="returned", value=True)],
            )
            assert final.get_position(minion.id) == end
            assert final.execution_context["returned"]
            # The expansion itself distinguishes walk from placement fallback.
            generated = case["expected"][0]["semantic_state"]["execution_stack"]
            assert any(
                step["type"] == ("move_unit" if passable else "place_unit") for step in generated
            )
            result.append(case)

    for cap in [-1, 0, 1, 2, 3, 9]:
        for actor in ["hero_arien", "hero_brogan", None, ""]:
            for actions_only in [False, True]:
                _, state, h, _, minion = next(states())
                state.board.zones["area"].hexes = {h(0, -2), h(1, -2)}
                state.current_actor_id = actor
                state.active_effects = [
                    effect(
                        state,
                        "return-cap",
                        EffectType.MOVEMENT_ZONE,
                        source="hero_arien",
                        max_value=cap,
                        limit_actions_only=actions_only,
                    )
                ]
                case, final = drive(
                    f"return-effect-cap-{cap}-{actor!s}-{actions_only}",
                    state,
                    [ReturnMinionToZoneStep(), SetContextFlagStep(key="returned", value=True)],
                )
                assert final.execution_context["returned"]
                result.append(case)
    return result


def export(path):
    from stack_traces import write_corpus

    manifest = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "cases": cases(),
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument(
        "--corpus", type=Path, default=ROOT / "rust/artifacts/minion-return-effects.json"
    )
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = export(args.corpus)
        print(json.dumps({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]}))
    else:
        from stack_traces import verify

        print(
            json.dumps(
                verify(args.corpus, args.worker[1:] if args.worker[:1] == ["--"] else args.worker),
                indent=2,
            )
        )
