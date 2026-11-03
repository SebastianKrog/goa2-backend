"""Frozen-source token placement, relocation, removal and supply overflow traces.

Raw steps isolate marker boundaries; actual roster token instances and the source
handler build and execute all continuations, including overflow selection.
"""

from __future__ import annotations


def scenes():
    from live_passive_traces import build
    from object_execution_traces import live_choice_operations

    from goa2.domain.hex import Hex
    from goa2.domain.models import TokenType
    from goa2.engine.steps import PlaceTokenStep, RemoveTokenStep

    def retain(state, steps, mode="first"):
        state.execution_stack = list(reversed(steps))
        state.execution_context.update(
            token_steps=steps,
            old_effects=state.active_effects,
            locations_alias=state.entity_locations,
        )
        return [
            *live_choice_operations(state, mode, stop_types=()),
            {"kind": "advance_prefix", "steps": 0},
        ]

    seen = set()
    for name in (
        "Min",
        "Mortimer",
        "Widget",
        "Trinkets",
        "Tali",
        "Wuk",
        "Mrak",
        "Ignatia",
        "Emmitt",
        "NebKher",
        "Gydion",
    ):
        state, _ = build(name)
        for kind, pool in state.token_pool.items():
            if not pool or kind in seen:
                continue
            seen.add(kind)
            for frozen in (False, True):
                state, hero = build(name)
                point = next(t.hex for t in state.board.tiles.values() if not t.is_occupied)
                state.execution_context.update(
                    target_hex=point if frozen else point.model_dump(),
                    token_alias=state.token_pool[kind][0],
                )
                step = PlaceTokenStep(
                    step_id="fixture/token/place",
                    token_type=kind,
                    output_key="placed",
                    is_immune_to_enemy_actions=True,
                )
                yield f"token-kind-{kind.value}-{frozen}", state, retain(state, [step])
    assert seen == set(TokenType), (seen, set(TokenType))

    variants = (
        "plain",
        "owner-null",
        "owner-empty",
        "owner-false",
        "owner-zero",
        "owner-foreign",
        "no-actor",
        "missing",
        "null",
        "false",
        "skip",
        "occupied",
        "virtual",
        "empty-pool",
        "relocate",
        "relocate-occupied",
        "relocate-unknown",
        "relocate-offboard",
        "relocate-wrong-kind",
        "pool-copy",
        "remove",
        "remove-key",
        "remove-missing-key",
        "remove-empty-key",
        "remove-offboard",
        "remove-unknown",
        "remove-hero",
        "remove-no-actor",
    )
    for variant in variants:
        state, hero = build("Wuk")
        kind = TokenType.TREE
        token = state.token_pool[kind][0]
        free = [t.hex for t in state.board.tiles.values() if not t.is_occupied]
        state.execution_context.update(
            target_hex=free[1],
            token_alias=token,
            token_owner="hero_hanu",
            existing=str(token.id),
            chosen=str(token.id),
        )
        step = PlaceTokenStep(
            step_id="fixture/token/step",
            token_type=kind,
            output_key="placed",
            owner_id_key="token_owner",
            is_immune_to_enemy_actions=True,
        )
        if variant in {"plain", "no-actor"}:
            step.owner_id_key = None
        owner_values = {
            "owner-null": None,
            "owner-empty": "",
            "owner-false": False,
            "owner-zero": 0,
            "owner-foreign": "foreign",
        }
        if variant in owner_values:
            state.execution_context["token_owner"] = owner_values[variant]
        if variant == "no-actor":
            state.current_actor_id = None
            token.owner_id = hero.id
        if variant == "missing":
            state.execution_context.pop("target_hex")
        if variant in {"null", "false", "skip"}:
            state.execution_context["target_hex"] = {"null": None, "false": False, "skip": "SKIP"}[
                variant
            ]
        if variant == "occupied":
            state.execution_context["target_hex"] = state.get_position(hero.id)
        if variant == "virtual":
            state.execution_context["target_hex"] = Hex(q=20, r=-20, s=0)
        if variant == "empty-pool":
            state.token_pool[kind] = []
        if variant.startswith("relocate"):
            step.existing_token_key = "existing"
            token.owner_id = hero.id
            token.is_immune_to_enemy_actions = False
            if variant != "relocate-offboard":
                state.place_entity(token.id, free[0])
            if variant == "relocate-occupied":
                state.execution_context["target_hex"] = free[0]
            if variant == "relocate-unknown":
                state.execution_context["existing"] = "unknown"
            if variant == "relocate-wrong-kind":
                step.token_type = TokenType.ILLUSION
        if variant == "pool-copy":
            state.token_pool[kind][0] = token.model_copy(deep=True)
            state.execution_context["pool_alias"] = state.token_pool[kind][0]
        if variant.startswith("remove"):
            step = RemoveTokenStep(step_id="fixture/token/step", token_id=str(token.id))
            if variant != "remove-offboard":
                state.place_entity(token.id, free[0])
            if variant in {"remove-key", "remove-missing-key", "remove-empty-key"}:
                step.token_key = "" if variant == "remove-empty-key" else "chosen"
                if variant == "remove-missing-key":
                    state.execution_context.pop("chosen")
            if variant == "remove-unknown":
                step.token_id = "unknown"
            if variant == "remove-hero":
                step.token_id = str(hero.id)
            if variant == "remove-no-actor":
                state.current_actor_id = None
        yield "token-" + variant, state, retain(state, [step])

    for mode in ("first", "last", "invalid"):
        for owned in (False, True):
            state, hero = build("Wuk")
            kind = TokenType.TREE
            free = [t.hex for t in state.board.tiles.values() if not t.is_occupied]
            for token, point in zip(state.token_pool[kind], free, strict=False):
                token.owner_id = hero.id
                state.place_entity(token.id, point)
            point = next(t.hex for t in state.board.tiles.values() if not t.is_occupied)
            state.execution_context.update(target_hex=point, token_owner=str(hero.id))
            step = PlaceTokenStep(
                step_id="fixture/token/overflow",
                token_type=kind,
                owner_id_key="token_owner" if owned else None,
                output_key="placed",
                overflow_selection_key="chosen",
                is_immune_to_enemy_actions=True,
            )
            yield f"token-overflow-{owned}-{mode}", state, retain(state, [step], mode)

    from combat_effect_traces import effect

    from goa2.domain.models import TargetType
    from goa2.domain.models.effect import EffectType
    from goa2.engine.filters import TokenTypeFilter, UnitTypeFilter
    from goa2.engine.steps import CollectUnitsStep, SetActorStep, SetContextFlagStep

    for variant in ("remove", "relocate", "offboard", "overflow"):
        state, hero = build("Wuk")
        token = state.token_pool[TokenType.TREE][0]
        free = [t.hex for t in state.board.tiles.values() if not t.is_occupied]
        if variant != "offboard":
            state.place_entity(token.id, free[0])
        card = hero.hand[0]
        card.is_active = True
        forbidden = SetContextFlagStep(
            step_id="fixture/token/forbidden", key="forbidden", value=True
        )
        linked = []
        for i in range(4):
            e = effect(
                state,
                "duplicate",
                EffectType.DELAYED_TRIGGER,
                source=str(token.id) if i == 1 else str(hero.id),
                token_id=str(token.id) if i == 0 else None,
                source_card_id=card.id,
                finishing_steps=[forbidden],
            )
            if i == 2:
                e.scope.origin_id = str(token.id)
            linked.append(e)
        state.active_effects = linked
        state.execution_context.update(existing=str(token.id), target_hex=free[1], card_alias=card)
        step = RemoveTokenStep(step_id="fixture/token/effects", token_id=str(token.id))
        if variant == "relocate":
            step = PlaceTokenStep(
                step_id="fixture/token/effects",
                token_type=TokenType.TREE,
                existing_token_key="existing",
                output_key="placed",
            )
        if variant == "overflow":
            for item, point in zip(state.token_pool[TokenType.TREE], free, strict=False):
                state.place_entity(item.id, point)
            state.execution_context["target_hex"] = next(
                t.hex for t in state.board.tiles.values() if not t.is_occupied
            )
            step = PlaceTokenStep(
                step_id="fixture/token/effects", token_type=TokenType.TREE, output_key="placed"
            )
        yield "token-effects-" + variant, state, retain(state, [step])

    for physical_copy in (False, True):
        for immune in (False, True):
            state, hero = build("Wuk")
            token = state.token_pool[TokenType.TREE][0]
            token.owner_id = "hero_brogan"
            if physical_copy:
                state.token_pool[TokenType.TREE][0] = token.model_copy(deep=True)
            point = next(t.hex for t in state.board.tiles.values() if not t.is_occupied)
            state.execution_context.update(
                target_hex=point, token_alias=token, pool_alias=state.token_pool[TokenType.TREE][0]
            )
            steps = [
                PlaceTokenStep(
                    step_id="fixture/token/query/place",
                    token_type=TokenType.TREE,
                    is_immune_to_enemy_actions=immune,
                    output_key="placed",
                )
            ]
            for actor, output in ((hero.id, "own_units"), ("hero_brogan", "enemy_units")):
                steps.extend(
                    [
                        SetActorStep(
                            step_id="fixture/token/query/actor/" + output, actor_id=str(actor)
                        ),
                        CollectUnitsStep(
                            step_id="fixture/token/query/collect/" + output,
                            target_type=TargetType.UNIT_OR_TOKEN,
                            output_key=output,
                            filters=[
                                UnitTypeFilter(unit_type="TOKEN"),
                                TokenTypeFilter(token_type=TokenType.TREE),
                            ],
                        ),
                    ]
                )
            yield f"token-query-{physical_copy}-{immune}", state, retain(state, steps)

    for overflow in (False, True):
        for applies in (False, True):
            state, hero = build("Tali")
            other, _ = build("NebKher")
            pool = other.token_pool[TokenType.ILLUSION]
            state.token_pool[TokenType.ILLUSION] = pool
            state.misc_entities.update({t.id: t for t in pool})
            free = [t.hex for t in state.board.tiles.values() if not t.is_occupied]
            if overflow:
                for token, point in zip(pool, free, strict=False):
                    state.place_entity(token.id, point)
            point = next(t.hex for t in state.board.tiles.values() if not t.is_occupied)
            state.execution_context.update(
                target_hex=point,
                token_type_override=TokenType.ILLUSION.value,
                substitution_actor_id=str(hero.id) if applies else "hero_brogan",
            )
            step = PlaceTokenStep(
                step_id="fixture/token/substitution",
                token_type=TokenType.TOTEM,
                is_immune_to_enemy_actions=True,
                output_key="placed",
            )
            yield f"token-substitution-{overflow}-{applies}", state, retain(state, [step])


def export(path):
    from object_execution_traces import reference
    from oracle import verify_sources
    from shared_objects import export_state_with_shared_objects
    from stack_traces import write_corpus_stream

    manifest = verify_sources()

    def cases():
        for name, state, operations in scenes():
            envelope, _ = export_state_with_shared_objects(state)
            yield {
                "name": "object-execution-" + name,
                "envelope": envelope,
                "operations": operations,
                "expected": reference(state, operations),
            }

    return write_corpus_stream(
        path,
        {
            "schema_version": 1,
            "oracle_revision": manifest["oracle_revision"],
            "source_sha256": manifest["source_sha256"],
        },
        cases(),
    )
