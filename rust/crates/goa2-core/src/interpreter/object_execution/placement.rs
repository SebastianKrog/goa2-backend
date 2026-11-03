//! Live placement keeps original location/tile containers and Hex aliases.
use super::phases::Field;
use super::*;
use crate::validation::DisplacementKind;

pub(super) enum SwapPlan {
    Skip,
    Blocked,
    Swap(Placement, Placement),
}

pub(super) enum PlacePlan {
    Skip,
    Blocked,
    Place(Placement),
}
pub(super) struct Placement {
    unit: Symbol,
    pub(super) from: Option<Hex>,
    pub(super) to: Hex,
    target: Option<ObjectId>,
    locations: ObjectId,
    old_tile: Option<ObjectId>,
    new_tile: Option<ObjectId>,
    token: bool,
}
impl NativeSession {
    pub(super) fn truthy_symbol_bound(
        &self,
        objects: &BoundObjects,
        value: Option<ObjectValue>,
    ) -> Result<Option<Symbol>> {
        match value.filter(|value| objects.arena.truthy(value)) {
            None => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(value))) => Ok(Some(value)),
            _ => Err(unsupported("placement unit/key must be text")),
        }
    }
    pub(super) fn spatial_hex_bound(
        &self,
        objects: &BoundObjects,
        value: &ObjectValue,
    ) -> Result<(Hex, Option<ObjectId>)> {
        match value {
            ObjectValue::Atom(Datum::Object(_)) => {
                let ObjectValue::Atom(value) = value else {
                    unreachable!()
                };
                Ok((
                    value
                        .hex(self.coordinates, &self.symbols)
                        .map_err(unsupported)?,
                    None,
                ))
            }
            ObjectValue::Object(object) => {
                let shared = objects.arena.is_shared_frozen_model(*object);
                if !shared && objects.arena.kind(*object) != ObjectKind::Dict {
                    return Err(unsupported(
                        "placement coordinates require a dict or immutable Hex",
                    ));
                }
                let fields = self
                    .coordinates
                    .iter()
                    .filter_map(|key| match objects.arena.field(*object, *key) {
                        Some(ObjectValue::Atom(value)) => Some((*key, value)),
                        _ => None,
                    })
                    .collect();
                let hex = Datum::Object(fields)
                    .hex(self.coordinates, &self.symbols)
                    .map_err(unsupported)?;
                Ok((hex, shared.then_some(*object)))
            }
            _ => Err(unsupported(
                "placement coordinates require a dict or immutable Hex",
            )),
        }
    }
    pub(super) fn tile_bound(&self, objects: &BoundObjects, hex: Hex) -> Result<Option<ObjectId>> {
        let Some(index) = self.world.tile_index(hex) else {
            return Ok(None);
        };
        let (tile, consistent) = objects
            .phases
            .tiles
            .get(index)
            .copied()
            .ok_or_else(|| unsupported("tile binding count drift"))?;
        if !consistent {
            return Err(unsupported(
                "shared/mismatched board tile bindings are not supported",
            ));
        }
        if !matches!(
            objects.arena.kind(tile),
            ObjectKind::Dict | ObjectKind::Model
        ) {
            return Err(unsupported("placement tile must be mutable"));
        }
        let occupant = match objects.arena.field(tile, objects.phases.k(Field::Occupant)) {
            Some(ObjectValue::Atom(Datum::Null)) => None,
            Some(ObjectValue::Atom(Datum::Text(id))) => Some(id),
            _ => return Err(unsupported("tile occupant binding missing")),
        };
        if occupant != self.world.occupant(hex) {
            return Err(unsupported("tile occupancy cache drift"));
        }
        Ok(Some(tile))
    }
    pub(super) fn preflight_swap_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<SwapPlan> {
        let resolve = |literal, key| -> Result<Option<Symbol>> {
            let direct = self.truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(literal)),
            )?;
            if direct.is_some() {
                return Ok(direct);
            }
            let key = self
                .truthy_symbol_bound(objects, objects.arena.field(step, objects.phases.k(key)))?;
            key.map(|key| self.truthy_symbol_bound(objects, objects.arena.context_value(key)))
                .transpose()
                .map(Option::flatten)
        };
        let a = resolve(Field::UnitAId, Field::UnitAKey)?;
        let b = resolve(Field::UnitBId, Field::UnitBKey)?;
        let (Some(a), Some(b)) = (a, b) else {
            return Ok(SwapPlan::Skip);
        };
        if a == self.skip || b == self.skip {
            return Ok(SwapPlan::Skip);
        }
        let (Some(from), Some(to)) = (self.world.position(a), self.world.position(b)) else {
            return Ok(SwapPlan::Skip);
        };
        self.sync_card_collections_bound(objects)?;
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty())
            .unwrap_or(objects.phases.k(Field::SystemName));
        for unit in [a, b] {
            if !self
                .world
                .can_displace(unit, Some(actor), DisplacementKind::Swap, None)
                .map_err(unsupported)?
                .allowed
            {
                return Ok(SwapPlan::Blocked);
            }
        }
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        let target = |unit| -> Result<ObjectId> {
            match objects.arena.field(locations, unit) {
                Some(ObjectValue::Object(hex)) if objects.arena.is_shared_frozen_model(hex) => {
                    Ok(hex)
                }
                _ => Err(unsupported("swap location must retain its immutable Hex")),
            }
        };
        Ok(SwapPlan::Swap(
            self.prepare_spatial_bound(objects, a, to, Some(target(b)?))?,
            self.prepare_spatial_bound(objects, b, from, Some(target(a)?))?,
        ))
    }
    pub(super) fn swap_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: SwapPlan,
        mandatory: bool,
    ) -> Result<Option<NativeEvent>> {
        let (mut a, mut b) = match plan {
            SwapPlan::Skip => return Ok(None),
            SwapPlan::Blocked => {
                if mandatory {
                    self.abort_objects(objects)?;
                }
                return Ok(None);
            }
            SwapPlan::Swap(a, b) => (a, b),
        };
        // Python removes both entries before placing either one. Keep the same
        // dictionary and Hex objects, including a self-swap's delete/reinsert.
        for placement in [&mut a, &mut b] {
            self.world
                .remove_entity(placement.unit)
                .map_err(EngineError::Rule)?;
            if objects
                .arena
                .field(placement.locations, placement.unit)
                .is_some()
            {
                objects
                    .arena
                    .apply(&crate::objects::Mutation::Delete {
                        target: crate::objects::Target::Object(placement.locations),
                        key: placement.unit,
                    })
                    .map_err(unsupported)?;
            }
            let occupant = objects.phases.k(Field::Occupant);
            if let Some(tile) = placement.old_tile
                && matches!(objects.arena.field(tile, occupant), Some(ObjectValue::Atom(Datum::Text(id))) if id == placement.unit)
            {
                objects
                    .arena
                    .set_field(tile, occupant, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
            }
            placement.old_tile = None;
        }
        self.apply_spatial_bound(objects, &a)?;
        self.apply_spatial_bound(objects, &b)?;
        Ok(Some(NativeEvent::Swapped {
            a: a.unit,
            b: b.unit,
            from: b.to,
            to: a.to,
        }))
    }
    pub(super) fn preflight_place_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<PlacePlan> {
        let mut unit = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::UnitId)),
        )?;
        if unit.is_none() {
            let key = self.truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(Field::UnitKey)),
            )?;
            if let Some(key) = key {
                unit = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
            }
        }
        let unit = unit.or(self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty()));
        let Some(unit) = unit else {
            return Ok(PlacePlan::Skip);
        };
        let mut value = objects
            .arena
            .field(step, objects.phases.k(Field::TargetHex))
            .filter(|value| objects.arena.truthy(value));
        if value.is_none() {
            let Some(ObjectValue::Atom(Datum::Text(key))) = objects
                .arena
                .field(step, objects.phases.k(Field::DestinationKey))
            else {
                return Err(unsupported("placement destination key must be text"));
            };
            value = objects
                .arena
                .context_value(key)
                .filter(|value| objects.arena.truthy(value));
        }
        let Some(value) = value else {
            return Ok(PlacePlan::Skip);
        };
        if matches!(value,ObjectValue::Atom(Datum::Text(id)) if id == self.skip) {
            return Ok(PlacePlan::Skip);
        }
        let (to, target) = self.spatial_hex_bound(objects, &value)?;
        if self.world.occupant(to).is_some_and(|other| other != unit) {
            return Ok(PlacePlan::Skip);
        }
        self.sync_card_collections_bound(objects)?;
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty())
            .unwrap_or(unit);
        if !self
            .world
            .can_displace(unit, Some(actor), DisplacementKind::Place, Some(to))
            .map_err(unsupported)?
            .allowed
        {
            return Ok(PlacePlan::Blocked);
        }
        Ok(PlacePlan::Place(
            self.prepare_spatial_bound(objects, unit, to, target)?,
        ))
    }
    pub(super) fn prepare_spatial_bound(
        &self,
        objects: &BoundObjects,
        unit: Symbol,
        to: Hex,
        target: Option<ObjectId>,
    ) -> Result<Placement> {
        let new_tile = self.tile_bound(objects, to)?;
        if self.world.has_loaded_map() && !self.world.on_map(to) {
            return Err(unsupported("placement destination is outside loaded map"));
        }
        let from = self.world.position(unit);
        let old_tile = from
            .map(|hex| self.tile_bound(objects, hex))
            .transpose()?
            .flatten();
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        if objects.arena.kind(locations) != ObjectKind::Dict {
            return Err(unsupported("placement locations must be a mutable dict"));
        }
        if let Some(from) = from {
            let current = objects
                .arena
                .field(locations, unit)
                .ok_or_else(|| unsupported("location cache binding missing"))?;
            let (point, original) = self.spatial_hex_bound(objects, &current)?;
            if point != from || original.is_none() {
                return Err(unsupported("location Hex/cache binding drift"));
            }
        }
        Ok(Placement {
            unit,
            from,
            to,
            target,
            locations,
            old_tile,
            new_tile,
            token: self
                .world
                .entity(unit)
                .is_some_and(|entity| entity.kind == EntityKind::Token),
        })
    }
    pub(super) fn place_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: PlacePlan,
        mandatory: bool,
    ) -> Result<Option<NativeEvent>> {
        let placement = match plan {
            PlacePlan::Skip => return Ok(None),
            PlacePlan::Blocked => {
                if mandatory {
                    self.abort_objects(objects)?;
                }
                return Ok(None);
            }
            PlacePlan::Place(placement) => placement,
        };
        self.apply_spatial_bound(objects, &placement)?;
        Ok(Some(if placement.token {
            NativeEvent::TokenMoved {
                actor: self
                    .planning
                    .actor
                    .filter(|id| !self.symbols.name(*id).is_empty()),
                token: placement.unit,
                from: placement.from,
                to: placement.to,
            }
        } else {
            NativeEvent::UnitPlaced {
                unit: placement.unit,
                from: placement.from,
                to: placement.to,
            }
        }))
    }
    pub(super) fn apply_spatial_bound(
        &mut self,
        objects: &mut BoundObjects,
        placement: &Placement,
    ) -> Result<()> {
        let target = if let Some(original) = placement.target {
            ObjectValue::Object(original)
        } else {
            let Datum::Object(fields) = native_hex(placement.to, self.coordinates) else {
                unreachable!()
            };
            objects
                .arena
                .shared_frozen_model(
                    fields
                        .into_iter()
                        .map(|(key, value)| (key, ObjectValue::Atom(value)))
                        .collect(),
                )
                .map_err(unsupported)?
        };
        self.world
            .place_entity(placement.unit, placement.to)
            .map_err(EngineError::Rule)?;
        let occupant = objects.phases.k(Field::Occupant);
        if let Some(tile) = placement.old_tile
            && matches!(objects.arena.field(tile,occupant),Some(ObjectValue::Atom(Datum::Text(id))) if id==placement.unit)
        {
            objects
                .arena
                .set_field(tile, occupant, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        objects
            .arena
            .set_field(placement.locations, placement.unit, target)
            .map_err(unsupported)?;
        if let Some(tile) = placement.new_tile {
            objects
                .arena
                .set_field(
                    tile,
                    occupant,
                    ObjectValue::Atom(Datum::Text(placement.unit)),
                )
                .map_err(unsupported)?;
        }
        if matches!(objects.arena.root_field(objects.phases.k(Field::Snapshot)),Some(ObjectValue::Object(snapshot)) if snapshot==placement.locations)
        {
            self.boundary.snapshot(&self.world);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-placement-alias-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn swap_workers_read_live_unit_fields_without_leaking_positions() {
        let case = fixture_case("object-execution-live-swap-alias-repeated");
        let root =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let a_id = root.symbols.lookup("hero_arien").unwrap();
        let b_id = root.symbols.lookup("hero_brogan").unwrap();
        let from = root.world.position(a_id).unwrap();
        let to = root.world.position(b_id).unwrap();
        let mut normal = root.clone();
        let mut same = root.clone();
        let objects = same.objects.as_mut().unwrap();
        let ObjectValue::Object(step) = objects
            .arena
            .context_value(same.symbols.lookup("swap_alias").unwrap())
            .unwrap()
        else {
            panic!("step missing")
        };
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::UnitBId),
                ObjectValue::Atom(Datum::Text(a_id)),
            )
            .unwrap();
        assert_eq!(normal.advance_prefix(1).unwrap().events.len(), 1);
        assert_eq!(same.advance_prefix(1).unwrap().events.len(), 1);
        assert_eq!(normal.world.position(a_id), Some(to));
        assert_eq!(normal.world.position(b_id), Some(from));
        assert_eq!(same.world.position(a_id), Some(from));
        assert_eq!(same.world.position(b_id), Some(to));
        let after = root.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
    }
    #[test]
    fn placement_forks_read_live_destinations_and_keep_occupancy_and_aliases_private() {
        let root = session("dict-target");
        let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let unit = root.symbols.lookup("hero_brogan").unwrap();
        let from = root.world.position(unit).unwrap();
        let first = Hex::new(2, -1, -1).unwrap();
        let second = root
            .world
            .tile_hexes()
            .iter()
            .copied()
            .find(|hex| *hex != first && root.world.occupant(*hex).is_none())
            .unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        let key = b.symbols.lookup("target_hex").unwrap();
        let value = native_hex(second, b.coordinates);
        let objects = b.objects.as_mut().unwrap();
        let value = objects.arena.allocate(&value);
        objects.arena.set_context(key, value).unwrap();
        for (worker, to) in [(&mut a, first), (&mut b, second)] {
            let result = worker.advance_prefix(1).unwrap();
            assert_eq!(result.events.len(), 1);
            assert_eq!(worker.world.position(unit), Some(to));
            assert_eq!(worker.world.occupant(to), Some(unit));
            assert!(worker.world.occupant(from).is_none());
            let semantic = worker.semantic_state().unwrap();
            for hex in [from, to] {
                let (q, r, s) = hex.coordinates();
                let expected = worker
                    .world
                    .occupant(hex)
                    .map_or(Value::Null, |id| json!(worker.symbols.name(id)));
                assert_eq!(
                    semantic["board"]["tiles"][format!("({q}, {r}, {s})")]["occupant_id"],
                    expected
                );
            }
        }
        assert_ne!(a.semantic_state().unwrap(), b.semantic_state().unwrap());
        assert_eq!(
            serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
            before
        );
    }
    #[test]
    fn placement_refreshes_shared_snapshot_cache_but_retains_old_independent_hexes() {
        for name in ["snapshot-alias", "old-snapshot"] {
            let mut session = session(name);
            session.advance_prefix(1).unwrap();
            let mut typed = json!({});
            session.boundary.export(&mut typed, &session.symbols);
            assert_eq!(
                typed["last_turn_positions"],
                session.semantic_state().unwrap()["last_turn_positions"],
                "{name}"
            );
        }
    }
    #[test]
    fn loaded_off_map_placement_rejects_before_stack_or_arena_mutation() {
        let case = fixture_case("object-execution-placement-alias-dict-target");
        let mut envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        envelope.state["board"]["map_id"] = json!("loaded-fixture");
        let mut session = NativeSession::compile(envelope).unwrap();
        let key = session.symbols.lookup("target_hex").unwrap();
        let value = native_hex(Hex::new(99, -99, 0).unwrap(), session.coordinates);
        let objects = session.objects.as_mut().unwrap();
        let value = objects.arena.allocate(&value);
        objects.arena.set_context(key, value).unwrap();
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        assert!(
            session
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("outside loaded map")
        );
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }
}
