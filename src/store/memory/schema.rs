//! Schema of `.ivar/memory.db`.

use rusqlite::Transaction;

/// `PRAGMA application_id` of a hall memory database: ASCII `ivm1`.
pub const APPLICATION_ID: i32 = 0x6976_6d31;

/// The memory schema this binary migrates to.
pub const MEMORY_SCHEMA_VERSION: i64 = 1;

/// Every schema object a memory database up to [`MEMORY_SCHEMA_VERSION`]
/// owns. Anything else in the file was put there by another program.
pub(crate) const MEMORY_OBJECTS: &[&str] = &["memory_schema"];

// The version lives in a table, not only in `user_version`: an ivar older than
// the split opens this file as its graph and overwrites `user_version` with the
// graph's version, and the repair needs the real one back.
const MIGRATION_V1: &str = "
CREATE TABLE memory_schema (version INTEGER NOT NULL);
INSERT INTO memory_schema (version) VALUES (1);
";

pub(crate) fn migrate(tx: &Transaction<'_>, from: i64) -> rusqlite::Result<()> {
    if from < 1 {
        tx.execute_batch(MIGRATION_V1)?;
    }
    tx.execute(
        "UPDATE memory_schema SET version = ?1",
        [MEMORY_SCHEMA_VERSION],
    )?;
    tx.execute_batch(&format!("PRAGMA user_version = {MEMORY_SCHEMA_VERSION};"))
}
