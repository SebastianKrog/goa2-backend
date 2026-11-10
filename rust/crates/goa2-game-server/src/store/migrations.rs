//! Atomic upgrades for legacy unversioned databases and numbered schemas.
use rusqlite::{Connection, Transaction};

const VERSION: u32 = 3;
const BASE: &str = include_str!("migrations/001-base.sql");

pub(super) fn apply(connection: &mut Connection) -> rusqlite::Result<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "Unsupported database schema {version}; maximum is {VERSION}. Restore a compatible binary or backup."
        )));
    }
    connection.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON;",
    )?;
    let tx = connection.transaction()?;
    upgrade(&tx, version)?;
    tx.commit()
}

fn upgrade(tx: &Transaction<'_>, version: u32) -> rusqlite::Result<()> {
    if version < 1 {
        tx.execute_batch(BASE)?;
        tx.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        add_column(tx, "replay_entries", "history", "BLOB")?;
        tx.pragma_update(None, "user_version", 2)?;
    }
    if version < 3 {
        add_column(tx, "replay_shares", "created_at", "INTEGER")?;
        tx.pragma_update(None, "user_version", 3)?;
    }
    Ok(())
}

fn add_column(tx: &Transaction<'_>, table: &str, column: &str, kind: &str) -> rusqlite::Result<()> {
    // All identifiers come exclusively from the two literal migration calls above.
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
        [table, column],
        |row| row.get(0),
    )?;
    if !exists {
        tx.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_and_each_retained_schema_upgrade_idempotently() {
        for version in 0..=VERSION {
            let mut db = Connection::open_in_memory().unwrap();
            if version > 0 {
                db.execute_batch(BASE).unwrap();
                if version >= 2 {
                    db.execute_batch("ALTER TABLE replay_entries ADD COLUMN history BLOB")
                        .unwrap();
                }
                if version >= 3 {
                    db.execute_batch("ALTER TABLE replay_shares ADD COLUMN created_at INTEGER")
                        .unwrap();
                }
                db.pragma_update(None, "user_version", version).unwrap();
                db.execute("INSERT INTO games VALUES ('game', 7, x'0102', x'0304')", [])
                    .unwrap();
                db.execute("INSERT INTO replay_starts VALUES ('game', 7, x'0304')", [])
                    .unwrap();
                db.execute(
                    "INSERT INTO accepted_actions VALUES ('game','hero','receipt',x'05',x'06')",
                    [],
                )
                .unwrap();
                db.execute("INSERT INTO replay_shares (token,game_id,total,metadata) VALUES ('share','game',1,x'07')", []).unwrap();
            }
            apply(&mut db).unwrap();
            apply(&mut db).unwrap();
            let actual: u32 = db
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(actual, VERSION);
            if version > 0 {
                let row: (u64, Vec<u8>, Vec<u8>) = db
                    .query_row(
                        "SELECT revision,credentials,checkpoint FROM games",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .unwrap();
                assert_eq!(row, (7, vec![1, 2], vec![3, 4]));
                let receipt: Vec<u8> = db
                    .query_row("SELECT reply FROM accepted_actions", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(receipt, vec![6]);
                let token: String = db
                    .query_row("SELECT token FROM replay_shares", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(token, "share");
            }
        }
    }

    #[test]
    fn unversioned_existing_database_preserves_backfilled_replay_start() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE games (id TEXT PRIMARY KEY, revision INTEGER NOT NULL, credentials BLOB NOT NULL, checkpoint BLOB NOT NULL); INSERT INTO games VALUES ('legacy',9,x'01',x'02');").unwrap();
        apply(&mut db).unwrap();
        let row: (u64, Vec<u8>) = db
            .query_row("SELECT revision,checkpoint FROM replay_starts", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(row, (9, vec![2]));
    }

    #[test]
    fn failed_upgrade_rolls_back_schema_version_and_ddl() {
        let mut db = Connection::open_in_memory().unwrap();
        // An incompatible legacy table makes the real backfill fail after CREATE statements.
        db.execute_batch(
            "CREATE TABLE games (id TEXT PRIMARY KEY); INSERT INTO games VALUES ('keep');",
        )
        .unwrap();
        assert!(apply(&mut db).is_err());
        let version: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 0);
        let count: u32 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='replay_entries'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let id: String = db
            .query_row("SELECT id FROM games", [], |r| r.get(0))
            .unwrap();
        assert_eq!(id, "keep");
    }

    #[test]
    fn future_schema_is_rejected_before_any_migration() {
        let mut db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", VERSION + 1).unwrap();
        assert!(
            apply(&mut db)
                .unwrap_err()
                .to_string()
                .contains("Unsupported database schema")
        );
        let count: u32 = db
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let version: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, VERSION + 1);
    }
}
