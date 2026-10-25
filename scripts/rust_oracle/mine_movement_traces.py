"""Real-handler movement/mine scheduling, retries and continuation fixtures."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from mine_traces import drive, states
from oracle import ROOT, digest, verify_sources


def setup(blast=False, corridor=True):
    from goa2.domain.board import Board

    _, state, h, pieces, _ = next(
        c for c in states() if c[0] == ("one-blast" if blast else "one-dud")
    )
    if corridor:
        state.board = Board(
            tiles={p: tile for p, tile in state.board.tiles.items() if p.r == 0 and -2 <= p.q <= 0}
        )
    return state, h, pieces


def cases(advance=None, answer=None):
    from goa2.domain.board import Board
    from goa2.domain.models import Token, TokenType
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.steps import MoveSequenceStep, MoveUnitStep, SetContextFlagStep

    result = []
    for blast in [False, True]:
        for corridor in [False, True]:
            for sequence in [False, True]:
                for mode in ["first", "last", "invalid", "numeric"]:
                    state, h, _ = setup(blast, corridor)
                    state.execution_context = {
                        "target_hex": h(0, 0).model_dump(),
                        "success": "stale",
                    }
                    cls = MoveSequenceStep if sequence else MoveUnitStep
                    kwargs = {"unit_id": "hero_arien", "range_val": 3}
                    if not sequence:
                        kwargs["success_output_key"] = "success"
                    name = f"mine-move-{'blast' if blast else 'dud'}-{'corridor' if corridor else 'choice'}-{'sequence' if sequence else 'direct'}-{mode}"
                    case, final = drive(
                        name,
                        state,
                        [cls(**kwargs), SetContextFlagStep(key="after", value=True)],
                        mode,
                        ("triggered_mine_ids",),
                    )
                    assert final.entity_locations["hero_arien"] == h(0, 0)
                    assert final.execution_context["after"] is True
                    if corridor:
                        assert "mine_a" not in final.entity_locations
                        assert len(final.get_hero("hero_arien").hand) == (4 if blast else 5)
                    result.append(case)

    for variant in [
        "friendly",
        "unowned",
        "unknown-owner",
        "supply-only",
        "entity-priority",
        "pool-mismatch",
        "minion",
        "piece",
        "actor-other",
        "actor-none",
        "actor-empty",
        "offboard",
        "zero",
        "negative",
        "landing-mine",
        "no-destination",
        "literal",
        "literal-requeue",
        "custom-key",
        "straight",
        "straight-no-owner",
        "straight-unknown-owner",
        "prechosen",
        "prechosen-empty",
        "prechosen-unknown",
        "same-hex",
        "skip",
        "crossed",
        "dud-effects",
        "controlled",
        "second-move",
        "two-straight",
        "two-prechosen",
        "sequence-select",
    ]:
        state, h, pieces = setup(
            False, variant not in ["literal-requeue", "controlled", "sequence-select"]
        )
        kwargs = {"unit_id": "hero_arien", "range_val": 3, "success_output_key": "success"}
        state.execution_context = {"target_hex": h(0, 0).model_dump(), "success": "old"}
        mine = state.misc_entities["mine_a"]
        steps = []
        if variant == "friendly":
            mine.owner_id = "hero_hanu"
        if variant in ["unowned", "straight-no-owner"]:
            mine.owner_id = None
        if variant in ["unknown-owner", "straight-unknown-owner"]:
            mine.owner_id = "unknown"
        if variant == "supply-only":
            state.misc_entities.pop("mine_a")
        if variant == "entity-priority":
            state.misc_entities["mine_a"] = mine.model_copy(update={"is_passable": False})
        if variant == "pool-mismatch":
            state.token_pool[mine.token_type] = [mine.model_copy(update={"is_passable": False})]
        if variant == "minion":
            unit = state.teams[next(t for t in state.teams if t.value == "BLUE")].minions[0]
            state.remove_entity("hero_arien")
            state.place_entity(str(unit.id), h(-2, 0))
            kwargs["unit_id"] = str(unit.id)
        if variant == "piece":
            state.remove_entity("hero_arien")
            state.place_entity(pieces[0], h(-2, 0))
            kwargs["unit_id"] = pieces[0]
            mine.owner_id = "hero_hanu"
        if variant.startswith("actor-"):
            state.current_actor_id = {
                "actor-other": "hero_min",
                "actor-none": None,
                "actor-empty": "",
            }[variant]
        if variant == "offboard":
            state.remove_entity("hero_arien")
        if variant in ["zero", "negative"]:
            kwargs["range_val"] = 0 if variant == "zero" else -1
        if variant == "landing-mine":
            state.execution_context["target_hex"] = h(-1, 0).model_dump()
        if variant in ["no-destination", "literal", "literal-requeue", "sequence-select"]:
            state.execution_context.pop("target_hex")
        if variant.startswith("literal"):
            kwargs["target_hex_arg"] = h(0, 0)
        if variant == "custom-key":
            kwargs["destination_key"] = "chosen"
            state.execution_context["chosen"] = state.execution_context.pop("target_hex")
        if variant.startswith("straight") or variant in ["crossed", "two-straight"]:
            kwargs["force_straight_line"] = True
        if variant.startswith("prechosen") or variant == "two-prechosen":
            kwargs["mine_path_prechosen"] = True
            state.execution_context["triggered_mine_ids"] = {
                "prechosen": ["mine_a"],
                "prechosen-empty": [],
                "prechosen-unknown": ["unknown"],
                "two-prechosen": ["mine_b", "mine_a"],
            }[variant]
        if variant == "same-hex":
            state.execution_context["target_hex"] = h(-2, 0).model_dump()
        if variant == "skip":
            kwargs["skip_if_key"] = "skip"
            state.execution_context["skip"] = True
        if variant == "crossed":
            kwargs["crossed_obstacles_output_key"] = "crossed"
        if variant in ["dud-effects", "controlled"]:
            state.active_effects.append(
                ActiveEffect(
                    id="move-scope",
                    source_id="hero_min",
                    effect_type=(
                        EffectType.CONTROL_NEXT_ACTION
                        if variant == "controlled"
                        else EffectType.STATIC_BARRIER
                    ),
                    scope=EffectScope(
                        shape=Shape.GLOBAL if variant == "controlled" else Shape.RADIUS,
                        origin_id="hero_arien" if variant == "controlled" else "hero_min",
                        range=0,
                    ),
                    duration=DurationType.PASSIVE,
                    created_at_turn=state.turn,
                    created_at_round=state.round,
                    barrier_radius=0,
                )
            )
            if variant == "controlled":
                state.get_hero("hero_arien").current_turn_card = state.get_hero("hero_arien").hand[
                    1
                ]
        if variant in ["second-move", "two-straight", "two-prechosen"]:
            # A five-space corridor; separate moves must not reuse a previous choice.
            from aura_traces import _board

            _board(state)
            state.board = Board(
                tiles={
                    p: tile for p, tile in state.board.tiles.items() if p.r == 0 and -2 <= p.q <= 2
                }
            )
            state.place_entity("hero_arien", h(-2, 0))
            state.place_entity("mine_a", h(-1, 0))
            token = Token(
                id="mine_b",
                name="Second",
                token_type=TokenType.MINE_BLAST,
                owner_id="hero_min",
                is_passable=True,
            )
            state.misc_entities["mine_b"] = token
            state.token_pool.setdefault(token.token_type, []).append(token)
            state.place_entity("mine_b", h(1, 0))
            if variant == "second-move":
                steps = [
                    MoveUnitStep(**kwargs),
                    SetContextFlagStep(key="target_hex", value=h(2, 0).model_dump()),
                ]
                kwargs = {"unit_id": "hero_arien", "range_val": 2}
            else:
                state.execution_context["target_hex"] = h(2, 0).model_dump()
                kwargs["range_val"] = 4
        step = (
            MoveSequenceStep(unit_id="hero_arien", range_val=3)
            if variant == "sequence-select"
            else MoveUnitStep(**kwargs)
        )
        steps += [step, SetContextFlagStep(key="after", value=True)]
        case, final = drive(
            "mine-move-" + variant,
            state,
            steps,
            "first",
            () if variant in ["two-straight", "two-prechosen"] else ("triggered_mine_ids",),
        )
        assert final.execution_context["after"] is True
        if variant == "second-move":
            assert final.entity_locations["hero_arien"] == h(2, 0)
            assert "mine_a" not in final.entity_locations and "mine_b" not in final.entity_locations
            assert len(final.get_hero("hero_arien").hand) == 4
        if variant in ["two-straight", "two-prechosen"]:
            assert final.entity_locations["hero_arien"] == h(2, 0)
            assert "mine_a" not in final.entity_locations and "mine_b" not in final.entity_locations
        if variant == "literal-requeue":
            # Source defect: generated choice reads the absent context destination,
            # although the requeued MoveUnit retains its literal destination.
            assert final.entity_locations["hero_arien"] == h(0, 0)
            assert "mine_a" in final.entity_locations
        result.append(case)
    return result


def export(path):
    from stack_traces import write_corpus

    m = verify_sources()
    data = {
        "schema_version": 1,
        "oracle_revision": m["oracle_revision"],
        "source_sha256": m["source_sha256"],
        "cases": cases(),
    }
    data["corpus_sha256"] = digest(data)
    return write_corpus(path, data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("mode", choices=["export", "verify"])
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/artifacts/mine-movement.json")
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        d = export(args.corpus)
        print(json.dumps({"cases": len(d["cases"]), "corpus_sha256": d["corpus_sha256"]}))
    else:
        from stack_traces import verify

        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        print(json.dumps(verify(args.corpus, command), indent=2))
