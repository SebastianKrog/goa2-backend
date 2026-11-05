//! Simultaneous upgrades and round reset retain authoritative container identities.
use super::phases::Field;
use super::*;
use crate::{
    cards::CardId,
    objects::{Mutation, Target},
    world::CardLifecycle,
};

impl NativeSession {
    pub(super) fn sync_hero_items_bound(&mut self, objects: &BoundObjects) -> Result<()> {
        for (hero, model) in &objects.phases.heroes {
            let items = match objects.arena.field(*model, self.cleanup_key("items")) {
                Some(ObjectValue::Object(id)) => id,
                _ => return Err(unsupported("upgrade items mapping missing")),
            };
            let rows = objects
                .arena
                .fields(items)
                .ok_or_else(|| unsupported("upgrade items must be a mapping"))?;
            let row = self.world.hero_row_mut(*hero).unwrap();
            row.items.fill(0);
            row.items_order.clear();
            for (key, value) in rows {
                let stat =
                    crate::stats::Stat::parse(self.symbols.name(*key)).map_err(unsupported)?;
                let ObjectValue::Atom(Datum::Int(n)) = value else {
                    return Err(unsupported("upgrade item must be an integer"));
                };
                row.items[stat.index()] = *n;
                row.items_order.push(stat);
            }
        }
        Ok(())
    }

    fn sync_upgrade_state_bound(&mut self, objects: &BoundObjects) -> Result<()> {
        self.sync_card_collections_bound(objects)?;
        self.sync_hero_items_bound(objects)?;
        let pending = self.battle_root_map(objects, "pending_upgrades")?;
        self.round.pending = objects
            .arena
            .fields(pending)
            .unwrap()
            .iter()
            .map(|(hero, value)| {
                let ObjectValue::Atom(Datum::Int(n)) = value else {
                    return Err(unsupported("pending upgrade count must be an integer"));
                };
                Ok((*hero, *n))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(())
    }
    pub(in crate::interpreter) fn validate_upgrade_submission_bound(
        &self,
        selection: &Datum,
    ) -> Result<()> {
        let mut preview = self.clone();
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| unsupported("upgrade arena missing"))?;
        preview.sync_upgrade_state_bound(objects)?;
        let rules = preview
            .round_rules
            .as_ref()
            .ok_or_else(|| unsupported("upgrade rules missing"))?;
        if rules
            .validate(&preview.world, &preview.round, selection)
            .map_err(card_error)?
            .is_none()
        {
            return Err(EngineError::InvalidInput(
                "Choose one legal upgrade or all remaining upgrades for your hero".into(),
            ));
        }
        Ok(())
    }
    fn upgrade_card_state_bound(
        &mut self,
        objects: &mut BoundObjects,
        card: CardId,
        state: &str,
        lifecycle: CardLifecycle,
    ) -> Result<()> {
        self.world.change_lifecycle(card, lifecycle);
        objects
            .arena
            .set_physical_field(
                card.0,
                objects.phases.k(Field::CardState),
                Datum::Text(self.cleanup_key(state)),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .remove_physical_field(card.0, objects.phases.k(Field::InitiativeOverride))
            .map_err(unsupported)?;
        Ok(())
    }
    fn apply_upgrade_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        printed: Symbol,
    ) -> Result<()> {
        self.sync_upgrade_state_bound(objects)?;
        let Some(model) = self.hero_model_bound(objects, hero) else {
            return Ok(());
        };
        let row = self.world.hero(hero).unwrap();
        let Some(chosen) = row.deck.iter().copied().find(|id| {
            self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id
                == printed
        }) else {
            return Ok(());
        };
        let definition = &self.world.definitions.cards[self.world.card(chosen).unwrap().definition];
        let color = definition.color;
        let tier = definition.tier;
        let ultimate = self.symbols.name(tier) == "IV";
        let previous = (!ultimate)
            .then(|| {
                row.hand.iter().copied().find(|id| {
                    self.world.definitions.cards[self.world.card(*id).unwrap().definition].color
                        == color
                })
            })
            .flatten();
        let pair = (!ultimate)
            .then(|| {
                row.deck.iter().copied().find(|id| {
                    let d = &self.world.definitions.cards[self.world.card(*id).unwrap().definition];
                    d.color == color && d.tier == tier && d.printed_id != printed
                })
            })
            .flatten();
        let hand = self.cleanup_list(objects, model, Field::Hand)?;
        if let Some(previous) = previous {
            let index = self
                .cleanup_equal(objects, hand, previous)?
                .ok_or_else(|| unsupported("upgrade previous card binding drift"))?;
            objects
                .arena
                .apply(&Mutation::Pop {
                    target: Target::Object(hand),
                    index: index as i64,
                })
                .map_err(unsupported)?;
            self.upgrade_card_state_bound(objects, previous, "RETIRED", CardLifecycle::Retired)?;
        }
        self.upgrade_card_state_bound(objects, chosen, "HAND", CardLifecycle::Hand)?;
        self.world.card_mut(chosen).unwrap().facedown = false;
        objects
            .arena
            .set_physical_field(
                chosen.0,
                objects.phases.k(Field::Facedown),
                Datum::Bool(false),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .append(hand, ObjectValue::Atom(Datum::Instance(chosen.0)))
            .map_err(unsupported)?;
        if let Some(pair) = pair {
            if let Some(stat) =
                self.world.definitions.cards[self.world.card(pair).unwrap().definition].item
            {
                let items = match objects.arena.field(model, self.cleanup_key("items")) {
                    Some(ObjectValue::Object(id)) if objects.arena.kind(id) == ObjectKind::Dict => {
                        id
                    }
                    _ => return Err(unsupported("upgrade items mapping missing")),
                };
                let key = self.cleanup_key(stat.name());
                let n = match objects.arena.field(items, key) {
                    None => 0,
                    Some(ObjectValue::Atom(Datum::Int(n))) => n,
                    _ => return Err(unsupported("upgrade item value must be an integer")),
                }
                .checked_add(1)
                .ok_or_else(|| unsupported("upgrade item overflow"))?;
                objects
                    .arena
                    .set_field(items, key, ObjectValue::Atom(Datum::Int(n)))
                    .map_err(unsupported)?;
                let h = self.world.hero_row_mut(hero).unwrap();
                h.items[stat.index()] = n;
                if !h.items_order.contains(&stat) {
                    h.items_order.push(stat);
                }
            }
            self.upgrade_card_state_bound(objects, pair, "ITEM", CardLifecycle::Item)?;
        }
        let pending = self.battle_root_map(objects, "pending_upgrades")?;
        if let Some(ObjectValue::Atom(Datum::Int(n))) = objects.arena.field(pending, hero) {
            let n = n
                .checked_sub(1)
                .ok_or_else(|| unsupported("upgrade count overflow"))?;
            if n <= 0 {
                objects
                    .arena
                    .apply(&Mutation::Delete {
                        target: Target::Object(pending),
                        key: hero,
                    })
                    .map_err(unsupported)?;
            } else {
                objects
                    .arena
                    .set_field(pending, hero, ObjectValue::Atom(Datum::Int(n)))
                    .map_err(unsupported)?;
            }
        }
        self.sync_upgrade_state_bound(objects)?;
        self.world.refresh_card_auras();
        Ok(())
    }
    fn round_reset_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let round = match objects.arena.root_field(self.cleanup_key("round")) {
            Some(ObjectValue::Atom(Datum::Int(n))) => n
                .checked_add(1)
                .ok_or_else(|| unsupported("round overflow"))?,
            _ => return Err(unsupported("round must be an integer")),
        };
        objects
            .arena
            .set_root(
                self.cleanup_key("round"),
                ObjectValue::Atom(Datum::Int(round)),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .set_root(
                objects.phases.k(Field::Turn),
                ObjectValue::Atom(Datum::Int(1)),
            )
            .map_err(unsupported)?;
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        let snapshot = objects
            .arena
            .dictionary(
                objects
                    .arena
                    .fields(locations)
                    .ok_or_else(|| unsupported("positions mapping missing"))?
                    .to_vec(),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .set_root(objects.phases.k(Field::Snapshot), snapshot)
            .map_err(unsupported)?;
        self.round.round = round;
        self.world.combat.round = round;
        self.world.combat.turn = 1;
        self.boundary.snapshot(&self.world);
        self.phase_set(objects, Phase::Planning)?;
        let defeated = objects.phases.root(&objects.arena, Field::DefeatedHeroes)?;
        objects.arena.clear_list(defeated).map_err(unsupported)?;
        self.world.combat.defeated.clear();
        Ok(())
    }
    fn upgrade_selection_bound(&self, objects: &BoundObjects, value: &ObjectValue) -> Datum {
        let ObjectValue::Object(id) = value else {
            return Datum::Null;
        };
        if objects.arena.kind(*id) != ObjectKind::Dict {
            return Datum::Null;
        }
        let mut fields = Vec::new();
        for name in ["hero_id", "card_id", "card_ids"] {
            let Some(key) = self.symbols.lookup(name) else {
                continue;
            };
            if let Some(value) = objects.arena.field(*id, key) {
                let value = match value {
                    ObjectValue::Atom(value) => value,
                    ObjectValue::Object(list) if objects.arena.is_mutable_list(list) => {
                        Datum::List(
                            objects
                                .arena
                                .sequence(list)
                                .unwrap()
                                .iter()
                                .map(|v| match v {
                                    ObjectValue::Atom(value) => value.clone(),
                                    _ => Datum::Null,
                                })
                                .collect(),
                        )
                    }
                    _ => Datum::Null,
                };
                fields.push((key, value));
            }
        }
        Datum::Object(fields)
    }
    fn round_transition_direct_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        reset: bool,
    ) -> Result<Option<NativeRequest>> {
        self.cleanup_prepare_keys(objects);
        if reset {
            self.round_reset_bound(objects)?;
            return Ok(None);
        }
        self.sync_upgrade_state_bound(objects)?;
        let rules = self
            .round_rules
            .as_ref()
            .ok_or_else(|| unsupported("upgrade rules missing"))?
            .clone();
        let pending_input = objects
            .arena
            .field(step, objects.pending)
            .ok_or_else(|| unsupported("upgrade pending input missing"))?;
        if objects.arena.truthy(&pending_input) {
            if let Some(selected) = objects.selected(step, self.selection_key) {
                let value = self.upgrade_selection_bound(objects, &selected);
                if let Some(choice) = rules
                    .validate(&self.world, &self.round, &value)
                    .map_err(card_error)?
                {
                    for card in choice.cards {
                        self.apply_upgrade_bound(objects, choice.hero, card)?;
                    }
                }
            }
            objects.clear_pending(step)?;
        }
        if self.round.pending.is_empty() {
            let reveals = self.battle_root_map(objects, "upgrade_reveal_snapshot")?;
            objects
                .arena
                .clear_dictionary(reveals)
                .map_err(unsupported)?;
            self.round.reveal.clear();
            self.phase_generation_check(0, 1)?;
            let next = self.phase_step(objects, "round_reset")?;
            self.push_live_steps_bound(objects, &[next])?;
            return Ok(None);
        }
        if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: rules.simultaneous,
            prompt: rules.prompt,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Upgrades(rules.players(&self.world, &self.round)),
            can_skip: false,
            discard: false,
        }))
    }
    pub(super) fn round_transition_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        reset: bool,
    ) -> Result<Option<NativeRequest>> {
        self.transition_bound(objects, step, |candidate, objects, frame| {
            candidate
                .round_transition_direct_bound(objects, step, reset)?
                .map(|request| candidate.issue_bound_request(objects, step, frame, request))
                .transpose()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::object_session;
    use super::*;
    fn answer(worker: &mut NativeSession, request: &NativeRequest, side: usize) -> Datum {
        let RequestChoices::Upgrades(players) = &request.choices else {
            panic!("upgrade request expected")
        };
        let p = &players[0];
        let chosen = p.options[0].pair[side];
        let printed = worker.world.definitions.cards[worker.world.card(chosen).unwrap().definition]
            .printed_id;
        worker.import_selection(&serde_json::json!({"hero_id":worker.symbols.name(p.hero),"card_id":worker.symbols.name(printed)})).unwrap()
    }
    #[test]
    fn request_exhaustion_rejects_before_pop_and_existing_request_is_reused() {
        let mut worker = object_session("object-execution-upgrade-live-Arien-1-single-0");
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
    fn final_upgrade_generation_guard_preserves_cards_items_pending_and_reveals() {
        let mut worker = object_session("object-execution-upgrade-live-pending-valid");
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
    fn invalid_batch_submit_keeps_issued_token_and_all_authoritative_state() {
        let mut worker = object_session("object-execution-upgrade-live-Arien-3-single-0");
        let req = worker.advance_prefix(1).unwrap().request.unwrap();
        let first = answer(&mut worker, &req, 0).to_json(&worker.symbols);
        let invalid=worker.import_selection(&serde_json::json!({"hero_id":first["hero_id"],"card_ids":[first["card_id"],first["card_id"]]})).unwrap();
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .submit(req.id, invalid)
                .unwrap_err()
                .to_string()
                .contains("Choose one legal upgrade")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
        let valid = answer(&mut worker, &req, 0);
        worker.submit(req.id, valid).unwrap();
        worker.advance_prefix(1).unwrap();
        assert_eq!(worker.round.pending[0].1, 2);
    }
    #[test]
    fn upgrade_clones_keep_physical_hands_items_and_pending_maps_private() {
        let mut root = object_session("object-execution-upgrade-live-Arien-3-single-0");
        let req = root.advance_prefix(1).unwrap().request.unwrap();
        let before = root.export_envelope().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        for (worker, side) in [(&mut a, 0), (&mut b, 1)] {
            let selection = answer(worker, &req, side);
            worker.submit(req.id, selection).unwrap();
            worker.advance_prefix(1).unwrap();
        }
        assert_ne!(
            a.export_envelope().unwrap().state,
            b.export_envelope().unwrap().state
        );
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn shared_item_dictionary_refreshes_every_hero_cache() {
        let mut worker = object_session("object-execution-upgrade-live-shared-items");
        let req = worker.advance_prefix(1).unwrap().request.unwrap();
        let selection = answer(&mut worker, &req, 0);
        worker.submit(req.id, selection).unwrap();
        worker.advance_prefix(1).unwrap();
        let a = worker.symbols.lookup("hero_arien").unwrap();
        let b = worker.symbols.lookup("hero_hanu").unwrap();
        assert_eq!(
            worker.world.hero(a).unwrap().items,
            worker.world.hero(b).unwrap().items
        );
        assert!(worker.world.hero(a).unwrap().items.iter().any(|n| *n > 0));
    }
    #[test]
    fn reset_requires_no_request_or_step_generation_and_retains_defeated_alias() {
        let mut worker = object_session("object-execution-upgrade-live-reset-defeated");
        worker.next_step = u64::MAX;
        worker.next_request = u64::MAX;
        worker.advance_prefix(1).unwrap();
        assert!(worker.world.combat.defeated.is_empty());
        assert_eq!(worker.planning.phase, Phase::Planning);
        assert_eq!(worker.round.round, 8);
        assert_eq!(worker.next_step, u64::MAX);
    }
}
