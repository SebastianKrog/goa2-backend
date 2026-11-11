//! Live singleton marker models, aliases, and post-placement passive dispatch.
use super::phases::Field;
use super::{
    BoundObjects, Datum, MarkerEvent, NativeEvent, NativeSession, ObjectId, ObjectKind,
    ObjectValue, Result, Symbol, unsupported,
};
use crate::markers::MarkerRules;
use crate::stats::{MarkerKind, NativeMarker};

pub(super) enum MarkerPlan {
    Skip,
    Update {
        dictionary: ObjectId,
        model: Option<ObjectId>,
        key: Symbol,
        target: Option<Symbol>,
        source: Option<Symbol>,
        value: i64,
        rows: Vec<NativeMarker>,
        event: MarkerEvent,
        passive: bool,
    },
}

impl NativeSession {
    fn marker_kind_bound(&self, key: Symbol) -> Result<MarkerKind> {
        match self.symbols.name(key) {
            "venom" => Ok(MarkerKind::Venom),
            "poison" => Ok(MarkerKind::Poison),
            "bounty" => Ok(MarkerKind::Bounty),
            _ => Err(unsupported("unknown marker kind")),
        }
    }
    pub(super) fn preflight_marker_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        remove: bool,
    ) -> Result<MarkerPlan> {
        if !remove {
            let skip = objects
                .arena
                .context_value(objects.phases.k(Field::SkipMarkers))
                .is_some_and(|value| objects.arena.truthy(&value));
            let substitution = match objects
                .arena
                .context_value(objects.phases.k(Field::SubstitutionActor))
            {
                Some(ObjectValue::Atom(Datum::Text(value))) => Some(value),
                _ => None,
            };
            if MarkerRules::suppressed(&self.world, self.planning.actor, skip, substitution) {
                return Ok(MarkerPlan::Skip);
            }
        }
        let key = self
            .spatial_text_bound(objects, step, Field::MarkerType)?
            .ok_or_else(|| unsupported("marker kind missing"))?;
        if remove {
            return self.prepare_marker_update(objects, key, None, false);
        }
        let mut target = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::TargetId)),
        )?;
        if target.is_none()
            && let Some(key) = self.truthy_symbol_bound(
                objects,
                objects
                    .arena
                    .field(step, objects.phases.k(Field::TargetKey)),
            )?
        {
            target = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let Some(target) = target else {
            return Ok(MarkerPlan::Skip);
        };
        let source = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty())
            .unwrap_or(objects.phases.k(Field::SystemName));
        let Some(ObjectValue::Atom(Datum::Int(value))) =
            objects.arena.field(step, objects.phases.k(Field::Value))
        else {
            return Err(unsupported("marker value must be an integer"));
        };
        self.phase_generation_check(0, 1)?;
        self.prepare_marker_update(objects, key, Some((target, source, value)), true)
    }

    pub(super) fn prepare_marker_update(
        &self,
        objects: &BoundObjects,
        key: Symbol,
        placement: Option<(Symbol, Symbol, i64)>,
        passive: bool,
    ) -> Result<MarkerPlan> {
        let kind = self.marker_kind_bound(key)?;
        let dictionary = objects.phases.root(&objects.arena, Field::Markers)?;
        if objects.arena.kind(dictionary) != ObjectKind::Dict {
            return Err(unsupported("markers must be a mutable dictionary"));
        }
        let model = match objects.arena.field(dictionary, key) {
            Some(ObjectValue::Object(model))
                if matches!(
                    objects.arena.kind(model),
                    ObjectKind::Model | ObjectKind::Dict
                ) =>
            {
                Some(model)
            }
            None => None,
            _ => return Err(unsupported("marker must be a mutable model")),
        };
        let (target, source, value, event) = match placement {
            Some((target, source, value)) => (
                Some(target),
                Some(source),
                value,
                MarkerEvent::Placed {
                    kind: key,
                    actor: source,
                    target,
                    value,
                },
            ),
            None if model.is_some() => (None, None, 0, MarkerEvent::Removed { kind: key }),
            None => return Ok(MarkerPlan::Skip),
        };
        let mut rows = Vec::new();
        for (row_key, row_value) in objects
            .arena
            .fields(dictionary)
            .ok_or_else(|| unsupported("marker dictionary fields missing"))?
        {
            let ObjectValue::Object(row_model) = row_value else {
                return Err(unsupported("marker model missing"));
            };
            let row_kind = self.marker_kind_bound(
                self.spatial_text_bound(objects, *row_model, Field::FilterType)?
                    .ok_or_else(|| unsupported("marker model kind missing"))?,
            )?;
            let (row_target, row_source, row_value) = if Some(*row_model) == model {
                (target, source, value)
            } else {
                let Some(ObjectValue::Atom(Datum::Int(value))) = objects
                    .arena
                    .field(*row_model, objects.phases.k(Field::Value))
                else {
                    return Err(unsupported("marker cache value must be an integer"));
                };
                (
                    self.passive_optional_text_bound(objects, *row_model, Field::TargetId)?,
                    self.passive_optional_text_bound(objects, *row_model, Field::SourceId)?,
                    value,
                )
            };
            rows.push(NativeMarker {
                key: *row_key,
                kind: row_kind,
                target: row_target,
                source: row_source,
                value: row_value,
            });
        }
        if model.is_none() {
            rows.push(NativeMarker {
                key,
                kind,
                target,
                source,
                value,
            });
        }
        Ok(MarkerPlan::Update {
            dictionary,
            model,
            key,
            target,
            source,
            value,
            rows,
            event,
            passive,
        })
    }
    pub(super) fn marker_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: MarkerPlan,
    ) -> Result<Option<NativeEvent>> {
        let MarkerPlan::Update {
            dictionary,
            model,
            key,
            target,
            source,
            value,
            rows,
            event,
            passive,
        } = plan
        else {
            return Ok(None);
        };
        let model = match model {
            Some(model) => model,
            None => {
                let ObjectValue::Object(model) = objects
                    .arena
                    .dictionary(vec![
                        (
                            objects.phases.k(Field::FilterType),
                            ObjectValue::Atom(Datum::Text(key)),
                        ),
                        (
                            objects.phases.k(Field::TargetId),
                            ObjectValue::Atom(Datum::Null),
                        ),
                        (
                            objects.phases.k(Field::Value),
                            ObjectValue::Atom(Datum::Int(0)),
                        ),
                        (
                            objects.phases.k(Field::SourceId),
                            ObjectValue::Atom(Datum::Null),
                        ),
                    ])
                    .map_err(unsupported)?
                else {
                    unreachable!();
                };
                objects
                    .arena
                    .classify_recipe_object(model, &[], ObjectKind::Model)
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_field(dictionary, key, ObjectValue::Object(model))
                    .map_err(unsupported)?;
                model
            }
        };
        for (field, value) in [
            (Field::TargetId, target.map_or(Datum::Null, Datum::Text)),
            (Field::Value, Datum::Int(value)),
            (Field::SourceId, source.map_or(Datum::Null, Datum::Text)),
        ] {
            objects
                .arena
                .set_field(model, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        self.world.markers = rows;
        if passive {
            objects
                .arena
                .set_context(
                    objects.phases.k(Field::MarkerTarget),
                    ObjectValue::Atom(target.map_or(Datum::Null, Datum::Text)),
                )
                .map_err(unsupported)?;
            let passive = self.phase_step(objects, "passive")?;
            objects
                .arena
                .set_field(
                    passive,
                    objects.phases.k(Field::Trigger),
                    ObjectValue::Atom(Datum::Text(objects.phases.k(Field::AfterMarker))),
                )
                .map_err(unsupported)?;
            self.push_live_steps_bound(objects, &[passive])?;
        }
        Ok(Some(NativeEvent::Marker(event)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{EngineError, fixture_case};
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-live-marker-poison-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn marker_workers_read_live_fields_and_refresh_every_shared_cache_row() {
        let root = session("shared-types");
        let before = root.export_envelope().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        let target = b.symbols.lookup("hero_arien").unwrap();
        let objects = b.objects.as_mut().unwrap();
        let ObjectValue::Object(step) = objects
            .arena
            .context_value(b.symbols.lookup("marker_step").unwrap())
            .unwrap()
        else {
            panic!("step missing")
        };
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::TargetId),
                ObjectValue::Atom(Datum::Text(target)),
            )
            .unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::Value),
                ObjectValue::Atom(Datum::Int(-4)),
            )
            .unwrap();
        for (worker, value, hero) in [(&mut a, -2, "hero_brogan"), (&mut b, -4, "hero_arien")] {
            assert_eq!(worker.advance_prefix(1).unwrap().events.len(), 1);
            assert_eq!(worker.world.markers.len(), 2);
            assert!(
                worker
                    .world
                    .markers
                    .iter()
                    .all(|row| row.value == value && row.target == worker.symbols.lookup(hero))
            );
            let objects = worker.objects.as_ref().unwrap();
            let dictionary = objects.phases.root(&objects.arena, Field::Markers).unwrap();
            let fields = objects.arena.fields(dictionary).unwrap();
            assert!(
                matches!((&fields[0].1, &fields[1].1), (ObjectValue::Object(a), ObjectValue::Object(b)) if a == b)
            );
        }
        let after = root.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
    }
    #[test]
    fn marker_generation_guard_leaves_stack_models_and_cache_untouched() {
        let mut worker = session("existing");
        worker.next_step = u64::MAX;
        let before = worker.export_envelope().unwrap();
        let cache = worker
            .world
            .markers
            .iter()
            .map(|row| (row.key, row.target, row.source, row.value))
            .collect::<Vec<_>>();
        assert!(
            matches!(worker.advance_prefix(1), Err(EngineError::Unsupported(message)) if message.contains("generation counter exhausted"))
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
        assert_eq!(
            cache,
            worker
                .world
                .markers
                .iter()
                .map(|row| (row.key, row.target, row.source, row.value))
                .collect::<Vec<_>>()
        );
    }
}
