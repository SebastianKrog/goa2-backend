"""Frozen EffectManager lifecycle and activation reference (no behavior execution)."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from model_traces import setup
from oracle import ROOT, Worker, digest, verify_sources
from state_codec import export_state


def cases():
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape
    from goa2.engine.effect_manager import EffectManager as M
    from goa2.engine.stats import _is_effect_active
    from goa2.engine.steps import SetContextFlagStep

    result = []

    def add(name, build, operations):
        state = build()
        envelope = export_state(state)
        records = []
        for op in operations:
            kind = op["kind"]
            value = None
            if kind == "create":
                fields = dict(op["effect"])
                for key in ("id", "created_at_turn", "created_at_round"):
                    fields.pop(key)
                fields["scope"] = EffectScope.model_validate(fields["scope"])
                fields["effect_type"] = EffectType(fields["effect_type"])
                fields["duration"] = DurationType(fields["duration"])
                before = state.next_entity_id
                effect = M.create_effect(state, **fields)
                index = next(
                    i for i, candidate in enumerate(state.active_effects) if candidate is effect
                )
                assert state.next_entity_id in (before, before + 1)
                if state.next_entity_id == before:
                    assert len(state.active_effects) > index
                else:
                    assert effect.id == f"eff_e_{before}"
                    assert (
                        effect.created_at_turn == state.turn
                        and effect.created_at_round == state.round
                    )
                value = {"index": index, "effect": effect}
            elif kind in ("activate_card", "deactivate_card"):
                getattr(M, kind.replace("_card", "_effects_by_card"))(state, op["card"])
            elif kind in ("activate_id", "deactivate_id"):
                getattr(M, kind.replace("_id", "_effect_by_id"))(state, op["id"])
            elif kind == "expire_id":
                M.expire_effect_by_id(state, op["id"])
            elif kind == "expire_card":
                M.expire_by_card(state, op["card"])
            elif kind == "expire_source":
                M.expire_by_source(state, op["source"])
            elif kind == "remove":
                M.remove_effects(state, op["ids"])
            elif kind == "cleanup":
                M.cleanup_stale_effects(state)
            elif kind == "collect_turn":
                finishing, ids = M.collect_turn_expiry(state)
                value = {"finishing": finishing, "ids": ids}
            elif kind == "expire_turn":
                value = M.expire_active_turn_effects(state)
            elif kind == "expire_duration":
                value = M.expire_effects(state, DurationType(op["duration"]))
            elif kind == "active":
                value = [_is_effect_active(e, state) for e in state.active_effects]
            elif kind == "clock":
                state.round, state.turn = op["round"], op["turn"]
            elif kind != "restore":
                raise AssertionError(kind)

            def wire(v):
                if hasattr(v, "model_dump"):
                    return v.model_dump(mode="json")
                if isinstance(v, (list, tuple)):
                    return [wire(x) for x in v]
                if isinstance(v, dict):
                    return {k: wire(x) for k, x in v.items()}
                return v

            records.append(
                {"semantic_state": state.model_dump(mode="json", context={}), "result": wire(value)}
            )
        result.append(
            {"name": name, "envelope": envelope, "operations": operations, "expected": records}
        )

    def build(duration=DurationType.THIS_TURN, card=True, active=False):
        state = setup(["Razzle", "Arien"], ["Brogan", "Hanu"])
        state.round, state.turn = 2, 3
        hero = state.get_hero("hero_arien")
        hero.hand[0].is_active_base = True
        printed = str(hero.hand[0].id)
        state.active_effects = [
            ActiveEffect(
                id="effect_a",
                source_id="hero_brogan",
                source_card_id=printed if card else None,
                effect_type=EffectType.DELAYED_TRIGGER,
                scope=EffectScope(shape=Shape.GLOBAL),
                duration=duration,
                created_at_turn=3,
                created_at_round=2,
                is_active=active,
                finishing_steps=[
                    SetContextFlagStep(
                        key="finished", value=True, step_id="fixture/effect-finishing"
                    )
                ],
            )
        ]
        return state

    for duration in DurationType:
        for card in (False, True):
            for active in (False, True):
                add(
                    f"activation-{duration}-{card}-{active}",
                    lambda d=duration, c=card, a=active: build(d, c, a),
                    [
                        {"kind": "active"},
                        {"kind": "collect_turn"},
                        {"kind": "restore"},
                        {"kind": "clock", "round": 2, "turn": 4},
                        {"kind": "active"},
                        {"kind": "collect_turn"},
                        {"kind": "clock", "round": 3, "turn": 1},
                        {"kind": "active"},
                        {"kind": "expire_turn"},
                        {"kind": "expire_duration", "duration": "THIS_ROUND"},
                        {"kind": "restore"},
                    ],
                )

    def all_types():
        state = build()
        base = state.active_effects[0]
        state.active_effects = []
        for i, kind in enumerate(EffectType):
            e = base.model_copy(deep=True)
            e.id = f"effect_{i}"
            e.effect_type = kind
            e.duration = list(DurationType)[i % 4]
            e.token_id = "" if i % 7 == 0 else "token_held" if i % 5 == 0 else None
            e.is_active = i % 2 == 0
            e.max_value = i + 3
            e.stat_value = -i
            e.granted_passive_effect_id = "big_sodding_gun" if i % 3 == 0 else None
            e.passive_uses_this_turn = i % 4
            state.active_effects.append(e)
        return state

    add(
        "all-types-ordered-lifecycle",
        all_types,
        [
            {"kind": "active"},
            {"kind": "restore"},
            {"kind": "collect_turn"},
            {"kind": "expire_turn"},
            {"kind": "expire_duration", "duration": "THIS_ROUND"},
            {"kind": "expire_duration", "duration": "PASSIVE"},
            {"kind": "expire_source", "source": "hero_arien"},
            {"kind": "restore"},
        ],
    )
    for action in (
        "activate_card",
        "deactivate_card",
        "activate_id",
        "deactivate_id",
        "expire_id",
        "expire_card",
        "expire_source",
        "cleanup",
        "remove",
    ):
        state = build()
        printed = str(state.get_hero("hero_arien").hand[0].id)
        op = {
            "kind": action,
            "card": printed,
            "id": "effect_a",
            "source": "hero_arien",
            "ids": ["effect_a", "missing"],
        }
        add(f"operation-{action}", build, [op, {"kind": "active"}, {"kind": "restore"}])
    for mode in (
        "performer",
        "owner",
        "missing-card",
        "extra-card",
        "empty-card",
        "token",
        "empty-token",
        "duplicate-id-source",
        "duplicate-id-duration",
        "duplicate-id-toggle",
        "duplicate-id-turn",
        "shared-card",
        "enraged",
        "global-copy",
        "after-cards",
    ):

        def fixture(mode=mode):
            state = build(active=True)
            hero = state.get_hero("hero_arien")
            e = state.active_effects[0]
            if mode == "missing-card":
                e.source_card_id = "absent"
            if mode == "empty-card":
                e.source_card_id = ""
            if mode == "extra-card":
                card = hero.hand.pop(0)
                hero.extra_turn_card = card
            if mode in ("token", "empty-token"):
                e.token_id = "token_x" if mode == "token" else ""
            if mode.startswith("duplicate-id"):
                other = e.model_copy(deep=True)
                other.source_id = "hero_hanu"
                other.source_card_id = None
                other.duration = DurationType.PASSIVE
                other.is_active = False
                state.active_effects.append(other)
            if mode == "shared-card":
                other = e.model_copy(deep=True)
                other.id = "effect_b"
                other.duration = DurationType.THIS_ROUND
                other.is_active = False
                state.active_effects.append(other)
            if mode == "enraged":
                hero.hand[0].enraged_active_override = True
            if mode == "global-copy":
                state.get_hero("hero_razzle").deck.append(hero.hand[0].model_copy(deep=True))
            if mode == "after-cards":
                e.effect_type = EffectType.AFTER_CARDS_PLAYED_TRIGGER
            return state

        op = {
            "kind": "expire_source",
            "source": (
                "hero_brogan"
                if mode in ("performer", "missing-card", "empty-card")
                else "hero_arien"
            ),
        }
        if mode == "duplicate-id-duration":
            op = {"kind": "expire_duration", "duration": "THIS_TURN"}
        if mode == "duplicate-id-toggle":
            op = {"kind": "deactivate_id", "id": "effect_a"}
        if mode == "duplicate-id-turn":
            op = {"kind": "expire_turn"}
        if mode in ("shared-card", "enraged", "global-copy", "after-cards"):
            op = {"kind": "expire_turn"}
        add(
            mode,
            fixture,
            [{"kind": "restore"}, op, {"kind": "active"}, {"kind": "cleanup"}, {"kind": "restore"}],
        )
    from effect_creation_traces import add_creation_cases

    add_creation_cases(add)
    return result


def corpus():
    payload = {
        "schema_version": 1,
        "source_sha256": verify_sources()["source_sha256"],
        "cases": cases(),
    }
    return {**payload, "corpus_sha256": digest(payload)}


def verify(path, command):
    data = json.loads(path.read_text())
    identity = data.pop("corpus_sha256")
    if digest(data) != identity or data["source_sha256"] != verify_sources()["source_sha256"]:
        raise ValueError("effect corpus identity mismatch")
    worker = Worker(command)
    try:
        for case in data["cases"]:
            actual = worker.request(
                "effect_trace", envelope=case["envelope"], operations=case["operations"]
            )["records"]
            if actual != case["expected"]:
                target = ROOT / "rust/artifacts/effect-lifecycle-mismatch.json"
                target.write_text(json.dumps({"case": case, "actual": actual}, indent=2))
                raise AssertionError(f"{case['name']}: saved {target}")
    finally:
        worker.close()
    return {
        "cases": len(data["cases"]),
        "operations": sum(len(c["operations"]) for c in data["cases"]),
        "corpus_sha256": identity,
    }


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--corpus", type=Path, default=ROOT / "rust/tests/fixtures/effects.json")
    parser.add_argument("mode", choices=("export", "verify"))
    parser.add_argument("worker", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == "export":
        data = corpus()
        temporary = args.corpus.with_suffix(args.corpus.suffix + ".tmp")
        temporary.unlink(missing_ok=True)
        temporary.write_text(json.dumps(data, separators=(",", ":")) + "\n")
        temporary.replace(args.corpus)
        print({"cases": len(data["cases"]), "corpus_sha256": data["corpus_sha256"]})
    else:
        command = args.worker[1:] if args.worker[:1] == ["--"] else args.worker
        if not command:
            parser.error("worker required")
        print(verify(args.corpus, command))


if __name__ == "__main__":
    main()
