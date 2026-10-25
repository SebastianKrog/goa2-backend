"""Freeze Python's wire defaults for native-generated continuation frames."""

import json
import sys

from oracle import ROOT, verify_sources

sys.path.insert(0, str(ROOT / "src"))
from goa2.domain.models import TargetType
from goa2.engine.filters_geometry import InStraightLineFilter, StraightLinePathFilter
from goa2.engine.filters_hex import (
    FastTravelDestinationFilter,
    MovementPathFilter,
    ObstacleFilter,
    RangeFilter,
)
from goa2.engine.steps import (
    AdvanceTurnStep,
    AttackSequenceStep,
    CheckLanePushStep,
    CheckMinionProtectionStep,
    CheckPassiveAbilitiesStep,
    ChooseActingPieceStep,
    ChooseMinionRemovalStep,
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
    MoveUnitStep,
    OfferPassiveStep,
    PlaceMarkerStep,
    PlaceUnitStep,
    ReactionWindowStep,
    RemoveMarkerStep,
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
    SelectStep,
    SetActorStep,
    SetContextFlagStep,
    TriggerGameOverStep,
    TriggerMineStep,
    ValidateRepeatStep,
)


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
    (ROOT / "rust/tests/fixtures/instruction-templates.json").write_text(
        json.dumps(payload, indent=2) + "\n"
    )
    print("Exported frozen continuation templates")


if __name__ == "__main__":
    export()
