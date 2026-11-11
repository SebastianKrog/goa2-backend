//! End-of-round cleanup preserves authoritative arena identities.
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, NativeEvent, NativeSession, ObjectId, ObjectValue, Phase, Result,
    Symbol, card_error, unsupported,
};
use crate::{
    cards::CardId,
    objects::{Mutation, Target},
    world::CardLifecycle,
};

impl NativeSession {
    pub(super) fn cleanup_key(&self, name: &str) -> Symbol {
        self.symbols.lookup(name).unwrap()
    }
    pub(super) fn cleanup_prepare_keys(&mut self, objects: &mut BoundObjects) {
        for name in [
            "round",
            "pending_upgrades",
            "upgrade_reveal_snapshot",
            "gold",
            "level",
            "LEVEL_UP",
            "DECK",
            "HAND",
            "RETIRED",
            "ITEM",
            "items",
            "enraged_active_override",
            "persists_end_of_round",
        ] {
            if self.symbols.lookup(name).is_none() {
                Arc::make_mut(&mut self.symbols).intern(name);
            }
        }
        objects.arena.symbols = self.symbols.clone();
    }
    pub(super) fn cleanup_list(
        &self,
        objects: &BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<ObjectId> {
        match objects.arena.field(model, objects.phases.k(field)) {
            Some(ObjectValue::Object(id)) if objects.arena.is_mutable_list(id) => Ok(id),
            _ => Err(unsupported("cleanup card list missing or immutable")),
        }
    }
    fn cleanup_cards(&self, objects: &BoundObjects, list: ObjectId) -> Result<Vec<Option<CardId>>> {
        objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("cleanup cards must be a list"))?
            .iter()
            .map(|v| match v {
                ObjectValue::Atom(Datum::Instance(id)) => Ok(Some(CardId(*id))),
                ObjectValue::Atom(Datum::Null) => Ok(None),
                _ => Err(unsupported("cleanup card instance invalid")),
            })
            .collect()
    }
    pub(super) fn cleanup_equal(
        &self,
        objects: &BoundObjects,
        list: ObjectId,
        target: CardId,
    ) -> Result<Option<usize>> {
        for (index, card) in self.cleanup_cards(objects, list)?.into_iter().enumerate() {
            if let Some(card) = card
                && self
                    .world
                    .card_model_equal(card, target)
                    .map_err(unsupported)?
            {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }
    fn cleanup_card_flag(
        &mut self,
        objects: &mut BoundObjects,
        card: CardId,
        field: Field,
        value: bool,
    ) -> Result<()> {
        objects
            .arena
            .set_physical_field(card.0, objects.phases.k(field), Datum::Bool(value))
            .map_err(unsupported)?;
        let row = self.world.card_mut(card).unwrap();
        match field {
            Field::Facedown => row.facedown = value,
            Field::PlayedRound => row.played_this_round = value,
            _ => return Err(unsupported("cleanup card flag not bound")),
        }
        Ok(())
    }
    fn cleanup_expire_card(&mut self, objects: &mut BoundObjects, card: CardId) -> Result<()> {
        let printed =
            self.world.definitions.cards[self.world.card(card).unwrap().definition].printed_id;
        let rows = self
            .world
            .effects
            .instances
            .iter()
            .enumerate()
            .filter_map(|(i, e)| (e.card == Some(printed)).then_some(i))
            .collect::<Vec<_>>();
        let before = self
            .world
            .cards()
            .iter()
            .map(|c| c.active)
            .collect::<Vec<_>>();
        self.world.expire_effects_by_card(printed);
        objects.remove_effect_rows(&rows)?;
        self.sync_effect_card_flags(objects, &before)
    }
    fn cleanup_return_card(
        &mut self,
        objects: &mut BoundObjects,
        model: ObjectId,
        card: CardId,
    ) -> Result<()> {
        self.world.normal_card(card).map_err(card_error)?;
        let hand = self.cleanup_list(objects, model, Field::Hand)?;
        let played = self.cleanup_list(objects, model, Field::Played)?;
        let discard = self.cleanup_list(objects, model, Field::Discard)?;
        if self.cleanup_equal(objects, hand, card)?.is_some() {
            return Err(unsupported(
                "cleanup already-in-hand card needs source partial-error fidelity",
            ));
        }
        self.world.change_lifecycle(card, CardLifecycle::Hand);
        self.world.card_mut(card).unwrap().facedown = false;
        objects
            .arena
            .set_physical_field(
                card.0,
                objects.phases.k(Field::CardState),
                Datum::Text(self.cleanup_key("HAND")),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .remove_physical_field(card.0, objects.phases.k(Field::InitiativeOverride))
            .map_err(unsupported)?;
        objects
            .arena
            .set_physical_field(
                card.0,
                objects.phases.k(Field::Facedown),
                Datum::Bool(false),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .append(hand, ObjectValue::Atom(Datum::Instance(card.0)))
            .map_err(unsupported)?;
        if let Some(index) = self.cleanup_equal(objects, played, card)? {
            objects
                .arena
                .set_index(played, index, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        if let Some(index) = self.cleanup_equal(objects, discard, card)? {
            objects
                .arena
                .apply(&Mutation::Pop {
                    target: Target::Object(discard),
                    index: index as i64,
                })
                .map_err(unsupported)?;
        }
        if let Some(ObjectValue::Atom(Datum::Instance(current))) =
            objects.arena.field(model, objects.phases.k(Field::Current))
            && self
                .world
                .card_model_equal(CardId(current), card)
                .map_err(unsupported)?
        {
            objects
                .arena
                .set_field(
                    model,
                    objects.phases.k(Field::Current),
                    ObjectValue::Atom(Datum::Null),
                )
                .map_err(unsupported)?;
        }
        self.sync_card_collections_bound(objects)
    }
    fn cleanup_unlock(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        model: ObjectId,
        events: &mut Vec<NativeEvent>,
    ) -> Result<()> {
        let row = self.world.hero(hero).unwrap();
        let Some(ultimate) = row.ultimate else {
            return Ok(());
        };
        let effect =
            self.world.definitions.cards[self.world.card(ultimate).unwrap().definition].effect;
        if !self
            .round_rules
            .as_ref()
            .ok_or_else(|| unsupported("cleanup round rules missing"))?
            .has_unlock_hook(effect)
        {
            return Ok(());
        }
        match self.symbols.name(effect) {
            "unbound_fury" => {
                let cards = row
                    .deck
                    .iter()
                    .chain(&row.hand)
                    .chain(row.played.iter().flatten())
                    .chain(&row.discard)
                    .chain(row.ultimate.iter())
                    .copied()
                    .collect::<Vec<_>>();
                for card in cards {
                    self.world.card_mut(card).unwrap().enraged = true;
                    objects
                        .arena
                        .set_physical_field(
                            card.0,
                            self.cleanup_key("enraged_active_override"),
                            Datum::Bool(true),
                        )
                        .map_err(unsupported)?;
                }
            }
            "ready_for_war" => {
                let silver = row.deck.iter().copied().find(|id| {
                    self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                        .color
                        .is_some_and(|color| self.symbols.name(color) == "SILVER")
                });
                let Some(silver) = silver else {
                    return Ok(());
                };
                if self.world.card(silver).unwrap().lifecycle == CardLifecycle::Deck {
                    return Ok(());
                }
                let printed = self.world.definitions.cards
                    [self.world.card(silver).unwrap().definition]
                    .printed_id;
                for field in [Field::Hand, Field::Discard] {
                    let list = self.cleanup_list(objects, model, field)?;
                    if let Some(index) = self.cleanup_equal(objects, list, silver)? {
                        objects
                            .arena
                            .apply(&Mutation::Pop {
                                target: Target::Object(list),
                                index: index as i64,
                            })
                            .map_err(unsupported)?;
                    }
                }
                let played = self.cleanup_list(objects, model, Field::Played)?;
                for (index, card) in self.cleanup_cards(objects, played)?.into_iter().enumerate() {
                    if card.is_some_and(|id| {
                        self.world.definitions.cards[self.world.card(id).unwrap().definition]
                            .printed_id
                            == printed
                    }) {
                        objects
                            .arena
                            .set_index(played, index, ObjectValue::Atom(Datum::Null))
                            .map_err(unsupported)?;
                    }
                }
                let current = match objects.arena.field(model, objects.phases.k(Field::Current)) {
                    Some(ObjectValue::Atom(Datum::Instance(id))) => Some(CardId(id)),
                    _ => None,
                };
                if current.is_some_and(|id| {
                    self.world.definitions.cards[self.world.card(id).unwrap().definition].printed_id
                        == printed
                }) {
                    objects
                        .arena
                        .set_field(
                            model,
                            objects.phases.k(Field::Current),
                            ObjectValue::Atom(Datum::Null),
                        )
                        .map_err(unsupported)?;
                }
                self.world.change_lifecycle(silver, CardLifecycle::Deck);
                for (field, value) in [(Field::Facedown, false), (Field::PlayedRound, false)] {
                    self.cleanup_card_flag(objects, silver, field, value)?;
                }
                self.world.card_mut(silver).unwrap().active = false;
                objects
                    .arena
                    .set_physical_field(
                        silver.0,
                        objects.phases.k(Field::CardState),
                        Datum::Text(self.cleanup_key("DECK")),
                    )
                    .map_err(unsupported)?;
                objects
                    .arena
                    .remove_physical_field(silver.0, objects.phases.k(Field::InitiativeOverride))
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_physical_field(silver.0, objects.active, Datum::Bool(false))
                    .map_err(unsupported)?;
                self.sync_card_collections_bound(objects)?;
                events.push(NativeEvent::Round(crate::round::RoundEvent::DeckSwapped {
                    hero,
                    card: printed,
                }));
                let golds = self
                    .world
                    .hero(hero)
                    .unwrap()
                    .deck
                    .iter()
                    .copied()
                    .filter(|id| {
                        let card = self.world.card(*id).unwrap();
                        card.lifecycle == CardLifecycle::Deck
                            && self.world.definitions.cards[card.definition]
                                .color
                                .is_some_and(|color| self.symbols.name(color) == "GOLD")
                    })
                    .collect::<Vec<_>>();
                for card in golds {
                    self.cleanup_return_card(objects, model, card)?;
                    events.push(NativeEvent::Round(crate::round::RoundEvent::Retrieved {
                        hero,
                        card: self.world.definitions.cards
                            [self.world.card(card).unwrap().definition]
                            .printed_id,
                    }));
                }
            }
            _ => return Err(unsupported("cleanup ultimate hook not bound")),
        }
        Ok(())
    }
    fn cleanup_direct_bound(&mut self, objects: &mut BoundObjects) -> Result<Vec<NativeEvent>> {
        self.cleanup_prepare_keys(objects);
        self.sync_card_collections_bound(objects)?;
        self.round.round = match objects.arena.root_field(self.cleanup_key("round")) {
            Some(ObjectValue::Atom(Datum::Int(round))) => round,
            _ => return Err(unsupported("cleanup round must be an integer")),
        };
        let before = self
            .world
            .cards()
            .iter()
            .map(|c| c.active)
            .collect::<Vec<_>>();
        let expiry = self
            .world
            .expire_duration_effects(crate::effects::Duration::ThisRound);
        objects.remove_effect_nodes(&expiry.ids)?;
        self.sync_effect_card_flags(objects, &before)?;
        let markers = objects.phases.root(&objects.arena, Field::Markers)?;
        for (_, value) in objects
            .arena
            .fields(markers)
            .ok_or_else(|| unsupported("cleanup markers mapping missing"))?
            .to_vec()
        {
            let ObjectValue::Object(marker) = value else {
                return Err(unsupported("cleanup marker model missing"));
            };
            for (field, value) in [
                (Field::TargetId, Datum::Null),
                (Field::Value, Datum::Int(0)),
                (Field::SourceId, Datum::Null),
            ] {
                objects
                    .arena
                    .set_field(marker, objects.phases.k(field), ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
        }
        for marker in &mut self.world.markers {
            marker.target = None;
            marker.source = None;
            marker.value = 0;
        }
        let stale = self
            .world
            .effects
            .instances
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                (e.card.is_some() && !e.active && e.duration != crate::effects::Duration::Passive)
                    .then_some(i)
            })
            .collect::<Vec<_>>();
        self.world.cleanup_stale_effects();
        objects.remove_effect_rows(&stale)?;
        for (hero, model) in objects.phases.heroes.clone() {
            self.sync_card_collections_bound(objects)?;
            let row = self.world.hero(hero).unwrap();
            let expiring = row
                .played
                .iter()
                .flatten()
                .chain(row.current.iter())
                .copied()
                .collect::<Vec<_>>();
            for card in expiring {
                self.cleanup_expire_card(objects, card)?;
            }
            let row = self.world.hero(hero).unwrap();
            let cards = row
                .played
                .iter()
                .flatten()
                .chain(&row.discard)
                .chain(row.current.iter())
                .copied()
                .collect::<Vec<_>>();
            for card in cards {
                self.cleanup_card_flag(objects, card, Field::Facedown, false)?;
                self.cleanup_card_flag(objects, card, Field::PlayedRound, false)?;
                self.cleanup_return_card(objects, model, card)?;
            }
            let played = self.cleanup_list(objects, model, Field::Played)?;
            objects.arena.clear_list(played).map_err(unsupported)?;
            objects
                .arena
                .set_field(
                    model,
                    objects.phases.k(Field::ResolvedCount),
                    ObjectValue::Atom(Datum::Int(0)),
                )
                .map_err(unsupported)?;
            self.world.hero_row_mut(hero).unwrap().resolved_turns = 0;
            self.sync_card_collections_bound(objects)?;
        }
        let mut events = Vec::new();
        for (token, persists) in self.cleanup_token_supply_bound(objects)? {
            if !persists && let Some(from) = self.world.position(token) {
                let count = self.world.effects.instances.len();
                self.remove_mine_spatial_bound(objects, token)?;
                events.push(NativeEvent::Round(crate::round::RoundEvent::TokenRemoved {
                    token,
                    from,
                    effects_removed: count - self.world.effects.instances.len(),
                }));
            }
        }
        let pending = objects
            .arena
            .root_field(self.cleanup_key("pending_upgrades"))
            .and_then(|v| {
                if let ObjectValue::Object(id) = v {
                    Some(id)
                } else {
                    None
                }
            })
            .ok_or_else(|| unsupported("cleanup pending mapping missing"))?;
        let reveals = objects
            .arena
            .root_field(self.cleanup_key("upgrade_reveal_snapshot"))
            .and_then(|v| {
                if let ObjectValue::Object(id) = v {
                    Some(id)
                } else {
                    None
                }
            })
            .ok_or_else(|| unsupported("cleanup reveal mapping missing"))?;
        let mut leveled = false;
        for (hero, model) in objects.phases.heroes.clone() {
            let integer = |key| match objects.arena.field(model, key) {
                Some(ObjectValue::Atom(Datum::Int(n))) => Ok(n),
                _ => Err(unsupported("cleanup level/gold must be integers")),
            };
            let mut level = integer(self.cleanup_key("level"))?;
            let mut gold = integer(self.cleanup_key("gold"))?;
            let mut upgrades = 0;
            let mut ultimate = false;
            while level < 8 {
                if level < 1 {
                    return Err(unsupported("cleanup invalid hero level"));
                }
                if gold < level {
                    break;
                }
                gold -= level;
                level += 1;
                leveled = true;
                objects
                    .arena
                    .set_field(
                        model,
                        self.cleanup_key("gold"),
                        ObjectValue::Atom(Datum::Int(gold)),
                    )
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_field(
                        model,
                        self.cleanup_key("level"),
                        ObjectValue::Atom(Datum::Int(level)),
                    )
                    .map_err(unsupported)?;
                self.world.hero_row_mut(hero).unwrap().gold = gold;
                self.world.hero_row_mut(hero).unwrap().level = level;
                if level == 8 {
                    ultimate = true;
                    self.cleanup_unlock(objects, hero, model, &mut events)?;
                } else {
                    upgrades += 1;
                }
            }
            if upgrades > 0 {
                objects
                    .arena
                    .set_field(pending, hero, ObjectValue::Atom(Datum::Int(upgrades)))
                    .map_err(unsupported)?;
                let items = match objects.arena.field(model, objects.phases.k(Field::Items)) {
                    Some(ObjectValue::Object(id)) => id,
                    _ => return Err(unsupported("cleanup items mapping missing")),
                };
                let copy = objects
                    .arena
                    .dictionary(
                        objects
                            .arena
                            .fields(items)
                            .ok_or_else(|| unsupported("cleanup items not a mapping"))?
                            .to_vec(),
                    )
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_field(reveals, hero, copy)
                    .map_err(unsupported)?;
            } else if !ultimate {
                gold = gold
                    .checked_add(1)
                    .ok_or_else(|| unsupported("cleanup gold overflow"))?;
                self.world.hero_row_mut(hero).unwrap().gold = gold;
                objects
                    .arena
                    .set_field(
                        model,
                        self.cleanup_key("gold"),
                        ObjectValue::Atom(Datum::Int(gold)),
                    )
                    .map_err(unsupported)?;
            }
        }
        if leveled {
            self.planning.phase = Phase::LevelUp;
            objects
                .arena
                .set_root(
                    objects.phases.k(Field::Phase),
                    ObjectValue::Atom(Datum::Text(self.cleanup_key("LEVEL_UP"))),
                )
                .map_err(unsupported)?;
        }
        self.round.pending = objects
            .arena
            .fields(pending)
            .unwrap()
            .iter()
            .map(|(hero, v)| match v {
                ObjectValue::Atom(Datum::Int(n)) => Ok((*hero, *n)),
                _ => Err(unsupported("cleanup upgrade count invalid")),
            })
            .collect::<Result<Vec<_>>>()?;
        self.round.reveal = objects
            .arena
            .fields(reveals)
            .unwrap()
            .iter()
            .map(|(hero, value)| {
                let ObjectValue::Object(items) = value else {
                    return Err(unsupported("cleanup reveal must be a mapping"));
                };
                let items = objects
                    .arena
                    .fields(*items)
                    .ok_or_else(|| unsupported("cleanup reveal items missing"))?
                    .iter()
                    .map(|(key, value)| {
                        let stat = crate::stats::Stat::parse(self.symbols.name(*key))
                            .map_err(unsupported)?;
                        let ObjectValue::Atom(Datum::Int(amount)) = value else {
                            return Err(unsupported("cleanup reveal item must be an integer"));
                        };
                        Ok((stat, *amount))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok((*hero, items))
            })
            .collect::<Result<Vec<_>>>()?;
        if !objects.arena.fields(pending).unwrap().is_empty() {
            self.phase_generation_check(0, 1)?;
            let step = self.phase_step(objects, "upgrades")?;
            self.push_live_steps_bound(objects, &[step])?;
        } else {
            self.round.round = self
                .round
                .round
                .checked_add(1)
                .ok_or_else(|| unsupported("cleanup round overflow"))?;
            self.world.combat.round = self.round.round;
            self.world.combat.turn = 1;
            objects
                .arena
                .set_root(
                    self.cleanup_key("round"),
                    ObjectValue::Atom(Datum::Int(self.round.round)),
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
                .dictionary(objects.arena.fields(locations).unwrap().to_vec())
                .map_err(unsupported)?;
            objects
                .arena
                .set_root(objects.phases.k(Field::Snapshot), snapshot)
                .map_err(unsupported)?;
            self.boundary.snapshot(&self.world);
            self.phase_set(objects, Phase::Planning)?;
        }
        self.world.refresh_card_auras();
        Ok(events)
    }
    pub(super) fn cleanup_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Vec<NativeEvent>> {
        self.transition_bound(objects, step, |candidate, objects, _| {
            candidate.cleanup_direct_bound(objects)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::object_session;
    use super::*;
    #[test]
    fn cleanup_generation_guard_leaves_cards_effects_gold_and_stack_unchanged() {
        let mut worker = object_session("object-execution-cleanup-live-level-gold-3");
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
    fn cleanup_ignored_finishing_payload_needs_no_step_generation_capacity() {
        let root = object_session("object-execution-cleanup-live-round-finishing-ignored");
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        worker.next_step = u64::MAX;
        worker.advance_prefix(1).unwrap();
        assert_eq!(worker.next_step, u64::MAX);
        let after = worker.export_envelope().unwrap();
        assert_eq!(after.state["phase"], "PLANNING");
        assert_eq!(after.state["round"], worker.world.combat.round);
        assert_eq!(after.state["turn"], worker.world.combat.turn);
        assert!(
            after.state["execution_context"]
                .get("should_not_finish")
                .is_none()
        );
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn cleanup_upgrade_boundary_restores_the_same_upgrade_prompt() {
        let mut worker = object_session("object-execution-cleanup-live-level-gold-3");
        worker.advance_prefix(1).unwrap();
        let mut restored = NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
        let a = worker.advance_prefix(1).unwrap().request.unwrap();
        let b = restored.advance_prefix(1).unwrap().request.unwrap();
        assert_eq!(worker.request_json(&a), restored.request_json(&b));
        assert_eq!(
            worker.export_envelope().unwrap().state,
            restored.export_envelope().unwrap().state
        );
        assert_eq!(
            worker.export_envelope().unwrap().object_bindings,
            restored.export_envelope().unwrap().object_bindings
        );
    }
    #[test]
    fn cleanup_uses_the_live_physical_token_persistence_override() {
        let mut worker = object_session("object-execution-cleanup-live-temporary-token");
        let physical = worker
            .source
            .instances
            .iter()
            .position(|row| {
                worker.source.definitions[row.definition as usize].fields["id"] == "cleanup-token"
            })
            .unwrap() as u32;
        worker
            .with_live_objects(|session, objects| {
                session.cleanup_prepare_keys(objects);
                objects
                    .arena
                    .set_physical_field(
                        physical,
                        session.cleanup_key("persists_end_of_round"),
                        Datum::Bool(true),
                    )
                    .map_err(unsupported)
            })
            .unwrap();
        worker.advance_prefix(1).unwrap();
        let token = worker.symbols.lookup("cleanup-token").unwrap();
        assert!(worker.world.position(token).is_some());
        assert!(!worker.world.effects.instances.is_empty());
    }
}
