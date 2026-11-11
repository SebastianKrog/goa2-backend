//! Source push validation/path rules commit through authoritative spatial handles.
use super::phases::Field;
use super::placement::Placement;
use super::{
    Arc, BoundObjects, Datum, Hex, NativeEvent, NativeRequest, NativeSession, ObjectId,
    ObjectValue, Registers, RequestChoices, RequestToken, Result, Symbol, native_hex, unsupported,
};
use crate::{displacement::PushResult, validation::DisplacementKind, world::EntityKind};

pub(super) enum PushPlan {
    Skip,
    Blocked,
    Push {
        target: Symbol,
        from: Hex,
        result: PushResult,
        placement: Option<Placement>,
        collision: Option<Symbol>,
        token: bool,
        mines: Vec<Symbol>,
    },
}
impl NativeSession {
    fn push_context_symbol_bound(
        &self,
        objects: &BoundObjects,
        value: Option<ObjectValue>,
    ) -> Result<Option<Symbol>> {
        match value.filter(|v| objects.arena.truthy(v)) {
            None => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(id))) => Ok(Some(id)),
            Some(ObjectValue::Atom(Datum::Int(_) | Datum::Float(_) | Datum::Bool(_))) => Ok(None),
            _ => Err(unsupported("push target identity is unhashable or unbound")),
        }
    }
    fn push_source_symbol_bound(
        &mut self,
        objects: &BoundObjects,
        value: ObjectValue,
    ) -> Result<Symbol> {
        let datum = objects
            .arena
            .typed_value(&value, Arc::make_mut(&mut self.symbols))
            .map_err(unsupported)?;
        let name = datum
            .python_scalar_text(&self.symbols)
            .ok_or_else(|| unsupported("push source aggregate string conversion not bound"))?;
        Ok(Arc::make_mut(&mut self.symbols).intern(&name))
    }
    pub(super) fn preflight_push_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<PushPlan> {
        let mut target = self.push_context_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::TargetId)),
        )?;
        if target.is_none()
            && let Some(key) = self
                .passive_optional_text_bound(objects, step, Field::TargetKey)?
                .filter(|key| !self.symbols.name(*key).is_empty())
        {
            target = self.push_context_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let Some(target) = target else {
            return Ok(PushPlan::Skip);
        };
        let mut distance = match objects
            .arena
            .field(step, objects.phases.k(Field::PushDistance))
        {
            Some(ObjectValue::Atom(Datum::Int(n))) => n,
            Some(ObjectValue::Atom(Datum::Bool(n))) => i64::from(n),
            _ => return Err(unsupported("push distance must be integer")),
        };
        if let Some(key) = self
            .passive_optional_text_bound(objects, step, Field::DistanceKey)?
            .filter(|key| !self.symbols.name(*key).is_empty())
            && let Some(value) = objects
                .arena
                .context_value(key)
                .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)))
        {
            let value = objects
                .arena
                .typed_value(&value, Arc::make_mut(&mut self.symbols))
                .map_err(unsupported)?;
            distance = value
                .to_integer(&self.symbols)
                .map_err(unsupported)?
                .ok_or_else(|| unsupported("push distance integer conversion failed"))?;
        }
        let Some(from) = self.world.position(target) else {
            return Ok(PushPlan::Skip);
        };
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty());
        let mut source = if let Some(value) = objects
            .arena
            .field(step, objects.phases.k(Field::SourceHex))
            .filter(|v| objects.arena.truthy(v))
        {
            if !matches!(&value,ObjectValue::Object(id) if objects.arena.is_shared_frozen_model(*id))
            {
                return Err(unsupported("push source requires immutable Hex"));
            }
            Some(self.spatial_hex_bound(objects, &value)?.0)
        } else {
            None
        };
        if source.is_none()
            && let Some(key) = self
                .passive_optional_text_bound(objects, step, Field::PushSourceKey)?
                .filter(|key| !self.symbols.name(*key).is_empty())
            && let Some(value) = objects
                .arena
                .context_value(key)
                .filter(|v| objects.arena.truthy(v))
        {
            let id = self.push_source_symbol_bound(objects, value)?;
            source = self.world.resolved_position(id);
        }
        let Some(source) = source.or_else(|| actor.and_then(|id| self.world.resolved_position(id)))
        else {
            return Ok(PushPlan::Skip);
        };
        if source == from {
            return Ok(PushPlan::Skip);
        }
        self.sync_card_collections_bound(objects)?;
        let system = objects.phases.k(Field::SystemName);
        if !self
            .world
            .can_displace(
                target,
                Some(actor.unwrap_or(system)),
                DisplacementKind::Push,
                None,
            )
            .map_err(unsupported)?
            .allowed
        {
            return Ok(PushPlan::Blocked);
        }
        if !source.straight(from) {
            return Ok(PushPlan::Skip);
        }
        if distance > 0 {
            self.world.ensure_move_closure(false).map_err(unsupported)?;
        }
        let ignore = self.spatial_bool_bound(objects, step, Field::PushIgnore)?;
        let Some((result, path)) = crate::displacement::push_path(
            &self.world,
            source,
            from,
            distance,
            ignore,
            actor.unwrap_or(target),
            target,
        )
        .map_err(unsupported)?
        else {
            return Ok(PushPlan::Skip);
        };
        let entity = self
            .world
            .entity(target)
            .ok_or_else(|| unsupported("allowed push target cache missing"))?;
        let token = entity.kind == EntityKind::Token;
        let mut mines = Vec::new();
        if result.distance > 0
            && matches!(entity.kind, EntityKind::Hero | EntityKind::Piece)
            && let Some(team) = entity.team
        {
            for point in path.iter().skip(1).take(path.len().saturating_sub(2)) {
                if self.world.passable_token(*point, &[])
                    && let Some(mine) = self
                        .world
                        .occupant(*point)
                        .and_then(|id| self.world.entity(id))
                        .filter(|e| e.kind == EntityKind::Token)
                    && let Some(owner) = mine
                        .owner
                        .filter(|id| !self.symbols.name(*id).is_empty())
                        .and_then(|id| self.world.hero(id))
                    && owner.team != team
                {
                    mines.push(mine.id);
                }
            }
        }
        let generated = u64::from(!token) + u64::from(!mines.is_empty());
        if generated != 0 {
            self.phase_generation_check(0, generated)?;
        }
        let collision = self.movement_output_bound(objects, step, Field::CollisionOutput)?;
        let placement = (result.distance > 0)
            .then(|| self.prepare_spatial_bound(objects, target, result.to, None))
            .transpose()?;
        Ok(PushPlan::Push {
            target,
            from,
            result,
            placement,
            collision,
            token,
            mines,
        })
    }
    pub(super) fn push_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: PushPlan,
        mandatory: bool,
    ) -> Result<Option<NativeEvent>> {
        objects.arena.symbols = self.symbols.clone();
        let PushPlan::Push {
            target,
            from,
            result,
            placement,
            collision,
            token,
            mines,
        } = plan
        else {
            if matches!(plan, PushPlan::Blocked) && mandatory {
                self.abort_objects(objects)?;
            }
            return Ok(None);
        };
        if let Some(placement) = placement {
            self.apply_spatial_bound(objects, &placement)?;
        }
        if let Some(key) = collision {
            objects.write(key, Datum::Bool(result.collision))?;
        }
        objects.write(objects.phases.k(Field::PushVictim), Datum::Text(target))?;
        let mut children = Vec::new();
        if !mines.is_empty() {
            self.write_mine_list_bound(objects, objects.phases.k(Field::MineIds), &mines)?;
            objects.write(objects.phases.k(Field::MineVictim), Datum::Text(target))?;
            let step = self.phase_step(objects, "trigger_mine")?;
            let list = objects.arena.list(
                mines
                    .into_iter()
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
                    .collect(),
            );
            objects
                .arena
                .set_field(step, objects.phases.k(Field::MinesLiteral), list)
                .map_err(unsupported)?;
            objects
                .arena
                .set_field(
                    step,
                    objects.phases.k(Field::Victim),
                    ObjectValue::Atom(Datum::Text(target)),
                )
                .map_err(unsupported)?;
            children.push(step);
        }
        if !token {
            let step = self.phase_step(objects, "passive")?;
            objects
                .arena
                .set_field(
                    step,
                    objects.phases.k(Field::Trigger),
                    ObjectValue::Atom(Datum::Text(objects.phases.k(Field::AfterPush))),
                )
                .map_err(unsupported)?;
            children.push(step);
        }
        self.push_live_steps_bound(objects, &children)?;
        Ok(Some(NativeEvent::Pushed {
            actor: self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()),
            target,
            token,
            from,
            to: result.to,
            distance: result.distance,
            collision: result.collision,
        }))
    }
}

impl NativeSession {
    pub(super) fn mirror_push_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        mandatory: bool,
    ) -> Result<Option<NativeRequest>> {
        let owner = self.passive_text_bound(objects, step, Field::HeroId)?;
        let actor = self.planning.actor.unwrap_or(owner);
        let piece = self.world.resolved_actor(owner);
        let Some(origin) = self.world.position(piece) else {
            if mandatory {
                self.abort_objects(objects)?;
            }
            return Ok(None);
        };
        let field = |name| {
            self.symbols
                .lookup(name)
                .expect("mirror push field compiled")
        };
        let selected_field = field("selected_target_id");
        let mut target =
            self.truthy_symbol_bound(objects, objects.arena.field(step, selected_field))?;
        let mut answer = objects.selected(step, self.selection_key);
        if answer.is_some() {
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        let request = |prompt, choices| {
            Some(NativeRequest {
                controlled: None,
                id: RequestToken::Generated(0),
                owner,
                prompt,
                movement_range: None,
                pre_action_range: None,
                choices,
                can_skip: false,
                discard: false,
            })
        };
        let target_prompt = field("Select an adjacent unit to push");
        let distance_prompt = field("Choose push distance");
        let max_field = field("max_distance");
        let mirror_key = field("razzle_mirror_dest");
        let move_range = field("range_val");
        let is_action = field("is_movement_action");
        if target.is_none() {
            let mut targets = Vec::new();
            let inputs = self.spatial_inputs_bound(objects, []);
            let context = crate::spatial::SpatialContext {
                world: &self.world,
                registers: &inputs,
                symbols: &self.symbols,
                actor: self.planning.actor,
                frame: None,
                coordinates: self.coordinates,
                count_observer: Some(&self.count_observer),
                range_override: None,
                through_override: None,
            };
            for &(id, hex) in &self.world.locations {
                if id != piece
                    && origin.distance(hex) == 1
                    && self.world.entity(id).is_some_and(|entity| {
                        matches!(
                            entity.kind,
                            EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
                        )
                    })
                    && self
                        .world
                        .effect_connected(origin, hex, &[])
                        .map_err(unsupported)?
                    && self
                        .world
                        .can_target(Some(actor), Some(id))
                        .map_err(unsupported)?
                        .allowed
                    && self
                        .world
                        .can_displace(id, Some(actor), DisplacementKind::Push, None)
                        .map_err(unsupported)?
                        .allowed
                    && self
                        .world
                        .immune_filter(id, &context)
                        .map_err(unsupported)?
                {
                    targets.push(id);
                }
            }
            if targets.is_empty() {
                if mandatory {
                    self.abort_objects(objects)?;
                }
                return Ok(None);
            }
            if let Some(ObjectValue::Atom(Datum::Text(id))) = answer.take()
                && targets.contains(&id)
            {
                target = Some(id);
                objects
                    .arena
                    .set_field(step, selected_field, ObjectValue::Atom(Datum::Text(id)))
                    .map_err(unsupported)?;
            } else {
                return Ok(request(
                    target_prompt,
                    RequestChoices::Units {
                        tokens: false,
                        values: targets.into_iter().map(|id| (id, None)).collect(),
                    },
                ));
            }
        }
        let target = target.unwrap();
        let Some(target_hex) = self
            .world
            .position(target)
            .filter(|hex| *hex != origin && origin.straight(*hex))
        else {
            if mandatory {
                self.abort_objects(objects)?;
            }
            return Ok(None);
        };
        let Some(ObjectValue::Atom(Datum::Int(max))) = objects.arena.field(step, max_field) else {
            return Err(unsupported("mirror push maximum missing"));
        };
        let (q, r, s) = origin.coordinates();
        let (tq, tr, ts) = target_hex.coordinates();
        let length = i64::from(origin.distance(target_hex));
        let direction = (
            (i64::from(tq) - i64::from(q)) / length,
            (i64::from(tr) - i64::from(r)) / length,
            (i64::from(ts) - i64::from(s)) / length,
        );

        let mut legal = Vec::new();
        let mut path = vec![target_hex];
        for distance in 0..=max {
            if distance > 0 && path.len() as i64 == distance {
                let previous = *path.last().unwrap();
                let next =
                    crate::displacement::offset(previous, direction, 1).map_err(unsupported)?;
                if self.world.on_map(next)
                    && self
                        .world
                        .effect_connected(previous, next, &[target])
                        .map_err(unsupported)?
                    && (!self
                        .world
                        .obstacle_for_actor(next, Some(actor))
                        .map_err(unsupported)?
                        || self.world.passable_token(next, &[]))
                {
                    path.push(next);
                }
            }
            let actual = path
                .iter()
                .rposition(|&hex| !self.world.passable_token(hex, &[]))
                .unwrap_or(0) as i64;
            if actual == 0 {
                legal.push((distance, actual, None));
                continue;
            }
            if !self
                .world
                .can_displace(piece, Some(actor), DisplacementKind::Move, None)
                .map_err(unsupported)?
                .allowed
                || !self
                    .world
                    .movement_legality(piece, actual, false, None, &Registers::default())
                    .map_err(unsupported)?
                    .allowed
            {
                continue;
            }
            let mut previous = origin;
            for offset in 1..=actual {
                let next =
                    crate::displacement::offset(origin, direction, -offset).map_err(unsupported)?;
                if !self.world.on_map(next)
                    || !self
                        .world
                        .effect_connected(previous, next, &[piece])
                        .map_err(unsupported)?
                    || self
                        .world
                        .obstacle_for_actor(next, Some(actor))
                        .map_err(unsupported)?
                {
                    break;
                }
                previous = next;
                if offset == actual {
                    legal.push((distance, actual, Some(next)));
                }
            }
        }
        if legal.is_empty() {
            if mandatory {
                self.abort_objects(objects)?;
            }
            return Ok(None);
        }
        let choice = if let Some(value) = answer {
            objects
                .arena
                .typed_value(&value, Arc::make_mut(&mut self.symbols))
                .ok()
                .and_then(|value| value.to_integer(&self.symbols).ok().flatten())
        } else {
            None
        };
        if let Some(&(distance, actual, destination)) =
            legal.iter().find(|row| Some(row.0) == choice)
        {
            self.phase_generation_check(
                0,
                u64::from(distance > 0) + u64::from(destination.is_some()),
            )?;
            let mut steps = Vec::new();
            if distance > 0 {
                let push = self.phase_step(objects, "mirror_push")?;
                for (key, value) in [
                    (Field::TargetId, Datum::Text(target)),
                    (Field::PushDistance, Datum::Int(distance)),
                ] {
                    objects
                        .arena
                        .set_field(push, objects.phases.k(key), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                steps.push(push);
            }
            if let Some(destination) = destination {
                let Datum::Object(fields) = native_hex(destination, self.coordinates) else {
                    unreachable!()
                };
                let hex = objects
                    .arena
                    .shared_frozen_model(
                        fields
                            .into_iter()
                            .map(|(key, value)| (key, ObjectValue::Atom(value)))
                            .collect(),
                    )
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_context(mirror_key, hex)
                    .map_err(unsupported)?;
                let movement = self.phase_step(objects, "move_unit")?;
                for (key, value) in [
                    (objects.phases.k(Field::UnitId), Datum::Text(piece)),
                    (
                        objects.phases.k(Field::DestinationKey),
                        Datum::Text(mirror_key),
                    ),
                    (move_range, Datum::Int(actual)),
                    (is_action, Datum::Bool(false)),
                ] {
                    objects
                        .arena
                        .set_field(movement, key, ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                steps.push(movement);
            }
            self.push_live_steps_bound(objects, &steps)?;
            return Ok(None);
        }
        Ok(request(
            distance_prompt,
            RequestChoices::MirrorDistances(
                legal
                    .into_iter()
                    .map(|(distance, actual, _)| (distance, actual))
                    .collect(),
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture_case;
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-arena-push-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn push_generation_guard_precedes_position_context_and_stack_mutation() {
        let mut worker = session("ordinary");
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
    fn obstacle_ignore_is_live_and_private_to_the_worker() {
        let root = session("terrain");
        let before = root.export_envelope().unwrap();
        let mut normal = root.clone();
        let mut changed = root.clone();
        let step = changed.stack.last().unwrap().object.unwrap();
        let objects = changed.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::PushIgnore),
                ObjectValue::Atom(Datum::Bool(true)),
            )
            .unwrap();
        for (worker, distance) in [(&mut normal, 0), (&mut changed, 3)] {
            let result = worker.advance_prefix(1).unwrap();
            assert_eq!(
                worker.event_json(&result.events[0])["metadata"]["distance"],
                distance
            );
        }
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn token_push_requires_no_step_generation_capacity() {
        let mut worker = session("token-zero");
        worker.next_step = u64::MAX;
        let result = worker.advance_prefix(1).unwrap();
        assert_eq!(
            worker.event_json(&result.events[0])["event_type"],
            "TOKEN_PUSHED"
        );
        assert_eq!(worker.next_step, u64::MAX);
        assert_eq!(worker.stack.len(), 1);
    }
    #[test]
    fn queued_mines_and_context_snapshots_have_distinct_ordered_list_identities() {
        let mut worker = session("mines-ordered-mines");
        worker.advance_prefix(1).unwrap();
        let step = worker.stack.last().unwrap().object.unwrap();
        let objects = worker.objects.as_ref().unwrap();
        let Some(ObjectValue::Object(context)) = objects
            .arena
            .context_value(objects.phases.k(Field::MineIds))
        else {
            panic!("mine context missing");
        };
        let Some(ObjectValue::Object(literal)) = objects
            .arena
            .field(step, objects.phases.k(Field::MinesLiteral))
        else {
            panic!("mine literal missing");
        };
        assert_ne!(context, literal);
        for list in [context, literal] {
            let ids = objects
                .arena
                .sequence(list)
                .unwrap()
                .iter()
                .map(|v| match v {
                    ObjectValue::Atom(Datum::Text(id)) => worker.symbols.name(*id),
                    _ => panic!("mine identity missing"),
                })
                .collect::<Vec<_>>();
            assert_eq!(ids, ["push-mine-1", "push-mine-2"]);
        }
    }
}
