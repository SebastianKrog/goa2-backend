//! Versioned, private live-game replay archives. Tokens and retry receipts stay in SQLite.
use crate::{ApiError, Result};
use goa2_core::api::{
    ExecutionBudget, GameCheckpoint, GameTransition, LiveGame, PatchCommand, PauseVoteChange,
    PlayerCommand, UnstickCommand,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_STEPS: usize = 10_000;
use crate::versions::REPLAY_SCHEMA as SCHEMA;
const KIND: &str = "rust-live-replay";

pub fn prepare_command<'a>(
    engine: &LiveGame,
    actor: &str,
    command: PlayerCommand,
    at_ms: u64,
    bot_seats: impl Iterator<Item = &'a str> + Clone,
) -> Result<(LiveGame, GameTransition)> {
    let ready = matches!(command, PlayerCommand::Ready { .. });
    let prepare = if bot_seats.clone().any(|bot| bot == actor) {
        LiveGame::prepare_bot_submission_at
    } else {
        LiveGame::prepare_submission_at
    };
    let (mut next, transition) = prepare(
        engine,
        actor,
        engine.position().revision,
        command,
        &ExecutionBudget::default(),
        at_ms,
    )?;
    if ready && bot_seats.clone().next().is_some() {
        next.ready_bot_seats(bot_seats, at_ms)?;
    }
    Ok((next, transition))
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Command {
        actor: String,
        command: PlayerCommand,
    },
    Timeout {
        entropy: u64,
    },
    PauseOpened {
        id: String,
    },
    PauseReleased,
    Patch {
        proposer: String,
        command: PatchCommand,
    },
    Unstick {
        proposer: String,
        command: UnstickCommand,
    },
    Rewind {
        proposer: String,
        to: u64,
    },
    PauseApproved {
        requester: String,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebase_ms: Option<u64>,
    pub operation: Operation,
}

impl Decision {
    pub fn pause(at_ms: u64, change: PauseVoteChange<'_>) -> Self {
        let operation = match change {
            PauseVoteChange::Open(id) => Operation::PauseOpened { id: id.into() },
            PauseVoteChange::Cancel => Operation::PauseReleased,
            PauseVoteChange::Approve(requester) => Operation::PauseApproved {
                requester: requester.into(),
            },
        };
        Self {
            at_ms,
            rebase_ms: None,
            operation,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub revision: u64,
    pub checkpoint_sha256: String,
    pub decision: Decision,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    kind: String,
    schema: u32,
    pub start_revision: u64,
    pub revision: u64,
    start_sha256: String,
    start: GameCheckpoint,
    hero_names: BTreeMap<String, String>,
    bot_seats: BTreeSet<String>,
    entries: Vec<Entry>,
}

pub fn setup_metadata(
    game: &str,
    start: &serde_json::Value,
    hero_names: impl Serialize,
    created_at: u64,
) -> serde_json::Value {
    let state = &start["envelope"]["state"];
    let roster = |team: &str| {
        state["teams"][team]["heroes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|hero| hero["name"].as_str())
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "v": 1, "type": "setup", "game_id": game, "map": state["board"]["map_id"],
        "red": roster("RED"), "blue": roster("BLUE"), "player_names": hero_names,
        "game_type": state["game_type"], "cheats": state["cheats_enabled"], "seed": state["rng_seed"],
        "engine": "rust", "created_at": created_at,
    })
}

impl Archive {
    pub fn new(
        start_revision: u64,
        revision: u64,
        start: &[u8],
        entries: Vec<Entry>,
        hero_names: BTreeMap<String, String>,
        bot_seats: BTreeSet<String>,
    ) -> Result<Self> {
        Ok(Self {
            kind: KIND.into(),
            schema: SCHEMA,
            start_revision,
            revision,
            start_sha256: sha256(start),
            start: serde_json::from_slice(start).map_err(ApiError::internal)?,
            hero_names,
            bot_seats,
            entries,
        })
    }

    pub fn setup_metadata(&self, game: &str) -> Result<serde_json::Value> {
        let start = serde_json::to_value(&self.start).map_err(ApiError::internal)?;
        let created_at = self
            .entries
            .first()
            .map_or(0, |entry| entry.decision.at_ms / 1000);
        Ok(setup_metadata(game, &start, &self.hero_names, created_at))
    }

    pub fn restore(self, target: Option<u64>) -> Result<(LiveGame, BTreeMap<String, String>)> {
        self.walk(target, |_, _| Ok(false))
    }

    pub fn walk(
        self,
        target: Option<u64>,
        mut visit: impl FnMut(&LiveGame, Option<&Decision>) -> Result<bool>,
    ) -> Result<(LiveGame, BTreeMap<String, String>)> {
        if self.kind != KIND || self.schema != SCHEMA || self.entries.len() > MAX_STEPS {
            return Err(ApiError::invalid("Unsupported Rust live replay"));
        }
        let target = target.unwrap_or(self.revision);
        if !(self.start_revision..=self.revision).contains(&target) {
            return Err(ApiError::invalid("Revision is outside this replay"));
        }
        if self.start_revision.checked_add(self.entries.len() as u64) != Some(self.revision)
            || self
                .entries
                .iter()
                .enumerate()
                .any(|(i, entry)| entry.revision != self.start_revision + i as u64 + 1)
        {
            return Err(ApiError::invalid(
                "The live replay revisions are not contiguous",
            ));
        }
        // Keep only checkpoints referenced by a future rewind in this prefix.
        // Every reference must point behind its record.
        let mut last_rewind_use = BTreeMap::new();
        for entry in self
            .entries
            .iter()
            .take_while(|entry| entry.revision <= target)
        {
            if let Operation::Rewind { to, .. } = entry.decision.operation {
                let source = self
                    .start_revision
                    .checked_add(to)
                    .filter(|revision| *revision < entry.revision)
                    .ok_or_else(|| ApiError::invalid("Invalid replay rewind target"))?;
                last_rewind_use.insert(source, entry.revision);
            }
        }
        let mut rewind_checkpoints = BTreeMap::new();
        let invalid_at = |revision, detail: &dyn std::fmt::Display| {
            ApiError::invalid(format!("Live replay revision {revision}: {detail}"))
        };
        let budget = ExecutionBudget::default();
        let start_bytes = serde_json::to_vec(&self.start).map_err(ApiError::internal)?;
        if sha256(&start_bytes) != self.start_sha256 {
            return Err(ApiError::invalid(
                "Live replay starting checkpoint hash differs",
            ));
        }
        if last_rewind_use.contains_key(&self.start_revision) {
            rewind_checkpoints.insert(self.start_revision, start_bytes);
        }
        let mut engine = LiveGame::restore(self.start, &budget)?;
        if engine.position().revision != self.start_revision
            || self
                .bot_seats
                .iter()
                .any(|bot| !engine.players().any(|(hero, _)| hero == bot))
        {
            return Err(ApiError::invalid(
                "Live replay starting revision or bot roster differs",
            ));
        }
        for entry in self
            .entries
            .into_iter()
            .take_while(|entry| entry.revision <= target)
        {
            if visit(&engine, Some(&entry.decision))? {
                return Ok((engine, self.hero_names));
            }
            let Decision {
                at_ms,
                rebase_ms,
                operation,
            } = entry.decision;
            let mut next = match operation {
                Operation::Command { actor, command } => {
                    prepare_command(
                        &engine,
                        &actor,
                        command,
                        at_ms,
                        self.bot_seats.iter().map(String::as_str),
                    )
                    .map_err(|error| invalid_at(entry.revision, &error))?
                    .0
                }
                Operation::Timeout { entropy } => {
                    engine
                        .prepare_timeout(at_ms, entropy, &budget)
                        .map_err(|error| invalid_at(entry.revision, &error))?
                        .ok_or_else(|| invalid_at(entry.revision, &"clock timeout was not due"))?
                        .0
                }
                Operation::Patch { command, .. } => {
                    engine
                        .prepare_patch(&command, at_ms, &budget)
                        .map_err(|error| invalid_at(entry.revision, &error))?
                        .0
                }
                Operation::Unstick { command, .. } => {
                    engine
                        .prepare_unstick(&command, at_ms, &budget)
                        .map_err(|error| invalid_at(entry.revision, &error))?
                        .0
                }
                Operation::Rewind { to, .. } => {
                    let source = self.start_revision + to;
                    let bytes = rewind_checkpoints
                        .get(&source)
                        .ok_or_else(|| invalid_at(entry.revision, &"rewind checkpoint missing"))?;
                    let checkpoint = serde_json::from_slice(bytes)
                        .map_err(|error| invalid_at(entry.revision, &error))?;
                    if last_rewind_use[&source] == entry.revision {
                        rewind_checkpoints.remove(&source);
                    }
                    let restored = LiveGame::restore(checkpoint, &budget)
                        .map_err(|error| invalid_at(entry.revision, &error))?;
                    engine
                        .prepare_table_rewind(restored, at_ms)
                        .map_err(|error| invalid_at(entry.revision, &error))?
                        .0
                }
                change => {
                    let change = match &change {
                        Operation::PauseOpened { id } => PauseVoteChange::Open(id),
                        Operation::PauseReleased => PauseVoteChange::Cancel,
                        Operation::PauseApproved { requester } => {
                            PauseVoteChange::Approve(requester)
                        }
                        _ => unreachable!(),
                    };
                    engine
                        .prepare_pause_vote(change, at_ms)
                        .map_err(|error| invalid_at(entry.revision, &error))?
                        .0
                }
            };
            if let Some(rebase_ms) = rebase_ms {
                next.rebase_clock_after_processing(rebase_ms);
            }
            let checkpoint = next
                .checkpoint()
                .map_err(|error| invalid_at(entry.revision, &error))?;
            let bytes = serde_json::to_vec(&checkpoint).map_err(ApiError::internal)?;
            if next.position().revision != entry.revision
                || sha256(&bytes) != entry.checkpoint_sha256
            {
                return Err(invalid_at(
                    entry.revision,
                    &"reconstructed checkpoint differs",
                ));
            }
            if last_rewind_use.contains_key(&entry.revision) {
                rewind_checkpoints.insert(entry.revision, bytes);
            }
            engine = next;
        }
        let _ = visit(&engine, None)?;
        Ok((engine, self.hero_names))
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decision_defaults_full_width_times_and_variant_field_policy() {
        let value = json!({"at_ms":u64::MAX,"operation":{"kind":"pause_released"}});
        let decision: Decision = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decision.at_ms, u64::MAX);
        assert!(decision.rebase_ms.is_none());
        assert_eq!(serde_json::to_value(decision).unwrap(), value);
        let mut invalid = value.clone();
        invalid["typo"] = json!(true);
        assert!(serde_json::from_value::<Decision>(invalid).is_err());
        let mut invalid = value.clone();
        invalid["operation"]["typo"] = json!(true);
        // Serde's internally tagged unit variants tolerate extra fields even
        // with deny_unknown_fields. Preserve this existing compatibility policy.
        assert!(serde_json::from_value::<Decision>(invalid).is_ok());
        let structured = json!({"at_ms":0,"operation":{"kind":"timeout","entropy":1,"typo":true}});
        assert!(serde_json::from_value::<Decision>(structured).is_err());
        let unknown = json!({"at_ms":0,"operation":{"kind":"future_operation"}});
        assert!(serde_json::from_value::<Decision>(unknown).is_err());
        let mut invalid = value;
        invalid["at_ms"] = json!(-1);
        assert!(serde_json::from_value::<Decision>(invalid).is_err());
    }
    #[test]
    fn replay_archive_mutations_reject_ranges_hashes_versions_and_rewind_overflow() {
        use goa2_core::api::{GameContent, GameMode, GameSetup};
        let content =
            GameContent::read(include_bytes!("../../../content/smoke-game.json").as_slice())
                .unwrap();
        let setup = GameSetup {
            map: "forgotten_island".into(),
            red_heroes: vec!["Arien".into()],
            blue_heroes: vec!["Brogan".into()],
            seed: 29,
            mode: GameMode::Quick,
            cheats_enabled: false,
            tie_breaker: None,
        };
        let game = LiveGame::create(&content, &setup, &ExecutionBudget::default())
            .unwrap()
            .0;
        let checkpoint = serde_json::to_vec(&game.checkpoint().unwrap()).unwrap();
        let archive = Archive::new(
            0,
            0,
            &checkpoint,
            Vec::new(),
            BTreeMap::new(),
            BTreeSet::new(),
        )
        .unwrap();
        let original = serde_json::to_value(archive).unwrap();
        for (key, value) in [
            ("kind", json!("future")),
            ("schema", json!(2)),
            ("start_revision", json!(u64::MAX)),
            ("revision", json!(u64::MAX)),
            ("start_sha256", json!("0".repeat(64))),
            ("bot_seats", json!(["unknown-seat"])),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            let archive: Archive = serde_json::from_value(changed).unwrap();
            assert!(archive.restore(None).is_err(), "{key}");
        }
        for to in [0, 1, u64::MAX] {
            let mut changed = original.clone();
            changed["revision"] = json!(1);
            changed["entries"] = json!([{"revision":1,"checkpoint_sha256":"0".repeat(64),"decision":{"at_ms":0,"operation":{"kind":"rewind","proposer":"hero_arien","to":to}}}]);
            let archive: Archive = serde_json::from_value(changed).unwrap();
            assert!(archive.restore(None).is_err());
        }
        let archive: Archive = serde_json::from_value(original).unwrap();
        assert_eq!(archive.restore(None).unwrap().0.position().revision, 0);
    }
}
