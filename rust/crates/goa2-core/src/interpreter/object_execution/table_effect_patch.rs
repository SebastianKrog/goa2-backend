//! Restore explicit effect models without invoking gameplay effect creation.
use super::{
    Arc, BoundObjects, Datum, EngineError, NativeSession, ObjectKind, ObjectPathPart, ObjectValue,
    Result, Value, json, passive_bodies::Recipe, unsupported,
};
use crate::{
    codec::ObjectBinding, effect_steps::table_effect_wire, effects::EffectInstance,
    objects::ObjectSlot,
};

const MAX_EFFECT_BYTES: usize = 64 * 1024;
const MAX_FINISHING_STEPS: u64 = 64;

fn invalid_effect() -> EngineError {
    EngineError::InvalidInput("Invalid active effect payload".into())
}

fn invalid_finishing() -> EngineError {
    EngineError::InvalidInput("Unsupported effect finishing steps".into())
}

// The wire carries serialized GameSteps, not an arena object graph. Rebuild
// the typed paths for nested step lists so the normal program binder can
// allocate them and checkpoint/replay can restore them.
fn describe_finishing_step(
    step: &mut Value,
    path: &mut Vec<ObjectPathPart>,
    bindings: &mut Vec<ObjectBinding>,
    count: &mut u64,
) -> Result<()> {
    if *count >= MAX_FINISHING_STEPS || !BoundObjects::supported(step) {
        return Err(invalid_finishing());
    }
    *count += 1;
    bindings.push(ObjectBinding {
        paths: vec![path.clone()],
        kind: ObjectKind::Step,
        deepcopy_shared: false,
    });
    let fields = step.as_object_mut().ok_or_else(invalid_finishing)?;
    fields.insert("step_id".into(), json!("table-effect-template"));
    fields.insert("pending_input".into(), Value::Null);
    fields.insert("pending_request_id".into(), Value::Null);
    fields.entry("is_mandatory").or_insert(Value::Bool(true));
    fields.entry("active_if_key").or_insert(Value::Null);
    fields.entry("skip_if_key").or_insert(Value::Null);
    if fields.get("scope").is_some_and(Value::is_object) {
        path.push(ObjectPathPart::Key("scope".into()));
        bindings.push(ObjectBinding {
            paths: vec![path.clone()],
            kind: ObjectKind::Model,
            deepcopy_shared: false,
        });
        path.pop();
    }
    for field in ["steps_template", "finishing_steps", "new_steps"] {
        let Some(children) = fields.get_mut(field) else {
            continue;
        };
        let children = children.as_array_mut().ok_or_else(invalid_finishing)?;
        path.push(ObjectPathPart::Key(field.into()));
        for (index, child) in children.iter_mut().enumerate() {
            path.push(ObjectPathPart::Index(index));
            describe_finishing_step(child, path, bindings, count)?;
            path.pop();
        }
        path.pop();
    }
    Ok(())
}

impl NativeSession {
    pub(in crate::interpreter) fn validate_effect_patch(&self, raw: &Value) -> Result<()> {
        if serde_json::to_vec(raw).map_err(|_| invalid_effect())?.len() > MAX_EFFECT_BYTES {
            return Err(invalid_effect());
        }
        table_effect_wire(raw).map_err(|_| invalid_effect())?;
        Ok(())
    }

    pub(in crate::interpreter) fn patch_add_effect(&mut self, raw: &Value) -> Result<()> {
        let mut wire = table_effect_wire(raw).map_err(|_| invalid_effect())?;
        let id = wire["id"].as_str().ok_or_else(invalid_effect)?;
        if self
            .symbols
            .lookup(id)
            .is_some_and(|id| self.world.effects.instances.iter().any(|e| e.id == id))
        {
            return Err(EngineError::InvalidInput(
                "An effect with that ID is already active".into(),
            ));
        }
        if id
            .strip_prefix("eff_e_")
            .and_then(|number| number.parse::<i64>().ok())
            .is_some_and(|number| number >= self.world.next_entity_id)
        {
            return Err(EngineError::InvalidInput(
                "Effect ID is reserved for a future generated effect".into(),
            ));
        }
        self.with_live_objects(|session, objects| {
            let mut count = 0;
            let mut recipes = Vec::new();
            let mut finishing_wire = std::mem::replace(&mut wire["finishing_steps"], json!([]));
            for step in finishing_wire
                .as_array_mut()
                .ok_or_else(invalid_finishing)?
            {
                let mut bindings = Vec::new();
                describe_finishing_step(step, &mut Vec::new(), &mut bindings, &mut count)?;
                recipes.push(
                    Recipe::compile(
                        &json!({"fields": step, "objects": bindings}),
                        Arc::make_mut(&mut session.steps),
                        Arc::make_mut(&mut session.symbols),
                    )
                    .map_err(|_| invalid_finishing())?,
                );
            }
            session.phase_generation_check(0, count)?;
            let symbols = Arc::make_mut(&mut session.symbols);
            let payload = Datum::from_json(&wire, symbols).map_err(|_| invalid_effect())?;
            let mut effect =
                EffectInstance::from_datum(&payload, symbols).map_err(|_| invalid_effect())?;
            objects.arena.symbols = session.symbols.clone();

            let mut finishing = Vec::new();
            // The payload is literal, so it needs no performer substitution.
            let owner = objects
                .scheduler
                .as_ref()
                .ok_or_else(|| unsupported("effect scheduler missing"))?
                .passive_bodies
                .template_owner();
            for recipe in &recipes {
                let step =
                    session.allocate_source_recipe_bound(objects, recipe, &Datum::Text(owner))?;
                finishing.push(ObjectValue::Object(step));
            }
            let finishing = objects.arena.list(finishing);
            let symbols = Arc::make_mut(&mut session.symbols);
            let Datum::List(steps) = objects
                .arena
                .typed_value(&finishing, symbols)
                .map_err(unsupported)?
            else {
                unreachable!();
            };
            let definition = Arc::make_mut(&mut effect.definition);
            definition.finishing = steps.clone();
            let fields = definition
                .fields
                .iter_mut()
                .find(|(key, _)| *key == objects.finishing)
                .ok_or_else(invalid_finishing)?;
            fields.1 = Datum::List(steps);
            objects.arena.symbols = session.symbols.clone();
            let ObjectValue::Object(model) = objects.arena.allocate(&payload) else {
                return Err(invalid_effect());
            };
            let lookup = |name| session.symbols.lookup(name).ok_or_else(invalid_effect);
            let scope_key = lookup("scope")?;
            objects
                .arena
                .classify_recipe_object(model, &[], ObjectKind::Model)
                .map_err(unsupported)?;
            objects
                .arena
                .classify_recipe_object(model, &[ObjectSlot::Field(scope_key)], ObjectKind::Model)
                .map_err(unsupported)?;
            objects
                .arena
                .set_field(model, objects.finishing, finishing)
                .map_err(unsupported)?;
            let Some(ObjectValue::Object(list)) = objects.arena.root_field(objects.effects) else {
                return Err(unsupported("effect collection missing"));
            };
            let before = session
                .world
                .cards()
                .iter()
                .map(|card| card.active)
                .collect::<Vec<_>>();
            objects
                .arena
                .append(list, ObjectValue::Object(model))
                .map_err(unsupported)?;
            effect.arena_model = Some(model);
            session.world.restore_effect_instance(effect);
            session.sync_effect_card_flags(objects, &before)?;
            objects.readonly.take();
            Ok(())
        })
    }

    pub(in crate::interpreter) fn patch_remove_effect(&mut self, id: &str) -> Result<()> {
        let id = self
            .symbols
            .lookup(id)
            .filter(|id| self.world.effects.instances.iter().any(|e| e.id == *id))
            .ok_or_else(|| EngineError::InvalidInput("No active effect matches that ID".into()))?;
        self.with_live_objects(|session, objects| {
            let before = session
                .world
                .cards()
                .iter()
                .map(|card| card.active)
                .collect::<Vec<_>>();
            objects.remove_effect_nodes(&[id])?;
            session.world.remove_effects(&[id]);
            session.sync_effect_card_flags(objects, &before)
        })
    }
}
