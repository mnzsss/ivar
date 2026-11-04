//! Opening, inspecting and repairing `.ivar/memory.db`.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use super::schema::{APPLICATION_ID, MEMORY_OBJECTS, MEMORY_SCHEMA_VERSION, migrate};

/// Why a hall memory database could not be opened or inspected.
#[derive(Debug, thiserror::Error)]
pub enum MemoryDbError {
    /// The underlying `SQLite` call failed.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// The file was written by a newer ivar; it is left untouched.
    #[error("memory database schema {found} is newer than this ivar supports ({supported})")]
    NewerSchema {
        /// Schema version stored in the file.
        found: i64,
        /// Newest schema version this binary knows.
        supported: i64,
    },
    /// The file is not a hall memory database (for example the legacy graph).
    #[error("not a hall memory database (application_id {found:#x})")]
    ForeignApplication {
        /// `PRAGMA application_id` found in the file.
        found: i32,
    },
}

/// A read-only look at a memory database's header and schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInspection {
    /// `PRAGMA application_id` of the file.
    pub application_id: i32,
    /// Schema version recorded in `memory_schema`.
    pub schema_version: i64,
    /// Whether the file holds objects the memory schema does not own.
    pub polluted: bool,
}

/// An open, migrated hall memory database.
#[derive(Debug)]
pub struct MemoryDb {
    conn: Connection,
}

impl MemoryDb {
    /// Opens or creates the memory database at `path`, migrates it to
    /// [`MEMORY_SCHEMA_VERSION`] and drops objects an older ivar left in it.
    ///
    /// # Errors
    ///
    /// [`MemoryDbError::ForeignApplication`] when the file belongs to another
    /// program, [`MemoryDbError::NewerSchema`] when a newer ivar wrote it, and
    /// [`MemoryDbError::Sqlite`] when `SQLite` fails.
    pub fn open(path: &Path) -> Result<Self, MemoryDbError> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        // Checked before any pragma, so a file that is not ours (the legacy
        // graph) is never switched to WAL or otherwise written.
        let found = application_id(&conn)?;
        let fresh = found == 0 && foreign_objects(&conn)?.is_empty();
        if found != APPLICATION_ID && !fresh {
            return Err(MemoryDbError::ForeignApplication { found });
        }
        crate::store::graph::schema::apply_pragmas(&conn, true)?;
        // Dropping graph tables that reference each other fails under
        // enforced foreign keys, and the pragma is a no-op inside a transaction.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = Transaction::new(&mut conn, TransactionBehavior::Immediate)?;
        let version = if fresh {
            tx.execute_batch(&format!("PRAGMA application_id = {APPLICATION_ID};"))?;
            0
        } else {
            let version = stored_version(&tx)?;
            if version > MEMORY_SCHEMA_VERSION {
                return Err(MemoryDbError::NewerSchema {
                    found: version,
                    supported: MEMORY_SCHEMA_VERSION,
                });
            }
            drop_foreign_objects(&tx)?;
            version
        };
        migrate(&tx, version)?;
        tx.commit()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    /// Reads the header and schema of the memory database at `path` without
    /// writing to it.
    ///
    /// # Errors
    ///
    /// [`MemoryDbError::ForeignApplication`] when the file is not a memory
    /// database, and [`MemoryDbError::Sqlite`] when `SQLite` fails.
    pub fn inspect(path: &Path) -> Result<MemoryInspection, MemoryDbError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let application_id = application_id(&conn)?;
        if application_id != APPLICATION_ID {
            return Err(MemoryDbError::ForeignApplication {
                found: application_id,
            });
        }
        Ok(MemoryInspection {
            application_id,
            schema_version: stored_version(&conn)?,
            polluted: !foreign_objects(&conn)?.is_empty(),
        })
    }

    /// The schema version recorded in `memory_schema`.
    ///
    /// # Errors
    ///
    /// [`MemoryDbError::Sqlite`] when the query fails.
    pub fn schema_version(&self) -> Result<i64, MemoryDbError> {
        Ok(stored_version(&self.conn)?)
    }

    /// Closes the connection, reporting the error `Drop` would swallow.
    ///
    /// # Errors
    ///
    /// [`MemoryDbError::Sqlite`] when `SQLite` cannot close the database.
    pub(crate) fn close(self) -> Result<(), MemoryDbError> {
        self.conn.close().map_err(|(_, error)| error.into())
    }

    /// The underlying connection.
    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

fn application_id(conn: &Connection) -> rusqlite::Result<i32> {
    conn.query_row("PRAGMA application_id", [], |row| row.get(0))
}

/// `memory_schema.version`, falling back to `user_version` only while the
/// table does not exist yet.
fn stored_version(conn: &Connection) -> rusqlite::Result<i64> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'memory_schema')",
        [],
        |row| row.get(0),
    )?;
    if has_table {
        conn.query_row("SELECT MAX(version) FROM memory_schema", [], |row| {
            row.get(0)
        })
    } else {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
    }
}

/// `(type, name, is_virtual)` of every object this schema does not own.
fn foreign_objects(conn: &Connection) -> rusqlite::Result<Vec<(String, String, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT type, name, COALESCE(sql, '') LIKE 'CREATE VIRTUAL TABLE%'
           FROM sqlite_master
          WHERE name NOT LIKE 'sqlite_%'",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let mut foreign = Vec::new();
    for row in rows {
        let (kind, name, is_virtual): (String, String, bool) = row?;
        if !MEMORY_OBJECTS.contains(&name.as_str()) {
            foreign.push((kind, name, is_virtual));
        }
    }
    Ok(foreign)
}

/// Virtual tables go first: dropping one drops its shadow tables, which a
/// plain `DROP TABLE` on a shadow table refuses. The list is re-read
/// afterwards because those shadows (and every index or trigger of a dropped
/// table) are already gone.
fn drop_foreign_objects(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    for (_, name, _) in foreign_objects(tx)?.into_iter().filter(|(_, _, v)| *v) {
        tx.execute_batch(&format!("DROP TABLE IF EXISTS {};", quote(&name)))?;
    }
    for kind in ["trigger", "view", "table", "index"] {
        for (_, name, _) in foreign_objects(tx)?
            .into_iter()
            .filter(|(k, _, _)| k == kind)
        {
            tx.execute_batch(&format!(
                "DROP {} IF EXISTS {};",
                kind.to_uppercase(),
                quote(&name)
            ))?;
        }
    }
    Ok(())
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/db.rs"]
mod tests;
