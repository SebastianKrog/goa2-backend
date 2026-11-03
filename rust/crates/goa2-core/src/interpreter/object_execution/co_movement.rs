//! Co-directional sequencer and token walking retain source movement ordering.
use super::phases::Field;
use super::placement::Placement;
use super::*;

pub(super) enum CoMovementPlan {
    Done,
    Drag {
        anchor_key: Symbol,
        partner_key: Symbol,
        anchor_dest: Symbol,
        partner_dest: Symbol,
        destination: Hex,
        distance: i64,
        through: bool,
        token: bool,
        partner_first: bool,
    },
    Token {
        placement: Placement,
        token: Symbol,
        from: Hex,
        to: Hex,
        range: i64,
    },
}
impl NativeSession {
    fn co_movement_identity_bound(
        &mut self,
        objects: &BoundObjects,
        key: Symbol,
    ) -> Result<Option<Symbol>> {
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(None);
        };
        let value = objects
            .arena
            .typed_value(&value, Arc::make_mut(&mut self.symbols))
            .map_err(unsupported)?;
        let text = value
            .python_scalar_text(&self.symbols)
            .ok_or_else(|| unsupported("co-movement aggregate ID conversion not bound"))?;
        Ok(Some(Arc::make_mut(&mut self.symbols).intern(&text)))
    }
    pub(super) fn preflight_co_movement_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        drag: bool,
    ) -> Result<CoMovementPlan> {
        let anchor_key = self.passive_text_bound(
            objects,
            step,
            if drag {
                Field::AnchorKey
            } else {
                Field::TokenKey
            },
        )?;
        let anchor_dest = self.passive_text_bound(
            objects,
            step,
            if drag {
                Field::AnchorDestinationKey
            } else {
                Field::DestinationKey
            },
        )?;
        let Some(anchor) = self.co_movement_identity_bound(objects, anchor_key)? else {
            return Ok(CoMovementPlan::Done);
        };
        let partner_key = if drag {
            Some(self.passive_text_bound(objects, step, Field::PartnerKey)?)
        } else {
            None
        };
        let partner = if let Some(key) = partner_key {
            let Some(partner) = self.co_movement_identity_bound(objects, key)? else {
                return Ok(CoMovementPlan::Done);
            };
            Some(partner)
        } else {
            None
        };
        let Some(value) = objects
            .arena
            .context_value(anchor_dest)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(CoMovementPlan::Done);
        };
        if !drag {
            let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
            if objects.arena.field(misc, anchor).is_none()
                || !self
                    .world
                    .entity(anchor)
                    .is_some_and(|e| e.kind == EntityKind::Token)
            {
                return Ok(CoMovementPlan::Done);
            }
        }
        let (to, original) = self.spatial_hex_bound(objects, &value)?;
        let Some(from) = self.world.position(anchor) else {
            return Ok(CoMovementPlan::Done);
        };
        if let Some(partner) = partner {
            let Some(partner_from) = self.world.position(partner) else {
                return Ok(CoMovementPlan::Done);
            };
            let (aq, ar, az) = from.coordinates();
            let (q, r, z) = to.coordinates();
            let destination = crate::displacement::offset(
                partner_from,
                (
                    i64::from(q) - i64::from(aq),
                    i64::from(r) - i64::from(ar),
                    i64::from(z) - i64::from(az),
                ),
                1,
            )
            .map_err(unsupported)?;
            let line = from
                .line_to(to)
                .ok_or_else(|| unsupported("co-directional source line is not straight"))?;
            let partner_dest =
                self.passive_text_bound(objects, step, Field::PartnerDestinationKey)?;
            let through = self.spatial_bool_bound(objects, step, Field::IgnorePathObstacles)?;
            let token = self.spatial_bool_bound(objects, step, Field::AnchorIsToken)?;
            self.phase_generation_check(0, 2)?;
            return Ok(CoMovementPlan::Drag {
                anchor_key,
                partner_key: partner_key.unwrap(),
                anchor_dest,
                partner_dest,
                destination,
                distance: i64::from(from.distance(to)),
                through,
                token,
                partner_first: line.contains(&partner_from),
            });
        }
        if from == to || self.world.occupant(to).is_some_and(|id| id != anchor) {
            return Ok(CoMovementPlan::Done);
        }
        let range = self
            .spatial_int_bound(objects, step, Field::Range)?
            .ok_or_else(|| unsupported("token walk range missing"))?;
        let through = self.spatial_bool_bound(objects, step, Field::Through)?;
        let range_i32 =
            i32::try_from(range).map_err(|_| unsupported("token walk range overflow"))?;
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty());
        if !self
            .world
            .movement_paths(
                &crate::geometry::MovementQuery {
                    start: from,
                    ends: vec![to],
                    max_steps: range_i32,
                    ignore_obstacles: false,
                    pass_through_obstacles: through,
                },
                actor,
                None,
                &[anchor],
            )
            .map_err(unsupported)?[0]
        {
            return Ok(CoMovementPlan::Done);
        }
        if self.spatial_bool_bound(objects, step, Field::Straight)? {
            let Some(line) = from.line_to(to) else {
                return Ok(CoMovementPlan::Done);
            };
            for hex in line.iter().take(line.len().saturating_sub(1)) {
                if !self.world.on_map(*hex)
                    || (!through
                        && self
                            .world
                            .obstacle_for_actor(*hex, Some(anchor))
                            .map_err(unsupported)?
                        && !self.world.passable_token(*hex, &[anchor]))
                {
                    return Ok(CoMovementPlan::Done);
                }
            }
        }
        let placement = self.prepare_spatial_bound(objects, anchor, to, original)?;
        Ok(CoMovementPlan::Token {
            placement,
            token: anchor,
            from,
            to,
            range,
        })
    }
    pub(super) fn co_movement_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: CoMovementPlan,
    ) -> Result<Option<NativeEvent>> {
        match plan {
            CoMovementPlan::Done => Ok(None),
            CoMovementPlan::Token {
                placement,
                token,
                from,
                to,
                range,
            } => {
                self.apply_spatial_bound(objects, &placement)?;
                Ok(Some(NativeEvent::TokenWalked {
                    actor: self
                        .planning
                        .actor
                        .filter(|id| !self.symbols.name(*id).is_empty()),
                    token,
                    from,
                    to,
                    range,
                }))
            }
            CoMovementPlan::Drag {
                anchor_key,
                partner_key,
                anchor_dest,
                partner_dest,
                destination,
                distance,
                through,
                token,
                partner_first,
            } => {
                self.coordinates_bound(
                    objects,
                    super::coordinates::CoordinatePlan::Hex(partner_dest, destination),
                )?;
                let anchor =
                    self.phase_step(objects, if token { "move_token" } else { "move_unit" })?;
                let partner = self.phase_step(objects, "move_unit")?;
                for (child, key, dest, is_token) in [
                    (anchor, anchor_key, anchor_dest, token),
                    (partner, partner_key, partner_dest, false),
                ] {
                    let mut fields = vec![
                        (
                            if is_token {
                                Field::TokenKey
                            } else {
                                Field::UnitKey
                            },
                            Datum::Text(key),
                        ),
                        (Field::DestinationKey, Datum::Text(dest)),
                        (Field::Range, Datum::Int(distance)),
                        (Field::Through, Datum::Bool(through)),
                        (Field::Straight, Datum::Bool(true)),
                    ];
                    if !is_token {
                        fields.push((Field::MoveAction, Datum::Bool(false)));
                    }
                    for (field, value) in fields {
                        objects
                            .arena
                            .set_field(child, objects.phases.k(field), ObjectValue::Atom(value))
                            .map_err(unsupported)?;
                    }
                }
                let ordered = if partner_first {
                    [partner, anchor]
                } else {
                    [anchor, partner]
                };
                self.push_live_steps_bound(objects, &ordered)?;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(token: bool) -> NativeSession {
        let name = format!(
            "object-execution-co-movement-drag-{}-False-plain",
            if token { "True" } else { "False" }
        );
        let case = fixture_case(&name);
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    fn snapshot(session: &NativeSession) -> Value {
        serde_json::to_value(session.export_envelope().unwrap()).unwrap()
    }
    #[test]
    fn drag_generation_guard_precedes_partner_hex_and_stack_mutation() {
        for token in [false, true] {
            let mut worker = root(token);
            worker.next_step = u64::MAX - 1;
            let before = snapshot(&worker);
            assert!(
                worker
                    .advance_prefix(1)
                    .unwrap_err()
                    .to_string()
                    .contains("generation")
            );
            assert_eq!(snapshot(&worker), before);
        }
    }
    #[test]
    fn drag_reads_live_destinations_without_mutating_the_root_or_retained_hex() {
        let original = root(false);
        let before = snapshot(&original);
        let mut moved = original.clone();
        let mut stationary = original.clone();
        let actor = stationary.symbols.lookup("hero_hanu").unwrap();
        let start = stationary.world.position(actor).unwrap();
        let key = stationary.symbols.lookup("anchor_dest").unwrap();
        let objects = stationary.objects.as_mut().unwrap();
        let value = objects
            .arena
            .allocate(&native_hex(start, stationary.coordinates));
        objects.arena.set_context(key, value).unwrap();
        moved.advance_prefix(1).unwrap();
        stationary.advance_prefix(1).unwrap();
        let a = moved.semantic_state().unwrap();
        let b = stationary.semantic_state().unwrap();
        assert_ne!(
            a["execution_context"]["co_drag_partner_dest"],
            b["execution_context"]["co_drag_partner_dest"]
        );
        assert_eq!(
            a["execution_context"]["old-partner-dest"],
            b["execution_context"]["old-partner-dest"]
        );
        assert_eq!(snapshot(&original), before);
    }
    #[test]
    fn nonstraight_raw_drag_is_explicitly_guarded_before_consumption() {
        let mut worker = root(false);
        let key = worker.symbols.lookup("anchor_dest").unwrap();
        let point = Hex::new(0, 1, -1).unwrap();
        let objects = worker.objects.as_mut().unwrap();
        let value = objects
            .arena
            .allocate(&native_hex(point, worker.coordinates));
        objects.arena.set_context(key, value).unwrap();
        let before = snapshot(&worker);
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("not straight")
        );
        assert_eq!(snapshot(&worker), before);
    }
    #[test]
    fn co_mover_filter_reads_private_live_partner_keys() {
        let case = fixture_case("object-execution-co-movement-filter-False-False-plain");
        let original =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        let before = snapshot(&original);
        let mut normal = original.clone();
        let mut coincident = original.clone();
        let step = coincident.stack.last().unwrap().object.unwrap();
        let anchor = coincident.symbols.lookup("anchor").unwrap();
        let objects = coincident.objects.as_mut().unwrap();
        let Some(ObjectValue::Object(list)) =
            objects.arena.field(step, objects.phases.k(Field::Filters))
        else {
            unreachable!()
        };
        let filter = match objects.arena.sequence(list).unwrap()[0] {
            ObjectValue::Object(id) => id,
            _ => unreachable!(),
        };
        objects
            .arena
            .set_field(
                filter,
                objects.phases.k(Field::PartnerKey),
                ObjectValue::Atom(Datum::Text(anchor)),
            )
            .unwrap();
        normal.advance_prefix(1).unwrap();
        coincident.advance_prefix(1).unwrap();
        assert!(
            normal.semantic_state().unwrap()["execution_context"]["choices"]
                .as_i64()
                .unwrap()
                > 0
        );
        assert_eq!(
            coincident.semantic_state().unwrap()["execution_context"]["choices"],
            json!(0)
        );
        assert_eq!(snapshot(&original), before);
    }
}
