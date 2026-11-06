//! Schema of `.ivar/memory.db`.

use rusqlite::Transaction;

/// `PRAGMA application_id` of a hall memory database: ASCII `ivm1`.
pub const APPLICATION_ID: i32 = 0x6976_6d31;

/// The memory schema this binary migrates to.
pub const MEMORY_SCHEMA_VERSION: i64 = 5;

/// Every schema object a memory database up to [`MEMORY_SCHEMA_VERSION`]
/// owns. Anything else in the file was put there by another program.
pub(crate) const MEMORY_OBJECTS: &[&str] = &[
    "memory_schema",
    OBSERVATIONS_TABLE,
    "observations_session",
    "observations_source_event",
    "observations_time",
    "observations_no_update",
    "pages",
    "pages_kind",
    "pages_fts",
    "pages_fts_data",
    "pages_fts_idx",
    "pages_fts_content",
    "pages_fts_docsize",
    "pages_fts_config",
    "page_links",
    "page_links_from",
    "page_links_to",
    "observations_fts",
    "observations_fts_data",
    "observations_fts_idx",
    "observations_fts_docsize",
    "observations_fts_config",
    "observations_fts_insert",
    "observations_fts_delete",
    "page_anchors",
    "page_anchors_page",
    "page_anchors_target",
    "page_anchors_symbol",
    "memory_usage",
    "memory_usage_session_ts",
    "memory_misses",
    "memory_misses_session_ts",
    "handoffs",
    "handoffs_state_queue",
    "handoffs_owner",
    "messages",
    "messages_state_queue",
    "messages_sender",
];

pub(crate) const OBSERVATIONS_TABLE: &str = "observations";

// The version lives in a table, not only in `user_version`: an ivar older than
// the split opens this file as its graph and overwrites `user_version` with the
// graph's version, and the repair needs the real one back.
pub(crate) const MIGRATION_V1: &str = "
CREATE TABLE memory_schema (version INTEGER NOT NULL);
INSERT INTO memory_schema (version) VALUES (1);
";

// DELETE stays allowed: the retention sweep removes expired observations.
pub(crate) const MIGRATION_V2: &str = "
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

pub(crate) const MIGRATION_V3: &str = "
CREATE TABLE pages (
    id           INTEGER PRIMARY KEY,
    path         TEXT NOT NULL UNIQUE,
    kind         TEXT NOT NULL,
    tier         TEXT NOT NULL,
    title        TEXT NOT NULL,
    tags         TEXT NOT NULL DEFAULT '[]',
    pinned       INTEGER NOT NULL DEFAULT 0,
    expires_at   TEXT,
    size         INTEGER NOT NULL,
    mtime_ns     INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    indexed_at   TEXT NOT NULL
);
CREATE INDEX pages_kind ON pages (kind);
CREATE VIRTUAL TABLE pages_fts USING fts5(title, tags, body, tokenize = 'unicode61 remove_diacritics 2');
CREATE TABLE page_links (
    from_path     TEXT NOT NULL,
    to_path       TEXT,
    source        TEXT NOT NULL,
    relation_type TEXT NOT NULL DEFAULT '',
    target_text   TEXT NOT NULL
);
CREATE INDEX page_links_from ON page_links (from_path);
CREATE INDEX page_links_to ON page_links (to_path);
CREATE VIRTUAL TABLE observations_fts USING fts5(source_event, body, content = 'observations', content_rowid = 'id');
CREATE TRIGGER observations_fts_insert AFTER INSERT ON observations BEGIN
    INSERT INTO observations_fts (rowid, source_event, body) VALUES (new.id, new.source_event, new.body);
END;
CREATE TRIGGER observations_fts_delete AFTER DELETE ON observations BEGIN
    INSERT INTO observations_fts (observations_fts, rowid, source_event, body)
    VALUES ('delete', old.id, old.source_event, old.body);
END;
INSERT INTO observations_fts (observations_fts) VALUES ('rebuild');
";

pub(crate) const MIGRATION_V4: &str = "
ALTER TABLE pages ADD COLUMN anchors_pending INTEGER NOT NULL DEFAULT 0;
CREATE TABLE page_anchors (
    page_path TEXT NOT NULL,
    repo      TEXT NOT NULL,
    path      TEXT NOT NULL,
    symbol    TEXT
);
CREATE INDEX page_anchors_page ON page_anchors (page_path);
CREATE INDEX page_anchors_target ON page_anchors (repo, path);
CREATE INDEX page_anchors_symbol ON page_anchors (symbol) WHERE symbol IS NOT NULL;
CREATE TABLE memory_usage (
    id                INTEGER PRIMARY KEY,
    ts                INTEGER NOT NULL,
    source            TEXT NOT NULL,
    session           TEXT,
    query             TEXT,
    filters           TEXT NOT NULL DEFAULT '{}',
    page_count        INTEGER NOT NULL DEFAULT 0,
    observation_count INTEGER NOT NULL DEFAULT 0,
    duration_ms       INTEGER NOT NULL DEFAULT 0,
    error             INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX memory_usage_session_ts ON memory_usage (session, ts);
CREATE TABLE memory_misses (
    id       INTEGER PRIMARY KEY,
    ts       INTEGER NOT NULL,
    session  TEXT,
    kind     TEXT NOT NULL,
    query    TEXT,
    filters  TEXT,
    pattern  TEXT,
    usage_id INTEGER,
    fallback INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX memory_misses_session_ts ON memory_misses (session, ts);
";

// Coordination rows are primary data: no reindex rebuilds them. The sweep prunes
// terminal rows after 30 days, and `ivar memory clean` removes the whole file.
// `queue` is a feature name, or '' for the hall queue.
pub(crate) const MIGRATION_V5: &str = "
CREATE TABLE handoffs (
    id             TEXT PRIMARY KEY,
    queue          TEXT NOT NULL,
    to_kind        TEXT NOT NULL DEFAULT '',
    to_value       TEXT NOT NULL DEFAULT '',
    owner_key      TEXT NOT NULL,
    owner_provider TEXT NOT NULL,
    summary        TEXT NOT NULL,
    pending_tasks  TEXT NOT NULL DEFAULT '[]',
    open_decisions TEXT NOT NULL DEFAULT '[]',
    modified_files TEXT NOT NULL DEFAULT '[]',
    state          TEXT NOT NULL DEFAULT 'open',
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    accepted_by    TEXT,
    accepted_at    INTEGER,
    closed_at      INTEGER
);
CREATE INDEX handoffs_state_queue ON handoffs (state, queue, expires_at);
CREATE INDEX handoffs_owner ON handoffs (owner_key);
CREATE TABLE messages (
    id              TEXT PRIMARY KEY,
    queue           TEXT NOT NULL,
    to_kind         TEXT NOT NULL DEFAULT '',
    to_value        TEXT NOT NULL DEFAULT '',
    sender_key      TEXT NOT NULL,
    sender_provider TEXT NOT NULL,
    subject         TEXT,
    body            TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending',
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    popped_by       TEXT,
    popped_at       INTEGER,
    closed_at       INTEGER
);
CREATE INDEX messages_state_queue ON messages (state, queue, expires_at);
CREATE INDEX messages_sender ON messages (sender_key);
";

pub(crate) fn migrate(tx: &Transaction<'_>, from: i64) -> rusqlite::Result<()> {
    if from < 1 {
        tx.execute_batch(MIGRATION_V1)?;
    }
    if from < 2 {
        tx.execute_batch(MIGRATION_V2)?;
    }
    if from < 3 {
        tx.execute_batch(MIGRATION_V3)?;
    }
    if from < 4 {
        tx.execute_batch(MIGRATION_V4)?;
    }
    if from < 5 {
        tx.execute_batch(MIGRATION_V5)?;
    }
    tx.execute(
        "UPDATE memory_schema SET version = ?1",
        [MEMORY_SCHEMA_VERSION],
    )?;
    tx.execute_batch(&format!("PRAGMA user_version = {MEMORY_SCHEMA_VERSION};"))
}
