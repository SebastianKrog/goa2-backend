//! Player-scoped labels for the existing Decisions tab.
use crate::replay::{Entry, Operation};
use goa2_core::interpreter::live_game::{GamePosition, PlayerCommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Deserialize, Serialize)]
pub struct HistoryContext {
    round: i64,
    turn: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor: Option<String>,
}

impl HistoryContext {
    pub fn new(position: &GamePosition, actor: Option<&str>) -> Self {
        Self {
            round: position.round,
            turn: position.turn,
            actor: actor.map(str::to_owned),
        }
    }
}

#[derive(Serialize)]
pub struct History {
    pub total: u64,
    pub decisions: Vec<Record>,
}

#[derive(Serialize)]
pub struct Record {
    index: u64,
    #[serde(rename = "type")]
    kind: &'static str,
    round: Option<i64>,
    turn: Option<i64>,
    hero_id: Option<String>,
    label: String,
    superseded: bool,
}

impl History {
    pub fn from_entries(
        start_revision: u64,
        revision: u64,
        entries: Vec<(Entry, Option<HistoryContext>)>,
        scoped_view: &Value,
    ) -> Self {
        let mut cards = HashMap::new();
        let mut names = HashMap::new();
        for (_, team) in scoped_view["view"]["teams"]
            .as_object()
            .into_iter()
            .flatten()
        {
            for hero in team["heroes"].as_array().into_iter().flatten() {
                if let (Some(id), Some(name)) = (hero["id"].as_str(), hero["name"].as_str()) {
                    names.insert(id, scoped_view["hero_names"][id].as_str().unwrap_or(name));
                }
                for field in [
                    "hand",
                    "deck",
                    "current_turn_card",
                    "extra_turn_card",
                    "played_cards",
                    "discard_pile",
                ] {
                    let value = &hero[field];
                    for card in value
                        .as_array()
                        .map_or_else(|| std::slice::from_ref(value), Vec::as_slice)
                    {
                        if let (Some(id), Some(name)) = (card["id"].as_str(), card["name"].as_str())
                        {
                            cards.insert(id, name);
                        }
                    }
                }
            }
        }
        let mut live_end = revision;
        let mut decisions: Vec<_> = entries
            .into_iter()
            .rev()
            .map(|(entry, context)| {
                let actor = context
                    .as_ref()
                    .and_then(|position| position.actor.as_deref());
                let is_rewind = matches!(entry.decision.operation, Operation::Rewind { .. });
                let superseded = !is_rewind && entry.revision > live_end;
                if entry.revision <= live_end
                    && let Operation::Rewind { to, .. } = entry.decision.operation
                {
                    live_end = start_revision + to;
                }
                let (kind, hero, label) = match &entry.decision.operation {
                    Operation::Command { actor, command } => {
                        let (kind, action) = match command {
                            PlayerCommand::Commit { card_id } => (
                                "commit",
                                format!(
                                    "committed {}",
                                    cards.get(card_id.as_str()).unwrap_or(&"a card")
                                ),
                            ),
                            PlayerCommand::Uncommit => {
                                ("uncommit", "took back a committed card".into())
                            }
                            PlayerCommand::Pass => ("pass", "passed".into()),
                            PlayerCommand::FinishPlanning => {
                                ("finish_planning", "finished planning".into())
                            }
                            PlayerCommand::StartingPosition { .. } => {
                                ("starting_position", "acted on starting positions".into())
                            }
                            PlayerCommand::CheatGold { hero_id, amount } => (
                                "cheat_gold",
                                format!(
                                    "gave {amount} gold to {}",
                                    names.get(hero_id.as_str()).unwrap_or(&hero_id.as_str())
                                ),
                            ),
                            PlayerCommand::Rollback => {
                                ("rollback", "rolled back their action".into())
                            }
                            PlayerCommand::Ready { ready: true } => {
                                ("ready", "marked ready".into())
                            }
                            PlayerCommand::Ready { ready: false } => {
                                ("ready", "marked unready".into())
                            }
                            // Raw selections may include hand, upgrade, mine or guess information.
                            PlayerCommand::Input { .. } => ("input", "submitted a decision".into()),
                        };
                        let name = names.get(actor.as_str()).copied().unwrap_or(actor);
                        (kind, Some(actor.as_str()), format!("{name} {action}"))
                    }
                    Operation::Patch { proposer, command } => {
                        ("ov_patch", Some(proposer.as_str()), command.summary())
                    }
                    Operation::Unstick { proposer, command } => (
                        "ov_unstick",
                        Some(proposer.as_str()),
                        format!("Table recovery: {}", command.name()),
                    ),
                    Operation::Rewind { proposer, to } => (
                        "ov_rewind",
                        Some(proposer.as_str()),
                        format!("The table rewound the game to decision {to}"),
                    ),
                    Operation::Timeout { .. } => (
                        "timer_timeout",
                        actor,
                        match actor.and_then(|id| names.get(id)) {
                            Some(name) => format!("Automatic decision for {name} (timer expired)"),
                            None => "Automatic decision (timer expired)".into(),
                        },
                    ),
                    Operation::PauseOpened { .. } => (
                        "pause_opened",
                        actor,
                        "The table held the clock for a vote".into(),
                    ),
                    Operation::PauseReleased => (
                        "pause_released",
                        actor,
                        "The table released the vote's clock hold".into(),
                    ),
                    Operation::PauseApproved { requester } => (
                        "pause_approved",
                        Some(requester.as_str()),
                        "The table paused the match".into(),
                    ),
                };
                Record {
                    index: entry.revision - start_revision - 1,
                    kind,
                    round: context.as_ref().map(|position| position.round),
                    turn: context.as_ref().map(|position| position.turn),
                    hero_id: hero.map(str::to_owned),
                    label,
                    superseded,
                }
            })
            .collect();
        decisions.reverse();
        Self {
            total: revision - start_revision,
            decisions,
        }
    }
}
