"""Full real-handler transitions for reviewed primary effects and primitive fallback."""

from __future__ import annotations

from state_codec import export_state


def cases(advance, answer, *, effects=None, include_common=True):
    from model_traces import setup
    from primary_templates import EFFECTS
    from stack_traces import reference_trace

    from goa2.domain.input import InputResponse
    from goa2.domain.models import ActionType, CardState, GamePhase, StatType, TeamColor
    from goa2.engine.handler import process_stack, push_steps, submit_input
    from goa2.engine.steps import ResolveCardStep, ResolveCardTextStep, SetContextFlagStep

    result = []

    def build(effect, name, variant, map_path=None):
        other = "Brogan" if name != "Brogan" else "Arien"
        if map_path is None:
            state = setup([name, "Hanu"], [other, "Razzle"])
        else:
            from goa2.engine.setup import GameSetup

            state = GameSetup.create_game(str(map_path), [name, "Hanu"], [other, "Razzle"], seed=29)
        hero = state.teams[TeamColor.RED].heroes[0]
        card = next(c for c in hero.deck if c.effect_id == effect)
        if card in hero.hand:
            hero.hand.remove(card)
        card.state = CardState.UNRESOLVED
        card.is_facedown = False
        hero.current_turn_card = card
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = hero.id
        piece = state.get_piece_ids("hero_razzle")[0]
        for unit in list(state.entity_locations):
            state.remove_entity(unit)

        def clear(hex):
            return hex in state.board.tiles and not state.board.tiles[hex].is_terrain

        candidates = [
            h for h in state.board.tiles if clear(h) and all(clear(n) for n in h.neighbors())
        ]
        new_effects = {
            "rogue_wave",
            "tidal_blast",
            "blink_strike",
            "mad_dash",
            "brutal_jab",
            "mighty_punch",
            "savage_kick",
        }
        origin = candidates[0]
        if effect in new_effects:
            from goa2.domain.hex import Hex

            def room(h):
                q, r, s = h.model_dump().values()
                return all(
                    clear(Hex(q=q + d, r=r, s=s - d)) and clear(Hex(q=q - d, r=r, s=s + d))
                    for d in range(1, 5)
                )

            spacious = [h for h in candidates if room(h)]
            origin = min(spacious or candidates, key=lambda h: h.distance(Hex(q=0, r=0, s=0)))
        state.place_entity(hero.id, origin)
        enemy = state.teams[TeamColor.BLUE].heroes[0]
        target = enemy.id
        if variant != "empty":
            neighbors = origin.neighbors()
            enemy_hex = neighbors[0]
            if effect == "mad_dash":
                from goa2.domain.hex import Hex

                enemy_hex = Hex(q=origin.q + 3, r=origin.r, s=origin.s - 3)
            state.place_entity(enemy.id, enemy_hex)
            state.place_entity("hero_hanu", neighbors[1])
            if variant == "piece":
                state.remove_entity(enemy.id)
                state.place_entity(piece, neighbors[0])
                target = piece
            else:
                state.place_entity(piece, neighbors[3])
            if effect == "arcane_whirlpool" or variant == "minion":
                minion = next(m for m in state.teams[TeamColor.BLUE].minions if not m.is_heavy)
                state.remove_entity(enemy.id)
                if variant == "piece":
                    state.remove_entity(piece)
                    state.place_entity(piece, neighbors[3])
                state.place_entity(minion.id, neighbors[0])
                target = minion.id
        if name == "Sabina" and effect != "point_blank_shot":
            # Make the newly reviewed minion skills exercise their real effects.
            # Pure recipe cases isolate repeat behavior from lane resolution;
            # separate repeat traces cover real action-boundary lane pushes.
            state.battle_zones = {}
            if variant != "empty":
                if variant != "piece":
                    state.remove_entity(piece)
                friendly = [m for m in state.teams[TeamColor.RED].minions if not m.is_heavy][:2]
                enemy_minions = [m for m in state.teams[TeamColor.BLUE].minions if not m.is_heavy][
                    :2
                ]
                for minion, index in zip([*friendly, *enemy_minions], [5, 2, 3, 4], strict=True):
                    state.place_entity(minion.id, origin.neighbors()[index])
        if variant == "items":
            hero.items = {StatType.ATTACK: 2, StatType.RANGE: 2, StatType.MOVEMENT: 1}
        if variant in ("zero-range", "negative-range"):
            card.is_ranged = True
            card.range_value = 0 if variant == "zero-range" else -1
        if variant in ("ranged", "melee"):
            card.is_ranged = variant == "ranged"
            card.range_value = 3
        if variant == "zero-value":
            card.primary_action_value = 0
        return state, hero, card, str(target)

    def run(state, kind, variant, target):
        probe = state.model_copy(deep=True)
        ops = []
        invalid = variant == "invalid"
        skip = variant == "skip"
        for _ in range(120):
            if variant == "atomic":
                operation = {"kind": "advance_prefix", "steps": 1}
                remaining = 1

                def boundary(_state, _step):
                    nonlocal remaining
                    if remaining == 0:
                        return True
                    remaining -= 1
                    return False

                resolution = process_stack(probe, stop_before_step=boundary)
            else:
                operation = advance
                resolution = process_stack(probe)
            ops.append(operation)
            request = resolution.input_request
            if request is None:
                if not probe.execution_stack:
                    return ops
                continue
            if invalid:
                value = "not_a_choice"
                invalid = False
            elif request.request_type.value == "SELECT_OPTION" and variant in [
                "decline",
                "one-repeat",
            ]:
                value = "NO" if variant == "decline" or "(1/" in request.prompt else "YES"
            elif request.request_type.value == "CHOOSE_ACTION":
                value = kind
            elif request.request_type.value == "SELECT_CARD_OR_PASS":
                value = "PASS"
            elif (
                request.can_skip
                and request.request_type.value == "SELECT_HEX"
                and variant != "move"
                and probe.get_hero(probe.current_actor_id).current_turn_card.effect_id
                in ("brutal_jab", "mighty_punch", "savage_kick")
            ):
                value = "SKIP"
            elif request.request_type.value == "SELECT_NUMBER":
                value = max(int(option.id) for option in request.options)
            elif skip and request.can_skip:
                value = "SKIP"
            elif request.request_type.value == "SELECT_HEX":
                from goa2.domain.hex import Hex

                destination = probe.get_position(target)
                options = request.options
                if destination is not None:
                    nearby = []
                    for option in options:
                        raw = option.metadata.get("raw")
                        if isinstance(raw, dict):
                            raw = Hex(**raw)
                        if isinstance(raw, Hex) and raw.distance(destination) <= 1:
                            nearby.append(option)
                    if nearby:
                        options = nearby
                value = options[0].metadata.get("raw", options[0].id)
            elif any(o.id == target for o in request.options):
                value = target
            else:
                value = request.options[0].id
            if hasattr(value, "model_dump"):
                value = value.model_dump(mode="json")
            ops.append(answer(value))
            submit_input(probe, InputResponse(request_id=request.id, selection=value))
        raise AssertionError("primary trace did not finish")

    def add(name, state, hero, card, target, variant, action=False):
        step = (
            ResolveCardStep(hero_id=hero.id)
            if action
            else ResolveCardTextStep(hero_id=hero.id, card_id="ignored-literal-card")
        )
        steps = [step, SetContextFlagStep(key="primary_done", value=True)]
        for i, step in enumerate(steps):
            step.step_id = f"fixture/primary/{name}/{i}"
        push_steps(state, steps)
        kind = card.current_primary_action.value if card.current_primary_action else "HOLD"
        if kind == "DEFENSE_SKILL":
            kind = "SKILL"
        ops = run(state, kind, variant, target)
        result.append(
            {
                "name": f"primary-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

        if variant in ("ordinary", "action", "items") and card.effect_id in (
            "noble_blade",
            "onslaught",
            "hit_and_run",
            "leaping_strike",
            "blink_strike",
            "mad_dash",
        ):
            assert any(
                event["event_type"] == "UNIT_DEFEATED" and event["target_id"] == target
                for record in result[-1]["expected"]
                for event in record["events"]
            ), f"{name}: positive attack must defeat its intended victim"

        if variant in ("ordinary", "action", "items") and card.effect_id in (
            "rogue_wave",
            "tidal_blast",
            "brutal_jab",
            "mighty_punch",
            "savage_kick",
        ):
            assert any(
                event["event_type"] in ("UNIT_PUSHED", "TOKEN_PUSHED")
                and event["metadata"]["distance"] > 0
                for record in result[-1]["expected"]
                for event in record["events"]
            ), f"{name}: positive push must move its victim"

        if variant in ("ordinary", "action", "items") and card.effect_id in {
            "back_to_back",
            "listen_up",
            "roger_roger",
            "ready_and_waiting",
        }:
            assert any(
                event["event_type"] == "UNITS_SWAPPED"
                for record in result[-1]["expected"]
                for event in record["events"]
            ), name
        if variant in ("ordinary", "action", "items") and card.effect_id in {
            "troop_movement",
            "marching_orders",
            "path_to_victory",
        }:
            count = sum(
                event["event_type"] == "UNIT_MOVED"
                for record in result[-1]["expected"]
                for event in record["events"]
            )
            assert count == (3 if card.effect_id == "path_to_victory" else 2), (name, count)
        if variant in ("ordinary", "action", "items") and card.effect_id in {
            "close_support",
            "covering_fire",
        }:
            final = result[-1]["expected"][-1]["semantic_state"]
            context = final["execution_context"]
            assert context["discard_source"] == "HAND", name
            assert final["turn_discard_log"][context["discarded_card_owner_id"]] == [
                context["discarded_card_id"]
            ], name
        if variant in ("ordinary", "action", "items") and card.effect_id in {
            "shootout",
            "bullet_hell",
        }:
            removed = [
                event
                for record in result[-1]["expected"]
                for event in record["events"]
                if event["event_type"] == "UNIT_REMOVED" and "minion" in str(event["target_id"])
            ]
            assert len(removed) == (2 if card.effect_id == "bullet_hell" else 1), (name, removed)

    for effect, name in (EFFECTS if effects is None else effects).items():
        for variant in [
            "ordinary",
            "action",
            "skip",
            "items",
            "zero-range",
            "negative-range",
            "zero-value",
            "invalid",
            "empty",
            "piece",
            "atomic",
            *(
                ["decline", "one-repeat"]
                if effect in {"troop_movement", "marching_orders", "path_to_victory"}
                else []
            ),
            *(
                ["move", "ranged", "melee"]
                if effect
                in {
                    "rogue_wave",
                    "tidal_blast",
                    "blink_strike",
                    "mad_dash",
                    "brutal_jab",
                    "mighty_punch",
                    "savage_kick",
                }
                else []
            ),
        ]:
            state, hero, card, target = build(effect, name, variant)
            add(
                f"{effect}-{variant}",
                state,
                hero,
                card,
                target,
                variant,
                action=variant == "action",
            )
    if not include_common:
        return result

    for kind in [
        ActionType.MOVEMENT,
        ActionType.ATTACK,
        ActionType.SKILL,
        ActionType.DEFENSE,
        ActionType.DEFENSE_SKILL,
        ActionType.HOLD,
    ]:
        for variant in ["ordinary", "items", "atomic", "hidden", "unknown-effect"]:
            state, hero, card, target = build("noble_blade", "Arien", variant)
            card.effect_id = "native-unregistered" if variant == "unknown-effect" else ""
            card.primary_action = kind
            if variant == "hidden":
                card.is_facedown = True
            add(f"fallback-{kind.value}-{variant}", state, hero, card, target, variant)

    from oracle import ROOT

    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        for effect in ["liquid_leap", "stranger_tide"]:
            state, hero, card, target = build(effect, "Arien", "ordinary", map_path=path)
            add(f"{effect}-map-{path.stem}", state, hero, card, target, "ordinary")

    from goa2.domain.hex import Hex
    from goa2.domain.models import TargetType, Token, TokenType
    from goa2.engine.steps import PlaceUnitStep, RecordHexStep, SelectStep, SwapUnitsStep

    def direct(name, state, steps):
        steps = [*steps, SetContextFlagStep(key="primitive_done", value=True)]
        for i, step in enumerate(steps):
            step.step_id = f"fixture/primary/{name}/{i}"
        push_steps(state, steps)
        result.append(
            {
                "name": f"primary-primitive-{name}",
                "envelope": export_state(state),
                "operations": [advance],
                "expected": reference_trace(state, [advance]),
            }
        )

    for variant in [
        "literal",
        "key",
        "literal-wins",
        "actor",
        "no-actor",
        "unplaced",
        "self",
        "occupied",
        "terrain",
        "virtual",
        "unknown",
        "missing-destination",
        "skip",
        "token",
        "token-unplaced",
        "piece",
    ]:
        state, hero, card, target = build("noble_blade", "Arien", "ordinary")
        free = next(h for h, tile in state.board.tiles.items() if not tile.is_obstacle)
        state.execution_context = {"dest": free.model_dump(), "who": hero.id}
        unit = hero.id
        key = None
        if variant == "key":
            unit = None
            key = "who"
        if variant == "literal-wins":
            key = "who"
            state.execution_context["who"] = True
        if variant in ("actor", "no-actor"):
            unit = None
        if variant == "no-actor":
            state.current_actor_id = None
        if variant == "unplaced":
            state.remove_entity(hero.id)
        if variant == "self":
            state.execution_context["dest"] = state.get_position(hero.id).model_dump()
        if variant == "occupied":
            state.execution_context["dest"] = state.get_position(target).model_dump()
        if variant == "terrain":
            state.execution_context["dest"] = next(
                h
                for h, tile in state.board.tiles.items()
                if tile.is_terrain and not tile.is_occupied
            ).model_dump()
        if variant == "virtual":
            state.board.map_id = ""
            state.execution_context["dest"] = Hex(q=1000, r=-1000, s=0).model_dump()
        if variant == "unknown":
            unit = "unknown"
        if variant == "missing-destination":
            state.execution_context.pop("dest")
        if variant == "skip":
            state.execution_context["dest"] = "SKIP"
        if variant.startswith("token"):
            token = Token(
                id="primary_token", name="Token", token_type=TokenType.TOTEM, owner_id=hero.id
            )
            state.misc_entities[token.id] = token
            unit = token.id
            if variant == "token":
                state.place_entity(
                    unit,
                    next(
                        h
                        for h, tile in state.board.tiles.items()
                        if not tile.is_obstacle and h != free
                    ),
                )
        if variant == "piece":
            unit = state.get_piece_ids("hero_razzle")[0]
        direct(
            f"place-{variant}",
            state,
            [PlaceUnitStep(unit_id=unit, unit_key=key, destination_key="dest")],
        )
    for variant in [
        "literal",
        "keys",
        "literal-wins",
        "same",
        "missing",
        "skip",
        "unplaced",
        "unknown",
        "piece",
    ]:
        state, hero, card, target = build("noble_blade", "Arien", "ordinary")
        state.execution_context = {"a": hero.id, "b": target}
        a = hero.id
        b = target
        ak = None
        bk = None
        if variant == "keys":
            a = None
            b = None
            ak = "a"
            bk = "b"
        if variant == "literal-wins":
            ak = "a"
            bk = "b"
            state.execution_context = {"a": True, "b": True}
        if variant == "same":
            b = a
        if variant == "missing":
            b = None
        if variant == "skip":
            b = "SKIP"
        if variant == "unplaced":
            state.remove_entity(b)
        if variant == "unknown":
            state.remove_entity(target)
            location = state.get_position("hero_hanu")
            state.remove_entity("hero_hanu")
            state.place_entity("ghost", location)
            b = "ghost"
        if variant == "piece":
            b = state.get_piece_ids("hero_razzle")[0]
        direct(
            f"swap-{variant}",
            state,
            [
                SwapUnitsStep(unit_a_id=a, unit_b_id=b, unit_a_key=ak, unit_b_key=bk),
                SelectStep(
                    target_type=TargetType.UNIT,
                    prompt="After swap",
                    output_key="after_swap",
                    skip_self_filter=True,
                    skip_immunity_filter=True,
                    is_mandatory=False,
                ),
            ],
        )
    for variant in ["literal", "key", "literal-wins", "offboard", "no-unit"]:
        state, hero, card, target = build("noble_blade", "Arien", "ordinary")
        state.execution_context = {"who": hero.id, "recorded": "old"}
        unit = hero.id
        key = None
        if variant == "key":
            unit = None
            key = "who"
        if variant == "literal-wins":
            key = "who"
            state.execution_context["who"] = True
        if variant == "offboard":
            state.remove_entity(hero.id)
        if variant == "no-unit":
            unit = None
        direct(
            f"record-{variant}",
            state,
            [RecordHexStep(unit_id=unit, unit_key=key, output_key="recorded")],
        )
    return result
