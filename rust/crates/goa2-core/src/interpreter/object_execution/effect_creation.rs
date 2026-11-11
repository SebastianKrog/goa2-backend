//! Live effect construction; arena models own identity, typed payloads serve rules.
use super::{
    Arc, BoundObjects, Datum, NativeEvent, NativeSession, ObjectArena, ObjectId, ObjectKind,
    ObjectValue, Result, Symbol, Symbols, unsupported,
};
use crate::{
    data::SparseRegisters,
    effects::{CreationContext, EffectCreateSpec, EffectInstance},
    markers::MarkerRules,
};

#[derive(Clone, Debug)]
pub(super) struct EffectBindings {
    prototype: Datum,
    kinds: Vec<Symbol>,
    actions: Vec<Symbol>,
    colors: Vec<Symbol>,
}
impl EffectBindings {
    pub fn compile(arena: &mut ObjectArena) -> Result<Self> {
        let symbols = Arc::make_mut(&mut arena.symbols);
        let (prototype, kinds, actions, colors) =
            crate::effect_steps::live_catalog(symbols).map_err(unsupported)?;
        for name in [
            "new_steps",
            "color_key",
            "color",
            "is_token_effect",
            "use_context_card",
            "origin_id_key",
            "except_attacker_key",
            "named_color_key",
            "token_type_override",
            "substitution_actor_id",
            "performing_card_id",
            "performing_card_owner_id",
            "next_entity_id",
            "current_action_type",
            "current_card_id",
            "system",
            "ATTACK",
        ] {
            symbols.intern(name);
        }
        Ok(Self {
            prototype,
            kinds,
            actions,
            colors,
        })
    }
}
fn key(symbols: &Symbols, name: &str) -> Symbol {
    symbols.lookup(name).expect("compiled effect field")
}
fn set(fields: &mut [(Symbol, Datum)], field: Symbol, value: Datum) -> Result<()> {
    let target = fields
        .iter_mut()
        .find(|(key, _)| *key == field)
        .ok_or_else(|| unsupported("effect payload field missing"))?;
    target.1 = value;
    Ok(())
}
fn get(fields: &[(Symbol, Datum)], field: Symbol) -> Option<&Datum> {
    fields.iter().find(|(key, _)| *key == field).map(|(_, v)| v)
}
#[derive(Debug)]
pub(super) struct FinishingPlan {
    original: ObjectId,
    color: Option<Symbol>,
    children: Vec<(Symbol, Vec<FinishingPlan>)>,
    payload: Datum,
}
pub(super) enum EffectFinishing {
    Templates(Vec<FinishingPlan>),
    Bound(Vec<ObjectId>),
}
pub(super) enum CreatePlan {
    Skip,
    Create {
        spec: Box<EffectCreateSpec>,
        scope: Option<ObjectId>,
        copy_scope: bool,
        finishing: EffectFinishing,
        isolated: Option<ObjectValue>,
        effects: ObjectId,
        context: SparseRegisters,
        event: bool,
        split: Option<Symbol>,
        split_value: i64,
        color: Option<Symbol>,
    },
}
impl CreatePlan {
    pub(super) fn creates(&self) -> bool {
        matches!(self, Self::Create { .. })
    }
    pub(super) fn bind_manager_finishing(&mut self, steps: Vec<ObjectId>) -> Result<()> {
        let Self::Create { finishing, .. } = self else {
            return Err(unsupported("cannot attach finishing to skipped effect"));
        };
        if !matches!(finishing, EffectFinishing::Bound(_)) {
            return Err(unsupported("manager finishing requires literal creation"));
        }
        *finishing = EffectFinishing::Bound(steps);
        Ok(())
    }
}
impl NativeSession {
    fn effect_optional_text(
        &self,
        _objects: &BoundObjects,
        value: Option<ObjectValue>,
    ) -> Result<Option<Symbol>> {
        match value {
            None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(id))) => Ok(Some(id)),
            _ => Err(unsupported("effect identity must be text")),
        }
    }
    fn effect_flag(&self, objects: &BoundObjects, step: ObjectId, name: &str) -> Result<bool> {
        match objects.arena.field(step, key(&self.symbols, name)) {
            Some(ObjectValue::Atom(Datum::Bool(value))) => Ok(value),
            _ => Err(unsupported("effect flag must be boolean")),
        }
    }
    fn effect_context_key(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        name: &str,
    ) -> Result<Option<ObjectValue>> {
        let Some(name) =
            self.truthy_symbol_bound(objects, objects.arena.field(step, key(&self.symbols, name)))?
        else {
            return Ok(None);
        };
        Ok(objects
            .arena
            .context_value(name)
            .filter(|value| objects.arena.truthy(value)))
    }
    fn effect_finishing_plan(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        depth: usize,
    ) -> Result<FinishingPlan> {
        if depth >= 128 || objects.program(step).is_none() {
            return Err(unsupported("effect finishing step graph is not bound"));
        }
        let mut payload = objects
            .arena
            .typed_value(&ObjectValue::Object(step), Arc::make_mut(&mut self.symbols))
            .map_err(unsupported)?;
        let Datum::Object(ref mut fields) = payload else {
            return Err(unsupported("effect finishing step fields missing"));
        };
        let mut color = None;
        if objects
            .arena
            .field(step, key(&self.symbols, "color"))
            .is_none_or(|value| matches!(value, ObjectValue::Atom(Datum::Null)))
            && let Some(value) = self.effect_context_key(objects, step, "color_key")?
        {
            let value = self
                .effect_optional_text(objects, Some(value))?
                .ok_or_else(|| unsupported("finishing color missing"))?;
            if !objects.effect_creation.colors.contains(&value) {
                return Err(unsupported("invalid finishing color"));
            }
            color = Some(value);
            if let Some((_, field)) = fields
                .iter_mut()
                .find(|(k, _)| *k == key(&self.symbols, "color"))
            {
                *field = Datum::Text(value);
            } else {
                fields.push((key(&self.symbols, "color"), Datum::Text(value)));
            }
        }
        let mut children = Vec::new();
        for name in ["steps_template", "new_steps"] {
            let field = key(&self.symbols, name);
            if let Some(ObjectValue::Object(list)) = objects.arena.field(step, field)
                && objects.arena.truthy(&ObjectValue::Object(list))
            {
                let values = objects
                    .arena
                    .sequence(list)
                    .ok_or_else(|| unsupported("effect finishing children must be a sequence"))?;
                let plans = values
                    .iter()
                    .map(|value| match value {
                        ObjectValue::Object(id) => {
                            self.effect_finishing_plan(objects, *id, depth + 1)
                        }
                        _ => Err(unsupported("effect finishing child must be a step")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                set(
                    fields,
                    field,
                    Datum::List(plans.iter().map(|plan| plan.payload.clone()).collect()),
                )?;
                children.push((field, plans));
            }
        }
        Ok(FinishingPlan {
            original: step,
            color,
            children,
            payload,
        })
    }
    pub(super) fn preflight_create_effect_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<CreatePlan> {
        let token = self.effect_flag(objects, step, "is_token_effect")?;
        if token
            && objects
                .arena
                .context_value(key(&self.symbols, "token_type_override"))
                .is_some_and(|value| objects.arena.truthy(&value))
        {
            let substitution = match objects
                .arena
                .context_value(key(&self.symbols, "substitution_actor_id"))
            {
                Some(ObjectValue::Atom(Datum::Text(id))) => Some(id),
                _ => None,
            };
            if MarkerRules::suppressed(&self.world, self.planning.actor, true, substitution) {
                return Ok(CreatePlan::Skip);
            }
        }
        let Datum::Object(mut fields) = objects.effect_creation.prototype.clone() else {
            unreachable!()
        };
        for (field, value) in &mut fields {
            if matches!(
                self.symbols.name(*field),
                "id" | "source_id"
                    | "created_at_turn"
                    | "created_at_round"
                    | "token_id"
                    | "subject_id"
                    | "named_color"
                    | "origin_attack_is_basic"
            ) {
                continue;
            }
            if let Some(source) = objects.arena.field(step, *field) {
                *value = objects
                    .arena
                    .typed_value(&source, Arc::make_mut(&mut self.symbols))
                    .map_err(unsupported)?;
            }
        }
        let scope_key = key(&self.symbols, "scope");
        let Some(ObjectValue::Object(scope)) = objects.arena.field(step, scope_key) else {
            return Err(unsupported("effect scope model missing"));
        };
        if !matches!(
            objects.arena.kind(scope),
            ObjectKind::Model | ObjectKind::Dict
        ) {
            return Err(unsupported("effect scope model must have fields"));
        }
        let mut copy_scope = false;
        let mut origin = None;
        if let Some(value) = self.effect_context_key(objects, step, "origin_id_key")? {
            let text = objects
                .arena
                .typed_value(&value, Arc::make_mut(&mut self.symbols))
                .map_err(unsupported)?
                .python_scalar_text(&self.symbols)
                .ok_or_else(|| unsupported("effect origin requires Python scalar conversion"))?;
            origin = Some(Arc::make_mut(&mut self.symbols).intern(&text));
            copy_scope = true;
        }
        if let Some(origin) = origin {
            let Some(Datum::Object(scope_fields)) = fields
                .iter_mut()
                .find(|(k, _)| *k == scope_key)
                .map(|(_, v)| v)
            else {
                return Err(unsupported("typed effect scope missing"));
            };
            set(
                scope_fields,
                key(&self.symbols, "origin_id"),
                Datum::Text(origin),
            )?;
        }
        let card = if token {
            None
        } else {
            let explicit = self.effect_optional_text(
                objects,
                objects
                    .arena
                    .field(step, key(&self.symbols, "source_card_id")),
            )?;
            if explicit.is_none() && self.effect_flag(objects, step, "use_context_card")? {
                self.effect_optional_text(
                    objects,
                    objects
                        .arena
                        .context_value(key(&self.symbols, "current_card_id")),
                )?
            } else {
                explicit
            }
        };
        set(
            &mut fields,
            key(&self.symbols, "source_card_id"),
            card.map_or(Datum::Null, Datum::Text),
        )?;
        let explicit = self.effect_optional_text(
            objects,
            objects
                .arena
                .field(step, key(&self.symbols, "origin_action_type")),
        )?;
        let action = match explicit {
            Some(value) => Some(value),
            None => self.effect_optional_text(
                objects,
                objects
                    .arena
                    .context_value(key(&self.symbols, "current_action_type")),
            )?,
        };
        if action.is_some_and(|value| !objects.effect_creation.actions.contains(&value)) {
            return Err(unsupported("invalid effect origin action"));
        }
        set(
            &mut fields,
            key(&self.symbols, "origin_action_type"),
            action.map_or(Datum::Null, Datum::Text),
        )?;
        if let Some(value) = self.effect_context_key(objects, step, "except_attacker_key")? {
            let value = self
                .effect_optional_text(objects, Some(value))?
                .ok_or_else(|| unsupported("effect attacker missing"))?;
            let Some(Datum::List(values)) = fields
                .iter_mut()
                .find(|(k, _)| *k == key(&self.symbols, "except_attacker_ids"))
                .map(|(_, v)| v)
            else {
                return Err(unsupported("effect attackers must be a list"));
            };
            if !values.contains(&Datum::Text(value)) {
                values.push(Datum::Text(value));
            }
        }
        let finishing_key = key(&self.symbols, "finishing_steps");
        let Some(ObjectValue::Object(list)) = objects.arena.field(step, finishing_key) else {
            return Err(unsupported("effect finishing list missing"));
        };
        let finishing = objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("effect finishing payload must be a sequence"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Object(id) => self.effect_finishing_plan(objects, *id, 0),
                _ => Err(unsupported("effect finishing payload must contain steps")),
            })
            .collect::<Result<Vec<_>>>()?;
        set(
            &mut fields,
            finishing_key,
            Datum::List(finishing.iter().map(|plan| plan.payload.clone()).collect()),
        )?;
        let color =
            if let Some(value) = self.effect_context_key(objects, step, "named_color_key")? {
                self.effect_optional_text(objects, Some(value))?
            } else {
                None
            };
        if color.is_some_and(|value| !objects.effect_creation.colors.contains(&value)) {
            return Err(unsupported("invalid named effect color"));
        }
        set(
            &mut fields,
            key(&self.symbols, "named_color"),
            color.map_or(Datum::Null, Datum::Text),
        )?;
        let source = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty())
            .unwrap_or(key(&self.symbols, "system"));
        set(
            &mut fields,
            key(&self.symbols, "source_id"),
            Datum::Text(source),
        )?;
        let mut template = EffectInstance::from_datum(&Datum::Object(fields), &self.symbols)
            .map_err(unsupported)?;
        if !objects
            .effect_creation
            .kinds
            .contains(&template.definition.kind)
        {
            return Err(unsupported("invalid effect kind"));
        }
        template.token = if token {
            template
                .scope_origin
                .filter(|id| !self.symbols.name(*id).is_empty())
        } else {
            None
        };
        let Some(effects) = self.preflight_effect_collection_bound(objects, &template)? else {
            return Ok(CreatePlan::Skip);
        };
        self.sync_card_collections_bound(objects)?;
        let mut context = SparseRegisters::default();
        for name in ["performing_card_id", "performing_card_owner_id"] {
            let k = key(&self.symbols, name);
            if let Some(value) = objects.arena.context_value(k) {
                context.0.push((
                    k,
                    objects
                        .arena
                        .typed_value(&value, Arc::make_mut(&mut self.symbols))
                        .map_err(unsupported)?,
                ));
            }
        }
        if action == Some(key(&self.symbols, "ATTACK"))
            && let Some(actor) = self.planning.actor
        {
            self.world
                .performing_card(actor, &context)
                .map_err(unsupported)?;
        }
        let next_id = format!("eff_e_{}", self.world.next_entity_id);
        let event = !self
            .world
            .effects
            .instances
            .iter()
            .any(|old| self.symbols.name(old.id) == next_id);
        let split = match get(
            &template.definition.fields,
            key(&self.symbols, "split_axis"),
        ) {
            Some(Datum::Text(value)) => Some(*value),
            _ => None,
        };
        let split_value = template.definition.behavior.split;
        let isolated = objects
            .arena
            .field(step, key(&self.symbols, "isolated_hex"));
        Ok(CreatePlan::Create {
            spec: Box::new(EffectCreateSpec { template }),
            scope: Some(scope),
            copy_scope,
            finishing: EffectFinishing::Templates(finishing),
            isolated,
            effects,
            context,
            event,
            split,
            split_value,
            color,
        })
    }
    fn preflight_effect_collection_bound(
        &self,
        objects: &BoundObjects,
        template: &EffectInstance,
    ) -> Result<Option<ObjectId>> {
        if template.linked_card.is_some()
            && self.world.effects.instances.iter().any(|old| {
                old.card == template.card
                    && old.definition.kind == template.definition.kind
                    && old.definition.affects == template.definition.affects
            })
        {
            return Ok(None);
        }
        let Some(ObjectValue::Object(effects)) = objects.arena.root_field(objects.effects) else {
            return Err(unsupported("effect collection missing"));
        };
        if !objects.arena.is_mutable_list(effects)
            || objects.effect_nodes()?.len() != self.world.effects.instances.len()
        {
            return Err(unsupported("effect collection binding drift"));
        }
        self.world
            .next_entity_id
            .checked_add(1)
            .ok_or_else(|| unsupported("effect entity counter exhausted"))?;
        Ok(Some(effects))
    }
    pub(super) fn preflight_manager_effect_bound(
        &mut self,
        objects: &BoundObjects,
        overrides: Vec<(&'static str, Datum)>,
        scope_overrides: Vec<(&'static str, Datum)>,
    ) -> Result<CreatePlan> {
        let Datum::Object(mut fields) = objects.effect_creation.prototype.clone() else {
            unreachable!()
        };
        for (name, value) in overrides {
            set(&mut fields, key(&self.symbols, name), value)?;
        }
        let scope_key = key(&self.symbols, "scope");
        let Some(Datum::Object(scope)) = fields
            .iter_mut()
            .find(|(k, _)| *k == scope_key)
            .map(|(_, v)| v)
        else {
            unreachable!()
        };
        for (name, value) in scope_overrides {
            set(scope, key(&self.symbols, name), value)?;
        }
        let template = EffectInstance::from_datum(&Datum::Object(fields), &self.symbols)
            .map_err(unsupported)?;
        if !objects
            .effect_creation
            .kinds
            .contains(&template.definition.kind)
        {
            return Err(unsupported("invalid manager effect kind"));
        }
        if !template.definition.finishing.is_empty() {
            return Err(unsupported("manager finishing program not bound"));
        }
        let Some(effects) = self.preflight_effect_collection_bound(objects, &template)? else {
            return Ok(CreatePlan::Skip);
        };
        Ok(CreatePlan::Create {
            spec: Box::new(EffectCreateSpec { template }),
            scope: None,
            copy_scope: false,
            finishing: EffectFinishing::Bound(Vec::new()),
            isolated: None,
            effects,
            context: SparseRegisters::default(),
            event: false,
            split: None,
            split_value: 0,
            color: None,
        })
    }
    fn allocate_effect_finishing(
        &mut self,
        objects: &mut BoundObjects,
        plan: FinishingPlan,
    ) -> Result<ObjectId> {
        let mut original = plan.original;
        if let Some(color) = plan.color {
            let copy = objects.arena.shallow_step(original).map_err(unsupported)?;
            objects.bind_copied_program(
                copy,
                objects
                    .program(original)
                    .ok_or_else(|| unsupported("finishing program missing"))?,
            );
            objects
                .arena
                .set_field(
                    copy,
                    key(&self.symbols, "color"),
                    ObjectValue::Atom(Datum::Text(color)),
                )
                .map_err(unsupported)?;
            original = copy;
        }
        for (field, children) in plan.children {
            let mut values = Vec::new();
            for child in children {
                values.push(ObjectValue::Object(
                    self.resolve_effect_finishing(objects, child)?,
                ));
            }
            let list = objects.arena.list(values);
            let copy = objects.arena.shallow_step(original).map_err(unsupported)?;
            objects.bind_copied_program(
                copy,
                objects
                    .program(original)
                    .ok_or_else(|| unsupported("finishing program missing"))?,
            );
            objects
                .arena
                .set_field(copy, field, list)
                .map_err(unsupported)?;
            original = copy;
        }
        Ok(original)
    }
    fn resolve_effect_finishing(
        &mut self,
        objects: &mut BoundObjects,
        plan: FinishingPlan,
    ) -> Result<ObjectId> {
        self.allocate_effect_finishing(objects, plan)
    }
    pub(super) fn create_effect_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: CreatePlan,
    ) -> Result<Option<NativeEvent>> {
        let CreatePlan::Create {
            mut spec,
            scope,
            copy_scope,
            finishing,
            isolated,
            effects,
            context,
            event,
            split,
            split_value,
            color,
        } = plan
        else {
            return Ok(None);
        };
        let mut steps = Vec::new();
        match finishing {
            EffectFinishing::Templates(plans) => {
                for plan in plans {
                    let step = self.resolve_effect_finishing(objects, plan)?;
                    let copy = objects.copy_template(step)?.0;
                    Self::validate_returned_step_bound(objects, copy)?;
                    steps.push(ObjectValue::Object(copy));
                }
            }
            EffectFinishing::Bound(programs) => {
                for step in programs {
                    Self::validate_returned_step_bound(objects, step)?;
                    steps.push(ObjectValue::Object(step));
                }
            }
        }
        let finishing = objects.arena.list(steps);
        let payload = objects
            .arena
            .typed_value(&finishing, Arc::make_mut(&mut self.symbols))
            .map_err(unsupported)?;
        let Datum::List(values) = &payload else {
            unreachable!()
        };
        let definition = Arc::make_mut(&mut spec.template.definition);
        definition.finishing = values.clone();
        set(
            &mut definition.fields,
            key(&self.symbols, "finishing_steps"),
            payload,
        )?;
        let before = self
            .world
            .cards()
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        let row = self
            .world
            .create_effect(
                &spec,
                &CreationContext {
                    round: self.round.round,
                    turn: self.world.combat.turn,
                    actor: self.planning.actor,
                    registers: &context,
                },
                Arc::make_mut(&mut self.symbols),
            )
            .map_err(unsupported)?;
        objects.arena.symbols = self.symbols.clone();
        let instance = &self.world.effects.instances[row];
        let scope = if let Some(scope) = scope {
            if copy_scope || instance.scope_origin != spec.template.scope_origin {
                let fields = objects
                    .arena
                    .fields(scope)
                    .ok_or_else(|| unsupported("effect scope fields missing"))?
                    .to_vec();
                let ObjectValue::Object(copy) =
                    objects.arena.dictionary(fields).map_err(unsupported)?
                else {
                    unreachable!()
                };
                objects
                    .arena
                    .classify_recipe_object(copy, &[], ObjectKind::Model)
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_field(
                        copy,
                        key(&self.symbols, "origin_id"),
                        ObjectValue::Atom(instance.scope_origin.map_or(Datum::Null, Datum::Text)),
                    )
                    .map_err(unsupported)?;
                copy
            } else {
                scope
            }
        } else {
            let payload = instance.payload(&self.symbols);
            let Datum::Object(fields) = payload else {
                unreachable!()
            };
            let value = get(&fields, key(&self.symbols, "scope"))
                .ok_or_else(|| unsupported("manager scope missing"))?;
            let ObjectValue::Object(scope) = objects.arena.allocate(value) else {
                unreachable!()
            };
            objects
                .arena
                .classify_recipe_object(scope, &[], ObjectKind::Model)
                .map_err(unsupported)?;
            scope
        };
        let ObjectValue::Object(model) = objects.arena.allocate(&instance.payload(&self.symbols))
        else {
            unreachable!()
        };
        objects
            .arena
            .classify_recipe_object(model, &[], ObjectKind::Model)
            .map_err(unsupported)?;
        objects
            .arena
            .set_field(
                model,
                key(&self.symbols, "scope"),
                ObjectValue::Object(scope),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .set_field(model, key(&self.symbols, "finishing_steps"), finishing)
            .map_err(unsupported)?;
        if let Some(value) = isolated {
            objects
                .arena
                .set_field(model, key(&self.symbols, "isolated_hex"), value)
                .map_err(unsupported)?;
        }
        objects
            .arena
            .append(effects, ObjectValue::Object(model))
            .map_err(unsupported)?;
        objects
            .arena
            .set_root(
                key(&self.symbols, "next_entity_id"),
                ObjectValue::Atom(Datum::Int(self.world.next_entity_id)),
            )
            .map_err(unsupported)?;
        let created = crate::effect_steps::EffectCreated {
            actor: instance.source,
            kind: instance.definition.kind,
            duration: match get(&instance.definition.fields, key(&self.symbols, "duration")) {
                Some(Datum::Text(value)) => *value,
                _ => return Err(unsupported("effect duration missing")),
            },
            split,
            split_value,
            color,
        };
        self.world.effects.instances[row].arena_model = Some(model);
        self.sync_effect_card_flags(objects, &before)?;
        objects.readonly.take();
        Ok(event.then_some(NativeEvent::EffectCreated(created)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{EngineError, HashSet, fixture_case};
    use super::*;
    fn session(name: &str) -> NativeSession {
        let fixture = fixture_case(name);
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap()
    }
    #[test]
    fn live_creation_workers_rebind_effect_payload_and_scope_without_leaking() {
        let root = session("object-execution-live-effect-step-plain");
        let before = root.export_envelope().unwrap();
        let mut ordinary = root.clone();
        let mut changed = root.clone();
        let objects = changed.objects.as_mut().unwrap();
        let ObjectValue::Object(step) = objects
            .arena
            .context_value(changed.symbols.lookup("create_alias").unwrap())
            .unwrap()
        else {
            panic!("create step missing")
        };
        let ObjectValue::Object(scope) = objects
            .arena
            .field(step, key(&changed.symbols, "scope"))
            .unwrap()
        else {
            panic!("scope missing")
        };
        objects
            .arena
            .set_field(
                scope,
                key(&changed.symbols, "range"),
                ObjectValue::Atom(Datum::Int(7)),
            )
            .unwrap();
        for (field, value) in [
            (
                "effect_type",
                Datum::Text(changed.symbols.lookup("area_stat_modifier").unwrap()),
            ),
            (
                "stat_type",
                Datum::Text(changed.symbols.lookup("ATTACK").unwrap()),
            ),
            ("stat_value", Datum::Int(-9)),
        ] {
            objects
                .arena
                .set_field(step, key(&changed.symbols, field), ObjectValue::Atom(value))
                .unwrap();
        }
        ordinary.advance_prefix(1).unwrap();
        changed.advance_prefix(1).unwrap();
        assert_eq!(
            ordinary.world.effects.instances[0]
                .definition
                .behavior
                .range,
            2
        );
        assert_eq!(
            changed.world.effects.instances[0].definition.behavior.range,
            7
        );
        assert_eq!(
            changed.world.effects.instances[0].definition.behavior.value,
            -9
        );
        assert_eq!(
            changed.world.effects.instances[0].definition.behavior.stat,
            Some(crate::stats::Stat::Attack)
        );
        assert_eq!(
            changed.world.effects.instances[0].definition.behavior.kind,
            crate::effect_rules::Kind::AreaStat
        );
        let after = root.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
    }
    #[test]
    fn finishing_constructor_validation_splits_nested_step_aliases() {
        let mut worker = session("object-execution-live-effect-create-alias-nested-aliases");
        worker.advance_prefix(1).unwrap();
        let objects = worker.objects.as_ref().unwrap();
        let model = objects.effect_nodes().unwrap()[0];
        let roots = objects
            .arena
            .sequence(objects.finishing_list(model).unwrap())
            .unwrap();
        let mut copies = HashSet::new();
        for root in roots {
            let ObjectValue::Object(root) = root else {
                panic!("root missing")
            };
            assert!(copies.insert(*root));
            let ObjectValue::Object(list) = objects
                .arena
                .field(*root, key(&worker.symbols, "steps_template"))
                .unwrap()
            else {
                panic!("children missing")
            };
            for child in objects.arena.sequence(list).unwrap() {
                let ObjectValue::Object(child) = child else {
                    panic!("child missing")
                };
                assert!(copies.insert(*child));
            }
        }
        assert_eq!(copies.len(), 6);
    }
    #[test]
    fn effect_counter_guard_is_atomic_and_duplicates_need_no_new_counter() {
        for (name, duplicate) in [
            ("object-execution-live-effect-step-plain", false),
            ("object-execution-live-effect-step-duplicate-dormant", true),
        ] {
            let mut worker = session(name);
            worker.world.next_entity_id = i64::MAX;
            let objects = worker.objects.as_mut().unwrap();
            objects
                .arena
                .set_root(
                    key(&worker.symbols, "next_entity_id"),
                    ObjectValue::Atom(Datum::Int(i64::MAX)),
                )
                .unwrap();
            let before = worker.export_envelope().unwrap();
            let count = worker.world.effects.instances.len();
            let result = worker.advance_prefix(1);
            if duplicate {
                assert!(result.unwrap().events.is_empty());
            } else {
                assert!(
                    matches!(result,Err(EngineError::Unsupported(message)) if message.contains("entity counter exhausted"))
                );
                let after = worker.export_envelope().unwrap();
                assert_eq!(after.state, before.state);
                assert_eq!(after.object_bindings, before.object_bindings);
            }
            assert_eq!(worker.world.effects.instances.len(), count);
            assert_eq!(worker.world.next_entity_id, i64::MAX);
        }
    }
}
