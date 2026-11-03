//! Source push validation/path rules commit through authoritative spatial handles.
use super::phases::Field;
use super::placement::Placement;
use super::*;
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

#[cfg(test)]
mod tests {
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
