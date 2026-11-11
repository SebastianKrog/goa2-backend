//! Fast travel revalidates destinations without movement/path/placement rules.
use super::phases::Field;
use super::{
    BoundObjects, Datum, NativeEvent, NativeSession, ObjectId, ObjectKind, ObjectValue, Result,
    Symbol, placement, unsupported,
};
use crate::{data::Registers, spatial::SpatialContext, world::ActionKind};

pub(super) enum TravelPlan {
    Skip,
    Blocked,
    Relocate {
        unit: Symbol,
        placement: placement::Placement,
    },
    Sequence {
        actor: Symbol,
        destination: Symbol,
        source_card: Option<Symbol>,
        change: bool,
        selected: bool,
    },
}
impl NativeSession {
    pub(super) fn preflight_travel_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        sequence: bool,
    ) -> Result<TravelPlan> {
        let unit = self
            .truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(Field::UnitId)),
            )?
            .or(self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()));
        let Some(actor) = unit.map(|id| self.world.resolved_actor(id)) else {
            return Ok(TravelPlan::Skip);
        };
        let destination = self.passive_text_bound(objects, step, Field::DestinationKey)?;
        let source_card = self.passive_optional_text_bound(objects, step, Field::SourceCard)?;
        let change = self.spatial_bool_bound(objects, step, Field::ZoneChange)?;
        let target = if sequence {
            None
        } else {
            let value = objects
                .arena
                .field(step, objects.phases.k(Field::TargetHex))
                .filter(|v| objects.arena.truthy(v))
                .or_else(|| {
                    objects
                        .arena
                        .context_value(destination)
                        .filter(|v| objects.arena.truthy(v))
                });
            let Some(value) = value else {
                return Ok(TravelPlan::Skip);
            };
            if matches!(value, ObjectValue::Atom(Datum::Text(id)) if id == self.skip) {
                return Ok(TravelPlan::Skip);
            }
            let coordinates = match &value {
                ObjectValue::Atom(Datum::Object(_)) => true,
                ObjectValue::Object(id) => {
                    objects.arena.is_shared_frozen_model(*id)
                        || objects.arena.kind(*id) == ObjectKind::Dict
                }
                _ => false,
            };
            if !coordinates {
                return Ok(TravelPlan::Blocked);
            }
            Some(self.spatial_hex_bound(objects, &value)?)
        };
        self.sync_card_collections_bound(objects)?;
        let inputs = self.spatial_inputs_bound(
            objects,
            [
                self.symbols.lookup("card").unwrap(),
                objects.phases.k(Field::CurrentCard),
            ],
        );
        let context = SpatialContext {
            count_observer: Some(&self.count_observer),
            world: &self.world,
            registers: &inputs,
            symbols: &self.symbols,
            actor: self.planning.actor,
            frame: None,
            coordinates: self.coordinates,
            range_override: None,
            through_override: None,
        };
        let card = context.fast_travel_card(source_card);
        if sequence {
            if !self
                .world
                .action_legality(actor, ActionKind::FastTravel, card, &Registers::default())
                .map_err(unsupported)?
                .allowed
            {
                return Ok(TravelPlan::Skip);
            }
            let selected = objects
                .arena
                .context_value(destination)
                .is_some_and(|v| objects.arena.truthy(&v));
            self.phase_generation_check(0, if selected { 1 } else { 2 })?;
            return Ok(TravelPlan::Sequence {
                actor,
                destination,
                source_card,
                change,
                selected,
            });
        }
        let (to, target) = target.unwrap();
        if !self
            .world
            .fast_travel_destinations_with_card(Some(actor), change, &[to], card)
            .map_err(unsupported)?[0]
        {
            return Ok(TravelPlan::Blocked);
        }
        Ok(TravelPlan::Relocate {
            unit: actor,
            placement: self.prepare_spatial_bound(objects, actor, to, target)?,
        })
    }
    pub(super) fn travel_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: TravelPlan,
        mandatory: bool,
    ) -> Result<Option<NativeEvent>> {
        match plan {
            TravelPlan::Skip => {}
            TravelPlan::Blocked => {
                if mandatory {
                    self.abort_objects(objects)?;
                }
            }
            TravelPlan::Relocate { unit, placement } => {
                self.apply_spatial_bound(objects, &placement)?;
                return Ok(Some(NativeEvent::UnitPlaced {
                    unit,
                    from: placement.from,
                    to: placement.to,
                }));
            }
            TravelPlan::Sequence {
                actor,
                destination,
                source_card,
                change,
                selected,
            } => {
                let rules = objects.scheduler.as_ref().unwrap().action_bodies.clone();
                let mut children = Vec::new();
                if !selected {
                    let selector = self.allocate_source_recipe_bound(
                        objects,
                        rules.travel_recipe(true),
                        &Datum::Text(actor),
                    )?;
                    objects
                        .arena
                        .set_field(
                            selector,
                            objects.phases.k(Field::Output),
                            ObjectValue::Atom(Datum::Text(destination)),
                        )
                        .map_err(unsupported)?;
                    let Some(ObjectValue::Object(filters)) = objects
                        .arena
                        .field(selector, objects.phases.k(Field::Filters))
                    else {
                        return Err(unsupported("travel recipe filters missing"));
                    };
                    let Some(ObjectValue::Object(filter)) = objects
                        .arena
                        .sequence(filters)
                        .and_then(|v| v.first())
                        .cloned()
                    else {
                        return Err(unsupported("travel destination filter missing"));
                    };
                    for (field, value) in [
                        (
                            Field::SourceCard,
                            source_card.map_or(Datum::Null, Datum::Text),
                        ),
                        (Field::ZoneChange, Datum::Bool(change)),
                    ] {
                        objects
                            .arena
                            .set_field(filter, objects.phases.k(field), ObjectValue::Atom(value))
                            .map_err(unsupported)?;
                    }
                    Self::validate_returned_step_bound(objects, selector)?;
                    children.push(selector);
                }
                let travel = self.allocate_source_recipe_bound(
                    objects,
                    rules.travel_recipe(false),
                    &Datum::Text(actor),
                )?;
                for (field, value) in [
                    (
                        Field::SourceCard,
                        source_card.map_or(Datum::Null, Datum::Text),
                    ),
                    (Field::ZoneChange, Datum::Bool(change)),
                    (Field::DestinationKey, Datum::Text(destination)),
                ] {
                    objects
                        .arena
                        .set_field(travel, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                Self::validate_returned_step_bound(objects, travel)?;
                children.push(travel);
                self.push_live_steps_bound(objects, &children)?;
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Hex, StateEnvelope, fixture_case, json, native_hex};
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-arena-travel-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn sequence_generation_guard_precedes_stack_or_context_changes() {
        let mut worker = session("sequence-first");
        worker.next_step = u64::MAX;
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("step generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    fn direct_travel_needs_no_generation_capacity_and_keeps_the_target_hex_model() {
        let mut worker = session("edge-literal-hex");
        worker.next_step = u64::MAX;
        let unit = worker.symbols.lookup("hero_arien").unwrap();
        let alias = worker.symbols.lookup("destination_alias").unwrap();
        let Some(ObjectValue::Object(target)) =
            worker.objects.as_ref().unwrap().arena.context_value(alias)
        else {
            panic!("destination Hex model missing");
        };
        let result = worker.advance_prefix(1).unwrap();
        assert_eq!(
            worker.event_json(&result.events[0])["event_type"],
            "UNIT_PLACED"
        );
        let objects = worker.objects.as_ref().unwrap();
        let locations = objects
            .phases
            .root(&objects.arena, Field::Locations)
            .unwrap();
        assert!(
            matches!(objects.arena.field(locations, unit), Some(ObjectValue::Object(id)) if id == target)
        );
        assert_eq!(worker.next_step, u64::MAX);
    }
    #[test]
    fn workers_read_live_travel_destinations_without_mutating_the_root() {
        let root = session("edge-context-hex");
        let before = root.export_envelope().unwrap();
        let mut first = root.clone();
        let mut second = root.clone();
        let unit = root.symbols.lookup("hero_arien").unwrap();
        let destination = second.symbols.lookup("target_hex").unwrap();
        let to = Hex::new(2, 0, -2).unwrap();
        let value = native_hex(to, second.coordinates);
        let objects = second.objects.as_mut().unwrap();
        let value = objects.arena.allocate(&value);
        objects.arena.set_context(destination, value).unwrap();
        first.advance_prefix(1).unwrap();
        second.advance_prefix(1).unwrap();
        assert_eq!(second.world.position(unit), Some(to));
        assert_ne!(first.world.position(unit), second.world.position(unit));
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn prohibited_sequence_needs_no_generation_capacity_and_preserves_the_tail() {
        let mut worker = session("edge-blocked-movement");
        worker.next_step = u64::MAX;
        assert!(worker.advance_prefix(1).unwrap().events.is_empty());
        assert_eq!(worker.stack.len(), 1);
        assert_eq!(worker.next_step, u64::MAX);
    }
    #[test]
    fn search_reuses_fast_travel_mask_and_rechecks_a_changed_board() {
        let root = session("edge-literal-hex");
        let unit = root.symbols.lookup("hero_arien").unwrap();
        let mut world = root.world.clone();
        let candidates = world.tile_hexes().to_vec();
        let scan = |world: &crate::world::NativeWorld| {
            world
                .fast_travel_destinations_with_card(Some(unit), false, &candidates, None)
                .unwrap()
        };
        let expected = scan(&world);
        let destination = candidates
            .iter()
            .copied()
            .zip(&expected)
            .find_map(|(hex, &legal)| legal.then_some(hex))
            .unwrap();
        let mut changed = world.clone();
        changed.move_entity(unit, destination).unwrap();
        let expected_changed = scan(&changed);
        assert_ne!(expected, expected_changed);
        let (_, reuse) =
            crate::terrain::with_search_traversals(world.definitions.clone(), || {
                assert_eq!(scan(&world), expected);
                assert_eq!(world.fast_travel_destinations_with_card(
                Some(unit), false, &[destination], None,
            ).unwrap(), vec![true]);
                world.move_entity(unit, destination).unwrap();
                assert_eq!(scan(&world), expected_changed);
                assert_eq!(scan(&world), expected_changed);
            });
        assert_eq!(reuse.travel_misses, 2);
        assert_eq!(reuse.travel_hits, 2);
    }

    #[test]
    fn travel_cannot_cross_an_enemy_static_barrier() {
        let case = fixture_case("object-execution-arena-travel-edge-barrier");
        let mut envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        // The envelope repeats values along every retained path to one model.
        envelope.state["active_effects"][0]["barrier_radius"] = json!(2);
        envelope.state["execution_context"]["effects_alias"][0]["barrier_radius"] = json!(2);
        let mut worker = NativeSession::compile(envelope).unwrap();
        assert!(worker.advance_prefix(1).unwrap().events.is_empty());
        assert!(worker.stack.is_empty());
        let actor = worker.symbols.lookup("hero_arien").unwrap();
        assert_eq!(
            worker.world.position(actor),
            Some(Hex::new(0, 0, 0).unwrap())
        );
    }
}
