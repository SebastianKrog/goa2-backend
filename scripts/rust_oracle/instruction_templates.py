"""Freeze Python's wire defaults for native-generated continuation frames."""

import json
import sys

from fixture_cache import write_text
from oracle import ROOT, verify_sources

sys.path.insert(0, str(ROOT / "src"))
from goa2.domain.models import TargetType, TokenType, Turret
from goa2.engine.filters_geometry import InStraightLineFilter, StraightLinePathFilter
from goa2.engine.filters_hex import (
    FastTravelDestinationFilter,
    HexBatchCompletableFilter,
    MovementPathFilter,
    ObstacleFilter,
    RangeFilter,
    TokenRemovalCompletableFilter,
)
from goa2.engine.steps import (
    AdvanceTurnStep,
    AttackSequenceStep,
    CastSpellStep,
    CheckContextConditionStep,
    CheckLanePushStep,
    CheckMinionProtectionStep,
    CheckPassiveAbilitiesStep,
    ChooseActingPieceStep,
    ChooseMinionRemovalStep,
    CoDirectionalDragStep,
    ConfirmResolutionStep,
    DefeatUnitStep,
    DiscardCardStep,
    EndPhaseCleanupStep,
    EndPhaseStep,
    FastTravelSequenceStep,
    FastTravelUnitStep,
    FinalizeHeroTurnStep,
    FindNextActorStep,
    FinishedExpiringEffectStep,
    FlipTieBreakerCoinStep,
    ForceDiscardOrDefeatStep,
    ForceDiscardStep,
    ForEachStep,
    LanePushStep,
    LogMessageStep,
    MarkPassiveUsedStep,
    MayRepeatNTimesStep,
    MinePathChoiceStep,
    MinionBattleStep,
    MoveSequenceStep,
    MoveTokenStep,
    MoveUnitStep,
    PushUnitStep,
    OfferPassiveStep,
    OfferRockUltimateStep,
    PerformCardActionStep,
    PlaceMarkerStep,
    PlaceTokenStep,
    PlaceUnitStep,
    PrepareSpellbookStep,
    ReactionWindowStep,
    RemoveMarkerStep,
    RemovePreparedSpellsStep,
    RemoveTokenStep,
    RemoveUnitStep,
    ResolveCardStep,
    ResolveCardTextStep,
    ResolveCombatStep,
    ResolveDefenseTextStep,
    ResolveDisplacementStep,
    ResolveOnBlockEffectStep,
    ResolvePreActionDiscardStep,
    ResolvePreActionMovementStep,
    ResolveTieBreakerStep,
    ResolveUpgradesStep,
    RespawnHeroStep,
    RestoreActionTypeStep,
    RetrieveUnresolvedCardStep,
    ReturnMinionToZoneStep,
    RoundResetStep,
    ScheduleActionControlStep,
    ScheduleJourneyReturnStep,
    SelectStep,
    SetActorStep,
    SetCardInitiativeStep,
    SetContextFlagStep,
    TriggerGameOverStep,
    TriggerMineStep,
    ValidateRepeatStep,
    WishSequenceStep,
)


def journey_finishing():
    """Freeze actual manager-created children, including nested filter models."""
    from model_traces import setup
    from primary_builders import freeze_steps
    from goa2.engine.steps import ScheduleJourneyReturnStep

    state = setup()
    state.current_actor_id = "template-passive-owner"
    ScheduleJourneyReturnStep(enemy_key="enemy", move_after=True).resolve(
        state, {"enemy": "template-journey-enemy"}
    )
    return freeze_steps(state.active_effects[-1].finishing_steps)


def passive_continuations():
    """Freeze source-created structure; Rust binds live owners and rule values."""
    from live_passive_traces import build, enable
    from model_traces import setup
    from shared_objects import capture

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.models import CardState, RuneType
    from goa2.engine.effects import CardEffect, CardEffectRegistry

    setup()
    methods = {
        key: type(effect).get_passive_steps.__qualname__
        for key, effect in CardEffectRegistry._effects.items()
        if type(effect).get_passive_steps is not CardEffect.get_passive_steps
    }
    recipes = {}

    def freeze(key, steps, owner):
        whole_bindings, _ = capture(steps, [step.model_dump(mode="json") for step in steps])
        if any(len(binding["paths"]) != 1 for binding in whole_bindings):
            raise ValueError("passive continuation has shared body children")
        rows = []
        for step in steps:
            raw = step.model_dump(mode="json")
            bindings, _ = capture(step, raw)
            if any(len(binding["paths"]) != 1 for binding in bindings):
                raise ValueError("passive continuation requires shared-child recipe bindings")

            def bind(value):
                if isinstance(value, dict):
                    return {k: "template" if k == "step_id" else bind(v) for k, v in value.items()}
                if isinstance(value, list):
                    return [bind(v) for v in value]
                return "template-passive-owner" if value == owner else value

            rows.append({"fields": bind(raw), "objects": bindings})
        recipes[key] = rows

    for name, definition in sorted(HeroRegistry._heroes.items()):
        for original in [
            *definition.deck,
            *([definition.ultimate_card] if definition.ultimate_card else []),
        ]:
            effect = CardEffectRegistry.get(original.effect_id)
            configs = effect.get_passive_configs() if effect else []
            if not configs or original.effect_id in {"flurry_of_blows", "cloak_and_daggers"}:
                continue
            state, hero = build(name)
            card = next(
                c for c in [*hero.deck, hero.ultimate_card] if c is not None and c.id == original.id
            )
            hero.level = 8
            card.state, card.is_facedown = CardState.RESOLVED, False
            enable(state, hero, card, original.effect_id)
            enemy = next(team.heroes[0] for team in state.teams.values() if team.color != hero.team)
            if original.effect_id == "a_complicated_profession":
                state.execution_context["marker_target_id"] = str(enemy.id)
            if original.effect_id == "big_sodding_gun":
                state.execution_context["push_victim_id"] = str(enemy.id)
            if original.effect_id == "rune_mastery":
                hero.rune_slots = {i + 1: rune for i, rune in enumerate(RuneType)}
            steps = effect.get_passive_steps(
                state, hero, card, configs[0].trigger, state.execution_context
            )
            if not steps:
                raise ValueError(f"passive continuation fixture is empty: {original.effect_id}")
            freeze(original.effect_id, steps, str(hero.id))
    state, hero = build("Min")
    hero.current_turn_card = hero.ultimate_card
    freeze(
        "flurry_of_blows",
        CardEffectRegistry.get("flurry_of_blows").get_passive_steps(
            state,
            hero,
            hero.ultimate_card,
            "after_attack",
            {"attack_effect_id": "flurry_of_blows", "attack_card_id": hero.ultimate_card.id},
        ),
        str(hero.id),
    )
    state, hero = build("Takahide")
    effect = CardEffectRegistry.get("cloak_and_daggers")
    from goa2.domain.models import ActionType, PassiveTrigger

    for action in (ActionType.ATTACK, ActionType.MOVEMENT):
        context = {
            "basic_action_type": action.value,
            "basic_action_value": 3,
            "basic_action_range": 2,
        }
        freeze(
            "cloak_and_daggers/" + action.value,
            effect.get_passive_steps(
                state, hero, hero.ultimate_card, PassiveTrigger.AFTER_BASIC_ACTION, context
            ),
            str(hero.id),
        )
    for mask in range(4):
        state, hero = build("Arien")
        sequence = MoveSequenceStep(
            unit_id=str(hero.id),
            range_val=1,
            force_straight_line=bool(mask & 1),
            force_full_distance=bool(mask & 2),
        )
        freeze("movement/" + str(mask), sequence.resolve(state, {}).new_steps, str(hero.id))
    state, hero = build("Wuk")
    point = next(h for h in state.board.tiles if h not in state.entity_locations.values())
    for token, hex in zip(
        state.token_pool[TokenType.TREE], list(state.board.tiles)[:3], strict=True
    ):
        if hex in state.entity_locations.values():
            hex = next(h for h in state.board.tiles if h not in state.entity_locations.values())
        state.place_entity(token.id, hex)
    point = next(h for h in state.board.tiles if h not in state.entity_locations.values())
    step = PlaceTokenStep(
        token_type=TokenType.TREE,
        hex_key="target_hex",
        owner_id_key="token_owner",
        output_key="placed",
        existing_token_key="existing",
        overflow_selection_key="overflow",
        is_immune_to_enemy_actions=True,
    )
    freeze("token/overflow", step.resolve(state, {"target_hex": point}).new_steps, str(hero.id))
    freeze(
        "token/batch_primitives",
        [
            SelectStep(target_type=TargetType.HEX, prompt="", is_mandatory=True),
            SelectStep(
                target_type=TargetType.NUMBER,
                prompt="",
                output_key="batch_choice",
                number_options=[1, 0],
                number_labels={1: "Place tokens", 0: "Skip"},
                override_player_id_key="batch_owner",
                is_mandatory=True,
            ),
            CheckContextConditionStep(
                input_key="batch_choice", operator="==", threshold=1, output_key="batch_go"
            ),
            SetContextFlagStep(key="batch_done", value=True),
        ],
        str(hero.id),
    )
    state, hero = build("Mrak")
    hero.level = 8
    enemy = state.get_hero("hero_brogan")
    from goa2.domain.hex import Hex

    state.place_entity(enemy.id, Hex(q=2, r=0, s=-2))
    rock = Hex(q=1, r=0, s=-1)
    step = OfferRockUltimateStep(rock_hex_keys=["rock"])
    freeze("token/rock_ultimate", step.resolve(state, {"rock": rock}).new_steps, str(hero.id))
    return {"methods": methods, "recipes": recipes}


def export():
    manifest = verify_sources()
    from goa2.bootstrap import register_all_effects
    from goa2.domain.models.enums import PassiveTrigger
    from goa2.engine.effects import CardEffect, CardEffectRegistry

    register_all_effects()
    from goa2.engine.step_types import rebuild_serialization_models

    rebuild_serialization_models()
    from goa2.engine.steps.cards import _clear_selection

    values = {
        "place_bounty": PlaceMarkerStep(step_id="template", marker_type="bounty"),
        "remove_bounty": RemoveMarkerStep(step_id="template", marker_type="bounty"),
        "select_card": SelectStep(step_id="template", target_type=TargetType.CARD, prompt=""),
        "force_discard": ForceDiscardStep(step_id="template", victim_key="template"),
        "select_unit": SelectStep(step_id="template", target_type=TargetType.UNIT, prompt=""),
        "resolve_combat": ResolveCombatStep(step_id="template", damage=1),
        "set_actor": SetActorStep(step_id="template"),
        "finished_expiring_effect": FinishedExpiringEffectStep(step_id="template"),
        "discard_card": DiscardCardStep(step_id="template"),
        "reaction": ReactionWindowStep(step_id="template"),
        "defense_text": ResolveDefenseTextStep(step_id="template"),
        "on_block": ResolveOnBlockEffectStep(step_id="template"),
        "restore_action": RestoreActionTypeStep(step_id="template"),
        "defeat": DefeatUnitStep(step_id="template"),
        "minion_protection": CheckMinionProtectionStep(step_id="template", minion_id="template"),
        "remove_unit": RemoveUnitStep(step_id="template"),
        "game_over": TriggerGameOverStep(
            step_id="template", winner="RED", condition="ANNIHILATION"
        ),
        "action_piece": ChooseActingPieceStep(step_id="template", hero_id="template"),
        "offer_passive": OfferPassiveStep(
            step_id="template", card_id="template", trigger="after_push"
        ),
        "mark_passive": MarkPassiveUsedStep(step_id="template", card_id="template"),
        "discard_or_defeat": ForceDiscardOrDefeatStep(
            step_id="template", victim_key="push_victim_id"
        ),
        "passive": CheckPassiveAbilitiesStep(step_id="template", trigger="before_action"),
        "pre_movement": ResolvePreActionMovementStep(step_id="template", hero_id="template"),
        "pre_discard": ResolvePreActionDiscardStep(step_id="template", hero_id="template"),
        "pre_force_discard": ForceDiscardStep(
            step_id="template", victim_key="pre_action_discard_victim"
        ),
        "card_text": ResolveCardTextStep(
            step_id="template", hero_id="template", card_id="template"
        ),
        "movement_sequence": MoveSequenceStep(step_id="template", unit_id="template", range_val=1),
        "fast_travel": FastTravelSequenceStep(
            step_id="template", unit_id="template", source_card_id="template"
        ),
        "attack_sequence": AttackSequenceStep(step_id="template", damage=1),
        "clear_select": _clear_selection(),
        "clear_each": ForEachStep(
            step_id="template",
            list_key="clear_targets",
            item_key="target_id",
            steps_template=[RemoveTokenStep(step_id="template", token_key="target_id")],
        ),
        "log": LogMessageStep(step_id="template", message="template"),
        "context": SetContextFlagStep(step_id="template", key="template", value=None),
        "select_hex": SelectStep(step_id="template", target_type=TargetType.HEX, prompt=""),
        "move_unit": MoveUnitStep(step_id="template"),
        "mirror_push": PushUnitStep(step_id="template"),
        "set_initiative": SetCardInitiativeStep(step_id="template", hero_key="template"),
        "schedule_control": ScheduleActionControlStep(step_id="template", hero_key="template"),
        "schedule_journey": ScheduleJourneyReturnStep(step_id="template", enemy_key="template"),
        "move_token": MoveTokenStep(step_id="template", token_key="template"),
        "co_drag": CoDirectionalDragStep(
            step_id="template", anchor_key="template", partner_key="template"
        ),
        "mine_path_choice": MinePathChoiceStep(step_id="template"),
        "trigger_mine": TriggerMineStep(step_id="template"),
        "place_unit": PlaceUnitStep(step_id="template"),
        "lane_push": LanePushStep(step_id="template", losing_team="RED"),
        "repeat": MayRepeatNTimesStep(step_id="template"),
        "validate_repeat": ValidateRepeatStep(step_id="template"),
        "check_lane_push": CheckLanePushStep(step_id="template"),
        "minion_battle": MinionBattleStep(step_id="template"),
        "advance_turn": AdvanceTurnStep(step_id="template"),
        "end_phase": EndPhaseStep(step_id="template"),
        "end_cleanup": EndPhaseCleanupStep(step_id="template"),
        "upgrades": ResolveUpgradesStep(step_id="template"),
        "round_reset": RoundResetStep(step_id="template"),
        "return_minions": ReturnMinionToZoneStep(step_id="template"),
        "minion_removal": ChooseMinionRemovalStep(
            step_id="template", losing_team="RED", remaining_to_remove=1, zone_id="template"
        ),
        "displacement": ResolveDisplacementStep(step_id="template"),
        "fast_travel_unit": FastTravelUnitStep(step_id="template"),
        "travel_filter": FastTravelDestinationFilter(),
        "cast_spell": CastSpellStep(step_id="template", allowed_spell_ids=[]),
        "perform_card": PerformCardActionStep(step_id="template"),
        "wish_sequence": WishSequenceStep(step_id="template", caster_id="template"),
        "prepare_spellbook": PrepareSpellbookStep(step_id="template"),
        "remove_prepared_spells": RemovePreparedSpellsStep(step_id="template"),
        "resolve_card": ResolveCardStep(step_id="template", hero_id="template"),
        "confirm_resolution": ConfirmResolutionStep(step_id="template", hero_id="template"),
        "finalize_hero": FinalizeHeroTurnStep(step_id="template", hero_id="template"),
        "respawn": RespawnHeroStep(step_id="template", hero_id="template"),
        "tie": ResolveTieBreakerStep(step_id="template", tied_hero_ids=[]),
        "retrieve_unresolved": RetrieveUnresolvedCardStep(step_id="template", hero_id="template"),
        "find_actor": FindNextActorStep(step_id="template"),
        "flip_coin": FlipTieBreakerCoinStep(step_id="template"),
        "obstacle": ObstacleFilter(),
        "movement_path": MovementPathFilter(range_val=1),
        "range": RangeFilter(),
        "straight": InStraightLineFilter(),
        "straight_path": StraightLinePathFilter(),
    }
    values["clear_select"].step_id = "template"
    from types import SimpleNamespace

    wild_hunt_steps = CardEffectRegistry.get("wild_hunt").get_passive_steps(
        None,
        SimpleNamespace(id="template-passive-owner"),
        None,
        PassiveTrigger.BEFORE_ACTION,
        {},
    )
    for step in wild_hunt_steps:
        step.step_id = "template"
    from goa2.domain.models.effect import ActiveEffect, DurationType, EffectScope, EffectType, Shape

    movement_provider = ActiveEffect(
        id="template",
        source_id="template",
        effect_type=EffectType.PRE_ACTION_MOVEMENT,
        scope=EffectScope(shape=Shape.GLOBAL),
        duration=DurationType.PASSIVE,
        max_value=1,
        is_active=True,
        created_at_turn=1,
        created_at_round=1,
    )
    pre_action_moves = (
        ResolvePreActionMovementStep(hero_id="template")
        .resolve(SimpleNamespace(active_effects=[movement_provider], round=1, turn=1), {})
        .new_steps
    )
    for step in pre_action_moves:
        step.step_id = "template"
    payload = {
        "pre_action_movement": [step.model_dump(mode="json") for step in pre_action_moves],
        "passive_bodies": {"wild_hunt": [step.model_dump(mode="json") for step in wild_hunt_steps]},
        "schema_version": 1,
        "source_sha256": manifest["source_sha256"],
        "two_card_effects": sorted(
            key for key, effect in CardEffectRegistry._effects.items() if effect.plays_two_cards
        ),
        "passive_triggers": [t.value for t in PassiveTrigger],
        "custom_on_block_effects": sorted(
            key
            for key, effect in CardEffectRegistry._effects.items()
            if type(effect).get_on_block_steps is not CardEffect.get_on_block_steps
            or type(effect).build_on_block_steps is not CardEffect.build_on_block_steps
        ),
        "ultimate_unlock_effects": sorted(
            key
            for key, effect in CardEffectRegistry._effects.items()
            if hasattr(effect, "on_ultimate_unlocked")
        ),
        "aura_effects": sorted(
            key
            for key, effect in CardEffectRegistry._effects.items()
            if any(
                getattr(type(effect), method) is not getattr(CardEffect, method)
                for method in (
                    "get_stat_auras",
                    "get_movement_aura",
                    "get_action_prevention_reason",
                )
            )
        ),
        "action_prevention_effects": sorted(
            key
            for key, effect in CardEffectRegistry._effects.items()
            if type(effect).get_action_prevention_reason
            is not CardEffect.get_action_prevention_reason
        ),
        "registered_effects": sorted(CardEffectRegistry._effects),
        "passive_continuations": passive_continuations(),
        "journey_finishing": journey_finishing(),
        "turret_default": Turret(id="trinkets_turret", name="Turret").model_dump(mode="json"),
        "token_batch_filters": {
            "obstacle": ObstacleFilter().model_dump(mode="json"),
            "hex": HexBatchCompletableFilter(
                slot_index=0, slot_keys=[], slot_filters=[]
            ).model_dump(mode="json"),
            "removal": TokenRemovalCompletableFilter(
                token_type=TokenType.TREE, slot_keys=[], slot_filters=[]
            ).model_dump(mode="json"),
        },
        "token_types": [kind.value for kind in TokenType],
        "passive_offer_predicates": {
            key: type(effect).should_offer_passive.__qualname__
            for key, effect in CardEffectRegistry._effects.items()
            if type(effect).should_offer_passive is not CardEffect.should_offer_passive
        },
        "passive_configs": {
            key: [
                {
                    "trigger": c.trigger.value,
                    "uses": c.uses_per_turn,
                    "optional": c.is_optional,
                    "prompt": c.prompt,
                }
                for c in effect.get_passive_configs()
            ]
            for key, effect in CardEffectRegistry._effects.items()
        },
        "templates": {key: value.model_dump(mode="json") for key, value in values.items()},
    }
    write_text(
        ROOT / "rust/content/catalogs/instruction-templates.json",
        json.dumps(payload, indent=2) + "\n",
    )
    print("Exported frozen continuation templates")


if __name__ == "__main__":
    export()
