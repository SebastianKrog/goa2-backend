//! Source-ordered rock-ultimate discovery and optional discard continuation.
use super::passive_bodies::Kind;
use super::phases::Field;
use super::*;
pub(super) struct RockUltimatePlan {
    used: Symbol,
    affected: Vec<Symbol>,
}
impl NativeSession {
    pub(super) fn preflight_rock_ultimate_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<RockUltimatePlan>> {
        let Some(actor) = self.planning.actor.and_then(|id| self.world.hero(id)) else {
            return Ok(None);
        };
        let actor_id = actor.id;
        let model = self
            .hero_model_bound(objects, actor_id)
            .ok_or_else(|| unsupported("rock ultimate actor model missing"))?;
        let level = self
            .spatial_int_bound(objects, model, Field::Level)?
            .ok_or_else(|| unsupported("rock ultimate level missing"))?;
        if level < 8
            || !objects
                .arena
                .field(model, objects.phases.k(Field::Ultimate))
                .is_some_and(|v| objects.arena.truthy(&v))
        {
            return Ok(None);
        }
        let used = self.passive_text_bound(objects, step, Field::UsedFlagKey)?;
        if objects
            .arena
            .context_value(used)
            .is_some_and(|v| objects.arena.truthy(&v))
        {
            return Ok(None);
        }
        let mut hexes = Vec::new();
        for key in self.unit_filter_texts_bound(objects, step, Field::RockKeys)? {
            if let Some(value) = objects
                .arena
                .context_value(key)
                .filter(|v| objects.arena.truthy(v))
            {
                hexes.push(self.spatial_hex_bound(objects, &value)?.0);
            }
        }
        if let Some(key) = self.spatial_text_bound(objects, step, Field::RockListKey)?
            && let Some(value) = objects
                .arena
                .context_value(key)
                .filter(|v| objects.arena.truthy(v))
        {
            let ObjectValue::Object(list) = value else {
                return Err(unsupported("rock ultimate hex list missing"));
            };
            for value in objects
                .arena
                .sequence(list)
                .ok_or_else(|| unsupported("rock ultimate hexes require a sequence"))?
            {
                hexes.push(self.spatial_hex_bound(objects, value)?.0);
            }
        }
        if hexes.is_empty() {
            return Ok(None);
        }
        let actor_team = self.world.entity(actor_id).and_then(|row| row.team);
        let mut affected = Vec::new();
        for enemy in &self.world.heroes {
            if Some(enemy.team) == actor_team {
                continue;
            }
            for id in self.world.piece_ids(enemy.id) {
                let Some(point) = self.world.resolved_position(id) else {
                    continue;
                };
                let mut adjacent = false;
                for hex in &hexes {
                    if point.distance(*hex) == 1
                        && self
                            .world
                            .effect_connected(point, *hex, &[])
                            .map_err(unsupported)?
                    {
                        adjacent = true;
                        break;
                    }
                }
                if adjacent
                    && self.world.entity(id).is_some()
                    && !self.world.actor_immune(id, actor_id).map_err(unsupported)?
                {
                    affected.push(id);
                }
            }
        }
        if affected.is_empty() {
            return Ok(None);
        }
        self.phase_generation_check(0, 5)?;
        Ok(Some(RockUltimatePlan { used, affected }))
    }
    pub(super) fn rock_ultimate_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<RockUltimatePlan>,
    ) -> Result<()> {
        let Some(plan) = plan else { return Ok(()) };
        let affected = objects.arena.list(
            plan.affected
                .into_iter()
                .map(|id| ObjectValue::Atom(Datum::Text(id)))
                .collect(),
        );
        objects
            .arena
            .set_context(objects.phases.k(Field::RockAffected), affected)
            .map_err(unsupported)?;
        let program = objects.scheduler.as_ref().unwrap().clone();
        let recipes = program.passive_bodies.recipe(Kind::RockUltimate)?;
        let mut steps = Vec::new();
        for recipe in recipes {
            steps.push(self.allocate_source_recipe_bound(objects, recipe, &Datum::Null)?);
        }
        objects
            .arena
            .set_field(
                steps[2],
                objects.phases.k(Field::FlagKey),
                ObjectValue::Atom(Datum::Text(plan.used)),
            )
            .map_err(unsupported)?;
        self.push_live_steps_bound(objects, &steps)
    }
}
