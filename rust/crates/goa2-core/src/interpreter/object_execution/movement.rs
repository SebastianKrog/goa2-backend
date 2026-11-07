//! Live walking: validate before mutation, retain spatial object aliases.
use super::phases::Field;
use super::placement::Placement;
use super::*;
use crate::validation::DisplacementKind;

pub(super) enum MovePlan {
    Skip,
    Blocked,
    Choice {
        unit: Symbol,
        range: i32,
        destination_key: Symbol,
    },
    Move {
        placement: Placement,
        unit: Symbol,
        from: Hex,
        to: Hex,
        range: i32,
        crossed: Option<(Symbol, Vec<Hex>)>,
        success: Option<Symbol>,
        triggered: Vec<Symbol>,
    },
}
impl NativeSession {
    pub(super) fn movement_output_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Option<Symbol>> {
        self.truthy_symbol_bound(objects, objects.arena.field(step, objects.phases.k(field)))
    }
    fn movement_flag_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<bool> {
        match objects.arena.field(step, objects.phases.k(field)) {
            Some(ObjectValue::Atom(Datum::Bool(value))) => Ok(value),
            _ => Err(unsupported("live movement boolean binding missing")),
        }
    }
    pub(super) fn preflight_move_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<MovePlan> {
        let mut unit = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::UnitId)),
        )?;
        if unit.is_none()
            && let Some(key) = self.truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(Field::UnitKey)),
            )?
        {
            unit = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty());
        let Some(unit) = unit.or(actor) else {
            return Ok(MovePlan::Skip);
        };
        let mut destination = objects
            .arena
            .field(step, objects.phases.k(Field::TargetHex))
            .filter(|v| objects.arena.truthy(v));
        if destination.is_none() {
            let Some(ObjectValue::Atom(Datum::Text(key))) = objects
                .arena
                .field(step, objects.phases.k(Field::DestinationKey))
            else {
                return Err(unsupported("movement destination key must be text"));
            };
            destination = objects
                .arena
                .context_value(key)
                .filter(|v| objects.arena.truthy(v));
        }
        let Some(destination) = destination else {
            return Ok(MovePlan::Skip);
        };
        let (to, target) = self.spatial_hex_bound(objects, &destination)?;
        let Some(from) = self.world.position(unit) else {
            return Ok(MovePlan::Skip);
        };
        let Some(ObjectValue::Atom(Datum::Int(range))) =
            objects.arena.field(step, objects.phases.k(Field::Range))
        else {
            return Err(unsupported("movement range missing"));
        };
        let range = i32::try_from(range).map_err(|_| unsupported("movement range overflow"))?;
        let is_action = self.movement_flag_bound(objects, step, Field::MoveAction)?;
        let through = self.movement_flag_bound(objects, step, Field::Through)?;
        let straight = self.movement_flag_bound(objects, step, Field::Straight)?;
        self.sync_card_collections_bound(objects)?;
        self.world
            .ensure_move_closure(is_action)
            .map_err(unsupported)?;
        if !self
            .world
            .can_displace(
                unit,
                Some(actor.unwrap_or(unit)),
                DisplacementKind::Move,
                None,
            )
            .map_err(unsupported)?
            .allowed
        {
            return Ok(MovePlan::Blocked);
        }
        let distance = if self
            .world
            .effect_connected(from, to, &[unit])
            .map_err(unsupported)?
        {
            i64::from(from.distance(to))
        } else {
            0
        };
        // Movement has no card argument; the only context-reading action hook
        // is attack-specific. No stale register values are consulted here.
        if !self
            .world
            .movement_legality(unit, distance, is_action, None, &self.context)
            .map_err(unsupported)?
            .allowed
        {
            return Ok(MovePlan::Blocked);
        }
        let valid = if from == to {
            range >= 0
        } else {
            self.world
                .movement_paths(
                    &crate::geometry::MovementQuery {
                        start: from,
                        ends: vec![to],
                        max_steps: range,
                        ignore_obstacles: false,
                        pass_through_obstacles: through,
                    },
                    actor,
                    Some(&[unit]),
                    &[],
                )
                .map_err(unsupported)?[0]
        };
        if !valid {
            return Ok(MovePlan::Skip);
        }
        let line = if straight {
            let Some(line) = from.line_to(to) else {
                return Ok(MovePlan::Skip);
            };
            if !self
                .world
                .effect_connected(from, to, &[])
                .map_err(unsupported)?
            {
                return Ok(MovePlan::Skip);
            }
            for hex in line.iter().take(line.len().saturating_sub(1)) {
                if !self.world.on_map(*hex)
                    || (!through
                        && self
                            .world
                            .obstacle_for_actor(*hex, Some(unit))
                            .map_err(unsupported)?
                        && !self.world.passable_token(*hex, &[unit]))
                {
                    return Ok(MovePlan::Skip);
                }
            }
            line
        } else {
            Vec::new()
        };
        let prechosen = self.movement_flag_bound(objects, step, Field::Prechosen)?;
        let mut triggered = Vec::new();
        if prechosen {
            if let Some(value) = objects
                .arena
                .context_value(objects.phases.k(Field::MineIds))
            {
                triggered = self.mine_symbols_bound(objects, value)?;
            }
        } else if let Some(team) = self
            .world
            .entity(unit)
            .filter(|e| matches!(e.kind, EntityKind::Hero | EntityKind::Piece))
            .and_then(|e| e.team)
            && from != to
            && self.has_enemy_mine_supply_bound(objects, team)?
        {
            if straight {
                for hex in line.iter().take(line.len().saturating_sub(1)) {
                    if !self.world.passable_token(*hex, &[]) {
                        continue;
                    }
                    if let Some(entity) = self
                        .world
                        .occupant(*hex)
                        .and_then(|id| self.world.entity(id))
                        .filter(|e| e.kind == EntityKind::Token)
                        && let Some(owner) = entity.owner.and_then(|id| self.world.hero(id))
                        && owner.team != team
                    {
                        triggered.push(entity.id);
                    }
                }
            } else {
                let paths = self
                    .world
                    .mine_paths(from, range, actor, Some(team), &[unit])
                    .map_err(unsupported)?;
                let options = self
                    .world
                    .tile_index(to)
                    .map(|index| paths[index].as_slice())
                    .unwrap_or(&[]);
                if options.len() > 1 {
                    let Some(ObjectValue::Atom(Datum::Text(destination_key))) = objects
                        .arena
                        .field(step, objects.phases.k(Field::DestinationKey))
                    else {
                        return Err(unsupported("mine destination key missing"));
                    };
                    self.phase_generation_check(0, 2)?;
                    return Ok(MovePlan::Choice {
                        unit,
                        range,
                        destination_key,
                    });
                }
                if let Some(path) = options.first() {
                    triggered = path.mines.to_vec();
                }
            }
        }
        if !triggered.is_empty() {
            self.phase_generation_check(0, 1)?;
        }
        let crossed =
            if let Some(key) = self.movement_output_bound(objects, step, Field::Crossed)? {
                let mut crossed = Vec::new();
                for hex in line.iter().take(line.len().saturating_sub(1)) {
                    if self
                        .world
                        .obstacle_for_actor(*hex, Some(actor.unwrap_or(unit)))
                        .map_err(unsupported)?
                    {
                        crossed.push(*hex);
                    }
                }
                Some((key, crossed))
            } else {
                None
            };
        Ok(MovePlan::Move {
            placement: self.prepare_spatial_bound(objects, unit, to, target)?,
            unit,
            from,
            to,
            range,
            crossed,
            success: self.movement_output_bound(objects, step, Field::Success)?,
            triggered,
        })
    }
    pub(super) fn move_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: MovePlan,
        mandatory: bool,
    ) -> Result<Option<NativeEvent>> {
        if let MovePlan::Choice {
            unit,
            range,
            destination_key,
        } = plan
        {
            let choice = self.phase_step(objects, "mine_path_choice")?;
            for (field, value) in [
                (Field::UnitId, Datum::Text(unit)),
                (Field::Range, Datum::Int(i64::from(range))),
                (Field::DestinationKey, Datum::Text(destination_key)),
            ] {
                objects
                    .arena
                    .set_field(choice, objects.phases.k(field), ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
            let copy = objects.arena.shallow_step(step).map_err(unsupported)?;
            objects.bind_copied_program(
                copy,
                objects
                    .program(step)
                    .ok_or_else(|| unsupported("movement program missing"))?,
            );
            objects
                .arena
                .set_field(
                    copy,
                    objects.phases.k(Field::Prechosen),
                    ObjectValue::Atom(Datum::Bool(true)),
                )
                .map_err(unsupported)?;
            let step_id = objects.scheduler.as_ref().unwrap().step_id;
            if objects
                .arena
                .field(copy, step_id)
                .is_some_and(|value| match value {
                    ObjectValue::GeneratedStep(_) => true,
                    ObjectValue::Atom(Datum::Text(id)) => {
                        self.symbols.name(id).starts_with("native-step-")
                    }
                    _ => false,
                })
            {
                self.next_step += 1;
                objects
                    .arena
                    .set_field(copy, step_id, ObjectValue::GeneratedStep(self.next_step))
                    .map_err(unsupported)?;
            }
            self.push_live_steps_bound(objects, &[choice, copy])?;
            return Ok(None);
        }
        let MovePlan::Move {
            placement,
            unit,
            from,
            to,
            range,
            crossed,
            success,
            triggered,
        } = plan
        else {
            if matches!(plan, MovePlan::Blocked) && mandatory {
                self.abort_objects(objects)?;
            }
            return Ok(None);
        };
        if let Some((key, crossed)) = crossed {
            let values = crossed
                .into_iter()
                .map(|hex| objects.arena.allocate(&native_hex(hex, self.coordinates)))
                .collect();
            let value = objects.arena.list(values);
            objects.arena.set_context(key, value).map_err(unsupported)?;
        }
        self.apply_spatial_bound(objects, &placement)?;
        if let Some(key) = success {
            objects
                .arena
                .set_context(key, ObjectValue::Atom(Datum::Bool(true)))
                .map_err(unsupported)?;
        }
        if !triggered.is_empty() {
            self.write_mine_list_bound(objects, objects.phases.k(Field::MineIds), &triggered)?;
            objects
                .arena
                .set_context(
                    objects.phases.k(Field::MineVictim),
                    ObjectValue::Atom(Datum::Text(unit)),
                )
                .map_err(unsupported)?;
            let child = self.phase_step(objects, "trigger_mine")?;
            let list = objects.arena.list(
                triggered
                    .into_iter()
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
                    .collect(),
            );
            objects
                .arena
                .set_field(child, objects.phases.k(Field::MinesLiteral), list)
                .map_err(unsupported)?;
            objects
                .arena
                .set_field(
                    child,
                    objects.phases.k(Field::Victim),
                    ObjectValue::Atom(Datum::Text(unit)),
                )
                .map_err(unsupported)?;
            self.push_live_steps_bound(objects, &[child])?;
        }
        Ok(Some(NativeEvent::UnitMoved {
            unit,
            from,
            to,
            range,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> NativeSession {
        let case = fixture_case("object-execution-walking-alias-dict");
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn walking_workers_read_private_live_destinations_and_preserve_root() {
        let root = session();
        let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let unit = root.symbols.lookup("hero_arien").unwrap();
        let from = root.world.position(unit).unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        let key = b.symbols.lookup("target_hex").unwrap();
        let value = native_hex(from, b.coordinates);
        let objects = b.objects.as_mut().unwrap();
        let value = objects.arena.allocate(&value);
        objects.arena.set_context(key, value).unwrap();
        assert_eq!(a.advance_prefix(1).unwrap().events.len(), 1);
        assert_eq!(b.advance_prefix(1).unwrap().events.len(), 1);
        assert_ne!(a.world.position(unit), Some(from));
        assert_eq!(b.world.position(unit), Some(from));
        for worker in [&a, &b] {
            let at = worker.world.position(unit).unwrap();
            assert_eq!(worker.world.occupant(at), Some(unit));
            assert_eq!(
                worker.semantic_state().unwrap()["entity_locations"]["hero_arien"],
                serde_json::to_value(at).unwrap()
            );
        }
        assert_eq!(
            serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
            before
        );
    }
    #[test]
    fn malformed_prechosen_mines_reject_before_pop_and_success_clear() {
        for queue in [
            Datum::List(vec![Datum::Bool(true)]),
            Datum::Null,
            Datum::Bool(false),
        ] {
            let mut session = session();
            let step = session.stack.last().unwrap().object.unwrap();
            let objects = session.objects.as_mut().unwrap();
            let prechosen = objects.phases.k(Field::Prechosen);
            objects
                .arena
                .set_field(step, prechosen, ObjectValue::Atom(Datum::Bool(true)))
                .unwrap();
            let key = objects.phases.k(Field::MineIds);
            let value = objects.arena.allocate(&queue);
            objects.arena.set_context(key, value).unwrap();
            let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
            assert!(
                session
                    .advance_prefix(1)
                    .unwrap_err()
                    .to_string()
                    .contains("mine ")
            );
            assert_eq!(
                serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
                before
            );
        }
    }
}
