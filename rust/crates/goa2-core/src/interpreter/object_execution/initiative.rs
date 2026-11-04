//! Hurry Up! mutates the unresolved physical face, preserving all card aliases.
use super::phases::Field;
use super::*;
use crate::{cards::CardId, world::CardLifecycle};

pub(super) struct InitiativePlan {
    target: Symbol,
    card: CardId,
    physical: u32,
    printed: Symbol,
    value: i64,
}
impl NativeSession {
    pub(super) fn preflight_initiative_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<InitiativePlan>> {
        let key = self.passive_text_bound(objects, step, Field::HeroKey)?;
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(None);
        };
        let target = self.resource_identity_bound(objects, value)?;
        self.sync_card_collections_bound(objects)?;
        let Some(card) = self.world.hero(target).and_then(|hero| hero.current) else {
            return Ok(None);
        };
        let instance = self
            .world
            .card(card)
            .ok_or_else(|| unsupported("initiative physical card missing"))?;
        if instance.lifecycle != CardLifecycle::Unresolved {
            return Ok(None);
        }
        let value = match objects.arena.field(step, objects.value) {
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            _ => return Err(unsupported("initiative override value must be an integer")),
        };
        Ok(Some(InitiativePlan {
            target,
            card,
            physical: instance.source_instance,
            printed: self.world.definitions.cards[instance.definition].printed_id,
            value,
        }))
    }
    pub(super) fn initiative_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<InitiativePlan>,
    ) -> Result<Option<NativeEvent>> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        objects
            .arena
            .set_physical_field(
                plan.physical,
                objects.phases.k(Field::InitiativeOverride),
                Datum::Int(plan.value),
            )
            .map_err(unsupported)?;
        self.world.card_mut(plan.card).unwrap().initiative_override = Some(plan.value);
        Ok(Some(NativeEvent::InitiativeOverride {
            actor: self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()),
            target: plan.target,
            card: plan.printed,
            value: plan.value,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(suffix: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-initiative-{suffix}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    fn snapshot(session: &NativeSession) -> Value {
        serde_json::to_value(session.export_envelope().unwrap()).unwrap()
    }
    #[test]
    fn initiative_override_needs_no_generation_capacity_and_preserves_root_aliases() {
        let root = session("shared-current");
        let before = snapshot(&root);
        let target = root.symbols.lookup("hero_arien").unwrap();
        let other = root.symbols.lookup("hero_brogan").unwrap();
        let card = root.world.hero(target).unwrap().current.unwrap();
        assert_eq!(root.world.hero(other).unwrap().current, Some(card));
        let mut worker = root.clone();
        worker.next_step = u64::MAX - 1;
        assert_eq!(worker.advance_prefix(1).unwrap().events.len(), 1);
        assert_eq!(
            worker.world.card(card).unwrap().initiative_override,
            Some(11)
        );
        assert_eq!(
            worker.semantic_state().unwrap()["execution_context"]["current-alias"]["initiative_override"],
            json!(11)
        );
        assert_eq!(snapshot(&root), before);
    }
    #[test]
    fn initiative_override_reads_live_value_and_target_without_mutating_other_workers() {
        let root = session("no-override");
        let before = snapshot(&root);
        let target = root.symbols.lookup("hero_arien").unwrap();
        let card = root.world.hero(target).unwrap().current.unwrap();
        let mut changed = root.clone();
        let mut skipped = root.clone();
        let step = changed.stack.last().unwrap().object.unwrap();
        let objects = changed.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(step, objects.value, ObjectValue::Atom(Datum::Int(-4)))
            .unwrap();
        let who = skipped.symbols.lookup("who").unwrap();
        let actor = skipped.symbols.lookup("hero_hanu").unwrap();
        skipped
            .objects
            .as_mut()
            .unwrap()
            .arena
            .set_context(who, ObjectValue::Atom(Datum::Text(actor)))
            .unwrap();
        assert_eq!(changed.advance_prefix(1).unwrap().events.len(), 1);
        assert!(skipped.advance_prefix(1).unwrap().events.is_empty());
        assert_eq!(
            changed.world.card(card).unwrap().initiative_override,
            Some(-4)
        );
        assert_eq!(skipped.world.card(card).unwrap().initiative_override, None);
        assert_eq!(snapshot(&root), before);
    }
    #[test]
    fn malformed_live_override_guards_before_consumption_or_card_mutation() {
        let mut worker = session("no-override");
        let step = worker.stack.last().unwrap().object.unwrap();
        let text = worker.symbols.lookup("who").unwrap();
        let objects = worker.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(step, objects.value, ObjectValue::Atom(Datum::Text(text)))
            .unwrap();
        let before = snapshot(&worker);
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("integer")
        );
        assert_eq!(snapshot(&worker), before);
    }
}
