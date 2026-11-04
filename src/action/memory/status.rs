//! `ivar memory status`: where the memory database is and what it carries.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::memory::db::MemoryDb;

#[derive(Debug, Clone, Serialize)]
pub struct MemoryStatusOutcome {
    pub path: Utf8PathBuf,
    pub exists: bool,
    pub schema_version: Option<i64>,
    pub application_id: Option<i32>,
    pub size_bytes: Option<u64>,
}

/// Moves a pre-split hall first, so the file reported is always the memory
/// database and never the old graph, then opens it for writing, which
/// repairs a memory database an older ivar polluted.
///
/// # Errors
///
/// Outside a hall, while the split is busy, or when the file cannot be read.
pub fn status(ctx: &Ctx) -> Outcome<MemoryStatusOutcome> {
    let layout = discover_hall(ctx)?;
    crate::action::graph::graph_db_path(&layout)?;
    let path = layout.memory_db();
    if fs::stat(&path)?.is_none() {
        return Ok(Report::new(MemoryStatusOutcome {
            path,
            exists: false,
            schema_version: None,
            application_id: None,
            size_bytes: None,
        }));
    }
    drop(MemoryDb::open(path.as_std_path()).map_err(|error| {
        Failure::failed(
            "memory.open_failed",
            format!("could not open `{path}`: {error}"),
        )
    })?);
    let size_bytes = fs::stat(&path)?.map(|metadata| metadata.len());
    let inspection = MemoryDb::inspect(path.as_std_path()).map_err(|error| {
        Failure::failed(
            "memory.inspect_failed",
            format!("could not read `{path}`: {error}"),
        )
    })?;
    Ok(Report::new(MemoryStatusOutcome {
        path,
        exists: true,
        schema_version: Some(inspection.schema_version),
        application_id: Some(inspection.application_id),
        size_bytes,
    }))
}

impl WriteHuman for MemoryStatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Memory database: {}", self.path)?;
        if !self.exists {
            return writeln!(w, "  not created yet");
        }
        let or_unknown = |value: Option<String>| value.unwrap_or_else(|| "unknown".to_owned());
        writeln!(
            w,
            "  schema version: {}",
            or_unknown(self.schema_version.map(|v| v.to_string()))
        )?;
        writeln!(
            w,
            "  application id: {}",
            or_unknown(self.application_id.map(|v| format!("{v:#x}")))
        )?;
        writeln!(
            w,
            "  size:           {} bytes",
            or_unknown(self.size_bytes.map(|v| v.to_string()))
        )
    }
}
