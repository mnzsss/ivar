//! `ivar memory clean`: the one verb that deletes `.ivar/memory.db`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

#[derive(Debug, Clone, Serialize)]
pub struct MemoryCleanOutcome {
    pub path: Utf8PathBuf,
    pub removed: Vec<Utf8PathBuf>,
}

/// A pre-split hall is moved first, so the `memory.db` deleted here is never
/// the old graph database.
///
/// # Errors
///
/// `memory.clean_not_confirmed` unless a human answered yes; otherwise when
/// the split, the lock or a removal fails.
pub fn clean(ctx: &Ctx) -> Outcome<MemoryCleanOutcome> {
    let layout = discover_hall(ctx)?;
    crate::action::graph::graph_db_path(&layout)?;
    let path = layout.memory_db();
    if !ctx.confirm(
        &format!("Delete `{path}`?"),
        Some("Every observation in it is lost; the graph database is not touched."),
    )? {
        return Err(Failure::blocked(
            "memory.clean_not_confirmed",
            format!("`{path}` was not deleted: the deletion was not confirmed"),
        )
        .fix(FixAction::safe(
            "memory.clean_in_terminal",
            "Run `ivar memory clean` in a terminal, without `--json`, and answer `y`.",
        )));
    }
    let _lock = fs::lock_exclusive(&layout.memory_lock())?;
    let mut removed = Vec::new();
    for file in [
        path.clone(),
        Utf8PathBuf::from(format!("{path}-wal")),
        Utf8PathBuf::from(format!("{path}-shm")),
    ] {
        if fs::exists(&file)? {
            fs::remove_file(&file)?;
            removed.push(file);
        }
    }
    Ok(Report::new(MemoryCleanOutcome { path, removed }))
}

impl WriteHuman for MemoryCleanOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.removed.is_empty() {
            return writeln!(w, "Nothing to delete: `{}` does not exist.", self.path);
        }
        for file in &self.removed {
            writeln!(w, "Deleted {file}")?;
        }
        Ok(())
    }
}
