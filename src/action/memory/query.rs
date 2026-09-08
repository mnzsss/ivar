//! `ivar memory query` — query the memory index with optional scope filter and limit.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::memory::config::ScopeName;
use crate::domain::memory::query::{QueryFilter, QueryMatch};
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::memory::MemoryIndex;

/// Input for the `ivar memory query` command.
#[derive(Debug, Clone, Default)]
pub struct MemoryQueryInput {
    /// Query string.
    pub query: String,
    /// Optional scope name to restrict query to.
    pub scope: Option<String>,
    /// Optional max results limit (defaults to 20).
    pub limit: Option<usize>,
}

/// Outcome of the `ivar memory query` command.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemoryQueryOutcome {
    /// Search query that was executed.
    pub query: String,
    /// Matching results.
    pub matches: Vec<QueryMatch>,
}

impl WriteHuman for MemoryQueryOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.matches.is_empty() {
            writeln!(w, "No memory matches found for `{}`.", self.query)
        } else {
            writeln!(
                w,
                "Found {} match(es) for `{}`:",
                self.matches.len(),
                self.query
            )?;
            for m in &self.matches {
                writeln!(w, "  [{}] {} ({})", m.scope, m.title, m.slug)?;
                if !m.snippet.is_empty() {
                    writeln!(w, "    {}", m.snippet.trim())?;
                }
            }
            Ok(())
        }
    }
}

/// Query the memory store.
pub fn query(ctx: &Ctx, input: MemoryQueryInput) -> Outcome<MemoryQueryOutcome> {
    let layout = discover_hall(ctx)?;
    let index = MemoryIndex::open(&layout)?;

    // Reconcile index first before query
    let _ = index.reconcile(&layout)?;

    let scope_filter = if let Some(s) = input.scope {
        Some(ScopeName::new(s)?)
    } else {
        None
    };

    let filter = QueryFilter {
        scope: scope_filter,
        limit: input.limit.unwrap_or(20),
    };

    let matches = index.query(&input.query, &filter)?;

    Ok(Report::new(MemoryQueryOutcome {
        query: input.query,
        matches,
    }))
}
