//! Source-ordered token supply, physical properties, replacement and overflow.
use super::passive_bodies::Kind;
use super::phases::Field;
use super::placement::Placement;
use super::*;

#[derive(Clone, Copy, Debug)]
struct TokenInfo {
    id: Symbol,
    kind: Symbol,
    owner: Option<Symbol>,
    immune: bool,
    persists: bool,
}
#[derive(Clone, Debug)]
pub(super) struct TokenBindings {
    base: Arc<[Option<TokenInfo>]>,
    kinds: Arc<[Symbol]>,
    pub(super) batch_defaults: Arc<HashMap<&'static str, Datum>>,
    pub(super) turret_default: Datum,
}
impl TokenBindings {
    pub(super) fn compile(source: &StateEnvelope, arena: &mut ObjectArena) -> Result<Self> {
        let mut base = Vec::new();
        for (index, instance) in source.instances.iter().enumerate() {
            if source.definitions[instance.definition as usize].kind
                != crate::codec::InstanceKind::Token
            {
                base.push(None);
                continue;
            }
            let raw = source
                .expand(&json!({"$goa2_instance":index}))
                .map_err(unsupported)?;
            let symbols = Arc::make_mut(&mut arena.symbols);
            base.push(Some(TokenInfo {
                id: symbols.intern(
                    raw["id"]
                        .as_str()
                        .ok_or_else(|| unsupported("token identity missing"))?,
                ),
                kind: symbols.intern(
                    raw["token_type"]
                        .as_str()
                        .ok_or_else(|| unsupported("token type missing"))?,
                ),
                owner: raw["owner_id"].as_str().map(|id| symbols.intern(id)),
                persists: raw["persists_end_of_round"]
                    .as_bool()
                    .ok_or_else(|| unsupported("token persistence missing"))?,
                immune: raw["is_immune_to_enemy_actions"]
                    .as_bool()
                    .ok_or_else(|| unsupported("token immunity missing"))?,
            }));
        }
        let kinds = crate::templates::instructions().map_err(unsupported)?["token_types"]
            .as_array()
            .ok_or_else(|| unsupported("token enum metadata missing"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|name| Arc::make_mut(&mut arena.symbols).intern(name))
                    .ok_or_else(|| unsupported("token enum invalid"))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut batch_defaults = HashMap::new();
        for name in ["obstacle", "hex", "removal"] {
            let raw = &crate::templates::instructions().map_err(unsupported)?["token_batch_filters"]
                [name];
            batch_defaults.insert(
                name,
                Datum::from_json(raw, Arc::make_mut(&mut arena.symbols)).map_err(unsupported)?,
            );
        }
        let turret_default = Datum::from_json(
            &crate::templates::instructions().map_err(unsupported)?["turret_default"],
            Arc::make_mut(&mut arena.symbols),
        )
        .map_err(unsupported)?;
        Ok(Self {
            turret_default,
            batch_defaults: Arc::new(batch_defaults),
            base: base.into(),
            kinds: kinds.into(),
        })
    }
    fn get(&self, objects: &BoundObjects, physical: u32) -> Result<Option<TokenInfo>> {
        let base = objects.arena.physical_base(physical).map_err(unsupported)?;
        let Some(mut row) = self.base.get(base as usize).copied().flatten() else {
            return Ok(None);
        };
        if let Some(value) = objects
            .arena
            .physical_override(physical, objects.phases.k(Field::TokenOwner))
        {
            row.owner = match value {
                Some(Datum::Text(owner)) => Some(*owner),
                None | Some(Datum::Null) => None,
                _ => return Err(unsupported("token owner override invalid")),
            };
        }
        if let Some(value) = objects
            .arena
            .physical_override(physical, objects.phases.k(Field::ImmuneActions))
        {
            row.immune = match value {
                Some(Datum::Bool(value)) => *value,
                None => false,
                _ => return Err(unsupported("token immunity override invalid")),
            };
        }
        if let Some(key) = objects.arena.symbols.lookup("persists_end_of_round")
            && let Some(value) = objects.arena.physical_override(physical, key)
        {
            row.persists = match value {
                Some(Datum::Bool(value)) => *value,
                _ => return Err(unsupported("token persistence override invalid")),
            };
        }
        Ok(Some(row))
    }
}
pub(super) enum TokenPlacePlan {
    Skip,
    Echo(Symbol, Option<Symbol>),
    Place {
        placement: Placement,
        physical: Option<u32>,
        unit: Symbol,
        kind: Symbol,
        owner: Option<Symbol>,
        actor: Option<Symbol>,
        immune: bool,
        output: Option<Symbol>,
        from: Option<Hex>,
    },
    Overflow {
        kind: Symbol,
        hex: Symbol,
        owner: Option<Symbol>,
        output: Option<Symbol>,
        existing: Option<Symbol>,
        selection: Symbol,
        immune: bool,
    },
}
pub(super) struct TokenRemovePlan {
    unit: Symbol,
    from: Hex,
    effects: usize,
}
impl NativeSession {
    fn token_misc_bound(&self, objects: &BoundObjects, id: Symbol) -> Result<Option<u32>> {
        let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
        let Some(ObjectValue::Atom(Datum::Instance(physical))) = objects.arena.field(misc, id)
        else {
            return Ok(None);
        };
        Ok(objects.tokens.get(objects, physical)?.map(|_| physical))
    }
    pub(super) fn token_is_public_bound(&self, objects: &BoundObjects, id: Symbol) -> Result<bool> {
        if self.world.position(id).is_none() {
            return Ok(false);
        }
        let Some(physical) = self.token_misc_bound(objects, id)? else {
            return Ok(false);
        };
        let key = objects.phases.k(Field::Facedown);
        if let Some(value) = objects.arena.physical_override(physical, key) {
            return Ok(matches!(value, Some(Datum::Bool(false))));
        }
        let base = objects.arena.physical_base(physical).map_err(unsupported)?;
        let instance = &self.source.instances[base as usize];
        let definition = &self.source.definitions[instance.definition as usize];
        Ok(instance.fields.get("is_facedown")
            .or_else(|| definition.fields.get("is_facedown"))
            .and_then(Value::as_bool) == Some(false))
    }

    pub(super) fn cleanup_token_supply_bound(
        &self,
        objects: &BoundObjects,
    ) -> Result<Vec<(Symbol, bool)>> {
        let pool = objects.phases.root(&objects.arena, Field::TokenPool)?;
        let mut result = Vec::new();
        for (_, value) in objects
            .arena
            .fields(pool)
            .ok_or_else(|| unsupported("token pool must be a mapping"))?
        {
            let ObjectValue::Object(list) = value else {
                return Err(unsupported("token supply list missing"));
            };
            for token in objects
                .arena
                .sequence(*list)
                .ok_or_else(|| unsupported("token supply must be a list"))?
            {
                let ObjectValue::Atom(Datum::Instance(id)) = token else {
                    return Err(unsupported("pool token physical instance missing"));
                };
                let row = objects
                    .tokens
                    .get(objects, *id)?
                    .ok_or_else(|| unsupported("pool token metadata missing"))?;
                result.push((row.id, row.persists));
            }
        }
        Ok(result)
    }
    pub(super) fn token_effective_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<(Symbol, Symbol)> {
        let authored = self.passive_text_bound(objects, step, Field::TokenType)?;
        let mut kind = authored;
        if let Some(actor) = self.planning.actor {
            let owner = self.world.hero(actor).map_or(actor, |hero| hero.id);
            if matches!(objects.arena.context_value(objects.phases.k(Field::SubstitutionActor)),Some(ObjectValue::Atom(Datum::Text(id))) if id==owner)
                && let Some(value) = objects
                    .arena
                    .context_value(objects.phases.k(Field::TokenOverride))
                    .filter(|value| objects.arena.truthy(value))
            {
                kind = match value {
                    ObjectValue::Atom(Datum::Text(id)) => id,
                    _ => return Err(unsupported("token type substitution coercion not bound")),
                };
            }
        }
        if !objects.tokens.kinds.contains(&kind) {
            return Err(unsupported("unknown effective token type"));
        }
        Ok((authored, kind))
    }
    pub(super) fn token_on_board_bound(
        &self,
        objects: &BoundObjects,
        kind: Symbol,
    ) -> Result<Vec<(Symbol, Hex)>> {
        let pool = objects.phases.root(&objects.arena, Field::TokenPool)?;
        let Some(ObjectValue::Object(list)) = objects.arena.field(pool, kind) else {
            return Ok(Vec::new());
        };
        let mut result = Vec::new();
        for value in objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("token pool list missing"))?
        {
            let ObjectValue::Atom(Datum::Instance(id)) = value else {
                return Err(unsupported("token pool instance missing"));
            };
            let row = objects
                .tokens
                .get(objects, *id)?
                .ok_or_else(|| unsupported("token pool contains non-token"))?;
            if let Some(position) = self.world.position(row.id) {
                result.push((row.id, position));
            }
        }
        Ok(result)
    }
    pub(super) fn token_supply_bound(
        &self,
        objects: &BoundObjects,
        kind: Symbol,
    ) -> Result<(usize, usize)> {
        let pool = objects.phases.root(&objects.arena, Field::TokenPool)?;
        let Some(value) = objects.arena.field(pool, kind) else {
            return Ok((0, 0));
        };
        let ObjectValue::Object(list) = value else {
            return Err(unsupported("token supply list missing"));
        };
        let mut total = 0;
        let mut placed = 0;
        for value in objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("token supply list missing"))?
        {
            let ObjectValue::Atom(Datum::Instance(id)) = value else {
                return Err(unsupported("token supply instance missing"));
            };
            let row = objects
                .tokens
                .get(objects, *id)?
                .ok_or_else(|| unsupported("token supply contains non-token"))?;
            total += 1;
            placed += usize::from(self.world.position(row.id).is_some());
        }
        Ok((total, placed))
    }
    fn token_owner_bound(
        &mut self,
        objects: &mut BoundObjects,
        key: Option<Symbol>,
    ) -> Result<(Option<Symbol>, Option<Symbol>)> {
        let value = key
            .filter(|id| !self.symbols.name(*id).is_empty())
            .and_then(|key| objects.arena.context_value(key));
        let value = match value {
            None | Some(ObjectValue::Atom(Datum::Null)) => self
                .planning
                .actor
                .map(|id| ObjectValue::Atom(Datum::Text(id))),
            value => value,
        };
        let Some(value) = value else {
            return Ok((None, None));
        };
        let truthy = objects.arena.truthy(&value);
        let id = match value {
            ObjectValue::Atom(Datum::Text(id)) => id,
            ObjectValue::Atom(Datum::Int(value)) => {
                Arc::make_mut(&mut self.symbols).intern(&value.to_string())
            }
            ObjectValue::Atom(Datum::Bool(value)) => {
                Arc::make_mut(&mut self.symbols).intern(if value { "True" } else { "False" })
            }
            _ => return Err(unsupported("token owner coercion not compiled")),
        };
        objects.arena.symbols = self.symbols.clone();
        Ok((Some(id), truthy.then_some(id)))
    }
    pub(super) fn preflight_token_place_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<TokenPlacePlan> {
        let (authored, kind) = self.token_effective_bound(objects, step)?;
        let hex = self.passive_text_bound(objects, step, Field::HexKey)?;
        let Some(value) = objects
            .arena
            .context_value(hex)
            .filter(|value| objects.arena.truthy(value))
        else {
            return Ok(TokenPlacePlan::Skip);
        };
        if matches!(value,ObjectValue::Atom(Datum::Text(id)) if id==self.skip) {
            return Ok(TokenPlacePlan::Skip);
        }
        let (to, target) = self.spatial_hex_bound(objects, &value)?;
        self.tile_bound(objects, to)?;
        if self.world.occupant(to).is_some() {
            return Ok(TokenPlacePlan::Skip);
        }
        let output = self.passive_optional_text_bound(objects, step, Field::Output)?;
        let output_used = output.filter(|key| !self.symbols.name(*key).is_empty());
        let owner_key = self.passive_optional_text_bound(objects, step, Field::OwnerIdKey)?;
        let existing = self.passive_optional_text_bound(objects, step, Field::ExistingTokenKey)?;
        let existing_id = existing
            .filter(|key| !self.symbols.name(*key).is_empty())
            .map(|key| self.truthy_symbol_bound(objects, objects.arena.context_value(key)))
            .transpose()?
            .flatten();
        if let Some(unit) = existing_id {
            let Some(physical) = self.token_misc_bound(objects, unit)? else {
                return Ok(TokenPlacePlan::Skip);
            };
            let info = objects.tokens.get(objects, physical)?.unwrap();
            let Some(from) = self.world.position(unit) else {
                return Ok(TokenPlacePlan::Skip);
            };
            if info.kind != kind {
                return Ok(TokenPlacePlan::Skip);
            }
            if info.id != unit {
                return Err(unsupported("replacement token dictionary identity drift"));
            }
            if from == to {
                return Ok(TokenPlacePlan::Echo(unit, output_used));
            }
            let placement = self.prepare_spatial_bound(objects, unit, to, target)?;
            let (_, actor) = self.token_owner_bound(objects, owner_key)?;
            return Ok(TokenPlacePlan::Place {
                placement,
                physical: None,
                unit,
                kind,
                owner: None,
                actor,
                immune: info.immune,
                output: output_used,
                from: Some(from),
            });
        }
        let pool = objects.phases.root(&objects.arena, Field::TokenPool)?;
        let values = match objects.arena.field(pool, kind) {
            None => return Ok(TokenPlacePlan::Skip),
            Some(ObjectValue::Object(list)) => objects
                .arena
                .sequence(list)
                .ok_or_else(|| unsupported("token pool must be a list"))?
                .to_vec(),
            _ => return Err(unsupported("token pool list missing")),
        };
        let mut any = false;
        for value in values {
            let ObjectValue::Atom(Datum::Instance(physical)) = value else {
                return Err(unsupported("token supply physical instance missing"));
            };
            let info = objects
                .tokens
                .get(objects, physical)?
                .ok_or_else(|| unsupported("token pool contains a non-token"))?;
            any = true;
            if self.world.position(info.id).is_some() {
                continue;
            }
            let placement = self.prepare_spatial_bound(objects, info.id, to, target)?;
            let (owner, actor) = self.token_owner_bound(objects, owner_key)?;
            let immune = if kind != authored {
                false
            } else {
                self.spatial_bool_bound(objects, step, Field::ImmuneActions)?
            };
            return Ok(TokenPlacePlan::Place {
                placement,
                physical: Some(physical),
                unit: info.id,
                kind,
                owner,
                actor,
                immune,
                output: output_used,
                from: None,
            });
        }
        if !any {
            return Ok(TokenPlacePlan::Skip);
        }
        self.phase_generation_check(0, 3)?;
        let recipes = objects
            .scheduler
            .as_ref()
            .unwrap()
            .passive_bodies
            .recipe(Kind::TokenOverflow)?;
        if recipes.len() != 3 {
            return Err(unsupported("token overflow recipe missing"));
        }
        Ok(TokenPlacePlan::Overflow {
            kind,
            hex,
            owner: owner_key,
            output,
            existing,
            selection: self.passive_text_bound(objects, step, Field::OverflowKey)?,
            immune: self.spatial_bool_bound(objects, step, Field::ImmuneActions)?,
        })
    }
    pub(super) fn token_place_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: TokenPlacePlan,
    ) -> Result<Vec<NativeEvent>> {
        match plan {
            TokenPlacePlan::Skip => Ok(Vec::new()),
            TokenPlacePlan::Echo(id, output) => {
                if let Some(output) = output {
                    objects
                        .arena
                        .set_context(output, ObjectValue::Atom(Datum::Text(id)))
                        .map_err(unsupported)?;
                }
                Ok(Vec::new())
            }
            TokenPlacePlan::Place {
                placement,
                physical,
                unit,
                kind,
                owner,
                actor,
                immune,
                output,
                from,
            } => {
                if let Some(physical) = physical {
                    if let Some(owner) = owner {
                        objects
                            .arena
                            .set_physical_field(
                                physical,
                                objects.phases.k(Field::TokenOwner),
                                Datum::Text(owner),
                            )
                            .map_err(unsupported)?;
                    }
                    objects
                        .arena
                        .set_physical_field(
                            physical,
                            objects.phases.k(Field::ImmuneActions),
                            Datum::Bool(immune),
                        )
                        .map_err(unsupported)?;
                    let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
                    let ids = objects
                        .arena
                        .fields(misc)
                        .ok_or_else(|| unsupported("misc entities must be a dictionary"))?
                        .iter()
                        .filter_map(|(key, value)| {
                            matches!(value,ObjectValue::Atom(Datum::Instance(id)) if *id==physical)
                                .then_some(*key)
                        })
                        .collect::<Vec<_>>();
                    let info = objects.tokens.get(objects, physical)?.unwrap();
                    for id in ids {
                        self.world
                            .set_token_properties(id, info.owner, info.immune)
                            .map_err(unsupported)?;
                    }
                }
                self.apply_spatial_bound(objects, &placement)?;
                if let Some(output) = output {
                    objects
                        .arena
                        .set_context(output, ObjectValue::Atom(Datum::Text(unit)))
                        .map_err(unsupported)?;
                }
                Ok(vec![if physical.is_some() {
                    NativeEvent::TokenPlaced {
                        actor,
                        token: unit,
                        to: placement.to,
                        kind,
                    }
                } else {
                    NativeEvent::TokenReplaced {
                        actor,
                        token: unit,
                        from: from.unwrap(),
                        to: placement.to,
                        kind,
                    }
                }])
            }
            TokenPlacePlan::Overflow {
                kind,
                hex,
                owner,
                output,
                existing,
                selection,
                immune,
            } => {
                let program = objects.scheduler.as_ref().unwrap().clone();
                let recipes = program.passive_bodies.recipe(Kind::TokenOverflow)?;
                let mut steps = Vec::new();
                for recipe in recipes {
                    steps.push(self.allocate_source_recipe_bound(objects, recipe, &Datum::Null)?);
                }
                let text = format!(
                    "Select a {} token to remove from the board.",
                    self.symbols.name(kind)
                );
                let prompt = Arc::make_mut(&mut self.symbols).intern(&text);
                objects.arena.symbols = self.symbols.clone();
                let set = |objects: &mut BoundObjects, id, field, value| {
                    objects
                        .arena
                        .set_field(id, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)
                };
                set(objects, steps[0], Field::Prompt, Datum::Text(prompt))?;
                set(objects, steps[0], Field::Output, Datum::Text(selection))?;
                set(
                    objects,
                    steps[0],
                    Field::OverridePlayerKey,
                    owner.map_or(Datum::Null, Datum::Text),
                )?;
                let filters = self.unit_filter_list_bound(objects, steps[0], Field::Filters)?;
                let ObjectValue::Object(filter) = filters[1] else {
                    return Err(unsupported("overflow token filter missing"));
                };
                set(objects, filter, Field::TokenType, Datum::Text(kind))?;
                set(objects, steps[1], Field::TokenKey, Datum::Text(selection))?;
                for (field, value) in [
                    (Field::TokenType, Datum::Text(kind)),
                    (Field::HexKey, Datum::Text(hex)),
                    (Field::OwnerIdKey, owner.map_or(Datum::Null, Datum::Text)),
                    (Field::Output, output.map_or(Datum::Null, Datum::Text)),
                    (
                        Field::ExistingTokenKey,
                        existing.map_or(Datum::Null, Datum::Text),
                    ),
                    (Field::OverflowKey, Datum::Text(selection)),
                    (Field::ImmuneActions, Datum::Bool(immune)),
                ] {
                    set(objects, steps[2], field, value)?;
                }
                self.push_live_steps_bound(objects, &steps)?;
                Ok(Vec::new())
            }
        }
    }
    pub(super) fn preflight_token_remove_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<TokenRemovePlan>> {
        let mut id = self.spatial_text_bound(objects, step, Field::TokenId)?;
        if let Some(key) = self.spatial_text_bound(objects, step, Field::TokenKey)? {
            id = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let Some(unit) = id else {
            return Ok(None);
        };
        if self.token_misc_bound(objects, unit)?.is_none() {
            return Ok(None);
        }
        let Some(from) = self.world.position(unit) else {
            return Ok(None);
        };
        self.preflight_remove_spatial_bound(objects, unit)?;
        if objects.effect_nodes()?.len() != self.world.effects.instances.len() {
            return Err(unsupported("token removal effect binding drift"));
        }
        let effects = self
            .world
            .effects
            .instances
            .iter()
            .filter(|e| e.token == Some(unit) || e.source == unit || e.scope_origin == Some(unit))
            .count();
        Ok(Some(TokenRemovePlan {
            unit,
            from,
            effects,
        }))
    }
    pub(super) fn token_remove_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<TokenRemovePlan>,
    ) -> Result<Vec<NativeEvent>> {
        let Some(plan) = plan else {
            return Ok(Vec::new());
        };
        self.remove_mine_spatial_bound(objects, plan.unit)?;
        Ok(vec![NativeEvent::TokenRemoved {
            actor: self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()),
            token: plan.unit,
            from: plan.from,
            effects_removed: plan.effects,
        }])
    }
}
