//! Physical card exchange across the live hand, discard and turn slots.
use super::{phases::Field, *};
use crate::{cards::CardId, objects::ObjectSlot, world::CardLifecycle};

fn lifecycle_name(lifecycle: CardLifecycle) -> &'static str {
    match lifecycle {
        CardLifecycle::Hand => "HAND",
        CardLifecycle::Deck => "DECK",
        CardLifecycle::Discard => "DISCARD",
        CardLifecycle::Unresolved => "UNRESOLVED",
        CardLifecycle::Resolved => "RESOLVED",
        CardLifecycle::Item => "ITEM",
        CardLifecycle::Passive => "PASSIVE",
        CardLifecycle::Retired => "RETIRED",
        CardLifecycle::Spellbook => "SPELLBOOK",
        CardLifecycle::OutsideSpellbook => "OUTSIDE_SPELLBOOK",
    }
}

impl NativeSession {
    pub(super) fn swap_resolved_cards_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Option<NativeEvent>> {
        let text = |name| self.symbols.lookup(name).and_then(|key| objects.arena.field(step, key));
        let context = |name| -> Result<Option<Symbol>> {
            let Some(key) = self.truthy_symbol_bound(objects, text(name))? else {
                return Ok(None);
            };
            self.truthy_symbol_bound(objects, objects.arena.context_value(key))
        };
        let target = match self.truthy_symbol_bound(objects, text("hero_id"))? {
            Some(target) => Some(target),
            None => context("hero_key")?,
        };
        let (Some(target), Some(a), Some(b)) = (target, context("card_a_key")?, context("card_b_key")?)
        else {
            return Ok(None);
        };
        if a == b {
            return Ok(None);
        }
        self.sync_card_collections_bound(objects)?;
        let Some(model) = self.hero_model_bound(objects, target) else {
            return Ok(None);
        };
        let Some(ObjectValue::Object(played)) = objects.arena.field(model, objects.phases.k(Field::Played))
        else {
            return Err(unsupported("resolved swap played list missing"));
        };
        let cards = objects.arena.sequence(played).ok_or_else(|| unsupported("invalid played list"))?;
        let find = |printed| {
            cards.iter().enumerate().find_map(|(index, value)| {
                let ObjectValue::Atom(Datum::Instance(id)) = value else {
                    return None;
                };
                let card = self.world.card(CardId(*id))?;
                (card.lifecycle == CardLifecycle::Resolved
                    && self.world.definitions.cards[card.definition].printed_id == printed)
                    .then_some((index, CardId(*id), card.facedown, card.played_this_round))
            })
        };
        let (Some(first), Some(second)) = (find(a), find(b)) else {
            return Ok(None);
        };
        self.world.normal_card(first.1).map_err(card_error)?;
        self.world.normal_card(second.1).map_err(card_error)?;
        for (slot, other) in [(first, second), (second, first)] {
            objects.arena.set_index(played, slot.0, ObjectValue::Atom(Datum::Instance(other.1.0)))
                .map_err(unsupported)?;
            self.swap_card_flags_bound(objects, slot.1, CardLifecycle::Resolved, other.2, other.3)?;
        }
        self.sync_card_collections_bound(objects)?;
        self.world.refresh_card_auras();
        Ok(Some(NativeEvent::ResolvedCardsSwapped { actor: self.planning.actor, target, a, b }))
    }


    pub(super) fn swap_cards_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<()> {
        let p = &objects.phases;
        let text = |name| {
            self.symbols
                .lookup(name)
                .and_then(|key| objects.arena.field(step, key))
        };
        let context = |name| -> Result<Option<Symbol>> {
            if let Some(key) = self.truthy_symbol_bound(objects, text(name))? {
                self.truthy_symbol_bound(objects, objects.arena.context_value(key))
            } else {
                Ok(None)
            }
        };
        let hero = context("context_hero_id_key")?.or(self.planning.actor);
        let Some(model) = hero.and_then(|hero| self.hero_model_bound(objects, hero)) else {
            return Ok(());
        };
        let target = match self.truthy_symbol_bound(objects, text("target_card_id"))? {
            Some(target) => Some(target),
            None => context("target_card_key")?,
        };
        let Some(target) = target else {
            return Ok(());
        };
        let source = context("source_card_key")?;
        let source_is_keyed = self
            .truthy_symbol_bound(objects, text("source_card_key"))?
            .is_some();
        let current = self.model_card_bound(objects, model, Field::Current)?;
        let printed = |id: CardId| {
            self.world.definitions.cards[self.world.card(id).unwrap().definition].printed_id
        };
        let mut slots = Vec::new();
        for field in [Field::Hand, Field::Discard, Field::Played] {
            let Some(ObjectValue::Object(list)) = objects.arena.field(model, p.k(field)) else {
                return Err(unsupported("card swap list missing"));
            };
            for (index, value) in objects
                .arena
                .sequence(list)
                .ok_or_else(|| unsupported("card swap list invalid"))?
                .iter()
                .enumerate()
            {
                if let ObjectValue::Atom(Datum::Instance(id)) = value {
                    slots.push((CardId(*id), list, ObjectSlot::Index(index)));
                }
            }
        }
        if let Some(card) = current {
            slots.push((card, model, ObjectSlot::Field(p.k(Field::Current))));
        }
        let Some((a, _, _)) = slots.iter().find(|(id, _, _)| {
            if source_is_keyed {
                source == Some(printed(*id))
            } else {
                current == Some(*id)
            }
        }) else {
            return Ok(());
        };
        let Some((b, _, _)) = slots.iter().find(|(id, _, _)| printed(*id) == target) else {
            return Ok(());
        };
        let (a, b) = (*a, *b);
        let (a_printed, b_printed) = (printed(a), printed(b));
        if a_printed == b_printed {
            return Ok(());
        }
        self.world.normal_card(a).map_err(card_error)?;
        self.world.normal_card(b).map_err(card_error)?;
        let played = objects.arena.field(model, p.k(Field::Played));
        let location = |card| {
            slots
                .iter()
                .find(|(id, container, _)| {
                    *id == card
                        && !matches!(played, Some(ObjectValue::Object(list)) if list == *container)
                })
                .or_else(|| slots.iter().find(|(id, _, _)| *id == card))
                .unwrap()
        };
        let a_slot = location(a);
        let b_slot = location(b);
        let a_row = self.world.card(a).unwrap().clone();
        let b_row = self.world.card(b).unwrap().clone();
        let removed = self
            .world
            .effects
            .instances
            .iter()
            .enumerate()
            .filter_map(|(index, effect)| {
                matches!(effect.card, Some(card) if card == a_printed || card == b_printed)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        if objects.effect_nodes()?.len() != self.world.effects.instances.len() {
            return Err(unsupported("card swap effect collection binding drift"));
        }
        let before = self
            .world
            .cards()
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        for ((_, container, slot), replacement) in [(a_slot, b), (b_slot, a)] {
            let value = ObjectValue::Atom(Datum::Instance(replacement.0));
            match slot {
                ObjectSlot::Field(field) => objects.arena.set_field(*container, *field, value),
                ObjectSlot::Index(index) => objects.arena.set_index(*container, *index, value),
            }
            .map_err(unsupported)?;
        }
        for (card, other) in [(a, &b_row), (b, &a_row)] {
            self.swap_card_flags_bound(
                objects,
                card,
                other.lifecycle,
                other.facedown,
                other.played_this_round,
            )?;
        }
        self.sync_card_collections_bound(objects)?;
        self.world.refresh_card_auras();
        self.world.expire_effects_by_card(a_printed);
        self.world.expire_effects_by_card(b_printed);
        objects.remove_effect_rows(&removed)?;
        self.sync_effect_card_flags(objects, &before)
    }

    fn swap_card_flags_bound(
        &mut self,
        objects: &mut BoundObjects,
        card: CardId,
        lifecycle: CardLifecycle,
        facedown: bool,
        played: bool,
    ) -> Result<()> {
        let state = lifecycle_name(lifecycle);
        let state = self
            .symbols
            .lookup(state)
            .ok_or_else(|| unsupported("card swap state missing"))?;
        let p = &objects.phases;
        for (field, value) in [
            (Field::CardState, Datum::Text(state)),
            (Field::Facedown, Datum::Bool(facedown)),
            (Field::PlayedRound, Datum::Bool(played)),
        ] {
            objects
                .arena
                .set_physical_field(card.0, p.k(field), value)
                .map_err(unsupported)?;
        }
        if lifecycle != CardLifecycle::Unresolved {
            objects
                .arena
                .remove_physical_field(card.0, p.k(Field::InitiativeOverride))
                .map_err(unsupported)?;
        }
        self.world.change_lifecycle(card, lifecycle);
        let row = self.world.card_mut(card).unwrap();
        row.facedown = facedown;
        row.played_this_round = played;
        Ok(())
    }

    pub(super) fn swap_deck_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Option<NativeEvent>> {
        let field = |name| {
            self.symbols
                .lookup(name)
                .and_then(|key| objects.arena.field(step, key))
        };
        let literal = |name| self.truthy_symbol_bound(objects, field(name));
        let keyed = |name| match literal(name)? {
            Some(key) => self.truthy_symbol_bound(objects, objects.arena.context_value(key)),
            None => Ok(None),
        };
        let Some(owner) = keyed("hero_key")?
            .or(literal("hero_id")?)
            .or(self.planning.actor)
        else {
            return Ok(None);
        };
        let Some((outgoing, incoming)) = keyed("outgoing_card_key")?
            .or(literal("outgoing_card_id")?)
            .zip(keyed("incoming_card_key")?)
            .filter(|(a, b)| a != b)
        else {
            return Ok(None);
        };
        let Some(model) = self.hero_model_bound(objects, owner) else {
            return Ok(None);
        };
        let rider = field("facedown_if_from_discard_or_resolved")
            .is_some_and(|value| objects.arena.truthy(&value));
        self.sync_card_collections_bound(objects)?;
        let hero = self.world.hero(owner).unwrap();
        let lookup = |printed| {
            hero.deck.iter().copied().find(|id| {
                self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id
                    == printed
            })
        };
        let Some((outgoing_id, incoming_id)) = lookup(outgoing).zip(lookup(incoming)) else {
            return Ok(None);
        };
        if self.world.card(incoming_id).unwrap().lifecycle != CardLifecycle::Deck {
            return Ok(None);
        }
        let mut location = None;
        for container in [Field::Hand, Field::Discard, Field::Current, Field::Played] {
            let key = objects.phases.k(container);
            let value = objects.arena.field(model, key);
            if matches!(container, Field::Current) {
                if matches!(value, Some(ObjectValue::Atom(Datum::Instance(id))) if id == outgoing_id.0) {
                    location = Some((model, ObjectSlot::Field(key), false));
                    break;
                }
            } else if let Some(ObjectValue::Object(list)) = value
                && let Some(index) = objects
                    .arena
                    .sequence(list)
                    .ok_or_else(|| unsupported("deck swap list invalid"))?
                    .iter()
                    .position(|value| matches!(value, ObjectValue::Atom(Datum::Instance(id)) if *id == outgoing_id.0))
            {
                location = Some((list, ObjectSlot::Index(index), !matches!(container, Field::Hand)));
                break;
            }
        }
        let Some((container, slot, rider_applies)) = location else {
            return Ok(None);
        };
        let previous = self.world.card(outgoing_id).unwrap().clone();
        let facedown = previous.facedown || rider && rider_applies;
        let removed = self
            .world
            .effects
            .instances
            .iter()
            .enumerate()
            .filter_map(|(index, effect)| {
                matches!(effect.card, Some(card) if card == outgoing || card == incoming)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        if objects.effect_nodes()?.len() != self.world.effects.instances.len() {
            return Err(unsupported("deck swap effect collection binding drift"));
        }
        let before = self
            .world
            .cards()
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        let replacement = ObjectValue::Atom(Datum::Instance(incoming_id.0));
        match slot {
            ObjectSlot::Field(key) => objects.arena.set_field(container, key, replacement),
            ObjectSlot::Index(index) => objects.arena.set_index(container, index, replacement),
        }
        .map_err(unsupported)?;
        for (id, state, face, played) in [
            (
                incoming_id,
                previous.lifecycle,
                facedown,
                previous.played_this_round,
            ),
            (outgoing_id, CardLifecycle::Deck, false, false),
        ] {
            let state_id = self
                .symbols
                .lookup(lifecycle_name(state))
                .ok_or_else(|| unsupported("deck swap state missing"))?;
            for (field, value) in [
                (Field::CardState, Datum::Text(state_id)),
                (Field::Facedown, Datum::Bool(face)),
                (Field::PlayedRound, Datum::Bool(played)),
            ] {
                objects
                    .arena
                    .set_physical_field(id.0, objects.phases.k(field), value)
                    .map_err(unsupported)?;
            }
            let row = self.world.card_mut(id).unwrap();
            row.lifecycle = state;
            row.facedown = face;
            row.played_this_round = played;
        }
        self.sync_card_collections_bound(objects)?;
        self.world.expire_effects_by_card(outgoing);
        self.world.expire_effects_by_card(incoming);
        objects.remove_effect_rows(&removed)?;
        self.sync_effect_card_flags(objects, &before)?;
        Ok(Some(NativeEvent::DeckCardSwapped {
            actor: owner,
            outgoing,
            incoming,
            state: lifecycle_name(previous.lifecycle),
            facedown,
        }))
    }

    pub(super) fn swap_item_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Option<NativeEvent>> {
        let p = &objects.phases;
        let context = |name| -> Result<Option<Symbol>> {
            let value = self
                .symbols
                .lookup(name)
                .and_then(|key| objects.arena.field(step, key));
            if let Some(key) = self.truthy_symbol_bound(objects, value)? {
                self.truthy_symbol_bound(objects, objects.arena.context_value(key))
            } else {
                Ok(None)
            }
        };
        let (Some(hero), Some(item), Some(target)) = (
            context("hero_key")?,
            context("item_card_key")?,
            context("target_card_key")?,
        ) else {
            return Ok(None);
        };
        if item == target {
            return Ok(None);
        }
        self.sync_card_collections_bound(objects)?;
        let Some(owner) = self.world.hero(hero) else {
            return Ok(None);
        };
        let owner = owner.id;
        let model = self
            .hero_model_bound(objects, owner)
            .ok_or_else(|| unsupported("item swap model missing"))?;
        let find = |printed| {
            self.world
                .hero(owner)
                .unwrap()
                .deck
                .iter()
                .copied()
                .find(|id| {
                    self.world.card(*id).is_some_and(|row| {
                        self.world.definitions.cards[row.definition].printed_id == printed
                    })
                })
        };
        let (Some(a), Some(b)) = (find(item), find(target)) else {
            return Ok(None);
        };
        let a_row = self.world.card(a).unwrap();
        let b_row = self.world.card(b).unwrap().clone();
        let a_def = &self.world.definitions.cards[a_row.definition];
        let b_def = &self.world.definitions.cards[b_row.definition];
        let (Some(old_stat), Some(new_stat)) = (a_def.item, b_def.item) else {
            return Ok(None);
        };
        if a_row.lifecycle != CardLifecycle::Item
            || b_row.lifecycle == CardLifecycle::Item
            || a_def.tier != b_def.tier
            || a_def.color != b_def.color
        {
            return Ok(None);
        }
        let mut slot = None;
        for field in [Field::Hand, Field::Discard, Field::Current, Field::Played] {
            let value = objects.arena.field(model, p.k(field));
            if matches!(field, Field::Current) {
                if matches!(value, Some(ObjectValue::Atom(Datum::Instance(id))) if id == b.0) {
                    slot = Some((model, ObjectSlot::Field(p.k(field))));
                    break;
                }
            } else if let Some(ObjectValue::Object(list)) = value
                && let Some(index) = objects.arena.sequence(list).ok_or_else(|| unsupported("item swap list invalid"))?
                    .iter().position(|value| matches!(value, ObjectValue::Atom(Datum::Instance(id)) if *id == b.0))
            {
                slot = Some((list, ObjectSlot::Index(index)));
                break;
            }
        }
        if slot.is_none() && b_row.lifecycle != CardLifecycle::Deck {
            return Ok(None);
        }
        let items_key = self
            .symbols
            .lookup("items")
            .ok_or_else(|| unsupported("item key missing"))?;
        let Some(ObjectValue::Object(items)) = objects.arena.field(model, items_key) else {
            return Err(unsupported("item swap bonuses missing"));
        };
        let old_key = self
            .symbols
            .lookup(old_stat.name())
            .ok_or_else(|| unsupported("old item stat missing"))?;
        let new_key = self
            .symbols
            .lookup(new_stat.name())
            .ok_or_else(|| unsupported("new item stat missing"))?;
        let count = |key| match objects.arena.field(items, key) {
            None => Ok(0),
            Some(ObjectValue::Atom(Datum::Int(n))) => Ok(n),
            _ => Err(unsupported("item bonus must be an integer")),
        };
        let old = count(old_key)?
            .checked_sub(1)
            .ok_or_else(|| unsupported("old item bonus overflow"))?;
        let next = if old_key == new_key {
            old
        } else {
            count(new_key)?
        }
        .checked_add(1)
        .ok_or_else(|| unsupported("new item bonus overflow"))?;
        if let Some((container, slot)) = slot {
            let value = ObjectValue::Atom(Datum::Instance(a.0));
            match slot {
                ObjectSlot::Field(key) => objects.arena.set_field(container, key, value),
                ObjectSlot::Index(index) => objects.arena.set_index(container, index, value),
            }
            .map_err(unsupported)?;
        }
        self.swap_card_flags_bound(
            objects,
            a,
            b_row.lifecycle,
            b_row.facedown,
            b_row.played_this_round,
        )?;
        self.swap_card_flags_bound(objects, b, CardLifecycle::Item, false, false)?;
        for (key, value) in [(old_key, old), (new_key, next)] {
            objects
                .arena
                .set_field(items, key, ObjectValue::Atom(Datum::Int(value)))
                .map_err(unsupported)?;
        }
        self.sync_card_collections_bound(objects)?;
        self.sync_hero_items_bound(objects)?;
        self.world.refresh_card_auras();
        Ok(Some(NativeEvent::ItemGained {
            actor: owner,
            stat: new_stat,
            card: target,
        }))
    }
}
