//! `ivar memory reindex` — rebuild or reconcile the SQLite FTS5 index.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::memory::query::ReconcileSummary;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::memory::MemoryIndex;

/// Input for the `ivar memory reindex` command.
#[derive(Debug, Clone, Default)]
pub struct MemoryReindexInput {
    /// Force a complete rebuild of the database.
    pub force: bool,
}

/// Outcome of the `ivar memory reindex` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryReindexOutcome {
    /// Summary of the reconciliation or rebuild.
    pub summary: ReconcileSummary,
}

impl WriteHuman for MemoryReindexOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Reindexed memory: {} indexed, {} updated, {} removed.",
            self.summary.indexed, self.summary.updated, self.summary.removed
        )
    }
}

/// Reindex the memory store.
pub fn reindex(ctx: &Ctx, input: MemoryReindexInput) -> Outcome<MemoryReindexOutcome> {
    let layout = discover_hall(ctx)?;
    let index = MemoryIndex::open(&layout)?;

    let summary = if input.force {
        index.rebuild(&layout)?
    } else {
        index.reconcile(&layout)?
    };

    Ok(Report::new(MemoryReindexOutcome { summary }))
}
