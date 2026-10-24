//! Script-side retrieval: ordered physical targets, model movement and expiry.
use crate::{
    cards::{CardError, CardId},
    data::Symbol,
    world::NativeWorld,
};
use std::collections::HashSet;
#[derive(Clone, Copy, Debug)]
pub struct Retrieved {
    pub card: Symbol,
    pub name: Symbol,
}
impl NativeWorld {
    pub fn retrieve_card(
        &mut self,
        actor: Symbol,
        printed: Option<Symbol>,
        all: bool,
    ) -> Result<Vec<Retrieved>, CardError> {
        let Some(hero) = self.hero(actor) else {
            return Ok(Vec::new());
        };
        let cards: Vec<CardId> = if all {
            hero.discard.clone()
        } else if let Some(printed) = printed {
            hero.played
                .iter()
                .flatten()
                .chain(&hero.discard)
                .chain(hero.current.iter())
                .copied()
                .find(|id| {
                    self.card(*id)
                        .is_some_and(|c| self.definitions.cards[c.definition].printed_id == printed)
                })
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        if cards.is_empty() {
            return Ok(Vec::new());
        }
        if self.has_effects {
            return Err(CardError::Unsupported(
                "retrieval effect expiry not compiled",
            ));
        }
        let owner = self.lifecycle_hero(actor)?;
        let mut seen = HashSet::new();
        for id in &cards {
            self.normal_card(*id)?;
            if !seen.insert(*id) {
                return Err(CardError::Unsupported(
                    "repeated physical retrieval requires partial-failure semantics",
                ));
            }
            if self.heroes[owner].hand.contains(id) {
                return Err(CardError::Illegal("card already in hand"));
            }
        }
        let mut events = Vec::with_capacity(cards.len());
        for id in cards {
            let card = self.card(id).unwrap();
            let definition = &self.definitions.cards[card.definition];
            let retrieved = Retrieved {
                card: definition.printed_id,
                name: definition.name,
            };
            self.return_card_to_hand(actor, id)?;
            self.expire_card_effects(retrieved.card)?;
            events.push(retrieved);
        }
        Ok(events)
    }
}
