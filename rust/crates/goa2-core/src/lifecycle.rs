//! Ordinary Hero model lifecycle, reused by planning and phase execution.
//! Collection references point to one mutable physical-card record.
use crate::{
    cards::{CardError, CardId},
    data::Symbol,
    world::{CardLifecycle, NativeWorld},
};
use std::collections::HashSet;
type Result<T> = std::result::Result<T, CardError>;
#[derive(Clone, Copy)]
enum Location {
    Hand(usize),
    Discard(usize),
    Current,
    Played(usize),
}
impl NativeWorld {
    pub(crate) fn lifecycle_hero(&self, id: Symbol) -> Result<usize> {
        let index = self
            .hero_index(id)
            .ok_or(CardError::Illegal("hero not found"))?;
        let hero = &self.heroes[index];
        let mut physical = HashSet::new();
        let mut printed = HashSet::new();
        for id in hero
            .deck
            .iter()
            .chain(&hero.hand)
            .chain(&hero.discard)
            .chain(&hero.spells)
            .chain(hero.played.iter().flatten())
            .chain(hero.current.iter())
            .chain(hero.extra.iter())
            .chain(hero.ultimate.iter())
        {
            if physical.insert(*id) {
                let card = self
                    .card(*id)
                    .ok_or(CardError::Illegal("unknown card instance"))?;
                if !printed.insert(self.definitions.cards[card.definition].printed_id) {
                    return Err(CardError::Unsupported(
                        "duplicate printed-card equality requires native model equality",
                    ));
                }
            }
        }
        Ok(index)
    }
    pub(crate) fn normal_card(&self, id: CardId) -> Result<()> {
        let card = self
            .card(id)
            .ok_or(CardError::Illegal("unknown card instance"))?;
        if self.definitions.cards[card.definition].spell {
            return Err(CardError::Illegal(
                "spell cannot enter normal card lifecycle",
            ));
        }
        Ok(())
    }
    /// Script-side premature expiry, kept separate from ordinary Hero methods.
    pub(crate) fn expire_card_effects(&mut self, printed: Symbol) -> Result<()> {
        if self.has_effects {
            return Err(CardError::Unsupported(
                "card-linked effect expiry not compiled",
            ));
        }
        self.expire_effects_by_card(printed);
        Ok(())
    }
    pub(crate) fn change_lifecycle(&mut self, id: CardId, state: CardLifecycle) {
        let card = self.card_mut(id).unwrap();
        card.lifecycle = state;
        if state != CardLifecycle::Unresolved {
            card.initiative_override = None;
        }
    }
    pub(crate) fn refresh_card_auras(&mut self) {
        let flags = self
            .heroes
            .iter()
            .map(|hero| {
                let has = |id: CardId| {
                    self.card(id).is_some_and(|card| {
                        !card.facedown
                            && self.definitions.cards[card.definition].aura
                                == crate::auras::CardAura::Uncompiled
                    })
                };
                (hero.level >= 8 && hero.ultimate.is_some_and(has))
                    || hero.played.iter().flatten().any(|id| {
                        self.card(*id)
                            .is_some_and(|c| c.lifecycle == CardLifecycle::Resolved)
                            && has(*id)
                    })
            })
            .collect::<Vec<_>>();
        self.has_uncompiled_auras = flags.iter().any(|flag| *flag);
        for (hero, flag) in self.heroes.iter_mut().zip(flags) {
            hero.uncompiled_auras = flag;
        }
    }
    pub fn play_card(&mut self, hero: Symbol, id: CardId) -> Result<()> {
        self.normal_card(id)?;
        let h = self.lifecycle_hero(hero)?;
        let pos = self.heroes[h]
            .hand
            .iter()
            .position(|value| *value == id)
            .ok_or(CardError::Illegal("card not in hand"))?;
        self.heroes[h].hand.remove(pos);
        self.change_lifecycle(id, CardLifecycle::Unresolved);
        let card = self.card_mut(id).unwrap();
        card.facedown = true;
        card.played_this_round = true;
        self.heroes[h].current = Some(id);
        self.refresh_card_auras();
        Ok(())
    }
    pub fn unplay_card(&mut self, hero: Symbol, id: CardId) -> Result<()> {
        self.normal_card(id)?;
        let h = self.lifecycle_hero(hero)?;
        self.change_lifecycle(id, CardLifecycle::Hand);
        let card = self.card_mut(id).unwrap();
        card.facedown = false;
        card.played_this_round = false;
        self.heroes[h].hand.push(id);
        self.refresh_card_auras();
        Ok(())
    }
    pub fn resolve_current_card(&mut self, hero: Symbol, turn: Option<i64>) -> Result<()> {
        let h = self.lifecycle_hero(hero)?;
        let turn = match turn {
            Some(turn) => turn,
            None => {
                if self.heroes[h].current.is_none() {
                    return Ok(());
                }
                self.heroes[h]
                    .resolved_turns
                    .checked_add(1)
                    .ok_or(CardError::Unsupported("turn slot exceeds i64"))?
            }
        };
        if turn < 1 {
            return Err(CardError::Illegal("turn slots are one-based"));
        }
        if turn > 1_000_000 {
            return Err(CardError::Unsupported(
                "turn slot allocation exceeds supported limit",
            ));
        }
        let length = self.heroes[h].played.len().max(turn as usize);
        self.heroes[h].played.resize(length, None);
        if let Some(card) = self.heroes[h].current {
            self.normal_card(card)?;
            self.change_lifecycle(card, CardLifecycle::Resolved);
            self.heroes[h].played[turn as usize - 1] = Some(card);
            self.heroes[h].current = None;
        }
        self.heroes[h].resolved_turns = self.heroes[h].resolved_turns.max(turn);
        self.refresh_card_auras();
        Ok(())
    }
    pub fn discard_card(&mut self, hero: Symbol, id: CardId, from_hand: bool) -> Result<()> {
        self.normal_card(id)?;
        let h = self.lifecycle_hero(hero)?;
        if from_hand {
            let index = self.heroes[h]
                .hand
                .iter()
                .position(|value| *value == id)
                .ok_or(CardError::Illegal("card not in hand"))?;
            self.heroes[h].hand.remove(index);
        } else if self.heroes[h].current == Some(id) {
            self.heroes[h].current = None;
        } else if let Some(index) = self.heroes[h]
            .played
            .iter()
            .position(|value| *value == Some(id))
        {
            self.heroes[h].played[index] = None;
        }
        self.change_lifecycle(id, CardLifecycle::Discard);
        self.card_mut(id).unwrap().facedown = false;
        self.heroes[h].discard.push(id);
        self.refresh_card_auras();
        Ok(())
    }
    pub fn return_card_to_hand(&mut self, hero: Symbol, id: CardId) -> Result<()> {
        self.normal_card(id)?;
        let h = self.lifecycle_hero(hero)?;
        if self.heroes[h].hand.contains(&id) {
            return Err(CardError::Illegal("card already in hand"));
        }
        self.change_lifecycle(id, CardLifecycle::Hand);
        self.card_mut(id).unwrap().facedown = false;
        self.heroes[h].hand.push(id);
        if let Some(index) = self.heroes[h]
            .played
            .iter()
            .position(|value| *value == Some(id))
        {
            self.heroes[h].played[index] = None;
        }
        if let Some(index) = self.heroes[h].discard.iter().position(|value| *value == id) {
            self.heroes[h].discard.remove(index);
        }
        if self.heroes[h].current == Some(id) {
            self.heroes[h].current = None;
        }
        self.refresh_card_auras();
        Ok(())
    }
    pub fn return_card_to_deck(&mut self, hero: Symbol, id: CardId) -> Result<()> {
        self.normal_card(id)?;
        let h = self.lifecycle_hero(hero)?;
        if self.heroes[h].deck.contains(&id) {
            return Err(CardError::Illegal("card already in deck"));
        }
        self.change_lifecycle(id, CardLifecycle::Deck);
        self.card_mut(id).unwrap().facedown = false;
        self.heroes[h].deck.push(id);
        self.refresh_card_auras();
        Ok(())
    }
    pub fn retrieve_cards(&mut self, hero: Symbol) -> Result<()> {
        let h = self.lifecycle_hero(hero)?;
        let record = &self.heroes[h];
        let mut cards = record
            .played
            .iter()
            .flatten()
            .copied()
            .chain(record.discard.iter().copied())
            .collect::<Vec<_>>();
        cards.extend(record.current);
        for id in cards {
            let card = self
                .card_mut(id)
                .ok_or(CardError::Illegal("unknown card"))?;
            card.facedown = false;
            card.played_this_round = false;
            self.return_card_to_hand(hero, id)?;
        }
        self.heroes[h].played.clear();
        self.heroes[h].resolved_turns = 0;
        self.refresh_card_auras();
        Ok(())
    }
    fn card_location(&self, h: usize, id: CardId) -> Option<Location> {
        let hero = &self.heroes[h];
        if let Some(i) = hero.hand.iter().position(|value| *value == id) {
            return Some(Location::Hand(i));
        }
        if let Some(i) = hero.discard.iter().position(|value| *value == id) {
            return Some(Location::Discard(i));
        }
        if hero.current == Some(id) {
            return Some(Location::Current);
        }
        hero.played
            .iter()
            .position(|value| *value == Some(id))
            .map(Location::Played)
    }
    fn put_card(&mut self, h: usize, location: Location, id: CardId) {
        match location {
            Location::Hand(i) => self.heroes[h].hand[i] = id,
            Location::Discard(i) => self.heroes[h].discard[i] = id,
            Location::Current => self.heroes[h].current = Some(id),
            Location::Played(i) => self.heroes[h].played[i] = Some(id),
        }
    }
    pub fn swap_cards(&mut self, hero: Symbol, a: CardId, b: CardId) -> Result<()> {
        self.normal_card(a)?;
        self.normal_card(b)?;
        let h = self.lifecycle_hero(hero)?;
        let loc_a = self
            .card_location(h, a)
            .ok_or(CardError::Illegal("card A not in hero lifecycle"))?;
        let loc_b = self
            .card_location(h, b)
            .ok_or(CardError::Illegal("card B not in hero lifecycle"))?;
        let a_state = self.card(a).unwrap().clone();
        let b_state = self.card(b).unwrap().clone();
        self.put_card(h, loc_a, b);
        self.put_card(h, loc_b, a);
        self.change_lifecycle(a, b_state.lifecycle);
        self.change_lifecycle(b, a_state.lifecycle);
        let card = self.card_mut(a).unwrap();
        card.facedown = b_state.facedown;
        card.played_this_round = b_state.played_this_round;
        let card = self.card_mut(b).unwrap();
        card.facedown = a_state.facedown;
        card.played_this_round = a_state.played_this_round;
        self.refresh_card_auras();
        Ok(())
    }
}
