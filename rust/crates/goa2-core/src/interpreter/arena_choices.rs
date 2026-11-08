//! Versioned semantic decision records for offline arena verification.
use super::{DecisionMove, EngineError, NativeSession, RequestChoices, Result};
use crate::{cards::CardId, codec::Phase, world::CardLifecycle};
use serde_json::{Value, json};

impl NativeSession {
    pub(super) fn arena_legal_choices(&mut self) -> Result<Value> {
        if self.planning.phase == Phase::Planning {
            let mut players = Vec::new();
            for hero in &self.world.heroes {
                let plans = self.hero_planning_commitments(hero.id)?;
                if plans.is_empty() {
                    continue;
                }
                let mut options = Vec::new();
                for plan in plans {
                    let first = plan
                        .first
                        .map(|card| self.printed_observation_card(card))
                        .transpose()?;
                    let second = plan
                        .second
                        .map(|card| self.printed_observation_card(card))
                        .transpose()?;
                    let option = json!([first, second]);
                    if options.contains(&option) {
                        return Err(unsupported("ambiguous printed card in arena planning"));
                    }
                    options.push(option);
                }
                players.push(json!({
                    "player": self.symbols.name(hero.id),
                    "second_slot": self.observed_planning_cards(hero.id)?.0.is_some(),
                    "moves": options,
                }));
            }
            return Ok(json!({"kind": "planning", "players": players}));
        }
        let request = self
            .last_request
            .as_ref()
            .ok_or_else(|| unsupported("arena input has no issued request"))?;
        if let RequestChoices::Upgrades(players) = &request.choices {
            let mut rows = Vec::new();
            for player in players.iter().filter(|player| !player.options.is_empty()) {
                let hero = self
                    .world
                    .hero(player.hero)
                    .ok_or_else(|| unsupported("arena upgrade hero missing"))?;
                let mut choices = Vec::new();
                for option in &player.options {
                    choices.push(json!([
                        self.symbols.name(option.color),
                        self.symbols.name(option.tier),
                        self.upgrade_card_ids(&option.pair)?,
                    ]));
                }
                let upgrade_cards = |cards: &[CardId], deck: bool| -> Result<Vec<Value>> {
                    let mut rows = Vec::new();
                    for &id in cards {
                        let card = self
                            .world
                            .card(id)
                            .ok_or_else(|| unsupported("arena upgrade card missing"))?;
                        let definition = &self.world.definitions.cards[card.definition];
                        let color = definition.color.map(|id| self.symbols.name(id));
                        let tier = self.symbols.name(definition.tier);
                        if matches!(color, Some("RED" | "BLUE" | "GREEN"))
                            && matches!(tier, "I" | "II" | "III")
                            && (!deck || (tier != "I" && card.lifecycle == CardLifecycle::Deck))
                        {
                            rows.push(json!([
                                color,
                                tier,
                                self.symbols.name(definition.printed_id)
                            ]));
                        }
                    }
                    Ok(rows)
                };
                rows.push(json!({
                    "player": self.symbols.name(player.hero),
                    "remaining": player.remaining,
                    "choices": choices,
                    "hand": upgrade_cards(&hero.hand, false)?,
                    "available": upgrade_cards(&hero.deck, true)?,
                }));
            }
            return Ok(json!({"kind": "upgrades", "players": rows}));
        }

        let owners: Vec<_> = self
            .world
            .heroes
            .iter()
            .filter(|hero| self.resolution_request_addresses(request, hero.id))
            .map(|hero| hero.id)
            .collect();
        let names: Vec<_> = owners
            .iter()
            .map(|id| self.symbols.name(*id).to_owned())
            .collect();
        match &request.choices {
            RequestChoices::RuneInscription { remaining, .. } => {
                let moves: Vec<_> = remaining.iter().map(|id| self.symbols.name(*id)).collect();
                let count = moves.len();
                return Ok(json!({
                    "kind": "runes", "owners": names,
                    "moves": moves, "counts": [1, count.saturating_sub(1), count],
                }));
            }
            RequestChoices::Mines(paths) => {
                let moves: Vec<_> = paths
                    .iter()
                    .enumerate()
                    .map(|(index, path)| {
                        let mines: Vec<_> = path
                            .mines
                            .iter()
                            .map(|id| self.world.position(*id))
                            .collect();
                        json!([index.to_string(), path.path, mines])
                    })
                    .collect();
                return Ok(json!({"kind": "mines", "owners": names, "moves": moves}));
            }
            _ => {}
        }
        let request_kind = self.request_json(request)["request_type"].clone();
        let multi = matches!(request.choices, RequestChoices::Multiple { .. });
        let owner = *owners
            .first()
            .ok_or_else(|| unsupported("arena input has no eligible owner"))?;
        let mut choices = Vec::new();
        for choice in self.resolution_choices(owner)? {
            let DecisionMove::Input { selection, .. } = choice else {
                return Err(unsupported("arena input has a non-resolution move"));
            };
            choices.push(self.selection_json(&selection));
        }
        Ok(json!({
            "kind": "input", "request": request_kind, "multi": multi,
            "owners": names, "moves": choices,
        }))
    }

    fn upgrade_card_ids(&self, ids: &[CardId]) -> Result<Vec<String>> {
        ids.iter()
            .map(|&id| self.printed_observation_card(id))
            .collect()
    }
}

fn unsupported(message: &str) -> EngineError {
    EngineError::Unsupported(message.into())
}
