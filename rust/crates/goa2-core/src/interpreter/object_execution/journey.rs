//! Journey immunity and delayed literal programs share manager registration.
use super::effect_creation::CreatePlan;
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::*;

pub(super) fn compile_finishing(
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<Vec<Recipe>> {
    let data = crate::templates::instructions().map_err(unsupported)?;
    let rows = data["journey_finishing"]
        .as_array()
        .ok_or_else(|| unsupported("journey finishing catalog missing"))?;
    if rows.len() != 3 {
        return Err(unsupported("journey finishing catalog shape changed"));
    }
    rows.iter()
        .map(|row| Recipe::compile(row, steps, symbols))
        .collect()
}
pub(super) struct JourneyPlan {
    immunity: CreatePlan,
    delayed: CreatePlan,
    actor: Symbol,
    enemy: Symbol,
    kind: Symbol,
    move_after: bool,
    before: HashSet<Symbol>,
}
impl NativeSession {
    pub(super) fn preflight_journey_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<JourneyPlan>> {
        let key = self.passive_text_bound(objects, step, Field::EnemyKey)?;
        let enemy = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v));
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty());
        let Some((enemy, actor)) = enemy.zip(actor) else {
            return Ok(None);
        };
        let enemy = self.resource_identity_bound(objects, enemy)?;
        let card = self.passive_optional_text_bound(objects, step, Field::SourceCard)?;
        let move_after = self.spatial_bool_bound(objects, step, Field::MoveAfter)?;
        self.sync_card_collections_bound(objects)?;
        let kind = Arc::make_mut(&mut self.symbols).intern("immunity_enemy_actions");
        let delayed_kind = Arc::make_mut(&mut self.symbols).intern("delayed_trigger");
        let immunity = self.preflight_manager_effect_bound(
            objects,
            vec![
                ("source_id", Datum::Text(actor)),
                ("subject_id", Datum::Text(enemy)),
                ("source_card_id", card.map_or(Datum::Null, Datum::Text)),
                ("effect_type", Datum::Text(kind)),
                ("is_active", Datum::Bool(true)),
                ("blocks_friendly_actors", Datum::Bool(true)),
            ],
            Vec::new(),
        )?;
        let delayed = self.preflight_manager_effect_bound(
            objects,
            vec![
                ("source_id", Datum::Text(actor)),
                ("source_card_id", card.map_or(Datum::Null, Datum::Text)),
                ("effect_type", Datum::Text(delayed_kind)),
                ("is_active", Datum::Bool(true)),
            ],
            Vec::new(),
        )?;
        let count = i64::from(immunity.creates()) + i64::from(delayed.creates());
        self.world
            .next_entity_id
            .checked_add(count)
            .ok_or_else(|| unsupported("journey entity counter exhausted"))?;
        if delayed.creates() {
            let program = objects.scheduler.as_ref().unwrap();
            let recipes = &program.journey_finishing[..if move_after { 3 } else { 1 }];
            let count = recipes
                .iter()
                .flat_map(|recipe| &recipe.objects)
                .filter(|(_, kind)| *kind == ObjectKind::Step)
                .count();
            self.phase_generation_check(
                0,
                u64::try_from(count).map_err(|_| unsupported("journey program count overflow"))?,
            )?;
        }
        Ok(Some(JourneyPlan {
            immunity,
            delayed,
            actor,
            enemy,
            kind,
            move_after,
            before: self
                .world
                .effects
                .instances
                .iter()
                .map(|effect| effect.id)
                .collect(),
        }))
    }
    pub(super) fn journey_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<JourneyPlan>,
    ) -> Result<Option<NativeEvent>> {
        let Some(mut plan) = plan else {
            return Ok(None);
        };
        self.create_effect_bound(objects, plan.immunity)?;
        if plan.delayed.creates() {
            let program = objects.scheduler.as_ref().unwrap().clone();
            let recipes = &program.journey_finishing[..if plan.move_after { 3 } else { 1 }];
            let mut steps = Vec::new();
            for recipe in recipes {
                steps.push(self.allocate_source_recipe_bound(
                    objects,
                    recipe,
                    &Datum::Text(plan.actor),
                )?);
            }
            objects
                .arena
                .set_field(
                    steps[0],
                    objects.phases.k(Field::UnitBId),
                    ObjectValue::Atom(Datum::Text(plan.enemy)),
                )
                .map_err(unsupported)?;
            plan.delayed.bind_manager_finishing(steps)?;
        }
        self.create_effect_bound(objects, plan.delayed)?;
        let changed = self
            .world
            .effects
            .instances
            .iter()
            .any(|effect| !plan.before.contains(&effect.id));
        Ok(changed.then_some(NativeEvent::JourneyScheduled {
            actor: plan.actor,
            target: plan.enemy,
            kind: plan.kind,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-journey-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    fn snapshot(session: &NativeSession) -> Value {
        serde_json::to_value(session.export_envelope().unwrap()).unwrap()
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn journey_aggregate_counter_guard_precedes_both_creations_and_stack_pop() {
        for counter in [i64::MAX, i64::MAX - 1] {
            let mut worker = session("create-plain-move-True");
            worker.world.next_entity_id = counter;
            let before = snapshot(&worker);
            assert!(
                worker
                    .advance_prefix(1)
                    .unwrap_err()
                    .to_string()
                    .contains("counter")
            );
            assert_eq!(snapshot(&worker), before);
        }
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn journey_generated_step_guard_precedes_immunity_creation() {
        let mut worker = session("create-plain-move-True");
        worker.next_step = u64::MAX - 1;
        let before = snapshot(&worker);
        assert!(worker.advance_prefix(1).is_err());
        assert_eq!(snapshot(&worker), before);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn journey_dedup_needs_no_generated_step_or_entity_capacity() {
        let mut worker = session("create-existing-both-move-True");
        worker.next_step = u64::MAX - 1;
        worker.world.next_entity_id = i64::MAX;
        assert!(worker.advance_prefix(1).unwrap().events.is_empty());
        assert_eq!(worker.world.effects.instances.len(), 2);
        assert_eq!(worker.next_step, u64::MAX - 1);
        assert_eq!(worker.world.next_entity_id, i64::MAX);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn journey_bound_finishing_models_are_private_between_search_workers() {
        let root = session("create-plain-move-True");
        let before = snapshot(&root);
        let mut left = root.clone();
        let mut right = root.clone();
        left.advance_prefix(1).unwrap();
        right.advance_prefix(1).unwrap();
        let delayed = left.world.effects.instances[1].arena_model.unwrap();
        let finishing = left.symbols.lookup("finishing_steps").unwrap();
        let other = left.symbols.lookup("hero_arien").unwrap();
        let objects = left.objects.as_mut().unwrap();
        let Some(ObjectValue::Object(list)) = objects.arena.field(delayed, finishing) else {
            unreachable!()
        };
        let Some(ObjectValue::Object(swap)) =
            objects.arena.sequence(list).unwrap().first().cloned()
        else {
            unreachable!()
        };
        objects
            .arena
            .set_field(
                swap,
                objects.phases.k(Field::UnitBId),
                ObjectValue::Atom(Datum::Text(other)),
            )
            .unwrap();
        assert_eq!(
            left.semantic_state().unwrap()["active_effects"][1]["finishing_steps"][0]["unit_b_id"],
            json!("hero_arien")
        );
        assert_eq!(
            right.semantic_state().unwrap()["active_effects"][1]["finishing_steps"][0]["unit_b_id"],
            json!("hero_brogan")
        );
        assert_eq!(snapshot(&root), before);
    }
}
