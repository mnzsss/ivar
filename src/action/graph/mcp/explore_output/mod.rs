//! Bounded MCP output rendering for graph exploration.
//!
//! Enforces exact byte limits (18,000 bytes for ordinary queries, 24,000 bytes for explicit
//! file requests) on structured MCP responses (`compact` and `json`), outputting complete
//! responses when within budget or structurally valid partial responses with accounted
//! omission metadata and actionable continuation calls when the budget is exceeded.

mod compact;
mod json;
mod next;

use crate::domain::graph::ExploreResult;
use compact::partial_compact_output;
use json::partial_json_output;

pub(super) const ORDINARY_BYTES: usize = 18_000;
pub(super) const REQUESTED_BYTES: usize = 24_000;

const PARTIAL_SCHEMA: &str = "#SCHEMA: partial|budget_bytes";
const OMITTED_SCHEMA: &str = "#SCHEMA: omitted_category|omitted_records";
const NEXT_SCHEMA: &str = "#SCHEMA: next_tool|next_arguments";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExploreFormat {
    Compact,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    FileMatches,
    PrimarySymbols,
    Flows,
    Sources,
    CallFlows,
    EntryPoints,
    DirectRelations,
    TransitiveConsumers,
    NotShown,
}

impl Section {
    const fn json_key(self) -> &'static str {
        match self {
            Self::FileMatches => "file_matches",
            Self::PrimarySymbols => "primary_symbols",
            Self::Flows => "flows",
            Self::Sources => "sources",
            Self::CallFlows => "call_flows",
            Self::EntryPoints => "entry_points",
            Self::DirectRelations => "direct_relations",
            Self::TransitiveConsumers => "transitive_consumers",
            Self::NotShown => "not_shown",
        }
    }
}

const JSON_PRIORITY: [Section; 9] = [
    Section::FileMatches,
    Section::PrimarySymbols,
    Section::Flows,
    Section::Sources,
    Section::CallFlows,
    Section::EntryPoints,
    Section::DirectRelations,
    Section::TransitiveConsumers,
    Section::NotShown,
];

const COMPACT_PRIORITY: [Section; 4] = [
    Section::FileMatches,
    Section::PrimarySymbols,
    Section::DirectRelations,
    Section::TransitiveConsumers,
];

#[derive(serde::Serialize)]
struct NextCall {
    tool: &'static str,
    arguments: serde_json::Value,
}

struct BudgetBuffer {
    bytes: Vec<u8>,
    limit: usize,
    overflowed: bool,
}

impl BudgetBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(4096)),
            limit,
            overflowed: false,
        }
    }

    fn reset(&mut self, limit: usize) {
        self.bytes.clear();
        self.limit = limit;
        self.overflowed = false;
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.overflowed = true;
            return Err(());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

impl std::fmt::Write for BudgetBuffer {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.append(text.as_bytes()).map_err(|()| std::fmt::Error)
    }
}

impl std::io::Write for BudgetBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.append(bytes).map_err(|()| {
            std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "graph response byte budget exceeded",
            )
        })?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn render_explore(
    result: &ExploreResult,
    format: ExploreFormat,
    requested_files: bool,
) -> Result<String, String> {
    let limit = if requested_files {
        REQUESTED_BYTES
    } else {
        ORDINARY_BYTES
    };
    if let Some(text) = complete_output(result, format, limit)? {
        return Ok(text);
    }
    partial_output(result, format, limit)
}

fn complete_output(
    result: &ExploreResult,
    format: ExploreFormat,
    limit: usize,
) -> Result<Option<String>, String> {
    let mut buffer = BudgetBuffer::new(limit);
    let emitted = match format {
        ExploreFormat::Compact => crate::action::graph::compact::write_explore(result, &mut buffer)
            .map_err(|e| e.to_string()),
        ExploreFormat::Json => {
            serde_json::to_writer_pretty(&mut buffer, result).map_err(|e| e.to_string())
        }
    };
    match emitted {
        Err(_) if buffer.overflowed => Ok(None),
        Err(error) => Err(error),
        Ok(()) => String::from_utf8(buffer.bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
    }
}

fn section_item_count(result: &ExploreResult, sec: Section) -> usize {
    match sec {
        Section::FileMatches => result.file_matches.len(),
        Section::PrimarySymbols => result.primary_symbols.len(),
        Section::Flows => result.flows.len(),
        Section::Sources => result.sources.len(),
        Section::CallFlows => result.call_flows.len(),
        Section::EntryPoints => result.entry_points.len(),
        Section::DirectRelations => result.direct_relations.len(),
        Section::TransitiveConsumers => result.transitive_consumers.len(),
        Section::NotShown => result.not_shown.len(),
    }
}

fn partial_output(
    result: &ExploreResult,
    format: ExploreFormat,
    limit: usize,
) -> Result<String, String> {
    match format {
        ExploreFormat::Compact => partial_compact_output(result, limit),
        ExploreFormat::Json => partial_json_output(result, limit),
    }
}

struct EncodedRow {
    section: Section,
    index: usize,
    start: usize,
    len: usize,
}

#[cfg(test)]
#[path = "../../../../../tests/unit/action/graph/mcp/explore_output.rs"]
mod tests;
