//! Hero respawn preserves live operands, token unlinking and fresh Hex identity.
use super::phases::Field;
use super::*;

impl NativeSession {
    fn hero_respawn_direct_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<(Option<NativeRequest>, Vec<NativeEvent>)> {
        let hero = self.passive_text_bound(objects, step, Field::HeroId)?;
        let Some(row) = self.world.hero(hero) else {
            return Ok((None, Vec::new()));
        };
        if self.world.has_board_presence(hero) {
            return Ok((None, Vec::new()));
        }
        let multi = self.world.entity(row.id).is_some_and(|e| e.multi_piece);
        let selected = objects.selected(step, self.selection_key);
        if matches!(selected,Some(ObjectValue::Atom(Datum::Text(id))) if self.symbols.name(id)=="PASS")
        {
            let key = Arc::make_mut(&mut self.symbols).intern("skipped_respawn");
            objects.arena.symbols = self.symbols.clone();
            objects.write(key, Datum::Bool(true))?;
            return Ok((None, Vec::new()));
        }
        self.sync_card_collections_bound(objects)?;
        let rules = self
            .turn_rules
            .as_ref()
            .ok_or_else(|| unsupported("hero respawn rules missing"))?
            .clone();
        let candidates = rules
            .respawn_candidates(&self.world, hero)
            .map_err(unsupported)?;
        if candidates.is_empty() {
            return Ok((None, Vec::new()));
        }
        let is_dict = matches!(&selected,Some(ObjectValue::Object(id)) if objects.arena.kind(*id)==ObjectKind::Dict && objects.arena.truthy(&ObjectValue::Object(*id)));
        if is_dict
            && let Some(to) = selected
                .as_ref()
                .and_then(|value| self.spatial_hex_bound(objects, value).ok())
                .map(|(hex, _)| hex)
                .filter(|hex| candidates.contains(hex))
        {
            let unit = if multi {
                self.world
                    .hero(hero)
                    .unwrap()
                    .pieces
                    .iter()
                    .copied()
                    .find(|id| self.world.position(*id).is_none())
                    .ok_or_else(|| unsupported("empty hero piece supply"))?
            } else {
                hero
            };
            let placement = self.prepare_spatial_bound(objects, unit, to, None)?;
            let mut events = Vec::new();
            if let Some(token) = crate::turn::TurnRules::token(&self.world, to) {
                let from = self
                    .world
                    .position(token)
                    .ok_or_else(|| unsupported("respawn token location missing"))?;
                self.remove_mine_spatial_bound(objects, token)?;
                events.push(NativeEvent::RespawnCleared { hero, token, from });
            }
            self.apply_spatial_bound(objects, &placement)?;
            return Ok((None, events));
        }
        let location = is_dict
            || matches!(selected,Some(ObjectValue::Atom(Datum::Text(id))) if self.symbols.name(id)=="RESPAWN");
        let (_, choice, hex) = rules
            .prompts
            .iter()
            .find(|(id, _, _)| *id == hero)
            .ok_or_else(|| unsupported("hero respawn prompt missing"))?;
        if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        Ok((
            Some(NativeRequest {
                controlled: None,
                id: RequestToken::Generated(0),
                owner: hero,
                prompt: if location { *hex } else { *choice },
                movement_range: None,
                pre_action_range: None,
                choices: RequestChoices::Respawn {
                    hexes: candidates,
                    location,
                },
                can_skip: false,
                discard: false,
            }),
            Vec::new(),
        ))
    }
    pub(super) fn hero_respawn_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Vec<NativeEvent>> {
        self.transition_bound(objects, step, |candidate, objects, frame| {
            let (request, events) = candidate.hero_respawn_direct_bound(objects, step)?;
            request
                .map(|request| candidate.issue_bound_request(objects, step, frame, request))
                .transpose()?;
            Ok(events)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::object_session;
    use super::*;
    fn selected_hex(worker: &mut NativeSession, hex: Hex) -> Datum {
        native_hex(hex, worker.coordinates)
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn respawn_request_guard_keeps_stack_pending_and_locations_unchanged() {
        let mut worker = object_session("object-execution-hero-respawn-live-ordinary-first");
        worker.next_request = u64::MAX;
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("request generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
        worker.next_request = 0;
        let req = worker.advance_prefix(1).unwrap().request.unwrap();
        worker.next_request = u64::MAX;
        assert_eq!(
            worker.advance_prefix(1).unwrap().request.unwrap().id,
            req.id
        );
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn pass_precedes_no_spawn_and_needs_no_generation_capacity() {
        let mut worker =
            object_session("object-execution-hero-respawn-live-pending-pass-no-spawns-first");
        worker.next_request = u64::MAX;
        worker.next_step = u64::MAX;
        let result = worker.advance_prefix(1).unwrap();
        assert!(result.request.is_none() && result.events.is_empty());
        assert_eq!(
            worker.semantic_state().unwrap()["execution_context"]["skipped_respawn"],
            true
        );
        assert_eq!(worker.next_request, u64::MAX);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn respawn_reads_live_hero_operand_before_spawns_or_pending_selection() {
        let root = object_session("object-execution-hero-respawn-live-token-first");
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        let missing = worker
            .import_selection(&serde_json::json!("missing"))
            .unwrap();
        worker
            .with_live_objects(|_, objects| {
                let step = objects
                    .arena
                    .sequence(objects.stack)
                    .unwrap()
                    .last()
                    .unwrap()
                    .clone();
                let ObjectValue::Object(step) = step else {
                    panic!("step missing")
                };
                objects
                    .arena
                    .set_field(
                        step,
                        objects.phases.k(Field::HeroId),
                        ObjectValue::Atom(missing),
                    )
                    .map_err(unsupported)
            })
            .unwrap();
        let result = worker.advance_prefix(1).unwrap();
        assert!(result.request.is_none() && result.events.is_empty());
        let token = worker.symbols.lookup("respawn_token").unwrap();
        assert!(worker.world.position(token).is_some());
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn token_respawn_preserves_maps_unlinks_effects_and_keeps_root_private() {
        let mut root = object_session("object-execution-hero-respawn-live-token-effects-first");
        let req = root.advance_prefix(1).unwrap().request.unwrap();
        let RequestChoices::Respawn { hexes, .. } = &req.choices else {
            panic!("respawn request missing")
        };
        let target = hexes[0];
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        let selection = selected_hex(&mut worker, target);
        worker.submit(req.id, selection).unwrap();
        let out = worker.advance_prefix(1).unwrap();
        assert_eq!(out.events.len(), 1);
        assert!(matches!(out.events[0], NativeEvent::RespawnCleared { .. }));
        assert_eq!(worker.world.effects.instances.len(), 1);
        assert_eq!(
            worker
                .world
                .position(worker.symbols.lookup("hero_arien").unwrap()),
            Some(target)
        );
        let objects = worker.objects.as_ref().unwrap();
        let Some(ObjectValue::Object(locations)) =
            objects.arena.root_field(objects.phases.k(Field::Locations))
        else {
            panic!("locations missing")
        };
        assert!(
            matches!(objects.arena.context_value(worker.symbols.lookup("locations_alias").unwrap()),Some(ObjectValue::Object(alias)) if alias==locations)
        );
        let state = worker.semantic_state().unwrap();
        assert_eq!(
            state["execution_context"]["effects_alias"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn respawn_new_hex_and_shared_snapshot_remain_restorable() {
        let mut worker = object_session("object-execution-hero-respawn-live-shared-snapshot-first");
        let req = worker.advance_prefix(1).unwrap().request.unwrap();
        let RequestChoices::Respawn { hexes, .. } = &req.choices else {
            panic!("respawn request missing")
        };
        let target = hexes[0];
        let selection = selected_hex(&mut worker, target);
        worker.submit(req.id, selection).unwrap();
        worker.advance_prefix(1).unwrap();
        let objects = worker.objects.as_ref().unwrap();
        let ObjectValue::Object(locations) = objects
            .arena
            .root_field(objects.phases.k(Field::Locations))
            .unwrap()
        else {
            panic!("locations missing")
        };
        let ObjectValue::Object(spawns) = objects
            .arena
            .context_value(worker.symbols.lookup("spawn_hex_aliases").unwrap())
            .unwrap()
        else {
            panic!("spawns missing")
        };
        let location = objects
            .arena
            .field(locations, worker.symbols.lookup("hero_arien").unwrap())
            .unwrap();
        let ObjectValue::Object(location) = location else {
            panic!("location Hex missing")
        };
        assert!(
            matches!(objects.arena.sequence(spawns).unwrap()[0],ObjectValue::Object(spawn) if spawn!=location)
        );
        assert!(
            matches!(objects.arena.root_field(objects.phases.k(Field::Snapshot)),Some(ObjectValue::Object(snapshot)) if snapshot==locations)
        );
        let before = worker.export_envelope().unwrap();
        let restored = NativeSession::compile(before.clone())
            .unwrap()
            .export_envelope()
            .unwrap();
        assert_eq!(before.state, restored.state);
        assert_eq!(before.object_bindings, restored.object_bindings);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn multipiece_respawn_uses_supply_order_not_misc_registry_order() {
        let mut worker =
            object_session("object-execution-hero-respawn-live-piece-reversed-registry-first");
        let req = worker.advance_prefix(1).unwrap().request.unwrap();
        let RequestChoices::Respawn { hexes, .. } = &req.choices else {
            panic!("respawn request missing")
        };
        let target = hexes[0];
        let selection = selected_hex(&mut worker, target);
        worker.submit(req.id, selection).unwrap();
        worker.advance_prefix(1).unwrap();
        assert_eq!(
            worker
                .world
                .position(worker.symbols.lookup("hero_razzle_piece_1").unwrap()),
            Some(target)
        );
        assert!(
            worker
                .world
                .position(worker.symbols.lookup("hero_razzle").unwrap())
                .is_none()
        );
    }
}
