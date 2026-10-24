//! Shared public card knowledge and turn discard history.
use crate::data::{Symbol, Symbols};
use serde_json::{Value, json};
#[derive(Clone, Debug)]
pub struct CardHistory {
    pub revealed: Vec<(Symbol, Symbol)>,
    pub discarded: Vec<(Symbol, Vec<Symbol>)>,
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
            revealed,
            discarded,
        })
    }
    pub fn reveal(&mut self, hero: Symbol, card: Symbol) {
        if !self.revealed.contains(&(hero, card)) {
            self.revealed.push((hero, card));
        }
    }
    pub fn discard(&mut self, hero: Symbol, card: Symbol) {
        self.reveal(hero, card);
        if let Some((_, cards)) = self.discarded.iter_mut().find(|(h, _)| *h == hero) {
            cards.push(card);
        } else {
            self.discarded.push((hero, vec![card]));
        }
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
