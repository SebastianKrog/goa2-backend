//! Shared activity/scope predicates for pre-action consumers.
use crate::{data::Symbol, effect_rules::Kind, effects::EffectInstance, world::NativeWorld};
impl NativeWorld {
    pub(crate) fn pre_action_movement_range(&self, hero: Symbol) -> Option<i64> {
        self.effects
            .instances
            .iter()
            .find(|effect| {
                effect.definition.behavior.kind == Kind::PreMovement
                    && effect.source == hero
                    && effect.active
                    && self.effect_active(effect)
            })
            .map(|effect| {
                effect
                    .definition
                    .behavior
                    .maximum
                    .filter(|value| *value != 0)
                    .unwrap_or(1)
            })
    }
    pub(crate) fn pre_action_discard_applies(
        &self,
        effect: &EffectInstance,
        hero: Symbol,
    ) -> Result<bool, String> {
        let actor = self.resolved_actor(hero);
        Ok(effect.definition.behavior.kind == Kind::PreDiscard
            && self.effect_active(effect)
            && self.effect_in_scope(effect, actor, false)?
            && !self.effect_ignored(effect, actor)?)
    }
}
