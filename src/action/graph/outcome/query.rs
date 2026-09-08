//! Outcomes and human-readable formatters for query graph subcommands.

use std::io;
use serde::Serialize;

use crate::action::graph::query;
use crate::domain::graph::GraphStats;
use crate::error::WriteHuman;

#[derive(Debug, Clone, Serialize)]
pub struct FindOutcome {
    pub query: String,
    pub symbols: Vec<query::SymbolLocation>,
}

impl WriteHuman for FindOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Symbols matching `{}` ({} found):",
            self.query,
            self.symbols.len()
        )?;
        for sym in &self.symbols {
            let id_str = sym
                .symbol
                .id
                .map(|id| format!("[#{id}] "))
                .unwrap_or_default();
            writeln!(
                w,
                "  - {}{}:{:?} in {}/{} ({}:{})",
                id_str,
                sym.symbol.name,
                sym.symbol.kind,
                sym.symbol.repo,
                sym.file_path,
                sym.symbol.span.start_line,
                sym.symbol.span.start_col
            )?;
            if let Some(sig) = &sym.symbol.signature {
                writeln!(w, "      Signature: {}", sig)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CallersOutcome {
    pub symbol: String,
    pub callers: Vec<query::CallerInfo>,
}

impl WriteHuman for CallersOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Callers of `{}` ({} found):",
            self.symbol,
            self.callers.len()
        )?;
        for caller in &self.callers {
            writeln!(
                w,
                "  - {} ({:?}) in {}/{} (line {}, confidence: {:.2}, prov: {:?})",
                caller.caller.name,
                caller.caller.kind,
                caller.caller.repo,
                caller.caller_file_path,
                caller.line,
                caller.confidence,
                caller.provenance
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CalleesOutcome {
    pub symbol_id: i64,
    pub callees: Vec<query::CalleeInfo>,
}

impl WriteHuman for CalleesOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Callees of symbol ID {} ({} found):",
            self.symbol_id,
            self.callees.len()
        )?;
        for callee in &self.callees {
            let target_name = callee
                .callee_symbol
                .as_ref()
                .map(|c| c.name.as_str())
                .unwrap_or(&callee.callee_name);
            let target_kind = callee
                .callee_symbol
                .as_ref()
                .map(|c| format!(" ({:?})", c.kind))
                .unwrap_or_default();
            writeln!(
                w,
                "  - {}{} (line {}, kind: {:?}, prov: {:?})",
                target_name, target_kind, callee.line, callee.edge_kind, callee.provenance
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileOutcome(pub query::FileOutline);

impl WriteHuman for FileOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let outline = &self.0;
        writeln!(
            w,
            "File outline for {}/{} ({} symbols):",
            outline.repo,
            outline.file_path,
            outline.symbols.len()
        )?;
        for sym in &outline.symbols {
            let id_str = sym.id.map(|id| format!("[#{id}] ")).unwrap_or_default();
            let exp = if sym.is_exported { " [pub]" } else { "" };
            writeln!(
                w,
                "  - {}{}: {:?}{} ({}:{})",
                id_str, sym.name, sym.kind, exp, sym.span.start_line, sym.span.start_col
            )?;
            if let Some(sig) = &sym.signature {
                writeln!(w, "      Signature: {}", sig)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsOutcome(pub GraphStats);

impl WriteHuman for StatsOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let s = &self.0;
        writeln!(w, "Codebase Graph Statistics:")?;
        writeln!(w, "  Repositories: {}", s.repo_count)?;
        writeln!(w, "  Files:        {}", s.file_count)?;
        writeln!(w, "  Symbols:      {}", s.symbol_count)?;
        writeln!(w, "  Edges:        {}", s.edge_count)?;
        writeln!(w, "  DB Size:      {} bytes", s.db_size_bytes)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImpactOutcome(pub query::ImpactResult);

impl WriteHuman for ImpactOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let res = &self.0;
        writeln!(
            w,
            "Blast Radius Impact Analysis for symbol `{}` (ID: {}):",
            res.root_symbol.name,
            res.root_symbol.id.unwrap_or(0)
        )?;
        writeln!(w, "  Total affected symbols: {}", res.total_affected)?;
        for item in &res.affected_symbols {
            writeln!(
                w,
                "  - [depth {}] {} ({:?}) in {}/{} (via {:?})",
                item.depth,
                item.symbol.name,
                item.symbol.kind,
                item.symbol.repo,
                item.file_path,
                item.path_via
            )?;
        }
        Ok(())
    }
}
