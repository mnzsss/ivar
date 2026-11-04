//! The two hall databases: `graph.db` (derived) and `memory.db` (primary).
//!
//! Read-only except for one write: a legacy `memory.db` that is really the
//! graph is moved to `graph.db` here, because a doctor that waited for
//! another command would report the legacy file as corrupt.

use std::path::Path;

use camino::Utf8Path;

use crate::action::graph::mcp_registration::runs_graph_mcp;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::schema::MEMORY_SCHEMA_VERSION;
use crate::store::memory::split::{SplitError, ensure_split, probe_legacy_graph_db};

use super::Diagnosis;

pub(super) fn database_diagnoses(layout: &Layout, manifest: &Manifest) -> Vec<Diagnosis> {
    let graph_db = layout.graph_db();
    let memory_db = layout.memory_db();

    if !graph_db.exists()
        && memory_db.exists()
        && !matches!(probe_legacy_graph_db(memory_db.as_std_path()), Ok(false))
    {
        return match ensure_split(layout) {
            Ok(()) => Vec::new(),
            Err(SplitError::Busy) => vec![Diagnosis {
                code: "memory.split_busy",
                what: format!(
                    "`{memory_db}` still holds the graph and another process has it open, so it was not moved to `graph.db`"
                ),
                fix: "Stop sessions and MCP servers started by an older `ivar`, then run `ivar doctor` again."
                    .to_owned(),
            }],
            Err(error) => vec![Diagnosis {
                code: "memory.split_failed",
                what: format!(
                    "could not move the legacy graph `{memory_db}` to `graph.db`: {error}"
                ),
                fix: "Check that `.ivar/` is writable and has free space, then run `ivar doctor` again."
                    .to_owned(),
            }],
        };
    }

    let mut findings = Vec::new();
    if !graph_db.exists() && manifest.mcp_servers().iter().any(runs_graph_mcp) {
        findings.push(Diagnosis {
            code: "graph.db_missing",
            what: format!("the graph MCP server is registered but `{graph_db}` does not exist"),
            fix: "Run `ivar graph index` to rebuild it.".to_owned(),
        });
    }
    if memory_db.exists() {
        findings.extend(memory_db_diagnosis(&memory_db));
    } else if graph_db.exists() {
        findings.push(Diagnosis {
            code: "memory.db_missing",
            what: format!("`{memory_db}` does not exist next to `{graph_db}`"),
            fix: "Run `ivar memory status` to create it.".to_owned(),
        });
    }
    findings
}

fn memory_db_diagnosis(path: &Utf8Path) -> Option<Diagnosis> {
    let corrupt = |reason: String| Diagnosis {
        code: "memory.db_corrupt",
        what: format!("`{path}` is not a usable memory database: {reason}"),
        fix: "Run `ivar memory clean`, then `ivar memory status` to create it afresh.".to_owned(),
    };
    let inspection = match MemoryDb::inspect(path.as_std_path()) {
        Ok(inspection) => inspection,
        Err(error) => return Some(corrupt(error.to_string())),
    };
    match quick_check(path.as_std_path()) {
        Ok(verdict) if verdict == "ok" => {}
        Ok(verdict) => return Some(corrupt(verdict)),
        Err(error) => return Some(corrupt(error.to_string())),
    }
    if inspection.schema_version > MEMORY_SCHEMA_VERSION {
        return Some(Diagnosis {
            code: "memory.schema_newer",
            what: format!(
                "memory schema is version {}, this ivar knows up to {MEMORY_SCHEMA_VERSION}",
                inspection.schema_version
            ),
            fix: "Run `ivar upgrade` to a build that knows this schema.".to_owned(),
        });
    }
    if inspection.polluted {
        return Some(Diagnosis {
            code: "memory.db_polluted",
            what: format!("an older `ivar` created graph tables inside `{path}`"),
            fix: "Stop every process of an older `ivar`, then run `ivar memory status` with this build to repair it."
                .to_owned(),
        });
    }
    None
}

// Only memory.db gets a full quick_check: it is small and primary, while
// graph.db can be hundreds of MB and is rebuilt by `ivar graph index`.
fn quick_check(path: &Path) -> rusqlite::Result<String> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.query_row("PRAGMA quick_check", [], |row| row.get(0))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/hall/doctor/database.rs"]
mod tests;
