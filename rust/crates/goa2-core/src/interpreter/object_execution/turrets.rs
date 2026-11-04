//! Persistent turret models and spatial updates without token effect expiry.
use super::phases::Field;
use super::placement::Placement;
use super::*;
pub(super) enum TurretPlan {
    Fail,
    Place {
        placement: Placement,
        model: Option<ObjectId>,
        owner: Datum,
        actor: Option<Symbol>,
        name: Symbol,
        obstacle: bool,
        output: Option<Symbol>,
    },
    Remove {
        id: Symbol,
        from: Hex,
        output: Option<Symbol>,
    },
}
impl NativeSession {
    fn turret_model_bound(&self, objects: &BoundObjects) -> Result<Option<ObjectId>> {
        let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
        let Some(value) = objects.arena.field(misc, objects.phases.k(Field::TurretId)) else {
            return Ok(None);
        };
        let ObjectValue::Object(model) = value else {
            return Err(unsupported(
                "turret identity collides with a physical entity",
            ));
        };
        match objects
            .arena
            .field(model, objects.phases.k(Field::EntityKind))
        {
            Some(ObjectValue::Atom(Datum::Text(kind))) if self.symbols.name(kind) == "turret" => {
                Ok(Some(model))
            }
            _ => Err(unsupported(
                "turret identity collides with a different entity",
            )),
        }
    }
    pub(super) fn preflight_turret_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        remove: bool,
    ) -> Result<TurretPlan> {
        let id = objects.phases.k(Field::TurretId);
        let output = self.spatial_text_bound(objects, step, Field::Output)?;
        if remove {
            let model = match self.turret_model_bound(objects) {
                Ok(model) => model,
                Err(_) => return Ok(TurretPlan::Fail),
            };
            if model.is_none() {
                return Ok(TurretPlan::Fail);
            }
            let Some(from) = self.world.position(id) else {
                return Ok(TurretPlan::Fail);
            };
            self.preflight_remove_spatial_bound(objects, id)?;
            return Ok(TurretPlan::Remove { id, from, output });
        }
        let key = self.passive_text_bound(objects, step, Field::HexKey)?;
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(TurretPlan::Fail);
        };
        let (to, target) = self.spatial_hex_bound(objects, &value)?;
        self.tile_bound(objects, to)?;
        if self.world.occupant(to).is_some_and(|other| other != id) {
            return Ok(TurretPlan::Fail);
        }
        let mut owner = self
            .passive_optional_text_bound(objects, step, Field::TokenOwner)?
            .map_or(Datum::Null, Datum::Text);
        if let Some(key) = self.spatial_text_bound(objects, step, Field::OwnerIdKey)?
            && let Some(value) = objects.arena.context_value(key)
        {
            owner = match value {
                ObjectValue::Atom(value) => value,
                _ => return Err(unsupported("turret aggregate owner not bound")),
            };
        }
        if matches!(owner, Datum::Null)
            && let Some(actor) = self.planning.actor
        {
            owner = Datum::Text(actor);
        }
        let model = self.turret_model_bound(objects)?;
        if model.is_none() && self.world.entity(id).is_some() {
            return Err(unsupported("turret registration ID collision"));
        }
        if model.is_none() && !matches!(owner, Datum::Null | Datum::Text(_)) {
            return Err(unsupported("new turret owner requires a string"));
        }
        let actor = match &owner {
            Datum::Text(id) if !self.symbols.name(*id).is_empty() => Some(*id),
            Datum::Text(_) | Datum::Null | Datum::Bool(false) | Datum::Int(0) => None,
            Datum::Float(value) if *value == 0.0 => None,
            _ => return Err(unsupported("turret owner event coercion not bound")),
        };
        let (name, obstacle) = if let Some(model) = model {
            if self.passive_text_bound(objects, model, Field::EntityId)? != id {
                return Err(unsupported("turret dictionary identity drift"));
            }
            (
                self.passive_text_bound(objects, model, Field::EntityName)?,
                self.spatial_bool_bound(objects, model, Field::IsObstacle)?,
            )
        } else {
            (
                self.symbols
                    .lookup("Turret")
                    .ok_or_else(|| unsupported("turret default name missing"))?,
                true,
            )
        };
        Ok(TurretPlan::Place {
            placement: self.prepare_spatial_bound(objects, id, to, target)?,
            model,
            owner,
            actor,
            name,
            obstacle,
            output,
        })
    }
    pub(super) fn turret_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: TurretPlan,
        mandatory: bool,
    ) -> Result<Vec<NativeEvent>> {
        match plan {
            TurretPlan::Fail => {
                if mandatory {
                    self.abort_objects(objects)?;
                }
                Ok(Vec::new())
            }
            TurretPlan::Remove { id, from, output } => {
                self.remove_entity_spatial_bound(objects, id)?;
                if let Some(key) = output {
                    objects
                        .arena
                        .set_context(key, ObjectValue::Atom(Datum::Text(id)))
                        .map_err(unsupported)?;
                }
                Ok(vec![NativeEvent::BoardEntityRemoved {
                    actor: self
                        .planning
                        .actor
                        .filter(|actor| !self.symbols.name(*actor).is_empty()),
                    entity: id,
                    from,
                }])
            }
            TurretPlan::Place {
                placement,
                model,
                owner,
                actor,
                name,
                obstacle,
                output,
            } => {
                let id = objects.phases.k(Field::TurretId);
                let model = if let Some(model) = model {
                    model
                } else {
                    let ObjectValue::Object(model) =
                        objects.arena.allocate(&objects.tokens.turret_default)
                    else {
                        return Err(unsupported("turret model default missing"));
                    };
                    objects
                        .arena
                        .classify_recipe_object(model, &[], ObjectKind::Model)
                        .map_err(unsupported)?;
                    let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
                    objects
                        .arena
                        .set_field(misc, id, ObjectValue::Object(model))
                        .map_err(unsupported)?;
                    model
                };
                let dense_owner = match &owner {
                    Datum::Text(id) => Some(*id),
                    _ => None,
                };
                objects
                    .arena
                    .set_field(
                        model,
                        objects.phases.k(Field::TokenOwner),
                        ObjectValue::Atom(owner),
                    )
                    .map_err(unsupported)?;
                self.world
                    .upsert_board_entity(id, name, dense_owner)
                    .map_err(unsupported)?;
                self.apply_spatial_bound(objects, &placement)?;
                if let Some(key) = output {
                    objects
                        .arena
                        .set_context(key, ObjectValue::Atom(Datum::Text(id)))
                        .map_err(unsupported)?;
                }
                objects.readonly.take();
                Ok(vec![NativeEvent::BoardEntityPlaced {
                    actor,
                    entity: id,
                    from: placement.from,
                    to: placement.to,
                    obstacle,
                }])
            }
        }
    }
}
