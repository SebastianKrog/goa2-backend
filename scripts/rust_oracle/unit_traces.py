"""Raw-handler unit/piece fixtures, with actual filters and lane immunity rules."""

from __future__ import annotations

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
    from goa2.domain.models.base import Turret
    from goa2.domain.models.unit import HeroPiece
    from goa2.domain.state import GameState
    from goa2.domain.tile import Tile
    from goa2.engine.filters_hex import RangeFilter
    from goa2.engine.filters_units import (
        ContextIdsFilter,
        ExcludeIdentityFilter,
        HeroPieceFilter,
        ImmunityFilter,
        MinionTypesFilter,
        TeamFilter,
        TokenTypeFilter,
        UnitTypeFilter,
    )
    from goa2.engine.handler import push_steps
    from goa2.engine.steps import (
        ChooseActingPieceStep,
        MoveSequenceStep,
        SelectStep,
        SetActingPieceStep,
        SetContextFlagStep,
    )

    result = []

    def build():
        coords = [
            Hex(q=q, r=r, s=-q - r) for q in range(-3, 4) for r in range(-3, 4) if abs(q + r) <= 3
        ]
        red = Hero(id="hero_arien", name="Arien", team=TeamColor.RED, deck=[])
        blue = Hero(id="hero_hanu", name="Hanu", team=TeamColor.BLUE, deck=[])
        razzle = Hero(id="hero_razzle", name="Razzle", team=TeamColor.RED, deck=[], piece_supply=3)
        board = Board(
            tiles={h: Tile(hex=h) for h in coords},
            zones={
                "battleA": Zone(id="battleA", hexes=set(coords)),
                "battleB": Zone(id="battleB", hexes={Hex(q=-3, r=3, s=0)}),
            },
            lanes={"laneA": ["battleA"], "laneB": ["battleB"]},
        )
        state = GameState(
            board=board,
            teams={
                TeamColor.RED: Team(color=TeamColor.RED, heroes=[red, razzle]),
                TeamColor.BLUE: Team(color=TeamColor.BLUE, heroes=[blue]),
            },
            phase=GamePhase.RESOLUTION,
            rules_version=9,
            current_actor_id=red.id,
            battle_zones={"laneA": "battleA", "laneB": "battleB"},
        )
        for hero, loc in [(red, (0, 0, 0)), (blue, (2, -1, -1))]:
            state.place_entity(hero.id, Hex(q=loc[0], r=loc[1], s=loc[2]))
        for i, loc in enumerate([(-2, 1, 1), (-1, 2, -1), (-2, 2, 0)], 1):
            piece = HeroPiece(
                id=f"hero_razzle_piece_{i}",
                name="Razzle piece",
                team=TeamColor.RED,
                owner_hero_id=razzle.id,
            )
            state.misc_entities[piece.id] = piece
            state.place_entity(piece.id, Hex(q=loc[0], r=loc[1], s=loc[2]))
        for mid, kind, color, lane, loc in [
            ("heavy", MinionType.HEAVY, TeamColor.BLUE, "laneA", (1, -1, 0)),
            ("support", MinionType.MELEE, TeamColor.BLUE, "laneA", (2, -2, 0)),
            ("other_lane", MinionType.RANGED, TeamColor.BLUE, "laneB", (-3, 3, 0)),
            ("friendly", MinionType.MELEE, TeamColor.RED, "laneA", (0, 2, -2)),
        ]:
            minion = Minion(id=mid, name=mid, type=kind, team=color, lane_id=lane)
            state.teams[color].minions.append(minion)
            state.place_entity(mid, Hex(q=loc[0], r=loc[1], s=loc[2]))
        token = Token(
            id="protected",
            name="Totem",
            token_type=TokenType.TOTEM,
            owner_id=blue.id,
            is_immune_to_enemy_actions=True,
        )
        state.misc_entities[token.id] = token
        state.place_entity(token.id, Hex(q=-1, r=1, s=0))
        turret = Turret(id="turret", name="Turret", owner_id=blue.id)
        state.misc_entities[turret.id] = turret
        state.place_entity(turret.id, Hex(q=0, r=-1, s=1))
        state.place_entity("unknown", Hex(q=-1, r=0, s=1))
        return state

    def add(name, steps, operations, prepare=None):
        state = build()
        if prepare:
            prepare(state)

        def label(step, prefix):
            step.step_id = f"fixture/unit/{name}/{prefix}"
            for i, child in enumerate(getattr(step, "steps_template", [])):
                label(child, f"{prefix}-{i}")

        for i, step in enumerate(steps):
            label(step, str(i))
        push_steps(state, steps)
        envelope = export_state(state)
        result.append(
            {
                "name": f"unit-{name}",
                "envelope": envelope,
                "operations": operations,
                "expected": reference_trace(state, operations),
            }
        )

    def select(**kw):
        return SelectStep(
            target_type=TargetType.UNIT, prompt="Select a unit", output_key="target", **kw
        )

    variants = [
        ("default", {}, "hero_hanu"),
        ("include-self", {"skip_self_filter": True}, "hero_arien"),
        ("enemy", {"filters": [TeamFilter(relation="ENEMY")]}, "hero_hanu"),
        ("friendly", {"filters": [TeamFilter(relation="FRIENDLY")]}, "friendly"),
        (
            "self",
            {
                "filters": [TeamFilter(relation="SELF")],
                "skip_self_filter": True,
                "auto_select_if_one": True,
            },
            None,
        ),
        ("heroes", {"filters": [UnitTypeFilter(unit_type="HERO")]}, "hero_razzle_piece_1"),
        ("minions", {"filters": [UnitTypeFilter(unit_type="MINION")]}, "support"),
        ("heavy-immune", {"filters": [MinionTypesFilter(minion_types=[MinionType.HEAVY])]}, None),
        (
            "heavy-optout",
            {
                "filters": [MinionTypesFilter(minion_types=[MinionType.HEAVY])],
                "skip_immunity_filter": True,
                "auto_select_if_one": True,
            },
            None,
        ),
        ("range", {"filters": [RangeFilter(min_range=1, max_range=2)]}, "hero_hanu"),
        (
            "context-origin",
            {"filters": [RangeFilter(origin_key="origin", max_range_key="distance")]},
            "hero_hanu",
        ),
        (
            "exclude-list",
            {"filters": [ExcludeIdentityFilter(exclude_keys=["excluded"])]},
            "friendly",
        ),
        (
            "captured-ids",
            {"filters": [ContextIdsFilter(ids_key="allowed")], "auto_select_if_one": True},
            None,
        ),
        ("empty-minion-types", {"filters": [MinionTypesFilter(minion_types=[])]}, None),
        (
            "player-override",
            {"override_player_id": "hero_hanu", "filters": [TeamFilter(relation="ENEMY")]},
            "support",
        ),
        ("player-key", {"override_player_id_key": "controller"}, "hero_hanu"),
        ("discard-context", {"decision_purpose": "discard"}, "hero_hanu"),
    ]
    for name, kw, value in variants:

        def prepare(state):
            state.execution_context.update(
                origin="hero_hanu",
                distance=2,
                excluded=["hero_hanu", "support"],
                allowed=["heavy", "hero_hanu"],
                controller="hero_razzle_piece_1",
            )

        step = select(**kw)
        operations = [advance] if value is None else [advance, answer(value), advance]
        add(name, [step, SetContextFlagStep(key="continued")], operations, prepare)
    add(
        "invalid-and-stale",
        [select()],
        [
            advance,
            answer("hero_hanu", "not-current"),
            advance,
            answer(None),
            advance,
            answer(True),
            advance,
            answer("turret"),
            advance,
            answer("hero_hanu"),
            advance,
        ],
    )
    add(
        "optional-skip",
        [select(is_mandatory=False), SetContextFlagStep(key="continued")],
        [advance, answer("SKIP"), advance],
        lambda s: s.execution_context.update(target="old"),
    )
    add(
        "optional-empty",
        [
            select(filters=[MinionTypesFilter(minion_types=[])], is_mandatory=False),
            SetContextFlagStep(key="continued"),
        ],
        [advance],
        lambda s: s.execution_context.update(target="old"),
    )
    add(
        "optional-single-does-not-auto",
        [
            select(
                filters=[TeamFilter(relation="SELF")],
                skip_self_filter=True,
                auto_select_if_one=True,
                is_mandatory=False,
            )
        ],
        [advance, answer("hero_arien"), advance],
    )
    for variant in ("tokens", "protected-allowed", "token-type", "token-only-auto"):
        step = SelectStep(
            target_type=TargetType.UNIT_OR_TOKEN,
            prompt="Tokens and units",
            output_key="target",
            skip_immunity_filter=variant != "tokens",
            filters=(
                [TokenTypeFilter(token_type=TokenType.TOTEM)]
                if variant == "token-type"
                else [UnitTypeFilter(unit_type="TOKEN")] if variant == "token-only-auto" else []
            ),
            auto_select_if_one=variant in ("token-type", "token-only-auto"),
        )
        operations = (
            [advance, answer("hero_hanu"), advance]
            if variant == "tokens"
            else [advance] if step.auto_select_if_one else [advance, answer("protected"), advance]
        )
        add(variant, [step], operations)
    for variant in ("no-support", "other-lane-only", "support-outside-zone", "self-heavy"):

        def prepare(state, variant=variant):
            if variant == "self-heavy":
                state.current_actor_id = "heavy"
            else:
                state.remove_entity("support")
                if variant == "no-support":
                    state.remove_entity("other_lane")
                elif variant == "support-outside-zone":
                    state.place_entity("support", Hex(q=3, r=-3, s=0))
                    state.board.zones["battleA"].hexes.remove(Hex(q=3, r=-3, s=0))

        add(
            variant,
            [
                select(
                    filters=[MinionTypesFilter(minion_types=[MinionType.HEAVY])],
                    skip_self_filter=True,
                    auto_select_if_one=True,
                )
            ],
            [advance],
            prepare,
        )
    add(
        "no-actor",
        [select()],
        [advance, answer("hero_hanu"), advance],
        lambda s: setattr(s, "current_actor_id", None),
    )
    add(
        "offboard-actor",
        [select(), SetContextFlagStep(key="continued")],
        [advance],
        lambda s: s.remove_entity("hero_arien"),
    )
    add(
        "unknown-controller-token",
        [
            SelectStep(
                target_type=TargetType.UNIT_OR_TOKEN,
                prompt="Token despite missing controller",
                override_player_id="unknown_controller",
                skip_immunity_filter=True,
            )
        ],
        [advance, answer("protected"), advance],
    )

    def razzle(state):
        state.current_actor_id = "hero_razzle"

    add(
        "choose-piece-invalid",
        [ChooseActingPieceStep(hero_id="hero_razzle")],
        [
            advance,
            answer("hero_arien"),
            advance,
            answer(None),
            advance,
            answer("hero_razzle_piece_2"),
            advance,
        ],
        razzle,
    )
    add(
        "piece-team-self-exclusion",
        [
            ChooseActingPieceStep(hero_id="hero_razzle"),
            select(filters=[TeamFilter(relation="FRIENDLY"), UnitTypeFilter(unit_type="HERO")]),
        ],
        [advance, answer("hero_razzle_piece_2"), advance, answer("hero_razzle_piece_1"), advance],
        razzle,
    )
    add(
        "piece-movement",
        [
            ChooseActingPieceStep(hero_id="hero_razzle"),
            MoveSequenceStep(unit_id="hero_razzle", range_val=1),
        ],
        [
            advance,
            answer("hero_razzle_piece_2"),
            advance,
            answer({"q": -1, "r": 3, "s": -2}),
            advance,
        ],
        razzle,
    )
    for variant in ("one", "none", "normal", "unknown"):
        hero = (
            "hero_arien"
            if variant == "normal"
            else "unknown" if variant == "unknown" else "hero_razzle"
        )

        def prepare(state, variant=variant):
            razzle(state)
            if variant in ("one", "none"):
                for i in range(2 if variant == "one" else 1, 4):
                    state.remove_entity(f"hero_razzle_piece_{i}")

        add(
            f"choose-piece-{variant}",
            [ChooseActingPieceStep(hero_id=hero), SetContextFlagStep(key="continued")],
            [advance],
            prepare,
        )
    for variant in ("literal", "context", "invalid", "optional-invalid", "empty-literal"):

        def prepare(state):
            razzle(state)
            state.execution_context["chosen"] = "hero_razzle_piece_2"

        kwargs = (
            {"hero_id": "hero_razzle", "piece_id": "hero_razzle_piece_1"}
            if variant == "literal"
            else {"hero_id": "hero_razzle", "piece_key": "chosen"}
        )
        if variant in ("invalid", "optional-invalid"):
            kwargs["piece_id"] = "protected"
        if variant == "optional-invalid":
            kwargs["is_mandatory"] = False
        if variant == "empty-literal":
            kwargs["piece_id"] = ""
        add(
            f"set-piece-{variant}",
            [SetActingPieceStep(**kwargs), SetContextFlagStep(key="continued")],
            [advance],
            prepare,
        )

    def bound(state):
        state.current_actor_id = "hero_razzle"
        state.acting_piece_id = "hero_razzle_piece_1"

    add(
        "piece-filter",
        [select(filters=[HeroPieceFilter()])],
        [advance, answer("hero_razzle_piece_2"), advance],
        bound,
    )
    add(
        "piece-filter-includes-self",
        [select(filters=[HeroPieceFilter(exclude_acting=False)], skip_self_filter=True)],
        [advance, answer("hero_razzle_piece_1"), advance],
        bound,
    )
    add("piece-filter-other-owner", [select(filters=[HeroPieceFilter()])], [advance])

    def physical_actor(state):
        state.current_actor_id = "hero_razzle_piece_1"
        state.acting_piece_id = "hero_razzle_piece_2"

    add(
        "piece-filter-physical-actor",
        [select(filters=[HeroPieceFilter()], auto_select_if_one=True)],
        [advance],
        physical_actor,
    )

    def friendly_token(state):
        state.current_actor_id = "hero_hanu"

    add(
        "protected-friendly-token",
        [
            SelectStep(
                target_type=TargetType.UNIT_OR_TOKEN,
                prompt="Friendly token",
                filters=[TokenTypeFilter()],
                auto_select_if_one=True,
            )
        ],
        [advance],
        friendly_token,
    )

    def unowned_token(state):
        state.misc_entities["protected"].owner_id = None

    add(
        "protected-unowned-token",
        [
            SelectStep(
                target_type=TargetType.UNIT_OR_TOKEN,
                prompt="Unowned token",
                filters=[UnitTypeFilter(unit_type="TOKEN"), ImmunityFilter()],
                auto_select_if_one=True,
            )
        ],
        [advance],
        unowned_token,
    )

    def orphan_piece(state):
        state.misc_entities["hero_razzle_piece_1"].owner_hero_id = "missing_hero"

    add(
        "orphan-piece-controller",
        [select(override_player_id="hero_razzle_piece_1")],
        [advance, answer("hero_hanu"), advance],
        orphan_piece,
    )
    # Hero-targeted steps accept a physical piece ID through get_hero as well.
    from model_traces import setup

    from goa2.engine.steps import RetrieveUnresolvedCardStep

    state = setup(["Razzle", "Arien"])
    hero = state.get_hero("hero_razzle")
    first, second = hero.hand[:2]
    hero.play_card(first)
    hero.play_card(second)
    hero.current_turn_card = first
    hero.extra_turn_card = second
    first.is_facedown = False
    second.is_facedown = False
    state.phase = GamePhase.RESOLUTION
    piece = state.get_piece_ids(hero.id)[0]
    step = RetrieveUnresolvedCardStep(step_id="fixture/unit/piece-owned-retrieval", hero_id=piece)
    push_steps(state, [step])
    operations = [advance, answer(str(first.id)), advance]
    envelope = export_state(state)
    result.append(
        {
            "name": "unit-piece-owned-retrieval",
            "envelope": envelope,
            "operations": operations,
            "expected": reference_trace(state, operations),
        }
    )
    return result
