"""Source round cleanup with retained lists, cards, effects and item snapshots."""

from __future__ import annotations


def scenes(names=None):
    from model_traces import setup
    from round_effect_traces import states

    from goa2.domain.models import GamePhase, StatType
    from goa2.engine.steps import EndPhaseCleanupStep

    def inputs():
        yield from states()
        for gold in (-2, 0, 1, 3, 6, 28, 100):
            state = setup()
            state.phase, state.turn = GamePhase.CLEANUP, 4
            state.get_hero("hero_arien").gold = gold
            state.get_hero("hero_arien").items = {StatType.ATTACK: 2, StatType.RADIUS: -1}
            if gold == 3:
                from combat_effect_traces import effect

                from goa2.domain.models.effect import EffectType

                state.active_effects = [
                    effect(state, "persistent-level-up", EffectType.MOVEMENT_ZONE)
                ]
            yield f"level-gold-{gold}", state
        for variant in ("physical-flags", "hand-flags", "markers", "post-round-commit"):
            state = setup()
            state.phase, state.turn = GamePhase.CLEANUP, 4
            hero = state.get_hero("hero_arien")
            if variant in ("physical-flags", "post-round-commit"):
                hero.play_card(hero.hand[0])
                hero.resolve_current_card(turn_number=1)
                hero.discard_card(hero.hand[0])
                hero.play_card(hero.hand[0])
                for card in [*hero.played_cards, *hero.discard_pile, hero.current_turn_card]:
                    card.is_facedown = True
                    card.played_this_round = True
                    card.initiative_override = 17
            elif variant == "hand-flags":
                hero.hand[0].is_facedown = True
                hero.hand[0].played_this_round = True
                hero.hand[0].initiative_override = 17
            else:
                for marker in state.markers.values():
                    marker.target_id = "hero_arien"
                    marker.source_id = "hero_hanu"
                    marker.value = 2
            yield variant, state
        for name in ("Takahide", "Ursafar"):
            for variant in ("ultimate", "ultimate-with-upgrades", "max-level"):
                state = setup([name, "Arien"], ["Hanu", "Brogan"])
                state.phase, state.turn = GamePhase.CLEANUP, 4
                hero = state.teams["RED"].heroes[0]
                hero.level = (
                    8 if variant == "max-level" else 6 if variant == "ultimate-with-upgrades" else 7
                )
                hero.gold = (
                    0
                    if variant == "max-level"
                    else 13 if variant == "ultimate-with-upgrades" else 7
                )
                yield f"{name}-{variant}", state

    for label, state in inputs():
        name = "cleanup-live-" + label
        if names is not None and name not in names:
            continue
        step = EndPhaseCleanupStep(step_id="fixture/cleanup/step")
        state.execution_stack = [step]
        context = state.execution_context
        context.update(
            cleanup_step_alias=step,
            effects_alias=state.active_effects,
            markers_alias=state.markers,
            locations_alias=state.entity_locations,
            snapshot_alias=state.last_turn_positions,
            pending_upgrades_alias=state.pending_upgrades,
            reveals_alias=state.upgrade_reveal_snapshot,
        )
        for team in state.teams.values():
            for hero in team.heroes:
                context[str(hero.id) + "-lists"] = [
                    hero.hand,
                    hero.played_cards,
                    hero.discard_pile,
                    hero.deck,
                ]
                context[str(hero.id) + "-items"] = hero.items
        operations = [{"kind": "advance_prefix", "steps": 1}]
        if label == "post-round-commit":
            from state_codec import export_state_with_instances

            hero = state.get_hero("hero_arien")
            card = hero.hand[-1]
            physical = export_state_with_instances(state)[1]
            operations.extend(
                [
                    {
                        "kind": "commit",
                        "hero": str(hero.id),
                        "card": next(i for i, c in enumerate(physical) if c is card),
                    },
                    {"kind": "uncommit", "hero": str(hero.id)},
                ]
            )
        operations.append({"kind": "advance_prefix", "steps": 0})
        yield name, state, operations
