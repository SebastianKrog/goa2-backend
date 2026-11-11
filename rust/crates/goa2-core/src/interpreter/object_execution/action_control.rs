//! Hanu's ultimate schedules control over the target's current physical face.
use super::effect_creation::CreatePlan;
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, NativeEvent, NativeSession, ObjectId, Result, Symbol, unsupported,
};
use crate::world::CardLifecycle;

pub(super) struct ActionControlPlan {
    creation: CreatePlan,
    actor: Symbol,
    target: Symbol,
    card: Symbol,
}
impl NativeSession {
    pub(super) fn preflight_action_control_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<ActionControlPlan>> {
        let Some(actor) = self.planning.actor else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(actor) else {
            return Ok(None);
        };
        let model = self
            .hero_model_bound(objects, hero.id)
            .ok_or_else(|| unsupported("control actor model missing"))?;
        let level = self
            .spatial_int_bound(objects, model, Field::Level)?
            .ok_or_else(|| unsupported("control actor level missing"))?;
        if level < 8 || hero.ultimate.is_none() {
            return Ok(None);
        }
        let key = self.passive_text_bound(objects, step, Field::HeroKey)?;
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(None);
        };
        let target = self.resource_identity_bound(objects, value)?;
        let Some(card) = self
            .world
            .hero(target)
            .and_then(|hero| hero.current)
            .and_then(|card| self.world.card(card))
            .filter(|card| card.lifecycle == CardLifecycle::Unresolved)
        else {
            return Ok(None);
        };
        let card = self.world.definitions.cards[card.definition].printed_id;
        let kind = Arc::make_mut(&mut self.symbols).intern("control_next_action");
        let duration = Arc::make_mut(&mut self.symbols).intern("THIS_ROUND");
        let point = Arc::make_mut(&mut self.symbols).intern("point");
        let creation = self.preflight_manager_effect_bound(
            objects,
            vec![
                ("source_id", Datum::Text(actor)),
                ("effect_type", Datum::Text(kind)),
                ("duration", Datum::Text(duration)),
                ("is_active", Datum::Bool(true)),
                ("controlled_card_id", Datum::Text(card)),
            ],
            vec![
                ("shape", Datum::Text(point)),
                ("origin_id", Datum::Text(target)),
            ],
        )?;
        Ok(Some(ActionControlPlan {
            creation,
            actor,
            target,
            card,
        }))
    }
    pub(super) fn action_control_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<ActionControlPlan>,
    ) -> Result<Option<NativeEvent>> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        self.create_effect_bound(objects, plan.creation)?;
        Ok(Some(NativeEvent::ActionControlScheduled {
            actor: plan.actor,
            target: plan.target,
            card: plan.card,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ObjectValue, Value, fixture_case, json};
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-action-control-{name}"));
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
    fn control_entity_counter_guard_precedes_effect_and_stack_mutation() {
        let mut worker = session("plain");
        worker.world.next_entity_id = i64::MAX;
        let before = snapshot(&worker);
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("entity counter")
        );
        assert_eq!(snapshot(&worker), before);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn control_reads_live_level_and_ultimate_without_needing_step_capacity() {
        let root = session("level-7");
        let before = snapshot(&root);
        let actor = root.symbols.lookup("hero_hanu").unwrap();
        let target = root.symbols.lookup("hero_arien").unwrap();
        let card = root.world.hero(target).unwrap().current.unwrap();
        let active = root.world.card(card).unwrap().active;
        let mut created = root.clone();
        let mut missing_ultimate = root.clone();
        for worker in [&mut created, &mut missing_ultimate] {
            let objects = worker.objects.as_mut().unwrap();
            let model = objects
                .phases
                .heroes
                .iter()
                .find(|(id, _)| *id == actor)
                .unwrap()
                .1;
            objects
                .arena
                .set_field(
                    model,
                    objects.phases.k(Field::Level),
                    ObjectValue::Atom(Datum::Int(8)),
                )
                .unwrap();
        }
        let objects = missing_ultimate.objects.as_mut().unwrap();
        let model = objects
            .phases
            .heroes
            .iter()
            .find(|(id, _)| *id == actor)
            .unwrap()
            .1;
        objects
            .arena
            .set_field(
                model,
                objects.phases.k(Field::Ultimate),
                ObjectValue::Atom(Datum::Null),
            )
            .unwrap();
        created.next_step = u64::MAX - 1;
        assert_eq!(created.advance_prefix(1).unwrap().events.len(), 1);
        assert!(
            missing_ultimate
                .advance_prefix(1)
                .unwrap()
                .events
                .is_empty()
        );
        assert_eq!(created.next_step, u64::MAX - 1);
        assert_eq!(created.world.effects.instances.len(), 1);
        assert!(created.world.effects.instances[0].card.is_none());
        assert!(created.world.effects.instances[0].origin_action.is_none());
        assert_eq!(created.world.card(card).unwrap().active, active);
        assert_eq!(snapshot(&root), before);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn manager_scopes_and_retained_effect_lists_are_private_between_workers() {
        let root = session("plain");
        let before = snapshot(&root);
        let mut a = root.clone();
        let mut b = root.clone();
        a.advance_prefix(1).unwrap();
        b.advance_prefix(1).unwrap();
        let model = a.world.effects.instances[0].arena_model.unwrap();
        let scope_key = a.symbols.lookup("scope").unwrap();
        let origin_key = a.symbols.lookup("origin_id").unwrap();
        let other = a.symbols.lookup("hero_brogan").unwrap();
        let objects = a.objects.as_mut().unwrap();
        let Some(ObjectValue::Object(scope)) = objects.arena.field(model, scope_key) else {
            unreachable!()
        };
        objects
            .arena
            .set_field(scope, origin_key, ObjectValue::Atom(Datum::Text(other)))
            .unwrap();
        let left = a.semantic_state().unwrap();
        let right = b.semantic_state().unwrap();
        assert_eq!(
            left["active_effects"][0]["scope"]["origin_id"],
            json!("hero_brogan")
        );
        assert_eq!(
            left["execution_context"]["effects-alias"][0]["scope"]["origin_id"],
            json!("hero_brogan")
        );
        assert_eq!(
            right["active_effects"][0]["scope"]["origin_id"],
            json!("hero_arien")
        );
        assert_eq!(snapshot(&root), before);
    }
}
