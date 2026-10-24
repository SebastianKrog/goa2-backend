"""Real-handler round cleanup, upgrade validation and round reset traces."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer):
    import copy

    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.input import InputResponse
    from goa2.domain.models import (
        CardColor,
        CardState,
        CardTier,
        GamePhase,
        TeamColor,
        Token,
        TokenType,
    )
    from goa2.domain.models.marker import MarkerType
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import (
        EndPhaseCleanupStep,
        FindNextActorStep,
        ResolveUpgradesStep,
        RoundResetStep,
        SetContextFlagStep,
    )
    from goa2.engine.steps.cards import apply_hero_upgrade

    result = []

    def build(name="Arien"):
        names = list(dict.fromkeys([name, "Arien", "Hanu", "Brogan", "Tigerclaw"]))[:4]
        state = setup(names[:2], names[2:])
        state.phase = GamePhase.CLEANUP
        state.turn = 4
        state.current_actor_id = state.teams[TeamColor.RED].heroes[0].id
        state.resolution_owner_id = state.current_actor_id
        state.execution_context = {"keep": True}
        return state, state.teams[TeamColor.RED].heroes[0]

    def upgrade_selection(state, req, batch, side):
        hero_id = next(
            h for h, p in req.context["players"].items() if p["remaining"] > 0 and p["options"]
        )
        count = req.context["players"][hero_id]["remaining"]
        if not batch:
            return {
                "hero_id": hero_id,
                "card_id": req.context["players"][hero_id]["options"][0]["pair"][side],
            }
        preview = state.model_copy(deep=True)
        ids = []
        for _ in range(count):
            request = (
                ResolveUpgradesStep().resolve(preview, preview.execution_context).input_request
            )
            selected = request.context["players"][hero_id]["options"][0]["pair"][side]
            ids.append(selected)
            preview = ResolveUpgradesStep().preview_choices(preview, hero_id, [selected])
        return {"hero_id": hero_id, "card_ids": ids}

    def add(
        name, state, step=None, batch=False, side=0, invalid=None, only_first=False, prefix=None
    ):
        step = step or EndPhaseCleanupStep()
        steps = [step, SetContextFlagStep(key="round_done", value=True)]
        for i, frame in enumerate(steps):
            frame.step_id = f"fixture/round/{name}/{i}"
        push_steps(state, steps)
        envelope = export_state(state)
        work = copy.deepcopy(state)
        operations = [{"kind": "advance_prefix", "steps": prefix}] if prefix is not None else []
        operations.append(advance)
        out = process_stack(work)
        if invalid is not None and out.input_request:
            req = out.input_request
            valid = upgrade_selection(work, req, False, 0)
            hero = valid["hero_id"]
            card = valid["card_id"]
            if invalid == "duplicate":
                value = {"hero_id": hero, "card_ids": [card, card]}
            elif invalid == "mixed":
                value = {"hero_id": hero, "card_id": card, "card_ids": [card]}
            elif invalid == "unknown-hero":
                value = {"hero_id": "unknown", "card_id": card}
            elif invalid == "unknown-card":
                value = {"hero_id": hero, "card_id": "unknown"}
            elif invalid == "partial":
                value = {
                    "hero_id": hero,
                    "card_ids": [p["pair"][0] for p in req.context["players"][hero]["options"][:2]],
                }
            elif invalid == "illegal-second":
                first = work.get_card_by_id(card)
                higher = next(
                    c.id
                    for c in work.get_hero(hero).deck
                    if c.color == first.color and c.tier == CardTier.III
                )
                value = {"hero_id": hero, "card_ids": [card, higher]}
            elif invalid == "same-pair":
                value = {
                    "hero_id": hero,
                    "card_ids": req.context["players"][hero]["options"][0]["pair"],
                }
            elif invalid == "wrong-type":
                value = {"hero_id": hero, "card_ids": "not-a-list"}
            else:
                value = invalid
            try:
                submit_input(work, InputResponse(request_id=req.id, selection=value))
            except ValueError:
                pass
            else:
                raise AssertionError(f"invalid upgrade accepted: {name}")
            operations.extend([answer(value), advance])
            out = process_stack(work)
        if not only_first:
            for _ in range(60):
                if not out.input_request:
                    break
                req = out.input_request
                assert req.request_type.value == "UPGRADE_PHASE", (name, req)
                value = upgrade_selection(work, req, batch, side)
                operations.extend([answer(value), advance])
                submit_input(work, InputResponse(request_id=req.id, selection=value))
                out = process_stack(work)
            else:
                raise AssertionError(f"round did not finish: {name}")
            assert (
                work.phase == GamePhase.PLANNING
                and work.turn == 1
                and work.round == state.round + 1
            ), name
            assert work.execution_context.get("round_done"), name
        expected = reference_trace(state, operations)
        result.append(
            {
                "name": f"round-{name}",
                "envelope": envelope,
                "operations": operations,
                "expected": expected,
            }
        )
        return work

    for variant in [
        "ordinary",
        "played-discard-current",
        "markers",
        "sparse-items",
        "negative-gold",
        "max-level",
        "current-active",
        "discard-active",
    ]:
        state, hero = build()
        if variant in ["played-discard-current", "current-active", "discard-active"]:
            card = hero.hand[0]
            hero.play_card(card)
            hero.resolve_current_card(turn_number=1)
            card.passive_uses_this_turn = 3
            card.is_active = True
            card = hero.hand[0]
            hero.discard_card(card)
            card.is_active = True
            card = hero.hand[0]
            hero.play_card(card)
            card.is_active = True
        if variant == "markers":
            for kind in [MarkerType.VENOM, MarkerType.POISON, MarkerType.BOUNTY]:
                state.place_marker(kind, hero.id, -2, "hero_hanu")
        if variant == "sparse-items":
            hero.items = {}
            hero.gold = 1
        if variant == "negative-gold":
            hero.gold = -3
        if variant == "max-level":
            hero.level = 8
            hero.gold = 4
        state.heroes_defeated_this_round = [hero.id]
        final = add(f"cleanup-{variant}", state, prefix=1)
        if variant == "markers":
            assert all(m.target_id is None for m in final.markers.values())
        if variant not in ["sparse-items"]:
            assert final.heroes_defeated_this_round == state.heroes_defeated_this_round
    for gold in [0, 1, 2, 3, 6, 10, 21, 28]:
        for batch in [False, True]:
            state, hero = build()
            hero.gold = gold
            final = add(
                f"gold-{gold}-batch-{batch}", state, batch=batch, side=1 if batch else 0, prefix=1
            )
            assert final.get_hero(hero.id).level <= 8
    for level, gold in [(2, 1), (2, 2), (5, 4), (5, 5), (7, 6), (7, 7), (8, 0)]:
        state, hero = build()
        hero.level = level
        hero.gold = gold
        add(f"level-{level}-gold-{gold}", state, batch=True)
    for variant in ["temporary", "persistent", "duplicate-pool", "unpooled"]:
        state, hero = build()
        point = next(p for p in state.board.tiles if not state.board.get_tile(p).is_obstacle)
        token = Token(
            id="round_token",
            name="Round token",
            token_type=TokenType.TOTEM,
            owner_id=str(hero.id),
            persists_end_of_round=variant == "persistent",
        )
        state.misc_entities[token.id] = token
        state.place_entity(token.id, point)
        if variant != "unpooled":
            state.token_pool.setdefault(TokenType.TOTEM, []).append(token)
        if variant == "duplicate-pool":
            state.token_pool[TokenType.TOTEM].append(token)
        final = add(f"token-{variant}", state)
        assert (final.get_position(token.id) is not None) == (variant in ["persistent", "unpooled"])
    for variant in [
        "duplicate",
        "mixed",
        "unknown-hero",
        "unknown-card",
        "partial",
        "illegal-second",
        "same-pair",
        "wrong-type",
        False,
        [],
        {},
        "bad",
    ]:
        state, hero = build()
        hero.gold = 6 if variant == "partial" else 3
        add(f"invalid-{variant!s}", state, invalid=variant, batch=True)
    for name in ["Ursafar", "Takahide"]:
        for variant in [
            "unlock",
            "unlock-six",
            "already-unlocked",
            "missing-ultimate",
            "silver-deck",
        ]:
            state, hero = build(name)
            hero.level = 1 if variant == "unlock-six" else 8 if variant == "already-unlocked" else 7
            hero.gold = 28 if variant == "unlock-six" else 7
            if variant == "missing-ultimate":
                hero.ultimate_card = None
            if variant == "silver-deck" and name == "Takahide":
                silver = next(c for c in hero.deck if c.color == CardColor.SILVER)
                if silver in hero.hand:
                    hero.hand.remove(silver)
                silver.state = CardState.DECK
            final = add(f"ultimate-{name}-{variant}", state, batch=True, prefix=1)
            actual = final.get_hero(hero.id)
            if name == "Ursafar" and variant in ["unlock", "unlock-six"]:
                assert all(c.enraged_active_override for c in actual.deck)
            if name == "Takahide" and variant in ["unlock", "unlock-six"]:
                assert (
                    next(c for c in actual.deck if c.color == CardColor.SILVER).state
                    == CardState.DECK
                )
    for variant in [
        "empty",
        "one",
        "tier-three",
        "no-hand",
        "unknown",
        "zero",
        "sparse",
        "preloaded",
        "reset",
    ]:
        state, hero = build()
        state.phase = GamePhase.LEVEL_UP
        step = ResolveUpgradesStep()
        state.pending_upgrades = {} if variant in ["empty", "reset"] else {hero.id: 1}
        state.upgrade_reveal_snapshot = {hero.id: dict(hero.items)}
        state.heroes_defeated_this_round = [hero.id]
        if variant == "tier-three":
            for _ in range(3):
                opt = ResolveUpgradesStep()._get_upgrade_options(state, hero.id)[0]
                apply_hero_upgrade(state, hero.id, opt["pair"][0])
            state.pending_upgrades = {hero.id: 1}
        if variant == "no-hand":
            hero.hand = []
        if variant == "unknown":
            state.pending_upgrades = {"unknown": 1}
        if variant == "zero":
            state.pending_upgrades = {hero.id: 0}
        if variant == "sparse":
            hero.items = {}
        if variant == "preloaded":
            step.pending_input = {
                "selection": {
                    "hero_id": str(hero.id),
                    "card_id": ResolveUpgradesStep()._get_upgrade_options(state, hero.id)[0][
                        "pair"
                    ][0],
                }
            }
        if variant == "reset":
            step = RoundResetStep()
        final = add(
            f"upgrade-{variant}",
            state,
            step,
            batch=True,
            only_first=variant in ["no-hand", "unknown", "zero"],
        )
        if variant not in ["no-hand", "unknown", "zero"]:
            assert not final.heroes_defeated_this_round
    for name in sorted(HeroRegistry.list_heroes(include_playtest=True)):
        for side in [0, 1]:
            state, hero = build(name)
            hero.gold = 1
            final = add(f"hero-{name}-{side}", state, side=side, batch=bool(side))
            assert final.get_hero(hero.id).level == 2
            assert sum(final.get_hero(hero.id).items.values()) == 1
    from oracle import ROOT

    from goa2.engine.setup import GameSetup

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        state.phase = GamePhase.RESOLUTION
        state.turn = 4
        state.unresolved_hero_ids = []
        for team in state.teams.values():
            for hero in team.heroes:
                card = next(c for c in hero.hand if c.color == CardColor.GOLD)
                hero.play_card(card)
                hero.resolve_current_card(turn_number=4)
        state.get_hero("hero_arien").gold = 1
        add(f"map-{path.stem}", state, FindNextActorStep(), batch=True, prefix=1)
    return result
