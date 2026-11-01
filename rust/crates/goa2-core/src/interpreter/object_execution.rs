//! Live shared-object execution and controlled effect/card-lifecycle bindings.
//! Other world-changing instructions stay guarded until their dense slots bind.
use super::*;
mod phases;
mod revelation;
use crate::{
    codec::{ObjectKind, ObjectPathPart},
    objects::{ObjectArena, ObjectId, ObjectValue},
};
use std::collections::{HashMap, HashSet};

/// Natural turn expiry retains the original finishing list identities, without
/// removing effects. Rule-v9 turn scheduling removes these IDs after the text.
#[derive(Clone, Debug)]
pub struct LiveEffectExpiry {
    pub finishing: Vec<(Symbol, ObjectId)>,
    pub ids: Vec<Symbol>,
}
#[derive(Clone, Debug)]
struct StepRecipe {
    instruction: usize,
    fields: Datum,
}
#[derive(Clone, Debug)]
struct FinishingProgram {
    actor: StepRecipe,
    marker: StepRecipe,
    step_id: Symbol,
    actor_id: Symbol,
    phase_recipes: Vec<(&'static str, StepRecipe)>,
}
#[derive(Clone, Debug)]
pub(super) struct BoundObjects {
    pub arena: ObjectArena,
    stack: ObjectId,
    readonly: Arc<HashSet<ObjectId>>,
    pending: Symbol,
    request: Symbol,
    value: Symbol,
    index: Symbol,
    templates: Symbol,
    programs: Arc<HashMap<ObjectId, usize>>,
    copied_programs: HashMap<ObjectId, usize>,
    copied_readonly: HashSet<ObjectId>,
    action_keys: [Symbol; 8],
    action_stack: Symbol,
    present: Symbol,
    type_stack: Symbol,
    actor: Symbol,
    effects: Symbol,
    effect_id: Symbol,
    active: Symbol,
    finishing: Symbol,
    number_options: Symbol,
    scheduler: Option<Arc<FinishingProgram>>,
    phases: phases::PhaseBindings,
}
impl BoundObjects {
    pub fn compile(source: &StateEnvelope) -> Result<Option<Self>> {
        if source.object_bindings.is_empty() {
            return Ok(None);
        }
        if !matches!(
            source.validate().map_err(unsupported)?.phase,
            Phase::Planning | Phase::Resolution | Phase::Cleanup
        ) {
            return Err(unsupported("shared-object phase flow not yet bound"));
        }
        for step in source.state["execution_stack"]
            .as_array()
            .ok_or_else(|| unsupported("stack missing"))?
        {
            if !Self::supported(step) {
                return Err(unsupported(
                    "shared-object instruction family not yet bound",
                ));
            }
        }
        let mut arena = ObjectArena::compile(source).map_err(unsupported)?;
        let pending = Arc::make_mut(&mut arena.symbols).intern("pending_input");
        let request = Arc::make_mut(&mut arena.symbols).intern("pending_request_id");
        let value = Arc::make_mut(&mut arena.symbols).intern("value");
        let index = Arc::make_mut(&mut arena.symbols).intern("current_index");
        let templates = Arc::make_mut(&mut arena.symbols).intern("steps_template");
        let action_keys = [
            "current_action_type",
            "current_card_id",
            "performing_card_id",
            "performing_card_owner_id",
            "reperforming_card_id",
            "token_type_override",
            "skip_markers",
            "substitution_actor_id",
        ]
        .map(|key| Arc::make_mut(&mut arena.symbols).intern(key));
        let action_stack = Arc::make_mut(&mut arena.symbols).intern("action_context_stack");
        let present = Arc::make_mut(&mut arena.symbols).intern("present");
        let type_stack = Arc::make_mut(&mut arena.symbols).intern("action_type_stack");
        let actor = Arc::make_mut(&mut arena.symbols).intern("current_actor_id");
        let effects = Arc::make_mut(&mut arena.symbols).intern("active_effects");
        let effect_id = Arc::make_mut(&mut arena.symbols).intern("id");
        let active = Arc::make_mut(&mut arena.symbols).intern("is_active_base");
        let finishing = Arc::make_mut(&mut arena.symbols).intern("finishing_steps");
        let number_options = Arc::make_mut(&mut arena.symbols).intern("number_options");
        let ObjectValue::Object(stack) = arena
            .value_at_path(&[ObjectPathPart::Key("execution_stack".into())])
            .map_err(unsupported)?
        else {
            return Err(unsupported("stack object missing"));
        };
        if !arena.is_mutable_list(stack) {
            return Err(unsupported("stack is not a list"));
        }
        let readonly = Arc::new(arena.readonly_objects());
        if readonly.contains(&arena.context_id()) {
            return Err(unsupported(
                "execution context aliases an unbound world/step operand",
            ));
        }
        let phases = phases::PhaseBindings::compile(source, &mut arena)?;
        Ok(Some(Self {
            arena,
            stack,
            readonly,
            pending,
            request,
            value,
            index,
            templates,
            programs: Arc::new(HashMap::new()),
            copied_programs: HashMap::new(),
            copied_readonly: HashSet::new(),
            action_keys,
            action_stack,
            present,
            type_stack,
            actor,
            effects,
            effect_id,
            active,
            finishing,
            number_options,
            scheduler: None,
            phases,
        }))
    }
    fn supported(step: &Value) -> bool {
        match step["type"].as_str() {
            Some(
                "set_context_flag"
                | "add_context_value"
                | "record_target"
                | "log_message"
                | "restore_action_context"
                | "restore_action_type"
                | "set_actor"
                | "finished_expiring_effect"
                | "advance_turn"
                | "find_next_actor"
                | "end_phase"
                | "minion_battle"
                | "check_lane_push"
                | "end_phase_cleanup"
                | "retrieve_unresolved_card"
                | "resolve_tie_breaker"
                | "resolve_card"
                | "confirm_resolution"
                | "finalize_hero_turn"
                | "respawn_hero"
                | "flip_tie_breaker_coin",
            ) => true,
            Some("select_step") => step["target_type"] == "NUMBER",
            Some("for_each") => step["steps_template"]
                .as_array()
                .is_some_and(|steps| steps.iter().all(Self::supported)),
            _ => false,
        }
    }
    fn parked_type(step: &Value) -> bool {
        matches!(
            step["type"].as_str(),
            Some("resolve_card" | "confirm_resolution" | "finalize_hero_turn" | "respawn_hero")
        )
    }
    pub fn compile_wire(
        wire: &Value,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
    ) -> Result<usize> {
        fn sanitize(wire: &mut Value) {
            if BoundObjects::parked_type(wire) {
                wire["type"] = json!("finished_expiring_effect");
            }
            if let Some(templates) = wire.get_mut("steps_template").and_then(Value::as_array_mut) {
                for child in templates {
                    sanitize(child);
                }
            }
        }
        fn restore(
            index: usize,
            wire: &Value,
            steps: &mut [CompiledStep],
            symbols: &mut Symbols,
        ) -> Result<()> {
            steps[index].wire = wire.clone();
            if BoundObjects::parked_type(wire) {
                steps[index].instruction =
                    Instruction::ParkedPhase(symbols.intern(wire["type"].as_str().unwrap()));
            }
            if let Instruction::ForEach { templates, .. } = &steps[index].instruction {
                let children = templates.clone();
                for (index, child) in children.into_iter().zip(
                    wire["steps_template"]
                        .as_array()
                        .ok_or_else(|| unsupported("loop templates missing"))?,
                ) {
                    restore(index, child, steps, symbols)?;
                }
            }
            Ok(())
        }
        let mut sanitized = wire.clone();
        sanitize(&mut sanitized);
        let index = compile_step(&sanitized, steps, symbols)?;
        restore(index, wire, steps, symbols)?;
        Ok(index)
    }
    pub fn compile_scheduler(
        &mut self,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
    ) -> Result<()> {
        let mut recipe = |name: &str| -> Result<StepRecipe> {
            let wire = combat_template(name)?;
            let instruction = Self::compile_wire(&wire, steps, symbols)?;
            let fields = Datum::from_json(&wire, symbols).map_err(unsupported)?;
            Ok(StepRecipe {
                instruction,
                fields,
            })
        };
        self.scheduler = Some(Arc::new(FinishingProgram {
            actor: recipe("set_actor")?,
            marker: recipe("finished_expiring_effect")?,
            phase_recipes: [
                "advance_turn",
                "end_phase",
                "find_actor",
                "minion_battle",
                "check_lane_push",
                "end_cleanup",
                "retrieve_unresolved",
                "tie",
                "resolve_card",
                "confirm_resolution",
                "finalize_hero",
                "respawn",
                "flip_coin",
            ]
            .into_iter()
            .map(|name| Ok((name, recipe(name)?)))
            .collect::<Result<Vec<_>>>()?,
            step_id: symbols.intern("step_id"),
            actor_id: symbols.intern("actor_id"),
        }));
        Ok(())
    }
    fn finishing_for_indices(
        &self,
        effects: &crate::effects::EffectStore,
        indices: &[usize],
    ) -> Result<Vec<(Symbol, ObjectId)>> {
        let nodes = self.effect_nodes()?;
        if nodes.len() != effects.instances.len() {
            return Err(unsupported("effect collection binding drift"));
        }
        let mut finishing = Vec::new();
        for &index in indices {
            let effect = &effects.instances[index];
            if !effect.definition.after_cards_played {
                let list = self.finishing_list(nodes[index])?;
                if self
                    .arena
                    .sequence(list)
                    .is_some_and(|steps| !steps.is_empty())
                {
                    finishing.push((effect.source, list));
                }
            }
        }
        Ok(finishing)
    }
    fn remove_effect_rows(&mut self, removed: &[usize]) -> Result<()> {
        let retained = self
            .effect_nodes()?
            .into_iter()
            .enumerate()
            .filter_map(|(index, object)| {
                (!removed.contains(&index)).then_some(ObjectValue::Object(object))
            })
            .collect();
        let list = self.arena.list(retained);
        self.arena
            .set_root(self.effects, list)
            .map_err(unsupported)?;
        self.readonly = Arc::new(self.arena.readonly_objects());
        Ok(())
    }
    fn generated_step(
        &mut self,
        recipe: &StepRecipe,
        program: &FinishingProgram,
        generation: u64,
        actor: Option<Symbol>,
    ) -> Result<ObjectId> {
        let object = self.arena.step(&recipe.fields).map_err(unsupported)?;
        self.arena
            .set_field(
                object,
                program.step_id,
                ObjectValue::GeneratedStep(generation),
            )
            .map_err(unsupported)?;
        if let Some(actor) = actor {
            self.arena
                .set_field(
                    object,
                    program.actor_id,
                    ObjectValue::Atom(Datum::Text(actor)),
                )
                .map_err(unsupported)?;
        }
        self.copied_programs.insert(object, recipe.instruction);
        Ok(object)
    }
    pub fn bind_stored_programs(
        &mut self,
        source: &StateEnvelope,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
    ) -> Result<()> {
        // Removed effect objects can remain visible through context aliases.
        // Rebind every reachable stored step, not just current effect rows.
        for binding in source
            .object_bindings
            .iter()
            .filter(|binding| binding.kind == ObjectKind::Step)
        {
            let path = binding
                .paths
                .first()
                .ok_or_else(|| unsupported("stored step path missing"))?;
            let wire = crate::codec::object_at_path(&source.state, path).map_err(unsupported)?;
            let ObjectValue::Object(object) =
                self.arena.value_at_path(path).map_err(unsupported)?
            else {
                return Err(unsupported("stored step handle missing"));
            };
            if !Self::supported(wire) || self.program(object).is_some() {
                continue;
            }
            let before = steps.len();
            match Self::compile_wire(wire, steps, symbols) {
                Ok(instruction) => self.bind_program(object, instruction, steps)?,
                Err(_) => steps.truncate(before), // Unused unsupported payloads remain data.
            }
        }
        Ok(())
    }
    fn effect_nodes(&self) -> Result<Vec<ObjectId>> {
        let Some(ObjectValue::Object(list)) = self.arena.root_field(self.effects) else {
            return Err(unsupported("effect collection missing"));
        };
        self.arena
            .sequence(list)
            .ok_or_else(|| unsupported("effect collection must be a list"))?
            .iter()
            .map(|value| {
                if let ObjectValue::Object(id) = value {
                    Ok(*id)
                } else {
                    Err(unsupported("effect model handle missing"))
                }
            })
            .collect()
    }
    fn finishing_list(&self, effect: ObjectId) -> Result<ObjectId> {
        let Some(ObjectValue::Object(list)) = self.arena.field(effect, self.finishing) else {
            return Err(unsupported("finishing list missing"));
        };
        if self.arena.sequence(list).is_none() {
            return Err(unsupported("finishing payload must be a list"));
        }
        Ok(list)
    }
    fn steps_in_list(&self, list: ObjectId) -> Result<Vec<ObjectId>> {
        self.arena
            .sequence(list)
            .ok_or_else(|| unsupported("step sequence missing"))?
            .iter()
            .map(|value| {
                if let ObjectValue::Object(id) = value
                    && self.program(*id).is_some()
                {
                    Ok(*id)
                } else {
                    Err(unsupported("stored step program is not compiled"))
                }
            })
            .collect()
    }
    fn remove_effect_nodes(&mut self, ids: &[Symbol]) -> Result<()> {
        let nodes = self.effect_nodes()?;
        let mut retained = Vec::new();
        for object in nodes {
            let Some(ObjectValue::Atom(Datum::Text(id))) = self.arena.field(object, self.effect_id)
            else {
                return Err(unsupported("effect identity missing"));
            };
            if !ids.contains(&id) {
                retained.push(ObjectValue::Object(object));
            }
        }
        let list = self.arena.list(retained);
        self.arena
            .set_root(self.effects, list)
            .map_err(unsupported)?;
        // Source replaces the list. Old aliases become detached mutable data.
        self.readonly = Arc::new(self.arena.readonly_objects());
        Ok(())
    }
    pub fn bind_program(
        &mut self,
        object: ObjectId,
        instruction: usize,
        steps: &[CompiledStep],
    ) -> Result<()> {
        if self.programs.contains_key(&object) {
            return Ok(());
        }
        Arc::make_mut(&mut self.programs).insert(object, instruction);
        if let Instruction::ForEach { templates, .. } = &steps[instruction].instruction {
            let objects = self.template_objects(object)?;
            if objects.len() != templates.len() {
                return Err(unsupported("loop template binding drift"));
            }
            for (object, instruction) in objects.into_iter().zip(templates) {
                self.bind_program(object, *instruction, steps)?;
            }
        }
        Ok(())
    }
    fn template_objects(&self, step: ObjectId) -> Result<Vec<ObjectId>> {
        let Some(ObjectValue::Object(id)) = self.arena.field(step, self.templates) else {
            return Err(unsupported("loop template list missing"));
        };
        self.arena
            .sequence(id)
            .ok_or_else(|| unsupported("loop template is not a sequence"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Object(id) if self.arena.kind(*id) == ObjectKind::Step => Ok(*id),
                _ => Err(unsupported("loop template step binding missing")),
            })
            .collect()
    }
    fn program(&self, step: ObjectId) -> Option<usize> {
        self.programs
            .get(&step)
            .or_else(|| self.copied_programs.get(&step))
            .copied()
    }
    fn protected(&self, object: ObjectId) -> bool {
        self.readonly.contains(&object) || self.copied_readonly.contains(&object)
    }
    fn copy_template(&mut self, step: ObjectId) -> Result<(ObjectId, usize)> {
        let (ObjectValue::Object(copy), objects) = self
            .arena
            .deep_copy(&ObjectValue::Object(step))
            .map_err(unsupported)?
        else {
            return Err(unsupported("copied template step missing"));
        };
        for (original, copied) in objects {
            if let Some(instruction) = self.program(original) {
                self.copied_programs.insert(copied, instruction);
            }
        }
        // Protection belongs to the copied operand role, not its provenance.
        // A detached Set.value copy of a world list is ordinary mutable data.
        self.copied_readonly
            .extend(self.arena.readonly_step_objects(copy));
        let instruction = self
            .program(copy)
            .ok_or_else(|| unsupported("copied step program missing"))?;
        Ok((copy, instruction))
    }
    pub fn step(&self, index: usize) -> Result<ObjectId> {
        let ObjectValue::Object(id) = self
            .arena
            .value_at_path(&[
                ObjectPathPart::Key("execution_stack".into()),
                ObjectPathPart::Index(index),
            ])
            .map_err(unsupported)?
        else {
            return Err(unsupported("step object missing"));
        };
        if self.arena.kind(id) != ObjectKind::Step {
            return Err(unsupported("step kind binding missing"));
        }
        Ok(id)
    }
    pub fn symbol_from_context(&self, key: Symbol, symbols: &Symbols) -> Result<Option<Symbol>> {
        match self.arena.context_value(key) {
            Some(ObjectValue::Atom(Datum::Text(id))) if !symbols.name(id).is_empty() => {
                Ok(Some(id))
            }
            Some(value) if self.arena.truthy(&value) => {
                Err(unsupported("non-string actor/controller context"))
            }
            _ => Ok(None),
        }
    }
    fn mutable_stack(&self, key: Symbol) -> Result<Option<ObjectId>> {
        match self.arena.context_value(key) {
            None => Ok(None),
            Some(ObjectValue::Object(id)) if self.arena.is_mutable_list(id) => {
                if self.protected(id) {
                    return Err(unsupported(
                        "context stack aliases an unbound world/step operand",
                    ));
                }
                Ok(Some(id))
            }
            Some(value) if !self.arena.truthy(&value) => Ok(None),
            _ => Err(unsupported("context stack must be a mutable list")),
        }
    }
    fn restore_scope(&mut self) -> Result<()> {
        let Some(stack) = self.mutable_stack(self.action_stack)? else {
            return self
                .arena
                .remove_context(self.action_stack)
                .map_err(unsupported);
        };
        let snapshot = match self.arena.list_pop(stack).map_err(unsupported)? {
            None => {
                return self
                    .arena
                    .remove_context(self.action_stack)
                    .map_err(unsupported);
            }
            Some(ObjectValue::Object(snapshot)) => snapshot,
            _ => return Err(unsupported("action snapshot must be a dictionary")),
        };
        if self.arena.kind(snapshot) != ObjectKind::Dict {
            return Err(unsupported("action snapshot must be a dictionary"));
        }
        for key in self.action_keys {
            match self.arena.field(snapshot, key) {
                None => self.arena.remove_context(key).map_err(unsupported)?,
                Some(ObjectValue::Object(saved)) if self.arena.kind(saved) == ObjectKind::Dict => {
                    let present = self
                        .arena
                        .field(saved, self.present)
                        .ok_or_else(|| unsupported("snapshot presence flag missing"))?;
                    if self.arena.truthy(&present) {
                        let value = self
                            .arena
                            .field(saved, self.value)
                            .ok_or_else(|| unsupported("snapshot value missing"))?;
                        self.arena.set_context(key, value).map_err(unsupported)?;
                    } else {
                        self.arena.remove_context(key).map_err(unsupported)?;
                    }
                }
                _ => return Err(unsupported("action snapshot field must be a dictionary")),
            }
        }
        if self
            .arena
            .sequence(stack)
            .is_some_and(|values| values.is_empty())
        {
            self.arena
                .remove_context(self.action_stack)
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn push_scope(&mut self, action: Symbol, card: Symbol, owner: Symbol) -> Result<()> {
        let stack = if self.arena.context_value(self.action_stack).is_none() {
            let ObjectValue::Object(stack) = self.arena.allocate(&Datum::List(Vec::new())) else {
                unreachable!()
            };
            self.arena
                .set_context(self.action_stack, ObjectValue::Object(stack))
                .map_err(unsupported)?;
            stack
        } else {
            self.mutable_stack(self.action_stack)?
                .ok_or_else(|| unsupported("action snapshot stack must be a list"))?
        };
        let mut fields = Vec::new();
        for key in self.action_keys {
            let value = self.arena.context_value(key);
            let saved = self
                .arena
                .dictionary(vec![
                    (
                        self.present,
                        ObjectValue::Atom(Datum::Bool(value.is_some())),
                    ),
                    (self.value, value.unwrap_or(ObjectValue::Atom(Datum::Null))),
                ])
                .map_err(unsupported)?;
            fields.push((key, saved));
        }
        let snapshot = self.arena.dictionary(fields).map_err(unsupported)?;
        self.arena.append(stack, snapshot).map_err(unsupported)?;
        for (key, value) in self.action_keys[..5]
            .iter()
            .zip([action, card, card, owner, card])
        {
            self.arena
                .set_context(*key, ObjectValue::Atom(Datum::Text(value)))
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn restore_type(&mut self) -> Result<()> {
        if let Some(stack) = self.mutable_stack(self.type_stack)?
            && let Some(value) = self.arena.list_pop(stack).map_err(unsupported)?
        {
            self.arena
                .set_context(self.action_keys[0], value)
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn clear_pending(&mut self, id: ObjectId) -> Result<()> {
        for key in [self.pending, self.request] {
            self.arena
                .set_field(id, key, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn token(&self, id: ObjectId) -> Result<Option<RequestToken>> {
        match self.arena.field(id, self.request) {
            None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(id))) => Ok(Some(RequestToken::Imported(id))),
            Some(ObjectValue::Request(id)) => Ok(Some(RequestToken::Generated(id))),
            _ => Err(unsupported("invalid live step request token")),
        }
    }
    fn selected(&self, id: ObjectId, selection: Symbol) -> Option<ObjectValue> {
        let ObjectValue::Object(pending) = self.arena.field(id, self.pending)? else {
            return None;
        };
        self.arena.field(pending, selection)
    }
    fn write(&mut self, key: Symbol, value: Datum) -> Result<()> {
        self.arena
            .set_context(key, ObjectValue::Atom(value))
            .map_err(unsupported)
    }
}

/// Derived ownership/controller and effect/card-lifecycle lookup projection.
/// Mutable stored step payloads belong exclusively to the authoritative arena.
pub(super) fn world_projection(source: &StateEnvelope) -> StateEnvelope {
    let mut projection = source.clone();
    projection.object_bindings.clear();
    projection.state["execution_context"] = json!({});
    projection.state["execution_stack"] = json!([]);
    if let Some(effects) = projection.state["active_effects"].as_array_mut() {
        for effect in effects {
            effect["finishing_steps"] = json!([]);
        }
    }
    projection
}

impl NativeSession {
    fn with_live_objects<T>(
        &mut self,
        action: impl FnOnce(&mut Self, &mut BoundObjects) -> Result<T>,
    ) -> Result<T> {
        let mut objects = self
            .objects
            .take()
            .ok_or_else(|| unsupported("operation requires live object bindings"))?;
        let result = action(self, &mut objects);
        self.objects = Some(objects);
        result
    }
    pub(super) fn advance_objects(
        &mut self,
        budget: &ExecutionBudget<'_>,
        stop_after: Option<usize>,
    ) -> Result<AdvanceResult> {
        let mut objects = self
            .objects
            .take()
            .ok_or_else(|| unsupported("object arena missing"))?;
        let result = self.run_objects(&mut objects, budget, stop_after);
        self.objects = Some(objects);
        result
    }
    fn run_objects(
        &mut self,
        objects: &mut BoundObjects,
        budget: &ExecutionBudget<'_>,
        stop_after: Option<usize>,
    ) -> Result<AdvanceResult> {
        let mut result = AdvanceResult::default();
        if !self.stack.is_empty() {
            budget.check(0)?;
        }
        self.last_request = None;
        while !self.stack.is_empty() {
            if stop_after.is_some_and(|limit| result.steps >= limit) {
                return Ok(result);
            }
            budget.check(result.steps)?;
            let top = self.stack.last().unwrap();
            let conditions = &self.steps[top.instruction].conditions;
            let skipped = conditions.active.is_some_and(|key| {
                !objects
                    .arena
                    .context_value(key)
                    .is_some_and(|value| objects.arena.truthy(&value))
            }) || conditions.skip.is_some_and(|key| {
                objects
                    .arena
                    .context_value(key)
                    .is_some_and(|value| !matches!(value, ObjectValue::Atom(Datum::Null)))
            });
            let actor_choice = if !skipped
                && matches!(
                    self.steps[top.instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Next)
                ) {
                Some(self.actor_choice_bound(objects)?)
            } else {
                None
            };
            if !skipped {
                if matches!(
                    self.steps[top.instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Advance)
                ) {
                    objects.phases.effect_ids(
                        &objects.arena,
                        top.object
                            .ok_or_else(|| unsupported("live advance step missing"))?,
                    )?;
                }
                self.check_bound_phase(
                    objects,
                    &self.steps[top.instruction].instruction,
                    actor_choice.as_ref(),
                )?;
            }
            result.steps += 1;
            let mut frame = self.stack.pop().unwrap();
            let id = frame
                .object
                .ok_or_else(|| unsupported("live step binding missing"))?;
            if !matches!(objects.arena.list_pop(objects.stack).map_err(unsupported)?, Some(ObjectValue::Object(top)) if top == id)
            {
                return Err(unsupported("stack/frame object binding drift"));
            }
            // Pending state is on the actual shared step, never the frame copy.
            frame.request = objects.token(id)?;
            let compiled = &self.steps[frame.instruction];
            let conditions = &compiled.conditions;
            if conditions.active.is_some_and(|key| {
                !objects
                    .arena
                    .context_value(key)
                    .is_some_and(|value| objects.arena.truthy(&value))
            }) || conditions.skip.is_some_and(|key| {
                objects
                    .arena
                    .context_value(key)
                    .is_some_and(|value| !matches!(value, ObjectValue::Atom(Datum::Null)))
            }) {
                continue;
            }
            let mandatory = conditions.mandatory;
            let mut request = None;
            match &compiled.instruction {
                Instruction::Set { key, .. } => {
                    let value = objects
                        .arena
                        .field(id, objects.value)
                        .ok_or_else(|| unsupported("live Set.value missing"))?;
                    objects
                        .arena
                        .set_context(*key, value)
                        .map_err(unsupported)?;
                }
                Instruction::Add { key, amount } => {
                    let value = match objects.arena.context_value(*key) {
                        Some(ObjectValue::Atom(Datum::Float(v))) => {
                            Datum::Float(v + *amount as f64)
                        }
                        Some(ObjectValue::Atom(Datum::Int(v))) => Datum::Int(
                            v.checked_add(*amount)
                                .ok_or_else(|| unsupported("integer addition outside i64"))?,
                        ),
                        Some(ObjectValue::Atom(Datum::Bool(v))) => Datum::Int(
                            i64::from(v)
                                .checked_add(*amount)
                                .ok_or_else(|| unsupported("integer addition outside i64"))?,
                        ),
                        _ => Datum::Int(*amount),
                    };
                    objects.write(*key, value)?;
                }
                Instruction::Record { input, output } => {
                    if let Some(value) = objects
                        .arena
                        .context_value(*input)
                        .filter(|value| objects.arena.truthy(value))
                    {
                        let target = match objects.arena.context_value(*output) {
                            None => {
                                let list = objects.arena.allocate(&Datum::List(Vec::new()));
                                objects
                                    .arena
                                    .set_context(*output, list.clone())
                                    .map_err(unsupported)?;
                                Some(list)
                            }
                            value => value,
                        };
                        if let Some(ObjectValue::Object(target)) = target
                            && objects.arena.is_mutable_list(target)
                        {
                            if objects.protected(target) {
                                return Err(unsupported(
                                    "context append aliases an unbound world/step operand",
                                ));
                            }
                            objects.arena.append(target, value).map_err(unsupported)?;
                        }
                    }
                }
                Instruction::Flow(crate::turn::FlowStep::Advance) => {
                    let ids = objects.phases.effect_ids(&objects.arena, id)?;
                    self.preflight_bound_advance(objects)?;
                    self.remove_effects_bound(objects, &ids)?;
                    self.advance_turn_bound(objects)?;
                }
                Instruction::Flow(crate::turn::FlowStep::Next) => {
                    self.apply_actor_choice_bound(
                        objects,
                        actor_choice.ok_or_else(|| unsupported("actor decision missing"))?,
                    )?;
                }
                Instruction::Flow(crate::turn::FlowStep::Tie(_)) => {
                    request = self.tie_bound(objects, id)?;
                }
                Instruction::RetrieveUnresolved { prompt, .. } => {
                    let prompt = *prompt;
                    let (next, event) = self.retrieve_bound(objects, id, prompt)?;
                    request = next;
                    result.events.extend(event);
                }
                Instruction::FlipCoin => {
                    self.team = if self.team == Team::Red {
                        Team::Blue
                    } else {
                        Team::Red
                    };
                    let color = if self.team == Team::Blue {
                        phases::Field::Blue
                    } else {
                        phases::Field::Red
                    };
                    objects
                        .arena
                        .set_root(
                            objects.phases.k(phases::Field::Coin),
                            ObjectValue::Atom(Datum::Text(objects.phases.k(color))),
                        )
                        .map_err(unsupported)?;
                    result.events.push(NativeEvent::TieBreakerFlipped {
                        blue: self.team == Team::Blue,
                    });
                }
                Instruction::Flow(crate::turn::FlowStep::EndPhase) => {
                    self.end_phase_bound(objects)?;
                }
                Instruction::FinishedEffect => {}
                Instruction::RestoreContext { .. } => objects.restore_scope()?,
                Instruction::RestoreAction => objects.restore_type()?,
                Instruction::SetActor { key, save, .. } => {
                    objects.write(
                        *save,
                        self.planning
                            .actor
                            .filter(|actor| !self.symbols.name(*actor).is_empty())
                            .map_or(Datum::Null, Datum::Text),
                    )?;
                    let literal = match objects
                        .arena
                        .field(id, objects.scheduler.as_ref().unwrap().actor_id)
                    {
                        Some(ObjectValue::Atom(Datum::Text(actor))) => Some(actor),
                        Some(ObjectValue::Atom(Datum::Null)) => None,
                        _ => return Err(unsupported("actor literal must be text or null")),
                    };
                    let actor = scope_actor(literal, *key, &self.symbols, |key| {
                        objects.symbol_from_context(key, &self.symbols)
                    })?;
                    if let Some(actor) = actor.filter(|actor| !self.symbols.name(*actor).is_empty())
                    {
                        objects
                            .arena
                            .set_root(objects.actor, ObjectValue::Atom(Datum::Text(actor)))
                            .map_err(unsupported)?;
                        self.planning.actor = Some(actor);
                    }
                }
                Instruction::ForEach { list, item, .. } => {
                    let index = match objects.arena.field(id, objects.index) {
                        Some(ObjectValue::Atom(Datum::Int(index))) => index,
                        _ => return Err(unsupported("live loop index missing")),
                    };
                    let (len, value) = match objects.arena.context_value(*list) {
                        None => continue,
                        Some(ObjectValue::Object(list)) => {
                            let values = objects
                                .arena
                                .sequence(list)
                                .ok_or_else(|| unsupported("ForEach source requires a sequence"))?;
                            let len = i64::try_from(values.len())
                                .map_err(|_| unsupported("loop length outside i64"))?;
                            if index >= len {
                                continue;
                            }
                            let offset = if index < 0 {
                                len.checked_add(index)
                            } else {
                                Some(index)
                            };
                            let value = offset
                                .and_then(|index| usize::try_from(index).ok())
                                .and_then(|index| values.get(index))
                                .cloned()
                                .ok_or_else(|| {
                                    EngineError::Rule("ForEach index outside list".into())
                                })?;
                            (len, value)
                        }
                        _ => return Err(unsupported("ForEach source requires a sequence")),
                    };
                    objects
                        .arena
                        .set_context(*item, value)
                        .map_err(unsupported)?;
                    let next = index
                        .checked_add(1)
                        .ok_or_else(|| unsupported("loop index outside i64"))?;
                    objects
                        .arena
                        .set_field(id, objects.index, ObjectValue::Atom(Datum::Int(next)))
                        .map_err(unsupported)?;
                    let mut children = Vec::new();
                    for template in objects.template_objects(id)? {
                        let (object, instruction) = objects.copy_template(template)?;
                        let mut child = self.steps[instruction].initial.clone();
                        child.object = Some(object);
                        child.pending = None;
                        children.push(child);
                    }
                    if next < len {
                        objects
                            .arena
                            .append(objects.stack, ObjectValue::Object(id))
                            .map_err(unsupported)?;
                        self.stack.push(frame);
                    }
                    for child in children.into_iter().rev() {
                        objects
                            .arena
                            .append(objects.stack, ObjectValue::Object(child.object.unwrap()))
                            .map_err(unsupported)?;
                        self.stack.push(child);
                    }
                    continue;
                }
                Instruction::Log => {}
                Instruction::Number {
                    output,
                    options: _,
                    labels,
                    prompt,
                    auto,
                    owner,
                    owner_key,
                    discard,
                } => {
                    let Some(ObjectValue::Object(options)) =
                        objects.arena.field(id, objects.number_options)
                    else {
                        return Err(unsupported("live number options missing"));
                    };
                    let options = objects
                        .arena
                        .sequence(options)
                        .ok_or_else(|| unsupported("live number options must be a list"))?
                        .iter()
                        .map(|value| match value {
                            ObjectValue::Atom(Datum::Int(value)) => Ok(*value),
                            _ => Err(unsupported("live number option must be an integer")),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if options.is_empty() {
                        if mandatory {
                            self.abort_objects(objects)?;
                        } else {
                            objects.arena.remove_context(*output).map_err(unsupported)?;
                        }
                        continue;
                    }
                    let selected = objects.selected(id, self.selection_key);
                    if *auto && options.len() == 1 && mandatory {
                        objects.write(*output, Datum::Int(options[0]))?;
                        objects.clear_pending(id)?;
                        continue;
                    }
                    if matches!(selected, Some(ObjectValue::Atom(Datum::Text(text))) if text == self.skip)
                        && !mandatory
                    {
                        objects.arena.remove_context(*output).map_err(unsupported)?;
                        objects.clear_pending(id)?;
                        continue;
                    }
                    let number = match selected {
                        Some(ObjectValue::Atom(value)) if !matches!(value, Datum::Null) => {
                            value.to_integer(&self.symbols).map_err(unsupported)?
                        }
                        _ => None,
                    };
                    if let Some(number) = number.filter(|number| options.contains(number)) {
                        objects.write(*output, Datum::Int(number))?;
                        objects.clear_pending(id)?;
                        continue;
                    }
                    let raw_owner = prompt_actor(
                        *owner,
                        *owner_key,
                        self.planning.actor,
                        &self.symbols,
                        |key| objects.symbol_from_context(key, &self.symbols),
                    )?;
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(raw_owner.unwrap_or(self.no_actor)),
                        prompt: *prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Numbers {
                            values: options.clone(),
                            labels: labels.clone(),
                        },
                        can_skip: !mandatory,
                        discard: *discard,
                    });
                }
                _ => return Err(unsupported("unbound shared-object instruction")),
            }
            if let Some(request) = request {
                let request = self.issue(&mut frame, request)?;
                let token = match request.id {
                    RequestToken::Imported(id) => ObjectValue::Atom(Datum::Text(id)),
                    RequestToken::Generated(id) => ObjectValue::Request(id),
                };
                objects
                    .arena
                    .set_field(id, objects.request, token)
                    .map_err(unsupported)?;
                objects
                    .arena
                    .append(objects.stack, ObjectValue::Object(id))
                    .map_err(unsupported)?;
                self.stack.push(frame);
                self.last_request = Some(request.clone());
                result.request = Some(request);
                return Ok(result);
            }
        }
        Ok(result)
    }
    fn abort_objects(&mut self, objects: &mut BoundObjects) -> Result<()> {
        while let Some(frame) = self.stack.last() {
            if matches!(
                self.steps[frame.instruction].instruction,
                Instruction::RestoreContext { other: true }
            ) {
                return Ok(());
            }
            if matches!(
                self.steps[frame.instruction].instruction,
                Instruction::FinishedEffect
            ) {
                break;
            }
            let frame = self.stack.pop().unwrap();
            let id = frame
                .object
                .ok_or_else(|| unsupported("aborted step object missing"))?;
            if !matches!(objects.arena.list_pop(objects.stack).map_err(unsupported)?, Some(ObjectValue::Object(top)) if top == id)
            {
                return Err(unsupported("aborted stack/frame binding drift"));
            }
            if matches!(
                self.steps[frame.instruction].instruction,
                Instruction::RestoreContext { .. }
            ) {
                objects.restore_scope()?;
            }
        }
        while objects
            .arena
            .context_value(objects.action_stack)
            .is_some_and(|value| objects.arena.truthy(&value))
        {
            objects.restore_scope()?;
        }
        Ok(())
    }
    fn sync_effect_card_flags(
        &mut self,
        objects: &mut BoundObjects,
        before: &[bool],
    ) -> Result<()> {
        for (card, before) in self.world.cards.iter().zip(before) {
            if card.active == *before {
                continue;
            }
            objects
                .arena
                .set_physical_field(
                    card.source_instance,
                    objects.active,
                    Datum::Bool(card.active),
                )
                .map_err(unsupported)?;
        }
        Ok(())
    }
    /// Natural duration expiry returns the original stored step handles. It
    /// does not schedule them or perform phase advancement.
    pub fn expire_effects(
        &mut self,
        duration: crate::effects::Duration,
    ) -> Result<Vec<(Symbol, ObjectId)>> {
        self.with_live_objects(|session, objects| session.expire_effects_bound(objects, duration))
    }
    fn expire_effects_bound(
        &mut self,
        objects: &mut BoundObjects,
        duration: crate::effects::Duration,
    ) -> Result<Vec<(Symbol, ObjectId)>> {
        let indices = self.world.effects.duration_indices(duration);
        let finishing = objects.finishing_for_indices(&self.world.effects, &indices)?;
        let before = self
            .world
            .cards
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        let expiry = self.world.expire_duration_effects(duration);
        objects.remove_effect_nodes(&expiry.ids)?;
        self.sync_effect_card_flags(objects, &before)?;
        Ok(finishing)
    }
    pub fn collect_turn_expiry(&self) -> Result<LiveEffectExpiry> {
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| unsupported("turn expiry requires live object bindings"))?;
        self.collect_turn_expiry_bound(objects)
    }
    fn collect_turn_expiry_bound(&self, objects: &BoundObjects) -> Result<LiveEffectExpiry> {
        let indices = self
            .world
            .effects
            .turn_expiry_indices(self.round.round, self.world.combat.turn);
        let finishing = objects.finishing_for_indices(&self.world.effects, &indices)?;
        let ids = indices
            .iter()
            .map(|index| self.world.effects.instances[*index].id)
            .collect();
        Ok(LiveEffectExpiry { finishing, ids })
    }
    pub fn expire_turn_effects(&mut self) -> Result<LiveEffectExpiry> {
        let expiry = self.collect_turn_expiry()?;
        self.remove_effects(&expiry.ids)?;
        Ok(expiry)
    }
    /// Queue the real after-reveal trigger payloads with fresh actor/abort
    /// wrappers and original mutable step handles. No JSON runs in this path.
    pub fn queue_after_cards_played(&mut self) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let steps = session.collect_after_cards_played_bound(objects)?;
            session.push_live_steps_bound(objects, &steps)
        })
    }
    fn collect_after_cards_played_bound(
        &mut self,
        objects: &mut BoundObjects,
    ) -> Result<Vec<ObjectId>> {
        let nodes = objects.effect_nodes()?;
        if nodes.len() != self.world.effects.instances.len() {
            return Err(unsupported("effect collection binding drift"));
        }
        let mut removed = Vec::new();
        let mut finishing = Vec::new();
        for (index, effect) in self.world.effects.instances.iter().enumerate() {
            if !effect.definition.after_cards_played {
                continue;
            }
            removed.push(index);
            if self.round.round == effect.created_round
                && self.world.combat.turn.checked_sub(1) == Some(effect.created_turn)
            {
                let list = objects.finishing_list(nodes[index])?;
                let steps = objects.steps_in_list(list)?;
                if !steps.is_empty() {
                    finishing.push((effect.source, steps));
                }
            }
        }
        if removed.is_empty() {
            return Ok(Vec::new());
        }
        // Validate every runnable payload and generation before mutating state.
        self.phase_generation_check(finishing.len(), 0)?;
        let before = self
            .world
            .cards
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        self.world.remove_after_cards_played_effects();
        objects.remove_effect_rows(&removed)?;
        self.sync_effect_card_flags(objects, &before)?;
        self.wrap_finishings(objects, finishing)
    }
    /// Removal by ID is premature: no finishing payloads are returned or queued.
    pub fn remove_effects(&mut self, ids: &[Symbol]) -> Result<()> {
        self.with_live_objects(|session, objects| session.remove_effects_bound(objects, ids))
    }
    fn remove_effects_bound(&mut self, objects: &mut BoundObjects, ids: &[Symbol]) -> Result<()> {
        objects.remove_effect_nodes(ids)?;
        let before = self
            .world
            .cards
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        self.world.remove_effects(ids);
        self.sync_effect_card_flags(objects, &before)
    }
    fn push_live_steps(&mut self, steps: &[ObjectId]) -> Result<()> {
        self.with_live_objects(|session, objects| session.push_live_steps_bound(objects, steps))
    }
    fn push_live_steps_bound(
        &mut self,
        objects: &mut BoundObjects,
        steps: &[ObjectId],
    ) -> Result<()> {
        let frames = steps
            .iter()
            .map(|object| {
                let instruction = objects
                    .program(*object)
                    .ok_or_else(|| unsupported("stored step program is not compiled"))?;
                let mut frame = self.steps[instruction].initial.clone();
                frame.object = Some(*object);
                frame.pending = None;
                Ok(frame)
            })
            .collect::<Result<Vec<_>>>()?;
        for frame in frames.into_iter().rev() {
            objects
                .arena
                .append(objects.stack, ObjectValue::Object(frame.object.unwrap()))
                .map_err(unsupported)?;
            self.stack.push(frame);
        }
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        Ok(())
    }
    /// Boundary adapter for the source push_steps helper. Engine callers use
    /// the typed handles returned by expiry rather than resolving JSON paths.
    pub fn queue_steps(&mut self, path: &[ObjectPathPart]) -> Result<()> {
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| unsupported("stored step scheduling requires live object bindings"))?;
        let ObjectValue::Object(list) = objects.arena.value_at_path(path).map_err(unsupported)?
        else {
            return Err(unsupported("step sequence missing"));
        };
        let steps = objects.steps_in_list(list)?;
        self.push_live_steps(&steps)
    }
    pub fn queue_finishing_steps(&mut self, list: ObjectId) -> Result<()> {
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| unsupported("stored step scheduling requires live object bindings"))?;
        let steps = objects.steps_in_list(list)?;
        self.push_live_steps(&steps)
    }
    /// Same shallow scope operation used when the source starts a nested action.
    /// The actual action body/legality remains a separate engine operation.
    pub fn push_action_context(
        &mut self,
        action: Symbol,
        card: Symbol,
        owner: Symbol,
    ) -> Result<()> {
        let objects = self
            .objects
            .as_mut()
            .ok_or_else(|| unsupported("action scopes require live object bindings"))?;
        objects.push_scope(action, card, owner)
    }
    pub(super) fn submit_objects(&mut self, request: RequestToken, selection: Datum) -> Result<()> {
        let frame = self
            .stack
            .last_mut()
            .ok_or_else(|| EngineError::InvalidInput("No pending step to receive input".into()))?;
        let objects = self
            .objects
            .as_mut()
            .ok_or_else(|| unsupported("object arena missing"))?;
        let id = frame
            .object
            .ok_or_else(|| unsupported("live step binding missing"))?;
        if objects.token(id)? != Some(request) {
            return Err(EngineError::InvalidInput(
                "stale or unissued input request".into(),
            ));
        }
        let pending = objects
            .arena
            .allocate(&Datum::Object(vec![(self.selection_key, selection)]));
        objects
            .arena
            .set_field(id, objects.pending, pending)
            .map_err(unsupported)?;
        objects
            .arena
            .set_field(id, objects.request, ObjectValue::Atom(Datum::Null))
            .map_err(unsupported)?;
        frame.request = None;
        Ok(())
    }
}
