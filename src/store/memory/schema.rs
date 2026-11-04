//! Schema of `.ivar/memory.db`.

use rusqlite::Transaction;

/// `PRAGMA application_id` of a hall memory database: ASCII `ivm1`.
pub const APPLICATION_ID: i32 = 0x6976_6d31;

/// The memory schema this binary migrates to.
pub const MEMORY_SCHEMA_VERSION: i64 = 2;

/// Every schema object a memory database up to [`MEMORY_SCHEMA_VERSION`]
/// owns. Anything else in the file was put there by another program.
pub(crate) const MEMORY_OBJECTS: &[&str] = &[
    "memory_schema",
    OBSERVATIONS_TABLE,
    "observations_session",
    "observations_source_event",
    "observations_time",
    "observations_no_update",
];

pub(crate) const OBSERVATIONS_TABLE: &str = "observations";

// The version lives in a table, not only in `user_version`: an ivar older than
// the split opens this file as its graph and overwrites `user_version` with the
// graph's version, and the repair needs the real one back.
const MIGRATION_V1: &str = "
CREATE TABLE memory_schema (version INTEGER NOT NULL);
INSERT INTO memory_schema (version) VALUES (1);
";

// DELETE stays allowed: the retention sweep removes expired observations.
const MIGRATION_V2: &str = "
CREATE TABLE observations (
    id                INTEGER PRIMARY KEY,
    event_id          TEXT NOT NULL UNIQUE,
    source_event      TEXT NOT NULL,
    extension         TEXT NOT NULL DEFAULT 'ivar',
    occurred_at       TEXT NOT NULL,
    session_id        TEXT NOT NULL DEFAULT '',
    native_session_id TEXT NOT NULL,
    provider          TEXT NOT NULL,
    ivar_version      TEXT NOT NULL,
    feature           TEXT NOT NULL DEFAULT '',
    parent_feature    TEXT NOT NULL DEFAULT '',
    repo              TEXT NOT NULL DEFAULT '',
    origin            TEXT NOT NULL DEFAULT '',
    importance        INTEGER NOT NULL,
    attempt_id        TEXT NOT NULL DEFAULT '',
    body              TEXT NOT NULL
);
CREATE INDEX observations_session ON observations (session_id, native_session_id, occurred_at);
CREATE INDEX observations_source_event ON observations (source_event, occurred_at);
CREATE INDEX observations_time ON observations (occurred_at);
CREATE TRIGGER observations_no_update BEFORE UPDATE ON observations
BEGIN SELECT RAISE(ABORT, 'observations are append-only'); END;
";

pub(crate) fn migrate(tx: &Transaction<'_>, from: i64) -> rusqlite::Result<()> {
    if from < 1 {
        tx.execute_batch(MIGRATION_V1)?;
    }
    if from < 2 {
        tx.execute_batch(MIGRATION_V2)?;
    }
    tx.execute(
        "UPDATE memory_schema SET version = ?1",
        [MEMORY_SCHEMA_VERSION],
    )?;
    tx.execute_batch(&format!("PRAGMA user_version = {MEMORY_SCHEMA_VERSION};"))
}
