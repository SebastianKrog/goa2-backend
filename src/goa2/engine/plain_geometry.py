"""Conservative effect dependencies of geometric distance and plain traversal.

This describes the existing path algorithms, not permission to perform an
actual movement action. Action permission must still be checked by callers.
Unknown effects and model subclasses always keep the general engine path.
"""

from goa2.domain.models.effect import ActiveEffect, EffectType

# Explicit so a newly added effect does not silently enter a shortcut.
_INERT_FOR_PATHS = frozenset(
    {
        EffectType.PLACEMENT_PREVENTION,
        EffectType.MOVEMENT_ZONE,
        EffectType.TARGET_PREVENTION,
        EffectType.LOS_BLOCKER,
        EffectType.AREA_STAT_MODIFIER,
        EffectType.BASIC_ACTION_STAT_BONUS,
        EffectType.ATTACK_IMMUNITY,
        EffectType.DELAYED_TRIGGER,
        EffectType.ILLUSION_MINION_EQUIVALENCE,
        EffectType.AFTER_CARDS_PLAYED_TRIGGER,
        EffectType.MINION_DEFEAT_BOUNTY,
        EffectType.MINION_PROTECTION,
        EffectType.MINION_BATTLE_EXCLUSION,
        EffectType.REPEAT_PREVENTION,
        EffectType.REVERSED_INITIATIVE,
        EffectType.IMMUNITY_ENEMY_ACTIONS,
        EffectType.ENRAGED,
        EffectType.DOUBLE_ITEMS,
        EffectType.PRE_ACTION_MOVEMENT,
        EffectType.MOVEMENT_AURA_ZONE,
        EffectType.PRE_ACTION_DISCARD,
        EffectType.DISCARD_SHIELD,
        EffectType.CONTROL_NEXT_ACTION,
        EffectType.EQUILIBRIUM,
    }
)
_INERT_FOR_DISTANCE = _INERT_FOR_PATHS | {
    EffectType.STATIC_BARRIER,
    EffectType.EMPTY_HEX_OBSTACLE,
    EffectType.PETRIFY,
}


def plain_path_effects(state):
    return all(
        type(effect) is ActiveEffect and effect.effect_type in _INERT_FOR_PATHS
        for effect in state.active_effects
    )


def plain_distance_effects(state):
    return all(
        type(effect) is ActiveEffect and effect.effect_type in _INERT_FOR_DISTANCE
        for effect in state.active_effects
    )
