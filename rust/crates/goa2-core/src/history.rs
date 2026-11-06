//! Shared public card knowledge and turn discard history.
use crate::data::{Symbol, Symbols};
use serde_json::{Value, json};
use std::sync::Arc;

type DiscardLog = Vec<(Symbol, Vec<Symbol>)>;
#[derive(Clone, Debug, Hash)]
pub struct CardHistory {
    revealed: Arc<Vec<(Symbol, Symbol)>>,
    discarded: Arc<DiscardLog>,
}
impl CardHistory {
    pub fn compile(state: &Value, symbols: &mut Symbols) -> Result<Self, String> {
        let mut revealed = Vec::new();
        for row in state["public_revealed_cards"]
            .as_array()
            .ok_or("missing revealed cards")?
        {
            revealed.push((
                symbols.intern(row["hero_id"].as_str().ok_or("missing reveal hero")?),
                symbols.intern(row["card_id"].as_str().ok_or("missing reveal card")?),
            ));
        }
        let mut discarded = Vec::new();
        for (hero, cards) in state["turn_discard_log"]
            .as_object()
            .ok_or("missing discard log")?
        {
            let cards = cards
                .as_array()
                .ok_or("invalid discard log")?
                .iter()
                .map(|c| {
                    c.as_str()
                        .map(|s| symbols.intern(s))
                        .ok_or("invalid discarded identity".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            discarded.push((symbols.intern(hero), cards));
        }
        Ok(Self {
            revealed: Arc::new(revealed),
            discarded: Arc::new(discarded),
        })
    }
    pub(crate) fn revealed_cards(&self) -> &[(Symbol, Symbol)] {
        &self.revealed
    }

    pub fn reveal(&mut self, hero: Symbol, card: Symbol) {
        if !self.revealed.contains(&(hero, card)) {
            Arc::make_mut(&mut self.revealed).push((hero, card));
        }
    }
    pub fn discard(&mut self, hero: Symbol, card: Symbol) {
        self.reveal(hero, card);
        let discarded = Arc::make_mut(&mut self.discarded);
        if let Some((_, cards)) = discarded.iter_mut().find(|(h, _)| *h == hero) {
            cards.push(card);
        } else {
            discarded.push((hero, vec![card]));
        }
    }
    pub(crate) fn clear_discards(&mut self) {
        if !self.discarded.is_empty() {
            self.replace_discards(Vec::new());
        }
    }
    pub(crate) fn replace_discards(&mut self, discarded: DiscardLog) {
        if let Some(current) = Arc::get_mut(&mut self.discarded) {
            *current = discarded;
        } else {
            self.discarded = Arc::new(discarded);
        }
    }
    #[cfg(test)]
    pub(crate) fn discarded(&self) -> &[(Symbol, Vec<Symbol>)] {
        &self.discarded
    }
    pub fn export(&self, state: &mut Value, symbols: &Symbols) {
        state["public_revealed_cards"] = json!(
            self.revealed
                .iter()
                .map(|(h, c)| json!({"hero_id":symbols.name(*h),"card_id":symbols.name(*c)}))
                .collect::<Vec<_>>()
        );
        state["turn_discard_log"] = Value::Object(
            self.discarded
                .iter()
                .map(|(h, cs)| {
                    (
                        symbols.name(*h).into(),
                        json!(cs.iter().map(|c| symbols.name(*c)).collect::<Vec<_>>()),
                    )
                })
                .collect(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_forks_share_unchanged_rows_and_isolate_writes() {
        let hero = Symbol(0);
        let first = Symbol(1);
        let second = Symbol(2);
        let root = CardHistory {
            revealed: Arc::new(vec![(hero, first)]),
            discarded: Arc::new(vec![(hero, vec![first])]),
        };
        let mut branch = root.clone();
        branch.reveal(hero, first);
        assert!(Arc::ptr_eq(&root.revealed, &branch.revealed));
        assert!(Arc::ptr_eq(&root.discarded, &branch.discarded));
        branch.discard(hero, second);
        assert_eq!(*root.revealed, [(hero, first)]);
        assert_eq!(*branch.revealed, [(hero, first), (hero, second)]);
        assert_eq!(root.discarded(), &[(hero, vec![first])]);
        assert_eq!(branch.discarded(), &[(hero, vec![first, second])]);
        branch = root.clone();
        branch.clear_discards();
        assert!(branch.discarded().is_empty());
        assert_eq!(root.discarded(), &[(hero, vec![first])]);
        assert!(Arc::ptr_eq(&root.revealed, &branch.revealed));
    }
}
