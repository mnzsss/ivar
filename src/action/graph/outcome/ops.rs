//! Outcomes and human-readable formatters for operation and traversal graph subcommands.

use serde::Serialize;
use std::io;

use crate::action::graph::index::IndexOutcome;
use crate::domain::graph::{AffectedResult, ExploreResult, PathResult};
use crate::error::WriteHuman;

#[derive(Debug, Clone, Serialize)]
pub struct ExploreOutcome(pub ExploreResult);

impl WriteHuman for ExploreOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let res = &self.0;
        writeln!(w, "Explore results for query `{}`:", res.query)?;
        if res.primary_symbols.is_empty() {
            writeln!(w, "  No symbols found matching query.")?;
        } else {
            writeln!(w, "  Primary Symbols:")?;
            for sym in &res.primary_symbols {
                writeln!(
                    w,
                    "    - {} ({:?}) in {}:{}-{}",
                    sym.symbol.name, sym.symbol.kind, sym.file_path, sym.start_line, sym.end_line
                )?;
                if let Some(sig) = &sym.symbol.signature {
                    writeln!(w, "      Signature: {}", sig)?;
                }
            }
        }
        if !res.call_flows.is_empty() {
            writeln!(w, "  Call Flows:")?;
            for flow in &res.call_flows {
                writeln!(
                    w,
                    "    - {} -> {} ({:?}, line {})",
                    flow.caller, flow.callee, flow.edge_kind, flow.line
                )?;
            }
        }
        if let Some(impact) = &res.impact_summary {
            writeln!(w, "  Impact: {}", impact)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AffectedOutcome(pub AffectedResult);

impl WriteHuman for AffectedOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let res = &self.0;
        writeln!(w, "Changed files ({}):", res.changed_files.len())?;
        for file in &res.changed_files {
            writeln!(w, "  - {}", file)?;
        }
        writeln!(
            w,
            "Affected test files ({}):",
            res.affected_test_files.len()
        )?;
        if res.affected_test_files.is_empty() {
            writeln!(w, "  No affected test files found.")?;
        } else {
            for test in &res.affected_test_files {
                writeln!(w, "  - {}", test)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PathOutcome(pub Option<PathResult>);

impl WriteHuman for PathOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if let Some(res) = &self.0 {
            writeln!(w, "Path from `{}` to `{}`:", res.from, res.to)?;
            if res.steps.is_empty() {
                writeln!(w, "  No path found.")?;
            } else {
                for (idx, step) in res.steps.iter().enumerate() {
                    writeln!(
                        w,
                        "  {}. {} -> {} ({:?})",
                        idx + 1,
                        step.source,
                        step.target,
                        step.edge_kind
                    )?;
                }
            }
        } else {
            writeln!(w, "No path found.")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexBatchOutcome {
    pub repos: Vec<IndexOutcome>,
    pub cross_edges_linked: usize,
}

impl WriteHuman for IndexBatchOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Codebase Graph Indexing Complete:")?;
        for outcome in &self.repos {
            if outcome.skipped_up_to_date {
                writeln!(
                    w,
                    "  - {}: up to date (took {}ms)",
                    outcome.repo, outcome.duration_ms
                )?;
            } else {
                writeln!(
                    w,
                    "  - {}: {} files indexed, {} deleted, {} symbols, {} edges (took {}ms)",
                    outcome.repo,
                    outcome.files_indexed,
                    outcome.files_deleted,
                    outcome.symbols_indexed,
                    outcome.edges_indexed,
                    outcome.duration_ms
                )?;
            }
        }
        if self.cross_edges_linked > 0 {
            writeln!(w, "  Linked {} cross-repo edges.", self.cross_edges_linked)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpOutcome;

impl WriteHuman for McpOutcome {
    fn write_human(&self, _w: &mut impl io::Write) -> io::Result<()> {
        Ok(())
    }
}
