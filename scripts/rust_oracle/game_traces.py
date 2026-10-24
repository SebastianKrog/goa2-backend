"""Planning commands through complete rounds against real Python phase calls."""

from __future__ import annotations

from state_codec import export_state_with_instances


def cases(advance, answer):
    from model_traces import setup
    from oracle import ROOT
    from stack_traces import reference_trace

    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import GamePhase
    from goa2.engine import phases
    from goa2.engine.handler import process_stack, submit_input
    from goa2.engine.setup import GameSetup

    result = []

    def add(name, state, rounds=1, mode=None):
        initial = state.model_copy(deep=True)
        envelope, _ = export_state_with_instances(initial)
        _, working = export_state_with_instances(state)
        card_ids = {id(card): i for i, card in enumerate(working)}
        ops = []

        def command(kind, hero, card=None, invalid=False):
            op = {"kind": kind, "hero": str(hero)}
            if card is not None:
                op["card"] = card_ids[id(card)]
            ops.append(op)
            try:
                if kind == "commit":
                    phases.commit_card(state, hero, card)
                elif kind == "uncommit":
                    phases.uncommit_card(state, hero)
                elif kind == "pass":
                    phases.pass_turn(state, hero)
                else:
                    phases.finish_planning(state, hero)
            except ValueError:
                assert invalid, (name, kind, hero)
            else:
                assert not invalid, (name, kind, hero)

        def drain():
            for _ in range(150):
                ops.append(advance)
                req = process_stack(state).input_request
                if req is None:
                    return
                ids = [opt.id for opt in req.options]
                if req.request_type.value == "UPGRADE_PHASE":
                    hero = next(
                        h
                        for h, p in req.context["players"].items()
                        if p["remaining"] > 0 and p["options"]
                    )
                    value = {
                        "hero_id": hero,
                        "card_id": req.context["players"][hero]["options"][0]["pair"][0],
                    }
                elif "HOLD" in ids:
                    value = "HOLD"
                elif "CONFIRM" in ids:
                    value = "CONFIRM"
                elif req.can_skip:
                    value = "SKIP"
                else:
                    value = selection_value(req.options[0])
                ops.append(answer(value))
                submit_input(state, InputResponse(request_id=req.id, selection=value))
            raise AssertionError(f"round stalled: {name}")

        start = state.round
        for cycle in range(rounds):
            for turn in range(1, 5):
                assert state.phase == GamePhase.PLANNING and state.turn == turn, (name, cycle, turn)
                heroes = [h for team in state.teams.values() for h in team.heroes]
                if mode == "invalid" and cycle == 0 and turn == 1:
                    command("finish", heroes[0].id, invalid=True)
                    command("uncommit", heroes[0].id, invalid=True)
                    command("pass", heroes[0].id, invalid=True)
                for i, hero in enumerate(heroes):
                    if not hero.hand:
                        command("pass", hero.id)
                        continue
                    card = hero.hand[0]
                    command("commit", hero.id, card)
                    if mode == "invalid" and i == 0 and cycle == 0 and turn == 1:
                        command("commit", hero.id, card, invalid=True)
                    if mode == "uncommit" and i == 0 and cycle == 0 and turn == 1:
                        command("uncommit", hero.id)
                        command("commit", hero.id, hero.hand[0])
                    if phases.planning_open_for_second_card(state, hero.id):
                        if mode == "double":
                            command("commit", hero.id, hero.hand[0])
                        else:
                            command("finish", hero.id)
                drain()
            assert (
                state.phase == GamePhase.PLANNING
                and state.round == start + cycle + 1
                and state.turn == 1
            ), name
            assert (
                not state.pending_upgrades
                and not state.pending_inputs
                and not state.unresolved_hero_ids
            ), name
            assert dict(state.last_turn_positions) == dict(state.entity_locations), name
        expected = reference_trace(initial, ops)
        result.append(
            {"name": f"game-{name}", "envelope": envelope, "operations": ops, "expected": expected}
        )

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        state.get_hero("hero_arien").gold = 1
        add(f"map-{path.stem}", state)
    for mode in ["ordinary", "invalid", "uncommit", "double", "finish"]:
        state = setup(["Emmitt", "Arien"] if mode in ["double", "finish"] else None)
        if mode in ["double", "finish"]:
            state.get_hero("hero_emmitt").level = 8
        state.teams[next(iter(state.teams))].heroes[0].gold = 1
        add(mode, state, rounds=2 if mode == "ordinary" else 1, mode=mode)
    # Legal pass commands with no hand cards exercise synchronous auto-pass turns.
    state = setup()
    for team in state.teams.values():
        for hero in team.heroes:
            hero.hand = []
    initial = state.model_copy(deep=True)
    envelope, _ = export_state_with_instances(initial)
    ops = []
    for team in state.teams.values():
        for hero in team.heroes:
            ops.append({"kind": "pass", "hero": str(hero.id)})
            phases.pass_turn(state, hero.id)
    ops.append(advance)
    out = process_stack(state)
    assert out.input_request is None and state.round == 2 and state.phase == GamePhase.PLANNING
    result.append(
        {
            "name": "game-empty-hands",
            "envelope": envelope,
            "operations": ops,
            "expected": reference_trace(initial, ops),
        }
    )
    return result
