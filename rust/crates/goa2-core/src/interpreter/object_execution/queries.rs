//! Live noninteractive queries read arena operands and allocate fresh results.
use super::phases::Field;
use super::*;
use crate::{queries::Query, spatial::SpatialContext};

pub(super) struct QueryPlan {
    output: Symbol,
    value: Datum,
}
impl NativeSession {
    pub(super) fn preflight_query_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        collect: bool,
    ) -> Result<QueryPlan> {
        self.sync_card_collections_bound(objects)?;
        let target = self.passive_text_bound(objects, step, Field::TargetType)?;
        let query = match self.symbols.name(target) {
            "UNIT" | "UNIT_OR_TOKEN" => Query::Units {
                tokens: self.symbols.name(target) == "UNIT_OR_TOKEN",
                filters: self.effective_unit_filters_bound(objects, step)?,
            },
            "HEX" if !collect => {
                if objects
                    .phases
                    .tiles
                    .iter()
                    .any(|(_, consistent)| !consistent)
                {
                    return Err(unsupported("hex count board tile/key geometry drift"));
                }
                let filters = self
                    .unit_filter_list_bound(objects, step, Field::Filters)?
                    .into_iter()
                    .map(|v| match v {
                        ObjectValue::Object(id) => self.hex_filter_bound(objects, id),
                        _ => Err(unsupported("hex count filter model missing")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                Query::Hexes(filters)
            }
            "HEX" | "CARD" | "NUMBER" | "TOKEN" => Query::Empty,
            _ => return Err(unsupported("query target type invalid")),
        };
        let inputs = match &query {
            Query::Units { filters, .. } => self.unit_inputs_bound(objects, filters)?,
            Query::Hexes(filters) => {
                let mut keys = Vec::new();
                for filter in filters {
                    self.spatial_filter_keys_bound(objects, filter, &mut keys);
                }
                self.spatial_inputs_bound(objects, keys)
            }
            Query::Empty => crate::data::SparseRegisters::default(),
        };
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
        let value = query.evaluate(&context, collect).map_err(unsupported)?;
        Ok(QueryPlan {
            output: self.passive_text_bound(objects, step, Field::Output)?,
            value,
        })
    }
    pub(super) fn preflight_unit_filters_query_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        self.sync_card_collections_bound(objects)?;
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let key = self.passive_text_bound(objects, step, Field::UnitKey)?;
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(QueryPlan {
                output,
                value: Datum::Null,
            });
        };
        let unit = self.resource_identity_bound(objects, value)?;
        let filters = self
            .unit_filter_list_bound(objects, step, Field::Filters)?
            .into_iter()
            .map(|v| match v {
                ObjectValue::Object(id) => self.unit_filter_bound(objects, id, 0),
                _ => Err(unsupported("unit check filter model missing")),
            })
            .collect::<Result<Vec<_>>>()?;
        let inputs = self.unit_inputs_bound(objects, &filters)?;
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
        let mut matches = true;
        for filter in filters {
            if !filter.accepts(unit, &context).map_err(unsupported)? {
                matches = false;
                break;
            }
        }
        Ok(QueryPlan {
            output,
            value: if matches {
                Datum::Bool(true)
            } else {
                Datum::Null
            },
        })
    }
    pub(super) fn preflight_unit_on_board_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let key = self.passive_text_bound(objects, step, Field::UnitKey)?;
        let mut value = Datum::Null;
        if let Some(unit) = objects
            .arena
            .context_value(key)
            .filter(|unit| objects.arena.truthy(unit))
        {
            let id = self.resource_identity_bound(objects, unit)?;
            if self.world.position(id).is_some() {
                value = Datum::Bool(true);
            }
        }
        Ok(QueryPlan { output, value })
    }

    pub(super) fn preflight_unit_type_check_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let literal = self
            .passive_optional_text_bound(objects, step, Field::UnitId)?
            .filter(|id| !self.symbols.name(*id).is_empty());
        let value = if let Some(id) = literal {
            Some(ObjectValue::Atom(Datum::Text(id)))
        } else {
            self.passive_optional_text_bound(objects, step, Field::UnitKey)?
                .filter(|key| !self.symbols.name(*key).is_empty())
                .and_then(|key| objects.arena.context_value(key))
        };
        let value = value.filter(|value| objects.arena.truthy(value));
        let result = if let Some(value) = value {
            let id = self.resource_identity_bound(objects, value)?;
            let is_hero = self.world.hero(id).is_some();
            let expected = self.passive_text_bound(objects, step, Field::ExpectedType)?;
            match self.symbols.name(expected) {
                "HERO" => is_hero,
                "MINION" => !is_hero,
                _ => false,
            }
        } else {
            false
        };
        // The source stores False, not None, including absent-unit paths.
        Ok(QueryPlan {
            output,
            value: Datum::Bool(result),
        })
    }
    pub(super) fn preflight_boolean_combine_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let truth = |field| -> Result<bool> {
            let key = self.passive_text_bound(objects, step, field)?;
            Ok(objects
                .arena
                .context_value(key)
                .is_some_and(|value| objects.arena.truthy(&value)))
        };
        let a = truth(Field::KeyA)?;
        let b = truth(Field::KeyB)?;
        let operation = self.passive_text_bound(objects, step, Field::Operation)?;
        let result = if self.symbols.name(operation) == "OR" {
            a || b
        } else {
            a && b
        };
        Ok(QueryPlan {
            output: self.passive_text_bound(objects, step, Field::Output)?,
            value: Datum::Bool(result),
        })
    }
    pub(super) fn preflight_distance_check_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let mut units = [None; 2];
        for (slot, (literal, key)) in units.iter_mut().zip([
            (Field::UnitAId, Field::UnitAKey),
            (Field::UnitBId, Field::UnitBKey),
        ]) {
            let literal = self
                .passive_optional_text_bound(objects, step, literal)?
                .filter(|id| !self.symbols.name(*id).is_empty());
            let value = if let Some(id) = literal {
                Some(ObjectValue::Atom(Datum::Text(id)))
            } else {
                self.passive_optional_text_bound(objects, step, key)?
                    .filter(|key| !self.symbols.name(*key).is_empty())
                    .and_then(|key| objects.arena.context_value(key))
            };
            let Some(value) = value.filter(|v| objects.arena.truthy(v)) else {
                return Ok(QueryPlan {
                    output,
                    value: Datum::Null,
                });
            };
            let unit = self.resource_identity_bound(objects, value)?;
            let Some(position) = self.world.position(unit) else {
                return Ok(QueryPlan {
                    output,
                    value: Datum::Null,
                });
            };
            *slot = Some(position);
        }
        let op = self.passive_text_bound(objects, step, Field::Operator)?;
        let comparison = crate::queries::Comparison::parse(self.symbols.name(op));
        let threshold = self
            .spatial_int_bound(objects, step, Field::Threshold)?
            .ok_or_else(|| unsupported("distance threshold missing"))?;
        let distance =
            crate::queries::topology_distance(&self.world, units[0].unwrap(), units[1].unwrap())
                .map_err(unsupported)?;
        Ok(QueryPlan {
            output,
            value: if comparison.matches_order(distance.threshold_order(threshold)) {
                Datum::Bool(true)
            } else {
                Datum::Null
            },
        })
    }
    pub(super) fn preflight_zone_change_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let unit_key = self.passive_text_bound(objects, step, Field::UnitKey)?;
        let before_key = self.passive_text_bound(objects, step, Field::BeforeHexKey)?;
        let unit = objects
            .arena
            .context_value(unit_key)
            .filter(|value| objects.arena.truthy(value));
        let before = objects
            .arena
            .context_value(before_key)
            .filter(|value| !matches!(value, ObjectValue::Atom(Datum::Null)));
        let mut value = Datum::Null;
        if let Some((unit, before)) = unit.zip(before) {
            let (before, _) = self.spatial_hex_bound(objects, &before)?;
            let unit = self.resource_identity_bound(objects, unit)?;
            if let Some(after) = self.world.position(unit)
                && self.world.zone_at(before) != self.world.zone_at(after)
            {
                value = Datum::Bool(true);
            }
        }
        Ok(QueryPlan { output, value })
    }
    pub(super) fn preflight_adjacent_enemies_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        use crate::world::EntityKind;
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let zero = || QueryPlan {
            output,
            value: Datum::Int(0),
        };
        let Some(actor) = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty())
        else {
            return Ok(zero());
        };
        let unit = |id| {
            self.world.entity(id).filter(|entity| {
                matches!(
                    entity.kind,
                    EntityKind::Hero | EntityKind::Minion | EntityKind::Piece
                ) && entity.has_team
            })
        };
        let Some(actor_unit) = unit(actor) else {
            return Ok(zero());
        };
        // The source uses the actor's direct location, not acting-piece resolution.
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        let Some(position) = objects.arena.field(locations, actor) else {
            return Ok(zero());
        };
        let (origin, _) = self.spatial_hex_bound(objects, &position)?;
        let board = objects.phases.root(&objects.arena, Field::Board)?;
        let Some(ObjectValue::Object(tiles)) =
            objects.arena.field(board, objects.phases.k(Field::Tiles))
        else {
            return Err(unsupported("adjacent enemy count board tiles missing"));
        };
        let mut count = 0i64;
        for neighbor in origin.neighbors() {
            let (q, r, s) = neighbor.coordinates();
            let Some(key) = objects.arena.symbols.lookup(&format!("({q}, {r}, {s})")) else {
                continue;
            };
            let Some(ObjectValue::Object(tile)) = objects.arena.field(tiles, key) else {
                continue;
            };
            let Some(occupant) =
                self.passive_optional_text_bound(objects, tile, Field::Occupant)?
            else {
                continue;
            };
            if unit(occupant).is_some_and(|enemy| enemy.team != actor_unit.team) {
                count += 1;
            }
        }
        let subtract = self
            .spatial_int_bound(objects, step, Field::Subtract)?
            .ok_or_else(|| unsupported("adjacent enemy subtraction missing"))?;
        let multiplier = self
            .spatial_int_bound(objects, step, Field::Multiplier)?
            .ok_or_else(|| unsupported("adjacent enemy multiplier missing"))?;
        let value = count
            .checked_sub(subtract)
            .and_then(|n| n.max(0).checked_mul(multiplier))
            .ok_or_else(|| unsupported("adjacent enemy bonus outside i64"))?;
        Ok(QueryPlan {
            output,
            value: Datum::Int(value),
        })
    }
    pub(super) fn preflight_card_count_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let literal = self
            .passive_optional_text_bound(objects, step, Field::HeroId)?
            .filter(|id| !self.symbols.name(*id).is_empty())
            .map(|id| ObjectValue::Atom(Datum::Text(id)));
        let value = if literal.is_some() {
            literal
        } else if let Some(key) = self
            .passive_optional_text_bound(objects, step, Field::HeroKey)?
            .filter(|id| !self.symbols.name(*id).is_empty())
        {
            objects
                .arena
                .context_value(key)
                .filter(|v| objects.arena.truthy(v))
        } else {
            None
        };
        let value = value.or_else(|| {
            self.planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty())
                .map(|id| ObjectValue::Atom(Datum::Text(id)))
        });
        let count = if let Some(value) = value {
            let raw = self.resource_identity_bound(objects, value)?;
            if let Some(model) = self.hero_model_bound(objects, raw) {
                let container = self.passive_text_bound(objects, step, Field::Container)?;
                let field = match self.symbols.name(container) {
                    "HAND" => Some(Field::Hand),
                    "DISCARD" => Some(Field::Discard),
                    "DECK" => Some(Field::Deck),
                    "PLAYED" => Some(Field::Played),
                    _ => None,
                };
                if let Some(field) = field {
                    let Some(ObjectValue::Object(list)) =
                        objects.arena.field(model, objects.phases.k(field))
                    else {
                        return Err(unsupported("card count collection missing"));
                    };
                    let cards = objects
                        .arena
                        .sequence(list)
                        .ok_or_else(|| unsupported("card count requires a sequence"))?;
                    if matches!(field, Field::Played) {
                        cards
                            .iter()
                            .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)))
                            .count()
                    } else {
                        cards.len()
                    }
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        };
        Ok(QueryPlan {
            output,
            value: Datum::Int(
                i64::try_from(count).map_err(|_| unsupported("card count outside i64"))?,
            ),
        })
    }
    pub(super) fn preflight_round_defeat_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let marks = objects
            .arena
            .root_field(objects.phases.k(Field::DefeatedHeroes))
            .ok_or_else(|| unsupported("round defeat marks missing"))?;
        Ok(QueryPlan {
            output: self.passive_text_bound(objects, step, Field::Output)?,
            value: if objects.arena.truthy(&marks) {
                Datum::Bool(true)
            } else {
                Datum::Null
            },
        })
    }
    pub(super) fn preflight_context_condition_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<QueryPlan> {
        let key = self.passive_text_bound(objects, step, Field::InputKey)?;
        let operator = self.passive_text_bound(objects, step, Field::Operator)?;
        let threshold = self
            .spatial_int_bound(objects, step, Field::Threshold)?
            .ok_or_else(|| unsupported("condition threshold missing"))?;
        let value = objects.arena.context_value(key);
        let scalar = match &value {
            Some(ObjectValue::Atom(value)) => Some(value),
            _ => None,
        };
        let yes = crate::queries::Comparison::parse(self.symbols.name(operator))
            .evaluate(scalar, threshold)
            .map_err(unsupported)?;
        Ok(QueryPlan {
            output: self.passive_text_bound(objects, step, Field::Output)?,
            value: if yes { Datum::Bool(true) } else { Datum::Null },
        })
    }
    pub(super) fn query_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: QueryPlan,
    ) -> Result<()> {
        let value = objects.arena.allocate(&plan.value);
        objects
            .arena
            .set_context(plan.output, value)
            .map_err(unsupported)
    }
}

#[cfg(test)]
mod adjacent_tests {
    use super::*;
    fn session(suffix: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-xargatha-count-{suffix}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    fn snapshot(session: &NativeSession) -> Value {
        serde_json::to_value(session.export_envelope().unwrap()).unwrap()
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn adjacency_reads_live_operands_and_output_without_generation_or_shared_worker_mutation() {
        let root = session("ordinary");
        let before = snapshot(&root);
        let mut worker = root.clone();
        let step = worker.stack.last().unwrap().object.unwrap();
        let output = worker.symbols.lookup("tail").unwrap();
        let objects = worker.objects.as_mut().unwrap();
        for (field, value) in [
            (Field::Multiplier, Datum::Int(-3)),
            (Field::Subtract, Datum::Int(1)),
            (Field::Output, Datum::Text(output)),
        ] {
            objects
                .arena
                .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
                .unwrap();
        }
        worker.next_step = u64::MAX - 1;
        assert!(worker.advance_prefix(1).unwrap().events.is_empty());
        assert_eq!(
            worker.semantic_state().unwrap()["execution_context"]["tail"],
            json!(-15)
        );
        assert_eq!(snapshot(&root), before);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn adjacency_overflow_rejects_before_pop_but_absent_actor_returns_zero_first() {
        for suffix in ["ordinary", "no-actor"] {
            let mut worker = session(suffix);
            let step = worker.stack.last().unwrap().object.unwrap();
            let objects = worker.objects.as_mut().unwrap();
            objects
                .arena
                .set_field(
                    step,
                    objects.phases.k(Field::Subtract),
                    ObjectValue::Atom(Datum::Int(i64::MIN)),
                )
                .unwrap();
            let before = snapshot(&worker);
            if suffix == "ordinary" {
                assert!(
                    matches!(worker.advance_prefix(1),Err(EngineError::Unsupported(message)) if message.contains("adjacent enemy bonus outside i64"))
                );
                assert_eq!(snapshot(&worker), before);
            } else {
                worker.advance_prefix(1).unwrap();
                assert_eq!(
                    worker.semantic_state().unwrap()["execution_context"]["adjacency_probe"],
                    json!(0)
                );
            }
        }
    }
}
