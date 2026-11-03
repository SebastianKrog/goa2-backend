//! Coordinate utilities preserve fresh dict versus immutable Hex identities.
use super::phases::Field;
use super::*;

pub(super) enum CoordinatePlan {
    Done,
    Value(Symbol, Datum),
    Hex(Symbol, Hex),
}
impl NativeSession {
    fn coordinate_identity_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        literal: Field,
        key: Field,
    ) -> Result<Option<ObjectValue>> {
        let value = objects.arena.field(step, objects.phases.k(literal));
        if value
            .as_ref()
            .is_some_and(|value| objects.arena.truthy(value))
        {
            return Ok(value);
        }
        let Some(key) = self
            .passive_optional_text_bound(objects, step, key)?
            .filter(|key| !self.symbols.name(*key).is_empty())
        else {
            return Ok(None);
        };
        Ok(objects
            .arena
            .context_value(key)
            .filter(|value| objects.arena.truthy(value)))
    }
    fn coordinate_position_bound(
        &self,
        objects: &BoundObjects,
        value: &ObjectValue,
    ) -> Result<Option<Hex>> {
        let id = match value {
            ObjectValue::Atom(Datum::Text(id)) => *id,
            ObjectValue::Atom(Datum::Int(_) | Datum::Float(_) | Datum::Bool(_) | Datum::Null) => {
                return Ok(None);
            }
            ObjectValue::Object(id)
                if objects.arena.kind(*id) == ObjectKind::List
                    && !objects.arena.is_mutable_list(*id) =>
            {
                return Ok(None);
            }
            _ => return Err(unsupported("unhashable coordinate identity")),
        };
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        let Some(value) = objects.arena.field(locations, id) else {
            return Ok(None);
        };
        if !matches!(&value,ObjectValue::Object(id) if objects.arena.is_shared_frozen_model(*id)) {
            return Err(unsupported("coordinate position requires immutable Hex"));
        }
        Ok(Some(self.spatial_hex_bound(objects, &value)?.0))
    }
    fn coordinate_coerced_position_bound(
        &mut self,
        objects: &BoundObjects,
        value: &ObjectValue,
    ) -> Result<Option<Hex>> {
        let value = objects
            .arena
            .typed_value(value, Arc::make_mut(&mut self.symbols))
            .map_err(unsupported)?;
        let text = value
            .python_scalar_text(&self.symbols)
            .ok_or_else(|| unsupported("coordinate aggregate string conversion not bound"))?;
        let id = Arc::make_mut(&mut self.symbols).intern(&text);
        self.coordinate_position_bound(objects, &ObjectValue::Atom(Datum::Text(id)))
    }
    pub(super) fn preflight_coordinates_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        op: CoordinateOp,
    ) -> Result<CoordinatePlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        match op {
            CoordinateOp::Record => {
                let Some(value) =
                    self.coordinate_identity_bound(objects, step, Field::UnitId, Field::UnitKey)?
                else {
                    return Ok(CoordinatePlan::Done);
                };
                let Some(hex) = self.coordinate_position_bound(objects, &value)? else {
                    return Ok(CoordinatePlan::Done);
                };
                Ok(CoordinatePlan::Value(
                    output,
                    native_hex(hex, self.coordinates),
                ))
            }
            CoordinateOp::Adjacency => {
                let a =
                    self.coordinate_identity_bound(objects, step, Field::UnitAId, Field::UnitAKey)?;
                let b =
                    self.coordinate_identity_bound(objects, step, Field::UnitBId, Field::UnitBKey)?;
                let adjacent = if let Some((a, b)) = a.zip(b) {
                    if let Some((a, b)) = self
                        .coordinate_position_bound(objects, &a)?
                        .zip(self.coordinate_position_bound(objects, &b)?)
                    {
                        a.distance(b) == 1
                            && self
                                .world
                                .effect_connected(a, b, &[])
                                .map_err(unsupported)?
                    } else {
                        false
                    }
                } else {
                    false
                };
                Ok(CoordinatePlan::Value(output, Datum::Bool(adjacent)))
            }
            CoordinateOp::Compute => {
                let origin = if let Some(key) = self
                    .passive_optional_text_bound(objects, step, Field::OriginKey)?
                    .filter(|key| !self.symbols.name(*key).is_empty())
                {
                    objects.arena.context_value(key)
                } else {
                    self.planning
                        .actor
                        .map(|id| ObjectValue::Atom(Datum::Text(id)))
                };
                let Some(origin) = origin.filter(|value| objects.arena.truthy(value)) else {
                    return Ok(CoordinatePlan::Done);
                };
                let Some(origin) = self.coordinate_coerced_position_bound(objects, &origin)? else {
                    return Ok(CoordinatePlan::Done);
                };
                let key = self.passive_text_bound(objects, step, Field::TargetKey)?;
                let Some(target) = objects
                    .arena
                    .context_value(key)
                    .filter(|value| objects.arena.truthy(value))
                else {
                    return Ok(CoordinatePlan::Done);
                };
                let Some(target) = self.coordinate_coerced_position_bound(objects, &target)? else {
                    return Ok(CoordinatePlan::Done);
                };
                let scale = match objects.arena.field(step, objects.phases.k(Field::Scale)) {
                    Some(ObjectValue::Atom(Datum::Int(value))) => value,
                    Some(ObjectValue::Atom(Datum::Bool(value))) => i64::from(value),
                    _ => return Err(unsupported("coordinate scale must be an integer")),
                };
                // Source constructs/validates unit_dir even when scale is zero.
                let distance = i64::from(origin.distance(target));
                if distance == 0 {
                    return Ok(CoordinatePlan::Done);
                }
                let (a, b, c) = origin.coordinates();
                let (q, r, s) = target.coordinates();
                let direction = (
                    (i64::from(q) - i64::from(a)).div_euclid(distance),
                    (i64::from(r) - i64::from(b)).div_euclid(distance),
                    (i64::from(s) - i64::from(c)).div_euclid(distance),
                );
                if direction.0 + direction.1 + direction.2 != 0 {
                    return Err(unsupported(
                        "computed direction violates source Hex validation",
                    ));
                }
                let result =
                    crate::displacement::offset(target, direction, scale).map_err(unsupported)?;
                Ok(CoordinatePlan::Hex(output, result))
            }
        }
    }
    pub(super) fn coordinates_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: CoordinatePlan,
    ) -> Result<()> {
        objects.arena.symbols = self.symbols.clone();
        let (key, value) = match plan {
            CoordinatePlan::Done => return Ok(()),
            CoordinatePlan::Value(key, value) => {
                let value = match value {
                    Datum::Object(fields) => objects
                        .arena
                        .dictionary(
                            fields
                                .into_iter()
                                .map(|(key, value)| (key, ObjectValue::Atom(value)))
                                .collect(),
                        )
                        .map_err(unsupported)?,
                    value => ObjectValue::Atom(value),
                };
                (key, value)
            }
            CoordinatePlan::Hex(key, hex) => {
                let Datum::Object(fields) = native_hex(hex, self.coordinates) else {
                    unreachable!()
                };
                (
                    key,
                    objects
                        .arena
                        .shared_frozen_model(
                            fields
                                .into_iter()
                                .map(|(key, value)| (key, ObjectValue::Atom(value)))
                                .collect(),
                        )
                        .map_err(unsupported)?,
                )
            }
        };
        objects.arena.set_context(key, value).map_err(unsupported)
    }
}
