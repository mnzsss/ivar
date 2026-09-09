use crate::action::feature::workspace::OpenAttempt;
use crate::action::graph::view::types::ViewSeed;

use serde::Serialize;
use std::io;
use std::path::PathBuf;

use crate::action::graph::compact::{self, ToCompact};
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
            writeln!(w, "  Primary Implementation & Source:")?;
            for sym in &res.primary_symbols {
                writeln!(
                    w,
                    "    - {} [{}] ({}) in {} ({}:{}-{})",
                    sym.symbol.name,
                    sym.symbol.repo,
                    sym.symbol.kind,
                    sym.file_path,
                    sym.file_path,
                    sym.start_line,
                    sym.end_line
                )?;
                if let Some(sig) = &sym.symbol.signature {
                    writeln!(w, "      Signature: {}", sig)?;
                }
                if !sym.code.is_empty() {
                    writeln!(w, "      Source:")?;
                    for line in sym.code.lines() {
                        writeln!(w, "        {}", line)?;
                    }
                }
            }
        }
        if !res.entry_points.is_empty() {
            writeln!(w, "  Entry Points:")?;
            for ep in &res.entry_points {
                let cross_str = if ep.cross_repo { " [cross-repo]" } else { "" };
                writeln!(
                    w,
                    "    - {} [{}:{}:{}] -> {} ({:?}, {:?}, {:.0}% conf{})",
                    ep.source.symbol_name,
                    ep.source.repo,
                    ep.source.file_path,
                    ep.line,
                    ep.target.symbol_name,
                    ep.edge_kind,
                    ep.provenance,
                    ep.confidence * 100.0,
                    cross_str
                )?;
            }
        }
        if !res.direct_relations.is_empty() {
            writeln!(w, "  Direct Relations:")?;
            for rel in &res.direct_relations {
                let cross_str = if rel.cross_repo { " [cross-repo]" } else { "" };
                let dir_arrow = match rel.direction {
                    crate::domain::graph::RelationDirection::Incoming => "<-",
                    crate::domain::graph::RelationDirection::Outgoing => "->",
                };
                writeln!(
                    w,
                    "    - {} [{}:{}] {} {} [{}:{}] ({:?}, {:?}, {:.0}% conf, line {}{})",
                    rel.source.symbol_name,
                    rel.source.repo,
                    rel.source.file_path,
                    dir_arrow,
                    rel.target.symbol_name,
                    rel.target.repo,
                    rel.target.file_path,
                    rel.edge_kind,
                    rel.provenance,
                    rel.confidence * 100.0,
                    rel.line,
                    cross_str
                )?;
            }
        } else if !res.call_flows.is_empty() {
            writeln!(w, "  Call Flows:")?;
            for flow in &res.call_flows {
                writeln!(
                    w,
                    "    - {} -> {} ({:?}, line {})",
                    flow.caller, flow.callee, flow.edge_kind, flow.line
                )?;
            }
        }
        if !res.transitive_consumers.is_empty() {
            writeln!(w, "  Transitive Consumers:")?;
            for c in &res.transitive_consumers {
                let cross_str = if c.cross_repo { " [cross-repo]" } else { "" };
                let via = if c.path_via.is_empty() {
                    String::new()
                } else {
                    format!(" via {}", c.path_via.join(" -> "))
                };
                writeln!(
                    w,
                    "    - {} [{}:{}] (depth {}{}{})",
                    c.symbol_name, c.repo, c.file_path, c.depth, via, cross_str
                )?;
            }
        }
        if let Some(impact) = &res.impact_summary {
            writeln!(w, "  Impact Summary: {}", impact)?;
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
        if !res.recommendations.is_empty() {
            writeln!(
                w,
                "\nVerification recommendations ({}):",
                res.recommendations.len()
            )?;
            for rec in &res.recommendations {
                writeln!(w, "  • {} ({})", rec.test_file, rec.repo)?;
                writeln!(w, "    Reason: {}", rec.reason)?;
                if let Some(cmd) = &rec.command {
                    writeln!(w, "    Command: {}", cmd)?;
                }
                if !rec.causal_path.is_empty() {
                    let path_desc: Vec<String> = rec
                        .causal_path
                        .iter()
                        .map(|step| {
                            format!(
                                "{} --[{}]--> {}",
                                step.source,
                                step.edge_kind.as_str(),
                                step.target
                            )
                        })
                        .collect();
                    writeln!(w, "    Path: {}", path_desc.join(" -> "))?;
                }
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

#[derive(Debug, Clone, Serialize)]
pub struct VizOutcome {
    pub output_path: PathBuf,
    pub node_count: usize,
    pub edge_count: usize,
}

impl WriteHuman for VizOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Generated standalone graph visualizer ({} nodes, {} edges) at {}",
            self.node_count,
            self.edge_count,
            self.output_path.display()
        )
    }
}

impl ToCompact for ExploreOutcome {
    fn to_compact(&self) -> String {
        compact::encode_explore(&self.0)
    }
}

impl ToCompact for AffectedOutcome {
    fn to_compact(&self) -> String {
        compact::encode_affected(&self.0)
    }
}

impl ToCompact for PathOutcome {
    fn to_compact(&self) -> String {
        compact::encode_path(self.0.as_ref())
    }
}

impl ToCompact for IndexBatchOutcome {
    fn to_compact(&self) -> String {
        let mut out =
            String::from("#SCHEMA: repo|files_indexed|symbols_indexed|edges_indexed|duration_ms");
        for outcome in &self.repos {
            out.push('\n');
            out.push_str(&format!(
                "{}|{}|{}|{}|{}",
                outcome.repo,
                outcome.files_indexed,
                outcome.symbols_indexed,
                outcome.edges_indexed,
                outcome.duration_ms
            ));
        }
        out
    }
}

impl ToCompact for McpOutcome {
    fn to_compact(&self) -> String {
        "#SCHEMA: status\nrunning".to_owned()
    }
}

impl ToCompact for VizOutcome {
    fn to_compact(&self) -> String {
        format!(
            "#SCHEMA: output_path|node_count|edge_count\n{}|{}|{}",
            self.output_path.display(),
            self.node_count,
            self.edge_count
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphViewOutcome {
    pub url: String,
    pub seed: ViewSeed,
    #[serde(skip)]
    pub open: OpenAttempt,
}

impl WriteHuman for GraphViewOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Serving graph viewer at {}", self.url)?;
        match &self.open {
            OpenAttempt::NotRequested => {}
            OpenAttempt::Opened => writeln!(w, "Opened in default browser.")?,
            OpenAttempt::Failed { reason } => {
                writeln!(w, "Warning: could not open browser automatically: {reason}")?;
            }
        }
        writeln!(w, "Press Ctrl+C to stop.")
    }
}

impl ToCompact for GraphViewOutcome {
    fn to_compact(&self) -> String {
        format!("url={}", self.url)
    }
}
