//! SetActor reads live operands, including save-before-read key aliasing.
use super::phases::Field;
use super::*;

pub(super) struct ActorPlan {
    save: Symbol,
    previous: Datum,
    actor: Option<Symbol>,
}
impl NativeSession {
    pub(super) fn preflight_actor_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<ActorPlan> {
        let save = self.passive_text_bound(objects, step, Field::SaveKey)?;
        let key = self
            .passive_optional_text_bound(objects, step, Field::ActorKey)?
            .filter(|key| !self.symbols.name(*key).is_empty());
        let previous = self
            .planning
            .actor
            .filter(|actor| !self.symbols.name(*actor).is_empty())
            .map_or(Datum::Null, Datum::Text);
        let value = if let Some(key) = key {
            if key == save {
                Some(ObjectValue::Atom(previous.clone()))
            } else {
                objects.arena.context_value(key)
            }
        } else {
            objects
                .arena
                .field(step, objects.scheduler.as_ref().unwrap().actor_id)
        };
        let actor = match value.filter(|value| objects.arena.truthy(value)) {
            None => None,
            Some(ObjectValue::Atom(Datum::Text(id))) => Some(id),
            Some(ObjectValue::Atom(value)) => {
                let text = value
                    .python_scalar_text(&self.symbols)
                    .ok_or_else(|| unsupported("actor scalar coercion not bound"))?;
                Some(Arc::make_mut(&mut self.symbols).intern(&text))
            }
            _ => return Err(unsupported("actor aggregate coercion not bound")),
        };
        Ok(ActorPlan {
            save,
            previous,
            actor,
        })
    }
    pub(super) fn actor_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: ActorPlan,
    ) -> Result<()> {
        objects.arena.symbols = self.symbols.clone();
        objects.write(plan.save, plan.previous)?;
        if let Some(actor) = plan.actor {
            objects
                .arena
                .set_root(objects.actor, ObjectValue::Atom(Datum::Text(actor)))
                .map_err(unsupported)?;
            self.planning.actor = Some(actor);
        }
        Ok(())
    }
}
