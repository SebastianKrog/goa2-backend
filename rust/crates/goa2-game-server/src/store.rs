mod migrations;
mod unsigned;
use unsigned::{SqlU64, unsigned};

use crate::{
    ApiError, Result,
    bots::BotSpec,
    history::HistoryContext,
    replay::{self, Archive, Decision, Entry},
    shares::{Metadata, Seek, Share},
};
use goa2_core::api::GameCheckpoint;
use rusqlite::{Connection, MAIN_DB, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::Path,
    sync::Mutex,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct PlayerToken {
    pub hero_id: String,
    pub token: String,
}

#[derive(Serialize, Deserialize)]
pub struct Credentials {
    pub player_tokens: Vec<PlayerToken>,
    pub spectator_token: String,
    pub hero_names: BTreeMap<String, String>,
    #[serde(default)]
    pub bots: BTreeMap<String, BotSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_clock_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisioning_hash: Option<String>,
}

pub struct SavedGame {
    pub id: String,
    pub revision: u64,
    pub credentials: Credentials,
    pub checkpoint: GameCheckpoint,
}

pub struct AcceptedAction<'a> {
    pub hero_id: &'a str,
    pub action_id: &'a str,
    pub command: &'a Value,
    pub reply: &'a Value,
}

type ReplayHistory = (u64, Vec<(Entry, Option<HistoryContext>)>);

pub struct Store {
    connection: Mutex<Connection>,
    _ownership: File,
}

impl Store {
    pub fn backup(
        source: &Path,
        output: &Path,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        if output.symlink_metadata().is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Backup destination exists",
            )
            .into());
        }
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let staged = parent.join(format!(".goa2-backup-{}.sqlite", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(&staged)?);

        let result = (|| -> std::result::Result<(), Box<dyn std::error::Error>> {
            {
                let mut source =
                    Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                let snapshot = source.transaction()?;
                // Pin a WAL snapshot across backup steps and reject other SQLite databases.
                snapshot
                    .query_row("SELECT 1 FROM games LIMIT 1", [], |_| Ok(()))
                    .optional()?;
                snapshot.backup(MAIN_DB, &staged, None::<fn(rusqlite::backup::Progress)>)?;
            }
            {
                let copy = Connection::open(&staged)?;
                copy.pragma_update(None, "journal_mode", "DELETE")?;
                let check: String = copy.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
                if check != "ok" {
                    return Err("Backup failed the SQLite integrity check".into());
                }
            }
            OpenOptions::new().write(true).open(&staged)?.sync_all()?;
            // Publish a complete copy without replacing a file created by another process.
            fs::hard_link(&staged, output)?;
            #[cfg(unix)]
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        let cleanup = fs::remove_file(staged);
        result?;
        cleanup?;
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let ownership = options.open(lock_path).map_err(ApiError::internal)?;
        ownership.try_lock().map_err(ApiError::internal)?;
        options.open(path).map_err(ApiError::internal)?;
        let mut connection = Connection::open(path).map_err(ApiError::internal)?;
        migrations::apply(&mut connection).map_err(ApiError::internal)?;
        Ok(Self {
            connection: Mutex::new(connection),
            _ownership: ownership,
        })
    }

    pub fn load(&self) -> Result<Vec<SavedGame>> {
        let db = self.connection.lock().map_err(ApiError::internal)?;
        let mut query = db
            .prepare("SELECT id, revision, credentials, checkpoint FROM games")
            .map_err(ApiError::internal)?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    unsigned(row, 1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })
            .map_err(ApiError::internal)?;
        rows.map(|row| {
            let (id, revision, credentials, checkpoint) = row.map_err(ApiError::internal)?;
            Ok(SavedGame {
                id,
                revision,
                credentials: serde_json::from_slice(&credentials).map_err(ApiError::internal)?,
                checkpoint: serde_json::from_slice(&checkpoint).map_err(ApiError::internal)?,
            })
        })
        .collect()
    }

    pub fn create(&self, game: &SavedGame) -> Result<()> {
        let snapshot = encode(&game.checkpoint)?;
        let mut db = self.connection.lock().map_err(ApiError::internal)?;
        let tx = db.transaction().map_err(ApiError::internal)?;
        tx.execute(
            "INSERT INTO games (id, revision, credentials, checkpoint) VALUES (?1, ?2, ?3, ?4)",
            params![
                game.id,
                SqlU64(game.revision),
                encode(&game.credentials)?,
                snapshot
            ],
        )
        .map_err(ApiError::internal)?;
        tx.execute(
            "INSERT INTO replay_starts (game_id, revision, checkpoint) VALUES (?1, ?2, ?3)",
            params![game.id, SqlU64(game.revision), snapshot],
        )
        .map_err(ApiError::internal)?;
        tx.commit().map_err(ApiError::internal)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        self.connection
            .lock()
            .map_err(ApiError::internal)?
            .execute("DELETE FROM games WHERE id=?1", params![id])
            .map_err(ApiError::internal)?;
        Ok(())
    }

    pub fn accepted(
        &self,
        game: &str,
        hero: &str,
        id: &str,
        command: &Value,
    ) -> Result<Option<Value>> {
        let row = self.connection.lock().map_err(ApiError::internal)?.query_row(
            "SELECT command, reply FROM accepted_actions WHERE game_id=?1 AND hero_id=?2 AND action_id=?3",
            params![game, hero, id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        ).optional().map_err(ApiError::internal)?;
        let Some((original, reply)) = row else {
            return Ok(None);
        };
        let original: Value = serde_json::from_slice(&original).map_err(ApiError::internal)?;
        if original != *command {
            return Err(ApiError::conflict(
                "Client action ID is already used for a different command",
            ));
        }
        Ok(Some(
            serde_json::from_slice(&reply).map_err(ApiError::internal)?,
        ))
    }

    // These arguments are one atomic journal/receipt transaction; keep the boundary explicit.
    #[allow(clippy::too_many_arguments)]
    pub fn commit(
        &self,
        game: &str,
        previous: u64,
        revision: u64,
        checkpoint: &GameCheckpoint,
        action: Option<AcceptedAction<'_>>,
        decision: Decision,
        history: HistoryContext,
    ) -> Result<()> {
        let snapshot = encode(checkpoint)?;
        let entry = encode(&Entry {
            revision,
            checkpoint_sha256: replay::sha256(&snapshot),
            decision,
        })?;
        let history = encode(&history)?;
        let receipt = action
            .map(|action| -> Result<_> {
                Ok((
                    action.hero_id,
                    action.action_id,
                    encode(action.command)?,
                    encode(action.reply)?,
                ))
            })
            .transpose()?;
        let mut db = self.connection.lock().map_err(ApiError::internal)?;
        let tx = db.transaction().map_err(ApiError::internal)?;
        let count = tx
            .execute(
                "UPDATE games SET revision=?1, checkpoint=?2 WHERE id=?3 AND revision=?4",
                params![SqlU64(revision), snapshot, game, SqlU64(previous)],
            )
            .map_err(ApiError::internal)?;
        if count != 1 {
            return Err(ApiError::conflict("Stored game revision changed"));
        }
        if let Some((hero, id, command, reply)) = receipt {
            tx.execute(
                "INSERT INTO accepted_actions (game_id, hero_id, action_id, command, reply) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![game, hero, id, command, reply],
            ).map_err(ApiError::internal)?;
        }
        tx.execute(
            "INSERT INTO replay_entries (game_id, revision, entry, history) VALUES (?1, ?2, ?3, ?4)",
            params![game, SqlU64(revision), entry, history],
        )
        .map_err(ApiError::internal)?;
        tx.commit().map_err(ApiError::internal)
    }

    pub fn pin_report_replay(&self, game: &str, archive: &Archive) -> Result<()> {
        let encoded = encode(archive)?;
        if encoded.len() > replay::MAX_ARCHIVE_BYTES {
            return Err(ApiError::invalid(
                "Reported replay exceeds the archive size limit",
            ));
        }
        self.connection
            .lock()
            .map_err(ApiError::internal)?
            .execute(
                "INSERT INTO reported_replays (game_id, revision, archive) VALUES (?1, ?2, ?3)
             ON CONFLICT(game_id) DO UPDATE SET revision=excluded.revision, archive=excluded.archive
             WHERE reported_replays.revision < excluded.revision",
                params![game, SqlU64(archive.revision), encoded],
            )
            .map_err(ApiError::internal)?;
        Ok(())
    }

    pub fn reported_replay(&self, game: &str) -> Result<Archive> {
        let bytes: Vec<u8> = self
            .connection
            .lock()
            .map_err(ApiError::internal)?
            .query_row(
                "SELECT archive FROM reported_replays WHERE game_id=?1",
                [game],
                |row| row.get(0),
            )
            .optional()
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("Replay not found"))?;
        serde_json::from_slice(&bytes).map_err(ApiError::internal)
    }

    pub fn replay_catalog(&self) -> Result<Vec<Value>> {
        let rows = {
            let db = self.connection.lock().map_err(ApiError::internal)?;
            let mut query = db
                .prepare(
                    "SELECT s.game_id, g.revision - s.revision, CAST(s.checkpoint AS TEXT),
                            json_extract(CAST(g.credentials AS TEXT), '$.hero_names'),
                            (SELECT json_extract(CAST(e.entry AS TEXT), '$.decision.at_ms')
                             FROM replay_entries e WHERE e.game_id=s.game_id ORDER BY e.revision LIMIT 1)
                     FROM replay_starts s JOIN games g ON g.id=s.game_id
                     UNION ALL
                     SELECT r.game_id, r.revision - json_extract(CAST(r.archive AS TEXT), '$.start_revision'),
                            json_extract(CAST(r.archive AS TEXT), '$.start'),
                            json_extract(CAST(r.archive AS TEXT), '$.hero_names'),
                            json_extract(CAST(r.archive AS TEXT), '$.entries[0].decision.at_ms')
                     FROM reported_replays r WHERE NOT EXISTS (SELECT 1 FROM games g WHERE g.id=r.game_id)",
                )
                .map_err(ApiError::internal)?;
            query
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        unsigned(row, 1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<SqlU64>>(4)?.map(|value| value.0),
                    ))
                })
                .map_err(ApiError::internal)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(ApiError::internal)?
        };
        rows.into_iter()
            .map(|(game, total, checkpoint, names, at_ms)| {
                let checkpoint = serde_json::from_str(&checkpoint).map_err(ApiError::internal)?;
                let names = serde_json::from_str::<Value>(&names).map_err(ApiError::internal)?;
                let mut setup =
                    replay::setup_metadata(&game, &checkpoint, names, at_ms.unwrap_or(0) / 1000);
                setup["num_decisions"] = json!(total);
                Ok(setup)
            })
            .collect()
    }

    pub fn share_catalog(&self) -> Result<Vec<Value>> {
        let db = self.connection.lock().map_err(ApiError::internal)?;
        let mut query = db
            .prepare(
                "SELECT s.token, s.game_id, s.total,
                        COALESCE(s.created_at, json_extract(CAST(s.metadata AS TEXT), '$.setup.created_at')),
                        length(s.metadata) + COALESCE(SUM(length(g.gzip)), 0)
                 FROM replay_shares s LEFT JOIN replay_share_groups g ON g.token=s.token
                 GROUP BY s.token",
            )
            .map_err(ApiError::internal)?;
        query
            .query_map([], |row| {
                Ok(json!({
                    "token": row.get::<_, String>(0)?, "game_id": row.get::<_, String>(1)?,
                    "total_decisions": unsigned(row,2)?, "created_at": row.get::<_,Option<SqlU64>>(3)?.map(|value|value.0),
                    "size_bytes": unsigned(row,4)?, "engine": "rust",
                }))
            })
            .map_err(ApiError::internal)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ApiError::internal)
    }

    pub fn revoke_share(&self, token: &str) -> Result<()> {
        let removed = self
            .connection
            .lock()
            .map_err(ApiError::internal)?
            .execute("DELETE FROM replay_shares WHERE token=?1", [token])
            .map_err(ApiError::internal)?;
        if removed == 0 {
            return Err(ApiError::not_found("Share not found"));
        }
        Ok(())
    }

    pub fn share_for_game(&self, game: &str) -> Result<Option<String>> {
        self.connection
            .lock()
            .map_err(ApiError::internal)?
            .query_row(
                "SELECT token FROM replay_shares WHERE game_id=?1",
                [game],
                |row| row.get(0),
            )
            .optional()
            .map_err(ApiError::internal)
    }

    pub fn save_share(&self, game: &str, share: Share) -> Result<String> {
        let token = format!("rust-{}", uuid::Uuid::new_v4().simple());
        let metadata = encode(&share.metadata)?;
        let mut db = self.connection.lock().map_err(ApiError::internal)?;
        let tx = db.transaction().map_err(ApiError::internal)?;
        tx.execute(
            "INSERT INTO replay_shares (token, game_id, total, metadata, created_at) VALUES (?1, ?2, ?3, ?4, unixepoch())",
            params![token, game, SqlU64(share.total), metadata],
        )
        .map_err(ApiError::internal)?;
        for (start, gzip) in share.groups {
            tx.execute(
                "INSERT INTO replay_share_groups (token, start, gzip) VALUES (?1, ?2, ?3)",
                params![token, SqlU64(start), gzip],
            )
            .map_err(ApiError::internal)?;
        }
        tx.commit().map_err(ApiError::internal)?;
        Ok(token)
    }

    pub fn share_metadata(&self, token: &str) -> Result<Vec<u8>> {
        self.connection
            .lock()
            .map_err(ApiError::internal)?
            .query_row(
                "SELECT metadata FROM replay_shares WHERE token=?1",
                [token],
                |row| row.get(0),
            )
            .optional()
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("Share not found"))
    }

    pub fn share_group(&self, token: &str, seek: &Seek) -> Result<Vec<u8>> {
        let metadata = if seek.decision.is_none() && seek.round.is_some() {
            let raw = self.share_metadata(token)?;
            Some(serde_json::from_slice::<Metadata>(&raw).map_err(ApiError::internal)?)
        } else {
            None
        };
        let db = self.connection.lock().map_err(ApiError::internal)?;
        let total = db
            .query_row(
                "SELECT total FROM replay_shares WHERE token=?1",
                [token],
                |row| unsigned(row, 0),
            )
            .optional()
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("Share not found"))?;
        let start = seek.group_start(total, metadata.as_ref());
        db.query_row(
            "SELECT gzip FROM replay_share_groups WHERE token=?1 AND start=?2",
            params![token, SqlU64(start)],
            |row| row.get(0),
        )
        .map_err(ApiError::internal)
    }

    pub fn replay_start_revision(&self, game: &str) -> Result<u64> {
        self.connection
            .lock()
            .map_err(ApiError::internal)?
            .query_row(
                "SELECT revision FROM replay_starts WHERE game_id=?1",
                [game],
                |row| unsigned(row, 0),
            )
            .optional()
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("Game replay not found"))
    }

    pub fn history(&self, game: &str, revision: u64) -> Result<ReplayHistory> {
        let db = self.connection.lock().map_err(ApiError::internal)?;
        let start: u64 = db
            .query_row(
                "SELECT revision FROM replay_starts WHERE game_id=?1",
                [game],
                |row| unsigned(row, 0),
            )
            .optional()
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("Game history not found"))?;
        let mut query = db
            .prepare(
                "SELECT entry, history FROM replay_entries
             WHERE game_id=?1 AND revision>?2 AND revision<=?3 ORDER BY revision",
            )
            .map_err(ApiError::internal)?;
        let first = start.max(revision.saturating_sub(replay::MAX_STEPS as u64));
        let rows = query
            .query_map(params![game, SqlU64(first), SqlU64(revision)], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
            })
            .map_err(ApiError::internal)?;
        let rows = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(ApiError::internal)?;
        drop(query);
        drop(db);
        let entries = rows
            .into_iter()
            .map(|(entry, history)| {
                Ok((
                    serde_json::from_slice(&entry).map_err(ApiError::internal)?,
                    history
                        .as_deref()
                        .map(serde_json::from_slice)
                        .transpose()
                        .map_err(ApiError::internal)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((start, entries))
    }

    pub fn replay(
        &self,
        game: &str,
        hero_names: BTreeMap<String, String>,
        bot_seats: BTreeSet<String>,
    ) -> Result<Archive> {
        let db = self.connection.lock().map_err(ApiError::internal)?;
        let (start_revision, revision, start): (u64, u64, Vec<u8>) = db
            .query_row(
                "SELECT replay_starts.revision, games.revision, replay_starts.checkpoint
             FROM replay_starts JOIN games ON games.id = replay_starts.game_id
             WHERE game_id=?1",
                [game],
                |row| Ok((unsigned(row, 0)?, unsigned(row, 1)?, row.get(2)?)),
            )
            .optional()
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("Game replay not found"))?;
        if revision.saturating_sub(start_revision) > replay::MAX_STEPS as u64 {
            return Err(ApiError::invalid("Live replay exceeds the revision limit"));
        }
        let mut query = db
            .prepare("SELECT entry FROM replay_entries WHERE game_id=?1 ORDER BY revision")
            .map_err(ApiError::internal)?;
        let rows = query
            .query_map([game], |row| row.get::<_, Vec<u8>>(0))
            .map_err(ApiError::internal)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(ApiError::internal)?;
        drop(query);
        drop(db);

        let entries = rows
            .into_iter()
            .map(|entry| serde_json::from_slice(&entry).map_err(ApiError::internal))
            .collect::<Result<Vec<_>>>()?;
        Archive::new(
            start_revision,
            revision,
            &start,
            entries,
            hero_names,
            bot_seats,
        )
    }
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(ApiError::internal)
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    #[test]
    fn legacy_credentials_default_optional_metadata_without_discarding_auth() {
        let value = json!({"player_tokens":[{"hero_id":"hero_arien","token":"synthetic-player"}],
            "spectator_token":"synthetic-spectator","hero_names":{},"future_optional":true});
        let credentials: Credentials = serde_json::from_value(value).unwrap();
        assert!(credentials.bots.is_empty());
        assert!(credentials.fixed_clock_ms.is_none() && credentials.provisioning_hash.is_none());
        assert_eq!(credentials.player_tokens[0].token, "synthetic-player");
        assert_eq!(credentials.spectator_token, "synthetic-spectator");
        let encoded = serde_json::to_value(credentials).unwrap();
        assert!(encoded.get("future_optional").is_none());
        assert!(encoded.get("fixed_clock_ms").is_none());
    }
}
