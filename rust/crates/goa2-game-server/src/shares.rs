//! Immutable, public replay artifacts produced only from finished games.
use crate::{
    ApiError, Result,
    replay::{Archive, Operation},
};
use flate2::{Compression, write::GzEncoder};
use goa2_core::{
    api::{LiveGame, PlayerCommand, StartingPositionCommand},
    codec::Phase,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::BufWriter;

pub const GROUP_SIZE: u64 = 32;
const MAX_SHARE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
pub struct Metadata {
    setup: Value,
    decisions: Vec<ReplayDecision>,
}

#[derive(Deserialize, Serialize)]
struct ReplayDecision {
    index: u64,
    #[serde(rename = "type")]
    kind: String,
    r: i64,
    t: i64,
    hero: String,
    card: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sel: Option<Value>,
}

#[derive(Deserialize)]
pub struct Seek {
    pub decision: Option<i64>,
    pub round: Option<i64>,
    pub turn: Option<i64>,
}

impl Metadata {
    pub fn debug(game: &str, archive: Archive) -> Result<Self> {
        let setup = archive.setup_metadata(game)?;
        let first = archive.start_revision;
        let mut decisions = Vec::new();
        archive.walk(None, |engine, next| {
            if let Some(next) = next {
                let pos = engine.position();
                decisions.push(replay_decision(
                    &next.operation,
                    pos.revision - first,
                    pos.round,
                    pos.turn,
                ));
            }
            Ok(false)
        })?;
        Ok(Self { setup, decisions })
    }
}

impl Seek {
    pub fn debug(&self, archive: Archive) -> Result<Value> {
        let first = archive.start_revision;
        let total = archive
            .revision
            .checked_sub(first)
            .ok_or_else(|| ApiError::invalid("Invalid replay range"))?;
        let (engine, _) = if let Some(index) = self.decision {
            archive.restore(Some(first + (index.max(0) as u64).min(total)))?
        } else if let Some(round) = self.round {
            archive.walk(None, |engine, _| {
                let pos = engine.position();
                Ok(pos.round > round
                    || (pos.round == round && self.turn.is_none_or(|turn| pos.turn >= turn)))
            })?
        } else {
            archive.restore(None)?
        };
        replay_frame(&engine, first, total)
    }

    pub fn group_start(&self, total: u64, metadata: Option<&Metadata>) -> u64 {
        let target = if let Some(index) = self.decision {
            (index.max(0) as u64).min(total)
        } else if let Some((round, meta)) = self.round.zip(metadata) {
            meta.decisions
                .iter()
                .find(|record| {
                    record.r > round
                        || (record.r == round && self.turn.is_none_or(|turn| record.t >= turn))
                })
                .map_or(total, |record| record.index)
        } else {
            total
        };
        target / GROUP_SIZE * GROUP_SIZE
    }
}

#[derive(Serialize)]
struct Group {
    start: u64,
    keyframe: Value,
    patches: Vec<Vec<Value>>,
}

pub struct Share {
    pub metadata: Metadata,
    pub total: u64,
    pub groups: Vec<(u64, Vec<u8>)>,
}

impl Share {
    pub fn bake(game: &str, archive: Archive) -> Result<Self> {
        let setup = archive.setup_metadata(game)?;
        let first = archive.start_revision;
        let total = archive
            .revision
            .checked_sub(first)
            .filter(|total| *total <= crate::replay::MAX_STEPS as u64)
            .ok_or_else(|| ApiError::invalid("Replay share exceeds the revision limit"))?;
        let mut decisions = Vec::with_capacity(total as usize);
        let mut groups = Vec::new();
        let mut group = None;
        let mut previous = Value::Null;
        let mut bytes = 0;
        let mut append = |group: Group| -> Result<()> {
            let encoder = GzEncoder::new(Vec::new(), Compression::fast());
            let mut buffered = BufWriter::with_capacity(64 * 1024, encoder);
            serde_json::to_writer(&mut buffered, &group).map_err(ApiError::internal)?;
            let encoder = buffered.into_inner().map_err(ApiError::internal)?;
            let blob = encoder.finish().map_err(ApiError::internal)?;
            bytes += blob.len();
            if bytes > MAX_SHARE_BYTES {
                return Err(ApiError::invalid("Replay share exceeds the size limit"));
            }
            groups.push((group.start, blob));
            Ok(())
        };
        let (finished, _) = archive.walk(None, |engine, next| {
            let position = engine.position();
            let index = position.revision - first;
            if let Some(next) = next {
                decisions.push(replay_decision(
                    &next.operation,
                    index,
                    position.round,
                    position.turn,
                ));
            }

            let frame = replay_frame(engine, first, total)?;
            if index.is_multiple_of(GROUP_SIZE) {
                if let Some(done) = group.take() {
                    append(done)?;
                }
                group = Some(Group {
                    start: index,
                    keyframe: frame.clone(),
                    patches: Vec::new(),
                });
            } else if let Some(group) = group.as_mut() {
                let mut ops = Vec::new();
                diff(&previous, &frame, "", &mut ops);
                group.patches.push(ops);
            }
            previous = frame;
            Ok(false)
        })?;
        if finished.position().phase != Phase::GameOver {
            return Err(unfinished());
        }
        if let Some(done) = group {
            append(done)?;
        }
        Ok(Self {
            metadata: Metadata { setup, decisions },
            total,
            groups,
        })
    }
}

fn replay_frame(engine: &LiveGame, first: u64, total: u64) -> Result<Value> {
    let pos = engine.position();
    let projection = engine.private_replay_projection()?;
    Ok(json!({
        "view": projection["view"], "winner": projection["winner"],
        "position": {"decision_index": pos.revision - first, "round": pos.round,
                     "turn": pos.turn, "total_decisions": total},
    }))
}

pub fn unfinished() -> ApiError {
    ApiError::conflict("Only finished games can be shared")
}

fn replay_decision(operation: &Operation, index: u64, r: i64, t: i64) -> ReplayDecision {
    let (kind, hero, card, sel) = match operation {
        Operation::Command { actor, command } => {
            let (kind, card, sel) = match command {
                PlayerCommand::Commit { card_id } => ("commit", Some(card_id.clone()), None),
                PlayerCommand::Uncommit => ("uncommit", None, None),
                PlayerCommand::Pass => ("pass", None, None),
                PlayerCommand::FinishPlanning => ("finish_planning", None, None),
                PlayerCommand::StartingPosition { operation } => (
                    "starting_position",
                    None,
                    match operation {
                        StartingPositionCommand::Move { destination } => {
                            Some(json!({"destination": destination}))
                        }
                        StartingPositionCommand::RequestSwap { target } => {
                            Some(json!({"swap_with": target}))
                        }
                        StartingPositionCommand::RespondSwap { .. }
                        | StartingPositionCommand::Cancel => None,
                    },
                ),
                PlayerCommand::CheatGold { hero_id, amount } => (
                    "cheat_gold",
                    None,
                    Some(json!({"hero_id": hero_id, "amount": amount})),
                ),
                PlayerCommand::Rollback => ("rollback", None, None),
                PlayerCommand::Ready { .. } => ("ready", None, None),
                PlayerCommand::Input { selection, .. } => {
                    ("input", None, replay_selection(selection))
                }
            };
            (kind, actor.as_str(), card, sel)
        }
        Operation::Patch { proposer, command } => {
            ("ov_patch", proposer.as_str(), None, Some(json!(command)))
        }
        Operation::Unstick { proposer, command } => {
            ("ov_unstick", proposer.as_str(), None, Some(json!(command)))
        }
        Operation::Rewind { proposer, to } => (
            "ov_rewind",
            proposer.as_str(),
            None,
            Some(json!({"to": to})),
        ),
        Operation::Timeout { .. } => ("timer_timeout", "table", None, None),
        Operation::PauseOpened { .. } => ("pause_opened", "table", None, None),
        Operation::PauseReleased => ("pause_released", "table", None, None),
        Operation::PauseApproved { requester } => {
            ("pause_approved", requester.as_str(), None, None)
        }
    };
    ReplayDecision {
        index,
        r,
        t,
        kind: kind.into(),
        hero: hero.into(),
        card,
        sel,
    }
}

// Export rule choices, leaving out request IDs and unrecognized object fields.
fn replay_selection(selection: &Value) -> Option<Value> {
    match selection {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Some(selection.clone()),
        Value::Array(entries) => entries
            .iter()
            .map(replay_selection)
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        Value::Object(fields) => {
            if let (Some(q), Some(r), Some(s)) = (
                fields.get("q").and_then(Value::as_i64),
                fields.get("r").and_then(Value::as_i64),
                fields.get("s").and_then(Value::as_i64),
            ) {
                return Some(json!({"q": q, "r": r, "s": s}));
            }
            if let Some(cards) = fields.get("card_ids").and_then(Value::as_array)
                && cards.iter().all(Value::is_string)
            {
                return Some(json!({"card_ids": cards}));
            }
            fields
                .get("card_id")
                .and_then(Value::as_str)
                .map(|card| json!({"card_id": card}))
        }
        Value::Null => None,
    }
}

// Preserve stable arrays such as the board and roster, and replace a whole
// array when its length changes rather than emitting a sequence of index shifts.
fn diff(before: &Value, after: &Value, path: &str, ops: &mut Vec<Value>) {
    if before == after {
        return;
    }
    if let (Some(before), Some(after)) = (before.as_array(), after.as_array())
        && before.len() == after.len()
    {
        for (index, (old, new)) in before.iter().zip(after).enumerate() {
            if old != new {
                diff(old, new, &format!("{path}/{index}"), ops);
            }
        }
        return;
    }
    let (Some(before), Some(after)) = (before.as_object(), after.as_object()) else {
        ops.push(json!({"op": "replace", "path": path, "value": after}));
        return;
    };
    let child = |key: &str| format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        ops.push(json!({"op": "remove", "path": child(key)}));
    }
    for (key, value) in after {
        let path = child(key);
        if let Some(old) = before.get(key) {
            diff(old, value, &path, ops);
        } else {
            ops.push(json!({"op": "add", "path": path, "value": value}));
        }
    }
}
