//! Live planning commands preserve source collection replacement and aliasing.
use super::phases::Field;
use super::*;
use crate::cards::CardId;

impl NativeSession {
    fn hero_card_list_bound(
        &self,
        objects: &BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<Vec<Option<CardId>>> {
        let Some(ObjectValue::Object(list)) = objects.arena.field(model, objects.phases.k(field))
        else {
            return Err(unsupported("planning card collection missing"));
        };
        objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("planning cards must be a list"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Atom(Datum::Instance(card))
                    if self.world.card(CardId(*card)).is_some() =>
                {
                    Ok(Some(CardId(*card)))
                }
                ObjectValue::Atom(Datum::Null) if matches!(field, Field::Played) => Ok(None),
                _ => Err(unsupported(
                    "planning physical-card collection binding missing",
                )),
            })
            .collect()
    }
    // An in-place list edit can affect another hero or another collection role.
    // Refresh every derived collection from the live handles, never serialized JSON.
    pub(super) fn sync_card_collections_bound(&mut self, objects: &BoundObjects) -> Result<()> {
        for &(hero, model) in &objects.phases.heroes {
            let index = self
                .world
                .heroes
                .iter()
                .position(|row| row.id == hero)
                .ok_or_else(|| unsupported("planning hero cache missing"))?;
            let deck = self.hero_card_list_bound(objects, model, Field::Deck)?;
            let hand = self.hero_card_list_bound(objects, model, Field::Hand)?;
            let discard = self.hero_card_list_bound(objects, model, Field::Discard)?;
            let spells = self.hero_card_list_bound(objects, model, Field::Spells)?;
            let played = self.hero_card_list_bound(objects, model, Field::Played)?;
            let current = self.model_card_bound(objects, model, Field::Current)?;
            let extra = self.model_card_bound(objects, model, Field::Extra)?;
            let ultimate = self.model_card_bound(objects, model, Field::Ultimate)?;
            let row = &mut self.world.heroes[index];
            row.deck.clear();
            row.deck.extend(deck.into_iter().flatten());
            row.hand.clear();
            row.hand.extend(hand.into_iter().flatten());
            row.discard.clear();
            row.discard.extend(discard.into_iter().flatten());
            row.spells.clear();
            row.spells.extend(spells.into_iter().flatten());
            row.played = played;
            row.current = current;
            row.extra = extra;
            row.ultimate = ultimate;
        }
        self.world.refresh_card_auras();
        Ok(())
    }
    fn planning_hand_bound(&self, objects: &BoundObjects, model: ObjectId) -> Result<ObjectId> {
        match objects.arena.field(model, objects.phases.k(Field::Hand)) {
            Some(ObjectValue::Object(hand)) if objects.arena.is_mutable_list(hand) => Ok(hand),
            _ => Err(unsupported("planning hand must be a mutable list")),
        }
    }
    fn planning_card_scalars_bound(
        &mut self,
        objects: &mut BoundObjects,
        card: CardId,
        committed: bool,
    ) -> Result<()> {
        let physical = self
            .world
            .card(card)
            .ok_or_else(|| unsupported("planning physical card missing"))?
            .source_instance;
        let p = &objects.phases;
        for (key, value) in [
            (
                Field::CardState,
                Datum::Text(p.k(if committed {
                    Field::UnresolvedState
                } else {
                    Field::HandState
                })),
            ),
            (Field::Facedown, Datum::Bool(committed)),
            (Field::PlayedRound, Datum::Bool(committed)),
        ] {
            objects
                .arena
                .set_physical_field(physical, p.k(key), value)
                .map_err(unsupported)?;
        }
        if !committed {
            objects
                .arena
                .remove_physical_field(physical, p.k(Field::InitiativeOverride))
                .map_err(unsupported)?;
        }
        Ok(())
    }
    pub(in crate::interpreter) fn commit_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        card: CardId,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Ok(());
        }
        let Some(model) = self.hero_model_bound(objects, hero) else {
            return Ok(());
        };
        self.sync_card_collections_bound(objects)?;
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let second = objects.phases.root(&objects.arena, Field::Second)?;
        let done = objects.phases.root(&objects.arena, Field::Done)?;
        let is_second = if let Some(first) = objects.arena.field(pending, hero) {
            if matches!(first, ObjectValue::Atom(Datum::Null))
                || !self.planning.two_cards(&self.world, hero)
            {
                return Err(EngineError::InvalidInput("already committed".into()));
            }
            if objects.arena.field(second, hero).is_some()
                || objects
                    .arena
                    .sequence(done)
                    .ok_or_else(|| unsupported("planning done list missing"))?
                    .iter()
                    .any(|value| matches!(value, ObjectValue::Atom(Datum::Text(id)) if *id == hero))
            {
                return Err(EngineError::InvalidInput("planning already closed".into()));
            }
            true
        } else {
            false
        };
        let hand = self.planning_hand_bound(objects, model)?;
        let cards = self.hero_card_list_bound(objects, model, Field::Hand)?;
        let Some(index) = cards.iter().position(|id| *id == Some(card)) else {
            let row = self
                .world
                .card(card)
                .ok_or_else(|| unsupported("runtime copied planning card is not bound"))?;
            let printed = self.world.definitions.cards[row.definition].printed_id;
            if cards.iter().flatten().any(|id| {
                self.world.card(*id).is_some_and(|row| {
                    self.world.definitions.cards[row.definition].printed_id == printed
                })
            }) {
                return Err(unsupported(
                    "foreign printed-card membership requires native model equality",
                ));
            }
            return Ok(());
        };
        match self.world.play_card(hero, card) {
            Ok(()) => {}
            Err(crate::cards::CardError::Illegal(_)) => return Ok(()),
            Err(error) => return Err(card_error(error)),
        }
        objects
            .arena
            .apply(&crate::objects::Mutation::Pop {
                target: crate::objects::Target::Object(hand),
                index: index as i64,
            })
            .map_err(unsupported)?;
        self.planning_card_scalars_bound(objects, card, true)?;
        objects
            .arena
            .set_field(
                model,
                objects.phases.k(Field::Current),
                ObjectValue::Atom(Datum::Instance(card.0)),
            )
            .map_err(unsupported)?;
        if !is_second {
            self.clear_planning_requests_bound(objects, hero)?;
        }
        objects
            .arena
            .set_field(
                if is_second { second } else { pending },
                hero,
                ObjectValue::Atom(Datum::Instance(card.0)),
            )
            .map_err(unsupported)?;
        self.sync_card_collections_bound(objects)?;
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        self.check_planning_transition_bound(objects)
    }
    pub(in crate::interpreter) fn uncommit_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Err(EngineError::InvalidInput(
                "cannot uncommit outside planning".into(),
            ));
        }
        let model = self
            .hero_model_bound(objects, hero)
            .ok_or_else(|| EngineError::InvalidInput("unknown planning hero".into()))?;
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let first = match objects.arena.field(pending, hero) {
            None | Some(ObjectValue::Atom(Datum::Null)) => {
                return Err(EngineError::InvalidInput("no committed card".into()));
            }
            Some(ObjectValue::Atom(Datum::Instance(card))) => CardId(card),
            _ => return Err(unsupported("pending physical card missing")),
        };
        let second = objects.phases.root(&objects.arena, Field::Second)?;
        let extra = match objects.arena.field(second, hero) {
            None | Some(ObjectValue::Atom(Datum::Null)) => None,
            Some(ObjectValue::Atom(Datum::Instance(card))) => Some(CardId(card)),
            _ => return Err(unsupported("second physical card missing")),
        };
        let card = extra.unwrap_or(first);
        self.sync_card_collections_bound(objects)?;
        // Unported equality/instance shapes reject before source-visible edits.
        self.world.lifecycle_hero(hero).map_err(card_error)?;
        let normal = self.world.normal_card(card);
        let hand = self.planning_hand_bound(objects, model)?;
        objects
            .arena
            .apply(&crate::objects::Mutation::Delete {
                target: crate::objects::Target::Object(if extra.is_some() {
                    second
                } else {
                    pending
                }),
                key: hero,
            })
            .map_err(unsupported)?;
        let planning_error = |error| match error {
            crate::cards::CardError::Illegal(message) => EngineError::InvalidInput(message.into()),
            crate::cards::CardError::Unsupported(message) => unsupported(message),
        };
        normal.map_err(planning_error)?;
        self.world.unplay_card(hero, card).map_err(planning_error)?;
        self.planning_card_scalars_bound(objects, card, false)?;
        objects
            .arena
            .append(hand, ObjectValue::Atom(Datum::Instance(card.0)))
            .map_err(unsupported)?;
        objects
            .arena
            .set_field(
                model,
                objects.phases.k(Field::Current),
                ObjectValue::Atom(if extra.is_some() {
                    Datum::Instance(first.0)
                } else {
                    Datum::Null
                }),
            )
            .map_err(unsupported)?;
        let done = objects.phases.root(&objects.arena, Field::Done)?;
        if let Some(index) = objects
            .arena
            .sequence(done)
            .ok_or_else(|| unsupported("planning done list missing"))?
            .iter()
            .position(|value| matches!(value, ObjectValue::Atom(Datum::Text(id)) if *id == hero))
        {
            objects
                .arena
                .apply(&crate::objects::Mutation::Pop {
                    target: crate::objects::Target::Object(done),
                    index: index as i64,
                })
                .map_err(unsupported)?;
        }
        self.sync_card_collections_bound(objects)?;
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        Ok(())
    }
    pub(super) fn hero_hand_nonempty_bound(
        &self,
        objects: &BoundObjects,
        hero: Symbol,
    ) -> Result<bool> {
        let Some(model) = self.hero_model_bound(objects, hero) else {
            return Ok(false);
        };
        let Some(ObjectValue::Object(hand)) =
            objects.arena.field(model, objects.phases.k(Field::Hand))
        else {
            return Err(unsupported("planning hand binding missing"));
        };
        Ok(!objects
            .arena
            .sequence(hand)
            .ok_or_else(|| unsupported("planning hand must be a list"))?
            .is_empty())
    }
    fn check_planning_transition_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let count = objects
            .arena
            .fields(pending)
            .ok_or_else(|| unsupported("planning inputs must be a dictionary"))?
            .len();
        if count >= objects.phases.heroes.len() && !self.pending_planning_open_bound(objects)? {
            self.start_revelation_bound(objects)?;
        }
        Ok(())
    }
    fn clear_planning_requests_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        let p = &objects.phases;
        // Source clear_requests always replaces the dictionary, retaining each
        // surviving request object. References to the old dictionary detach.
        let requests = p.root(&objects.arena, Field::Requests)?;
        let mut retained = Vec::new();
        for (key, value) in objects
            .arena
            .fields(requests)
            .ok_or_else(|| unsupported("starting requests must be a dictionary"))?
        {
            let ObjectValue::Object(request) = value else {
                return Err(unsupported("starting request row missing"));
            };
            let from = objects
                .arena
                .field(*request, p.k(Field::From))
                .ok_or_else(|| unsupported("starting request sender missing"))?;
            let to = objects
                .arena
                .field(*request, p.k(Field::To))
                .ok_or_else(|| unsupported("starting request recipient missing"))?;
            if !matches!(from, ObjectValue::Atom(Datum::Text(id)) if id == hero)
                && !matches!(to, ObjectValue::Atom(Datum::Text(id)) if id == hero)
            {
                retained.push((*key, value.clone()));
            }
        }
        let requests = objects.arena.dictionary(retained).map_err(unsupported)?;
        objects
            .arena
            .set_root(p.k(Field::Requests), requests)
            .map_err(unsupported)?;
        Ok(())
    }
    pub(in crate::interpreter) fn pass_planning_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Err(EngineError::InvalidInput(
                "cannot pass outside planning".into(),
            ));
        }
        if self.hero_model_bound(objects, hero).is_none() {
            return Err(EngineError::InvalidInput("unknown planning hero".into()));
        }
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        if objects.arena.field(pending, hero).is_some() {
            return Err(EngineError::InvalidInput(
                "hero has already committed".into(),
            ));
        }
        if self.hero_hand_nonempty_bound(objects, hero)? {
            return Err(EngineError::InvalidInput(
                "cannot pass with cards in hand".into(),
            ));
        }
        self.clear_planning_requests_bound(objects, hero)?;
        objects
            .arena
            .set_field(pending, hero, ObjectValue::Atom(Datum::Null))
            .map_err(unsupported)?;
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        self.check_planning_transition_bound(objects)
    }
    pub(in crate::interpreter) fn finish_planning_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Ok(());
        }
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        if objects.arena.field(pending, hero).is_none() {
            return Err(EngineError::InvalidInput(
                "must commit before finishing planning".into(),
            ));
        }
        let done = p.root(&objects.arena, Field::Done)?;
        let value = ObjectValue::Atom(Datum::Text(hero));
        if !objects
            .arena
            .sequence(done)
            .ok_or_else(|| unsupported("planning done must be a list"))?
            .iter()
            .any(|item| matches!(item, ObjectValue::Atom(Datum::Text(id)) if *id == hero))
        {
            objects.arena.append(done, value).map_err(unsupported)?;
        }
        self.check_planning_transition_bound(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_releases_detached_request_guards_but_protects_live_shared_rows() {
        let fixture = fixture_case("object-execution-planning-command-pass-requests");
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let Datum::Text(hero) = session
            .import_selection(&fixture["operations"][0]["hero"])
            .unwrap()
        else {
            unreachable!()
        };
        let objects = session.objects.as_ref().unwrap();
        let old = objects
            .phases
            .root(&objects.arena, Field::Requests)
            .unwrap();
        let key = objects.arena.symbols.lookup("keep").unwrap();
        let Some(ObjectValue::Object(kept)) = objects.arena.field(old, key) else {
            unreachable!()
        };
        assert!(objects.readonly.contains(&old));
        assert!(objects.readonly.contains(&kept));
        session.pass_planning(hero).unwrap();
        let objects = session.objects.as_ref().unwrap();
        let current = objects
            .phases
            .root(&objects.arena, Field::Requests)
            .unwrap();
        assert_ne!(old, current);
        assert!(!objects.readonly.contains(&old));
        assert!(objects.readonly.contains(&current));
        assert!(objects.readonly.contains(&kept));
    }

    #[test]
    fn card_planning_refreshes_all_shared_collection_caches_without_changing_other_workers() {
        for name in ["shared-deck", "shared-hero-hand"] {
            let fixture = fixture_case(&format!("object-execution-card-planning-{name}"));
            let root = NativeSession::compile(
                serde_json::from_value(fixture["envelope"].clone()).unwrap(),
            )
            .unwrap();
            let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
            let mut worker = root.clone();
            for operation in fixture["operations"].as_array().unwrap() {
                let Datum::Text(hero) = worker.import_selection(&operation["hero"]).unwrap() else {
                    unreachable!()
                };
                if operation["kind"] == "commit" {
                    worker
                        .commit_card(hero, CardId(operation["card"].as_u64().unwrap() as u32))
                        .unwrap();
                } else {
                    worker.uncommit_card(hero).unwrap();
                }
                if name == "shared-deck" {
                    assert_eq!(worker.world.heroes[0].hand, worker.world.heroes[0].deck);
                } else {
                    assert_eq!(worker.world.heroes[0].hand, worker.world.heroes[1].hand);
                }
                assert_eq!(
                    serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
                    before
                );
            }
        }
    }

    #[test]
    fn two_card_planning_workers_choose_second_or_finish_and_withdraw_independently() {
        let fixture = fixture_case("object-execution-card-planning-dual-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let Datum::Text(hero) = root
            .import_selection(&fixture["operations"][0]["hero"])
            .unwrap()
        else {
            unreachable!()
        };
        let first = CardId(fixture["operations"][0]["card"].as_u64().unwrap() as u32);
        let second = CardId(fixture["operations"][1]["card"].as_u64().unwrap() as u32);
        root.commit_card(hero, first).unwrap();
        let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let mut one = root.clone();
        let mut two = root.clone();
        one.commit_card(hero, second).unwrap();
        two.finish_planning(hero).unwrap();
        assert_eq!(one.world.hero(hero).unwrap().current, Some(second));
        assert_eq!(two.world.hero(hero).unwrap().current, Some(first));
        one.uncommit_card(hero).unwrap();
        two.uncommit_card(hero).unwrap();
        assert_eq!(one.world.hero(hero).unwrap().current, Some(first));
        assert_eq!(two.world.hero(hero).unwrap().current, None);
        assert_eq!(one.world.hero(hero).unwrap().hand.last(), Some(&second));
        assert_eq!(two.world.hero(hero).unwrap().hand.last(), Some(&first));
        assert_eq!(
            serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn foreign_equal_printed_card_is_guarded_before_mutation_until_model_equality_is_bound() {
        let fixture = fixture_case("object-execution-card-planning-override-copy");
        let envelope: StateEnvelope = serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let copy = crate::cards::reference(&envelope.state["execution_context"]["copy"])
            .unwrap()
            .unwrap();
        let mut session = NativeSession::compile(envelope).unwrap();
        let Datum::Text(hero) = session
            .import_selection(&fixture["operations"][0]["hero"])
            .unwrap()
        else {
            unreachable!()
        };
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        let error = session.commit_card(hero, copy).unwrap_err();
        assert!(error.to_string().contains("native model equality"));
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }
}
