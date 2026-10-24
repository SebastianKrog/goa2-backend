"""Real-handler attack expansion, target/defense choices and defeat markers."""

from __future__ import annotations

from oracle import ROOT
from state_codec import export_state


def cases(advance, answer):
    from model_traces import setup
    from stack_traces import reference_trace

    from goa2.domain.board import Board
    from goa2.domain.hex import Hex
    from goa2.domain.models import GamePhase, Hero, Minion, MinionType, Team, TeamColor
    from goa2.domain.models.enums import StatType
    from goa2.domain.models.unit import HeroPiece
    from goa2.domain.state import GameState
    from goa2.domain.tile import Tile
    from goa2.engine.filters_units import UnitTypeFilter
    from goa2.engine.handler import push_steps
    from goa2.engine.setup import GameSetup
    from goa2.engine.steps import AttackSequenceStep, CheckTargetDefeatedStep, SetContextFlagStep

    result = []

    def build():
        coords = [
            Hex(q=q, r=r, s=-q - r) for q in range(-3, 4) for r in range(-3, 4) if abs(q + r) <= 3
        ]
        red = Hero(id="red", name="Red", team=TeamColor.RED, deck=[])
        blue = Hero(id="blue", name="Blue", team=TeamColor.BLUE, deck=[])
        razzle = Hero(id="razzle", name="Razzle", team=TeamColor.RED, deck=[], piece_supply=2)
        enemy = Minion(id="enemy", name="Enemy", team=TeamColor.BLUE, type=MinionType.MELEE)
        friendly = Minion(id="friendly", name="Friendly", team=TeamColor.RED, type=MinionType.MELEE)
        state = GameState(
            board=Board(tiles={h: Tile(hex=h) for h in coords}),
            teams={
                TeamColor.RED: Team(color=TeamColor.RED, heroes=[red, razzle], minions=[friendly]),
                TeamColor.BLUE: Team(color=TeamColor.BLUE, heroes=[blue], minions=[enemy]),
            },
            phase=GamePhase.RESOLUTION,
            rules_version=9,
            current_actor_id=red.id,
        )
        for uid, p in [
            ("red", (0, 0, 0)),
            ("blue", (2, 0, -2)),
            ("enemy", (1, 0, -1)),
            ("friendly", (-1, 0, 1)),
        ]:
            state.place_entity(uid, Hex(q=p[0], r=p[1], s=p[2]))
        for i, p in enumerate([(-2, 1, 1), (-2, 2, 0)], 1):
            piece = HeroPiece(
                id=f"razzle_piece_{i}", name="Piece", team=TeamColor.RED, owner_hero_id="razzle"
            )
            state.misc_entities[piece.id] = piece
            state.place_entity(piece.id, Hex(q=p[0], r=p[1], s=p[2]))
        state.execution_context = {"current_action_type": "ATTACK"}
        return state

    def add(name, state, steps, ops):
        for i, step in enumerate(steps):
            step.step_id = f"fixture/attack/{name}/{i}"
        push_steps(state, steps)
        result.append(
            {
                "name": f"attack-{name}",
                "envelope": export_state(state),
                "operations": ops,
                "expected": reference_trace(state, ops),
            }
        )

    for variant in [
        "minion",
        "hero",
        "preselected",
        "preselected-hero",
        "custom-output",
        "missing-target",
        "optional-missing",
        "output-fallback",
        "optional-skip",
        "optional-minion",
        "optional-hero",
        "optional-preselected",
        "filter",
        "ranged",
        "initiative",
        "bonuses",
        "negative-range",
        "items-stats",
        "piece-stats",
        "no-actor",
        "no-actor-preselected",
        "offboard-preselected",
        "piece-actor",
        "bound-piece",
        "piece-victim",
        "stale-target",
        "two-attacks",
        "old-defeat-log",
        "by-other-actor",
        "negative-mark",
        "no-mark",
        "negative-mark-miss",
    ]:
        state = build()
        spec = AttackSequenceStep(damage=4, range_val=3, target_output_key="own_target")
        ops = [advance, answer("enemy"), advance]
        extra = []
        if variant in ("hero", "preselected-hero", "optional-hero"):
            ops = [advance, answer("blue"), advance, answer("PASS"), advance]
        if variant.startswith("preselected") or variant in (
            "optional-preselected",
            "no-actor-preselected",
            "offboard-preselected",
            "stale-target",
        ):
            spec.target_id_key = "chosen"
            spec.target_output_key = None
            state.execution_context["chosen"] = "blue" if variant == "preselected-hero" else "enemy"
            ops = [advance, answer("PASS"), advance] if variant == "preselected-hero" else [advance]
        elif variant in ("missing-target", "optional-missing"):
            spec.target_id_key = "absent"
            spec.target_output_key = None
            spec.is_mandatory = variant == "missing-target"
            ops = [advance]
            extra = [SetContextFlagStep(key="tail", value=True)]
        elif variant == "output-fallback":
            spec.target_id_key = "absent"
            spec.target_output_key = "own_target"
        elif variant in ("optional-skip", "optional-minion", "optional-hero"):
            spec.is_mandatory = False
            state.execution_context["own_target"] = "blue"
            if variant == "optional-skip":
                ops = [advance, answer("SKIP"), advance]
        elif variant == "filter":
            spec.target_filters = [UnitTypeFilter(unit_type="MINION")]
        elif variant == "ranged":
            spec.is_ranged = True
        elif variant == "initiative":
            spec.defense_uses_initiative = True
        elif variant == "bonuses":
            spec.damage_bonus_key = "d"
            spec.range_bonus_key = "r"
            state.execution_context.update(d="2", r=1.9)
        elif variant == "items-stats":
            state.teams[TeamColor.RED].heroes[0].items = {StatType.ATTACK: 2, StatType.RANGE: 1}
            spec.damage_stat_type = StatType.ATTACK
            spec.range_stat_type = StatType.RANGE
        elif variant == "piece-stats":
            from goa2.domain.models.marker import Marker, MarkerType

            state.current_actor_id = "razzle"
            state.acting_piece_id = "razzle_piece_2"
            spec.range_val = 6
            state.teams[TeamColor.RED].heroes[1].items = {StatType.ATTACK: 3, StatType.RANGE: 1}
            state.markers[MarkerType.VENOM] = Marker(
                type=MarkerType.VENOM, target_id="razzle_piece_1", value=-1, source_id="blue"
            )
            spec.damage_stat_type = StatType.ATTACK
            spec.range_stat_type = StatType.RANGE
        elif variant == "negative-range":
            spec.range_val = -1
            ops = [advance]
        elif variant == "piece-actor":
            state.current_actor_id = "razzle_piece_1"
            spec.range_val = 6
        elif variant == "bound-piece":
            state.current_actor_id = "razzle"
            state.acting_piece_id = "razzle_piece_2"
            spec.range_val = 6
        elif variant == "piece-victim":
            state.current_actor_id = "blue"
            spec.target_id_key = "chosen"
            spec.target_output_key = None
            state.execution_context["chosen"] = "razzle_piece_1"
            ops = [advance, answer("PASS"), advance]
        elif variant in ("old-defeat-log", "by-other-actor"):
            state.execution_context["defeat_log"] = [["enemy", "red"]]
            if variant == "by-other-actor":
                state.current_actor_id = "friendly"
        if variant in ("no-actor", "no-actor-preselected"):
            state.current_actor_id = None
        if variant == "no-actor":
            ops = [advance]
        if variant == "offboard-preselected":
            state.remove_unit("red")
        if variant == "optional-preselected":
            spec.is_mandatory = False
        key = spec.target_id_key or spec.target_output_key or "own_target"
        if variant not in ("negative-mark", "no-mark", "negative-mark-miss"):
            checks = [
                CheckTargetDefeatedStep(target_key=key, output_key="defeated"),
                CheckTargetDefeatedStep(target_key=key, output_key="by_actor", by_actor=True),
            ]
            steps = [spec, *checks, *extra]
        else:
            state.execution_context.update(
                own_target="enemy", defeat_log=[["blue", "red"], ["enemy", "razzle_piece_1"]]
            )
            if variant != "no-mark":
                state.execution_context["defeat_marks"] = {
                    "own_target": -1 if variant == "negative-mark" else 9
                }
            steps = [
                CheckTargetDefeatedStep(target_key="own_target", output_key="defeated"),
                CheckTargetDefeatedStep(
                    target_key="own_target", output_key="by_actor", by_actor=True
                ),
            ]
            ops = [advance]
        if variant == "stale-target":
            steps.insert(1, spec.model_copy(deep=True))
        if variant == "two-attacks":
            second = AttackSequenceStep(damage=4, target_id_key="second")
            state.execution_context["second"] = "blue"
            steps.insert(1, second)
            ops = [advance, answer("enemy"), advance, answer("PASS"), advance]
        add(variant, state, steps, ops)

    # Real registered cards determine attack_is_basic; target a preselected minion
    # so these cases isolate performed-card identity rather than spatial setup.
    for variant in [
        "turn-basic",
        "turn-nonbasic",
        "performed-basic",
        "performed-nonbasic",
        "performed-owner",
        "performed-unknown",
        "no-card",
        "facedown-basic",
        "extra-owner",
        "extra-global",
        "piece-performed",
    ]:
        state = (
            setup(["Razzle", "Hanu"], ["Brogan", "Tigerclaw"])
            if variant == "piece-performed"
            else setup()
        )
        hero = state.teams[TeamColor.RED].heroes[0]
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = hero.id
        basic = next(c for c in hero.deck if c.is_basic)
        other = next(c for c in hero.deck if not c.is_basic)
        hero.current_turn_card = basic
        ctx = {"chosen": state.teams[TeamColor.BLUE].minions[0].id, "current_action_type": "ATTACK"}
        if variant == "turn-nonbasic":
            hero.current_turn_card = other
        elif variant.startswith("performed"):
            ctx["performing_card_id"] = basic.id if variant == "performed-basic" else other.id
            if variant == "performed-owner":
                ctx["performing_card_owner_id"] = hero.id
            if variant == "performed-unknown":
                ctx["performing_card_id"] = "missing"
        elif variant == "no-card":
            hero.current_turn_card = None
        elif variant == "facedown-basic":
            basic.is_facedown = True
        elif variant in ("extra-owner", "extra-global"):
            hero.deck = [c for c in hero.deck if c is not other]
            hero.hand = [c for c in hero.hand if c is not other]
            hero.extra_turn_card = other
            ctx["performing_card_id"] = other.id
            if variant == "extra-owner":
                ctx["performing_card_owner_id"] = hero.id
        elif variant == "piece-performed":
            state.current_actor_id = state.get_piece_ids(hero.id)[0]
        state.execution_context = ctx
        add(variant, state, [AttackSequenceStep(damage=3, target_id_key="chosen")], [advance])
    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        state = GameSetup.create_game(
            str(path), ["Arien", "Hanu"], ["Brogan", "Tigerclaw"], seed=29
        )
        state.phase = GamePhase.RESOLUTION
        state.current_actor_id = "hero_arien"
        add(
            f"{path.stem}-target-menu",
            state,
            [AttackSequenceStep(damage=4, range_val=100, target_output_key="own_target")],
            [advance],
        )
    return result
