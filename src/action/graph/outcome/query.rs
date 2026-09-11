use serde::Serialize;
use std::io;

use crate::action::graph::compact::{self, ToCompact};
use crate::action::graph::query;
use crate::domain::graph::{ComplexityItem, DeadCodeItem, GraphStats, HierarchyItem};
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
        if !s.layers.is_empty() {
            writeln!(w, "\nFeature Layers ({}):", s.layers.len())?;
            for layer in &s.layers {
                writeln!(
                    w,
                    "  - [{}] repo: {}, files: {}, base: {}",
                    layer.feature, layer.repo, layer.file_count, layer.base_commit
                )?;
            }
        }
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

#[derive(Debug, Clone, Serialize)]
pub struct DeadCodeOutcome(pub Vec<DeadCodeItem>);

impl WriteHuman for DeadCodeOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Dead Code Analysis ({} unreferenced private symbols):",
            self.0.len()
        )?;
        for item in &self.0 {
            writeln!(
                w,
                "  - {} ({:?}) in {}:{}",
                item.symbol.name, item.symbol.kind, item.file_path, item.line
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComplexityOutcome(pub Vec<ComplexityItem>);

impl WriteHuman for ComplexityOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Cyclomatic Complexity ({} symbols above threshold):",
            self.0.len()
        )?;
        for item in &self.0 {
            writeln!(
                w,
                "  - [complexity {}] {} ({:?}) in {}:{}",
                item.complexity, item.symbol.name, item.symbol.kind, item.file_path, item.line
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HierarchyOutcome(pub Option<HierarchyItem>);

impl WriteHuman for HierarchyOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match &self.0 {
            Some(item) => {
                writeln!(
                    w,
                    "Hierarchy for symbol `{}` ({:?}) in {}:",
                    item.symbol.name, item.symbol.kind, item.file_path
                )?;
                let bases = if item.bases.is_empty() {
                    "(none)".to_owned()
                } else {
                    item.bases.join(", ")
                };
                let impls = if item.implementations.is_empty() {
                    "(none)".to_owned()
                } else {
                    item.implementations.join(", ")
                };
                writeln!(w, "  Bases: {}", bases)?;
                writeln!(w, "  Implementations/Subtypes: {}", impls)?;
            }
            None => {
                writeln!(w, "No hierarchy found for specified symbol.")?;
            }
        }
        Ok(())
    }
}

impl ToCompact for FindOutcome {
    fn to_compact(&self) -> String {
        compact::encode_symbols(&self.symbols)
    }
}

impl ToCompact for CallersOutcome {
    fn to_compact(&self) -> String {
        compact::encode_callers(&self.callers)
    }
}

impl ToCompact for CalleesOutcome {
    fn to_compact(&self) -> String {
        compact::encode_callees(&self.callees)
    }
}

impl ToCompact for FileOutcome {
    fn to_compact(&self) -> String {
        let mut out = String::from(compact::SYMBOL_SCHEMA);
        for sym in &self.0.symbols {
            let id_str = sym.id.map_or_else(String::new, |id| id.to_string());
            let kind = crate::store::graph::db::symbol_kind_to_str(&sym.kind);
            let complexity_str = sym.complexity.map_or_else(String::new, |c| c.to_string());
            out.push('\n');
            out.push_str(&format!(
                "{}|{}|{}|{}|{}|{}|{}",
                id_str,
                sym.name,
                kind,
                self.0.file_path,
                sym.span.start_line,
                sym.span.start_col,
                complexity_str
            ));
        }
        out
    }
}

impl ToCompact for StatsOutcome {
    fn to_compact(&self) -> String {
        format!(
            "#SCHEMA: repos|files|symbols|edges|db_size_bytes\n{}|{}|{}|{}|{}",
            self.0.repo_count,
            self.0.file_count,
            self.0.symbol_count,
            self.0.edge_count,
            self.0.db_size_bytes
        )
    }
}

impl ToCompact for ImpactOutcome {
    fn to_compact(&self) -> String {
        compact::encode_impact(&self.0)
    }
}

impl ToCompact for DeadCodeOutcome {
    fn to_compact(&self) -> String {
        compact::encode_dead_code(&self.0)
    }
}

impl ToCompact for ComplexityOutcome {
    fn to_compact(&self) -> String {
        compact::encode_complexity(&self.0)
    }
}

impl ToCompact for HierarchyOutcome {
    fn to_compact(&self) -> String {
        compact::encode_hierarchy(self.0.as_ref())
    }
}
