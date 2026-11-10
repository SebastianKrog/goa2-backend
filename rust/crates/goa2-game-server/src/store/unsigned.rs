//! Preserve the old driver's checked u64 ↔ signed SQLite INTEGER contract.
//! Full-width seeds/times inside JSON blobs are not narrowed by this adapter.
use rusqlite::{
    Row,
    types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Value, ValueRef},
};

pub(super) struct SqlU64(pub u64);
impl ToSql for SqlU64 {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let value = i64::try_from(self.0)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
        Ok(ToSqlOutput::Owned(Value::Integer(value)))
    }
}
impl FromSql for SqlU64 {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let integer = value.as_i64()?;
        u64::try_from(integer)
            .map(Self)
            .map_err(|_| FromSqlError::OutOfRange(integer))
    }
}
pub(super) fn unsigned(row: &Row<'_>, column: usize) -> rusqlite::Result<u64> {
    row.get::<_, SqlU64>(column).map(|value| value.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_unsigned_boundary_roundtrips_and_rejects_overflow_without_writing() {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE values_checked (value INTEGER NOT NULL)")
            .unwrap();
        for value in [0, 1, i64::MAX as u64] {
            db.execute("INSERT INTO values_checked VALUES (?1)", [SqlU64(value)])
                .unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT value FROM values_checked ORDER BY rowid DESC LIMIT 1",
                    [],
                    |row| unsigned(row, 0)
                )
                .unwrap(),
                value
            );
        }
        for value in [i64::MAX as u64 + 1, u64::MAX] {
            let tx = db.transaction().unwrap();
            assert!(
                tx.execute("INSERT INTO values_checked VALUES (?1)", [SqlU64(value)])
                    .is_err()
            );
            drop(tx);
        }
        assert_eq!(
            db.query_row("SELECT count(*) FROM values_checked", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
        for query in ["SELECT -1", "SELECT 1.5", "SELECT '1'", "SELECT NULL"] {
            assert!(db.query_row(query, [], |row| unsigned(row, 0)).is_err());
        }
        assert!(
            db.query_row("SELECT NULL", [], |row| row.get::<_, Option<SqlU64>>(0))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn bundled_sqlite_contains_the_wal_memory_disclosure_fix() {
        assert!(
            rusqlite::version_number() >= 3_054_000,
            "RUSTSEC-2026-0459 requires SQLite 3.54.0 or newer"
        );
    }
}
