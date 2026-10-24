"""Real-handler action handoff, turn-boundary and hero-respawn traces."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer):
    import copy

    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.domain.board import Board, Zone
    from goa2.domain.hex import Hex
    from goa2.domain.input import InputResponse, selection_value
    from goa2.domain.models import GamePhase, StatType, TargetType, TeamColor, Token, TokenType
    from goa2.domain.models.spawn import SpawnPoint, SpawnType
    from goa2.domain.tile import Tile
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import (
        AdvanceTurnStep,
        ConfirmResolutionStep,
        EndPhaseStep,
        FinalizeHeroTurnStep,
        FindNextActorStep,
        RespawnHeroStep,
        SelectStep,
        SetContextFlagStep,
    )

    result = []

    def h(q, r=0):
        return Hex(q=q, r=r, s=-q - r)

    def build(all_cards=False, multi=False):
        state = setup(["Razzle", "Hanu"] if multi else None)
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_razzle" if multi else "hero_arien"
        state.resolution_owner_id = state.current_actor_id
        state.execution_context = {"keep": True}
        state.turn_discard_log = {"hero_hanu": ["old-card"]}
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        for i, hero in enumerate(heroes):
            if all_cards or i == 0:
                card = next(
                    (c for c in hero.hand if c.color and c.color.value == "GOLD"), hero.hand[0]
                )
                hero.play_card(card)
                card.is_facedown = False
                card.initiative_override = 20 - i
                state.record_public_revealed_card(hero.id, card.id)
        state.unresolved_hero_ids = [hero.id for hero in heroes] if all_cards else []
        return state

    def add(name, state, steps, invalid=None, prefer=None, prefix=None, only_prefix=False):
        steps = [*steps, SetContextFlagStep(key="turn_done", value=True)]
        for i, step in enumerate(steps):
            step.step_id = f"fixture/turn/{name}/{i}"
        push_steps(state, steps)
        initial = export_state(state)
        if only_prefix:
            operations = [{"kind": "advance_prefix", "steps": prefix}]
        else:
            work = copy.deepcopy(state)
            operations = [{"kind": "advance_prefix", "steps": prefix}] if prefix is not None else []
            operations.append(advance)
            out = process_stack(work)
            if invalid is not None and out.input_request:
                submit_input(
                    work, InputResponse(request_id=out.input_request.id, selection=invalid)
                )
                operations.extend([answer(invalid), advance])
                out = process_stack(work)
            for _ in range(150):
                if not out.input_request:
                    break
                req = out.input_request
                ids = [o.id for o in req.options]
                if req.request_type.value == "CHOOSE_ACTION":
                    choice = (
                        prefer
                        if prefer in ids
                        else "HOLD" if "HOLD" in ids else "CONFIRM" if "CONFIRM" in ids else ids[0]
                    )
                elif req.can_skip:
                    choice = "SKIP"
                elif req.request_type.value == "CHOOSE_RESPAWN":
                    choice = "PASS" if prefer == "PASS" else "RESPAWN"
                else:
                    choice = selection_value(req.options[0])
                operations.extend([answer(choice), advance])
                submit_input(work, InputResponse(request_id=req.id, selection=choice))
                out = process_stack(work)
            else:
                raise AssertionError(f"turn did not finish: {name}")
            assert (
                work.execution_context.get("turn_done") or work.phase == GamePhase.GAME_OVER
            ), name
        expected = reference_trace(state, operations)
        result.append(
            {
                "name": f"turn-{name}",
                "envelope": initial,
                "operations": operations,
                "expected": expected,
            }
        )
        return state

    for variant in [
        "ordinary",
        "no-current",
        "unknown",
        "counters",
        "guess-own",
        "guess-other",
        "empty-guess",
        "facedown",
        "turn3",
        "empty-hand",
        "empty-other",
    ]:
        state = build()
        hero = state.get_hero("hero_arien")
        literal = str(hero.id)
        if variant == "no-current":
            hero.current_turn_card = None
        if variant == "unknown":
            literal = "unknown"
        if variant == "counters":
            hero.current_turn_card.passive_uses_this_turn = 3
            hero.ultimate_card.passive_uses_this_turn = 2
            hero.hand[0].passive_uses_this_turn = 4
        if variant.startswith("guess"):
            owner = literal if variant == "guess-own" else "hero_hanu"
            state.card_guess = {"guesser_id": owner, "private": "keep"}
            state.card_reveal = {"revealer_id": owner, "card": "keep"}
        if variant == "empty-guess":
            state.card_guess = {}
            state.card_reveal = {}
        if variant == "facedown":
            hero.current_turn_card.is_facedown = True
        if variant == "turn3":
            state.turn = 3
        if variant == "empty-hand":
            hero.hand = []
        if variant == "empty-other":
            state.get_hero("hero_hanu").hand = []
        final = add(f"finalize-{variant}", state, [FinalizeHeroTurnStep(hero_id=literal)], prefix=1)
        assert final.phase == GamePhase.PLANNING and final.turn == (4 if variant == "turn3" else 2)
    state = build(multi=True)
    piece = state.get_piece_ids("hero_razzle")[0]
    state.acting_piece_id = piece
    add("finalize-piece", state, [FinalizeHeroTurnStep(hero_id=piece)], prefix=1)
    for variant in ["ordinary", "frozen", "reanchor", "invalid", "pending"]:
        state = build()
        step = ConfirmResolutionStep(hero_id="hero_arien")
        if variant == "frozen":
            state.execution_context["rollback_frozen"] = True
        if variant == "reanchor":
            state.execution_context["rollback_reanchor_pending"] = True
        if variant == "pending":
            step.pending_input = {"selection": "CONFIRM"}
        add(f"confirm-{variant}", state, [step], invalid="NO" if variant == "invalid" else None)
    for variant in [
        "unique",
        "same-team-tie",
        "cross-team-tie",
        "item",
        "facedown",
        "offboard-pass",
        "offboard-respawn",
        "no-candidate",
        "unknown-candidate",
        "movement",
        "abort",
    ]:
        state = build(True)
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        if variant == "same-team-tie":
            heroes[1].current_turn_card.initiative_override = 20
        if variant == "cross-team-tie":
            heroes[2].current_turn_card.initiative_override = 20
        if variant == "item":
            heroes[-1].items[StatType.INITIATIVE] = 10
        if variant == "facedown":
            heroes[0].current_turn_card.is_facedown = True
        if variant.startswith("offboard"):
            state.remove_entity(heroes[0].id)
        if variant == "no-candidate":
            for hero in heroes:
                hero.current_turn_card = None
        if variant == "unknown-candidate":
            state.unresolved_hero_ids = ["unknown"]
        steps = [FindNextActorStep()]
        if variant == "abort":
            state.unresolved_hero_ids = []
            steps = [
                SelectStep(
                    target_type=TargetType.NUMBER,
                    prompt="Mandatory failure",
                    number_options=[],
                    output_key="bad",
                ),
                SetContextFlagStep(key="must_skip", value=True),
                ConfirmResolutionStep(hero_id="hero_arien"),
                FinalizeHeroTurnStep(hero_id="hero_arien"),
            ]
        before_positions = dict(state.entity_locations)
        final = add(
            f"handoff-{variant}",
            state,
            steps,
            invalid="bad" if "tie" in variant else None,
            prefer=(
                "PASS"
                if variant == "offboard-pass"
                else "MOVEMENT" if variant == "movement" else None
            ),
            prefix=1,
        )
        if variant not in ["no-candidate", "unknown-candidate"]:
            assert final.phase == GamePhase.PLANNING and final.turn == 2, variant
            assert not final.unresolved_hero_ids
        if variant == "abort":
            assert "must_skip" not in final.execution_context
        if variant == "movement":
            assert dict(final.entity_locations) != before_positions
            assert dict(final.last_turn_positions) == dict(final.entity_locations)
    for variant in [
        "all-empty",
        "round-end",
        "partial-empty",
        "existing-pending",
        "advance-log",
        "advance-expire",
    ]:
        state = build()
        state.unresolved_hero_ids = []
        step = FindNextActorStep()
        if variant == "all-empty":
            for team in state.teams.values():
                for hero in team.heroes:
                    hero.hand = []
        if variant == "round-end":
            state.turn = 4
        if variant == "partial-empty":
            state.get_hero("hero_hanu").hand = []
        if variant == "existing-pending":
            state.pending_inputs = {"hero_hanu": None}
            state.get_hero("hero_hanu").hand = []
        if variant.startswith("advance"):
            step = AdvanceTurnStep(
                expire_effect_ids=["missing"] if variant == "advance-expire" else []
            )
        add(
            f"advance-{variant}",
            state,
            [step],
            prefix=1,
            only_prefix=variant in ["all-empty", "round-end"],
        )
    state = build()
    state.turn = 4
    state.phase = GamePhase.CLEANUP
    add("end-phase-queue", state, [EndPhaseStep()], prefix=1, only_prefix=True)
    for variant in [
        "ordinary",
        "present",
        "pass",
        "no-spawns",
        "no-space",
        "fallback",
        "token",
        "terrain-token",
        "invalid",
        "empty-dict",
        "piece",
        "unknown",
    ]:
        state = build(multi=variant == "piece")
        hero = "hero_razzle" if variant == "piece" else "hero_arien"
        if variant != "present":
            for uid in state.get_piece_ids(hero) if variant == "piece" else [hero]:
                state.remove_entity(uid)
        if variant == "unknown":
            hero = "unknown"
        if variant in ["fallback", "token", "terrain-token", "no-space", "empty-dict"]:
            for uid in list(state.entity_locations):
                state.remove_entity(uid)
            points = [h(q, r) for q in range(-2, 3) for r in range(-2, 3) if abs(q + r) <= 2]
            spawn = h(0)
            state.board = Board(
                map_id="respawn_fixture",
                tiles={p: Tile(hex=p, zone_id="home") for p in points},
                zones={"home": Zone(id="home", hexes=set(points))},
                spawn_points=[SpawnPoint(location=spawn, team=TeamColor.RED, type=SpawnType.HERO)],
            )
            state.battle_zones = {}
            if variant in ["fallback", "no-space", "empty-dict"]:
                state.board.tiles[spawn].is_terrain = True
            else:
                token = Token(
                    id="respawn_token",
                    name="Token",
                    token_type=TokenType.TOTEM,
                    owner_id="hero_hanu",
                )
                state.misc_entities[token.id] = token
                state.place_entity(token.id, spawn)
                if variant == "terrain-token":
                    state.board.tiles[spawn].is_terrain = True
            if variant == "no-space":
                for tile in state.board.tiles.values():
                    tile.is_terrain = True
        if variant == "no-spawns":
            state.board.spawn_points = []
        add(
            f"respawn-{variant}",
            state,
            [RespawnHeroStep(hero_id=hero)],
            prefer="PASS" if variant == "pass" else None,
            invalid=(
                {}
                if variant == "empty-dict"
                else {"q": 0, "r": 0, "s": 1} if variant == "invalid" else None
            ),
        )
    from oracle import ROOT

    from goa2.engine.setup import GameSetup

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        state.phase = GamePhase.RESOLUTION
        heroes = [hero for team in state.teams.values() for hero in team.heroes]
        for hero in heroes:
            card = next(c for c in hero.hand if c.color and c.color.value == "GOLD")
            hero.play_card(card)
            card.is_facedown = False
            state.record_public_revealed_card(hero.id, card.id)
        state.unresolved_hero_ids = [hero.id for hero in heroes]
        final = add(f"map-{path.stem}", state, [FindNextActorStep()], prefix=1)
        assert final.phase == GamePhase.PLANNING and final.turn == 2
    return result
