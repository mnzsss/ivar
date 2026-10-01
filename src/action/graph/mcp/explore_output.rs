//! Bounded MCP output rendering for graph exploration.
//!
//! Enforces exact byte limits (18,000 bytes for ordinary queries, 24,000 bytes for explicit
//! file requests) on structured MCP responses (`compact` and `json`), outputting complete
//! responses when within budget or structurally valid partial responses with accounted
//! omission metadata and actionable continuation calls when the budget is exceeded.

use crate::action::graph::compact::{
    EXPLORE_IMPACT_SCHEMA, ExploreRecord, FILE_MATCH_SCHEMA, RELATION_SCHEMA, SYMBOL_SCHEMA,
};
use crate::domain::graph::ExploreResult;
use std::fmt::Write as _;

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

fn make_range_next_call(
    repo: &str,
    file_path: &str,
    start_line: usize,
    end_line: Option<usize>,
) -> NextCall {
    let start = if start_line == 0 { 1 } else { start_line };
    let end = end_line.map_or(start.saturating_add(39), |last| {
        start.saturating_add(39).min(last.max(start))
    });
    NextCall {
        tool: "graph_explore",
        arguments: serde_json::json!({
            "repo": repo,
            "paths": [format!("{file_path}:{start}-{end}")],
            "format": "markdown",
        }),
    }
}

fn make_query_next_call(query: &str) -> NextCall {
    NextCall {
        tool: "graph_explore",
        arguments: serde_json::json!({
            "query": query,
            "format": "markdown",
        }),
    }
}

fn next_for_file_match(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let fm = result.file_matches.get(index)?;
    Some(make_range_next_call(
        &fm.repo,
        &fm.file_path,
        fm.start_line,
        None,
    ))
}

fn next_for_primary_symbol(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let sym_snippet = result.primary_symbols.get(index)?;
    let start = if sym_snippet.start_line == 0 {
        if sym_snippet.symbol.span.start_line == 0 {
            1
        } else {
            sym_snippet.symbol.span.start_line
        }
    } else {
        sym_snippet.start_line
    };
    let known_last = if sym_snippet.end_line > 0 {
        Some(sym_snippet.end_line)
    } else if sym_snippet.symbol.span.end_line > 0 {
        Some(sym_snippet.symbol.span.end_line)
    } else {
        None
    };
    Some(make_range_next_call(
        &sym_snippet.symbol.repo,
        &sym_snippet.file_path,
        start,
        known_last,
    ))
}

fn next_for_flow(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let path_res = result.flows.get(index)?;
    for step in &path_res.steps {
        for sym in &result.primary_symbols {
            if (sym.symbol.name == step.source || sym.symbol.name == step.target)
                && !sym.file_path.is_empty()
            {
                let line = if step.source == sym.symbol.name && step.line > 0 {
                    step.line
                } else if sym.start_line > 0 {
                    sym.start_line
                } else {
                    sym.symbol.span.start_line
                };
                return Some(make_range_next_call(
                    &sym.symbol.repo,
                    &sym.file_path,
                    line,
                    None,
                ));
            }
        }
        for rel in &result.direct_relations {
            if rel.source.symbol_name == step.source && !rel.source.file_path.is_empty() {
                let line = if step.line > 0 { step.line } else { rel.line };
                return Some(make_range_next_call(
                    &rel.source.repo,
                    &rel.source.file_path,
                    line,
                    None,
                ));
            }
            if rel.target.symbol_name == step.target && !rel.target.file_path.is_empty() {
                return Some(make_range_next_call(
                    &rel.target.repo,
                    &rel.target.file_path,
                    rel.line,
                    None,
                ));
            }
        }
    }
    if !path_res.from.is_empty() {
        Some(make_query_next_call(&path_res.from))
    } else {
        None
    }
}

fn next_for_source(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let src = result.sources.get(index)?;
    let start = src
        .excerpts
        .first()
        .map_or(1, |ex| if ex.start_line == 0 { 1 } else { ex.start_line });
    let known_last = if src.line_count > 0 {
        Some(src.line_count)
    } else {
        None
    };
    Some(make_range_next_call(
        &src.repo,
        &src.file_path,
        start,
        known_last,
    ))
}

fn next_for_call_flow(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let flow_item = result.call_flows.get(index)?;
    for rel in &result.direct_relations {
        if rel.source.symbol_name == flow_item.caller && !rel.source.file_path.is_empty() {
            let line = if flow_item.line > 0 {
                flow_item.line
            } else {
                rel.line
            };
            return Some(make_range_next_call(
                &rel.source.repo,
                &rel.source.file_path,
                line,
                None,
            ));
        }
    }
    for ep in &result.entry_points {
        if ep.source.symbol_name == flow_item.caller && !ep.source.file_path.is_empty() {
            let line = if flow_item.line > 0 {
                flow_item.line
            } else {
                ep.line
            };
            return Some(make_range_next_call(
                &ep.source.repo,
                &ep.source.file_path,
                line,
                None,
            ));
        }
    }
    for sym in &result.primary_symbols {
        if sym.symbol.name == flow_item.caller && !sym.file_path.is_empty() {
            let line = if flow_item.line > 0 {
                flow_item.line
            } else if sym.start_line > 0 {
                sym.start_line
            } else {
                sym.symbol.span.start_line
            };
            return Some(make_range_next_call(
                &sym.symbol.repo,
                &sym.file_path,
                line,
                None,
            ));
        }
    }
    Some(make_query_next_call(&flow_item.caller))
}

fn next_for_endpoint_relation(
    endpoint_repo: &str,
    endpoint_file: &str,
    line: usize,
) -> Option<NextCall> {
    if endpoint_file.is_empty() {
        None
    } else {
        Some(make_range_next_call(
            endpoint_repo,
            endpoint_file,
            line,
            None,
        ))
    }
}

fn next_for_entry_point(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let rel = result.entry_points.get(index)?;
    let ep = if !rel.source.file_path.is_empty() {
        &rel.source
    } else {
        &rel.target
    };
    next_for_endpoint_relation(&ep.repo, &ep.file_path, rel.line)
}

fn next_for_direct_relation(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let rel = result.direct_relations.get(index)?;
    let ep = if !rel.source.file_path.is_empty() {
        &rel.source
    } else {
        &rel.target
    };
    next_for_endpoint_relation(&ep.repo, &ep.file_path, rel.line)
}

fn next_for_transitive_consumer(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let c = result.transitive_consumers.get(index)?;
    Some(make_range_next_call(&c.repo, &c.file_path, 1, None))
}

fn next_for_not_shown(result: &ExploreResult, index: usize) -> Option<NextCall> {
    let fm = result.not_shown.get(index)?;
    let start = fm
        .symbols
        .first()
        .map_or(1, |s| if s.line == 0 { 1 } else { s.line });
    Some(make_range_next_call(&fm.repo, &fm.file_path, start, None))
}

fn next_for_record(result: &ExploreResult, section: Section, index: usize) -> Option<NextCall> {
    match section {
        Section::FileMatches => next_for_file_match(result, index),
        Section::PrimarySymbols => next_for_primary_symbol(result, index),
        Section::Flows => next_for_flow(result, index),
        Section::Sources => next_for_source(result, index),
        Section::CallFlows => next_for_call_flow(result, index),
        Section::EntryPoints => next_for_entry_point(result, index),
        Section::DirectRelations => next_for_direct_relation(result, index),
        Section::TransitiveConsumers => next_for_transitive_consumer(result, index),
        Section::NotShown => next_for_not_shown(result, index),
    }
}

fn find_next_call(
    result: &ExploreResult,
    priority: &[Section],
    is_admitted: impl Fn(Section, usize) -> bool,
) -> Option<NextCall> {
    for &sec in priority {
        let count = section_item_count(result, sec);
        for i in 0..count {
            if !is_admitted(sec, i)
                && let Some(next) = next_for_record(result, sec, i)
            {
                return Some(next);
            }
        }
    }
    None
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

fn write_compact_record(
    result: &ExploreResult,
    sec: Section,
    index: usize,
    scratch: &mut BudgetBuffer,
) -> Result<(), String> {
    match sec {
        Section::FileMatches => {
            let fm = result
                .file_matches
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            ExploreRecord::FileMatch(fm)
                .write_to(scratch)
                .map_err(|e| e.to_string())
        }
        Section::PrimarySymbols => {
            let sym = result
                .primary_symbols
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            ExploreRecord::PrimarySymbol(sym)
                .write_to(scratch)
                .map_err(|e| e.to_string())
        }
        Section::DirectRelations => {
            let rel = result
                .direct_relations
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            ExploreRecord::DirectRelation(rel)
                .write_to(scratch)
                .map_err(|e| e.to_string())
        }
        Section::TransitiveConsumers => {
            let c = result
                .transitive_consumers
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            ExploreRecord::TransitiveConsumer(c)
                .write_to(scratch)
                .map_err(|e| e.to_string())
        }
        _ => Ok(()),
    }
}

struct PipeEscapingWriter<'a, W: std::io::Write> {
    inner: &'a mut W,
}

impl<'a, W: std::io::Write> std::io::Write for PipeEscapingWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            if b == b'|' {
                self.inner.write_all(b"\\u007c")?;
            } else {
                self.inner.write_all(&[b])?;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn encode_compact_next_arguments(
    nc: &NextCall,
    scratch: &mut BudgetBuffer,
) -> Result<bool, String> {
    let mut escaping = PipeEscapingWriter { inner: scratch };
    if let Err(err) = serde_json::to_writer(&mut escaping, &nc.arguments) {
        if scratch.overflowed {
            return Ok(false);
        }
        return Err(err.to_string());
    }
    Ok(!scratch.overflowed)
}

fn compact_max_next_len(result: &ExploreResult, limit: usize) -> Result<usize, String> {
    let mut max_next_len = 0;
    let mut scratch = BudgetBuffer::new(limit);
    for &sec in &COMPACT_PRIORITY {
        let count = section_item_count(result, sec);
        for i in 0..count {
            if let Some(nc) = next_for_record(result, sec, i) {
                scratch.reset(limit);
                let fits = encode_compact_next_arguments(&nc, &mut scratch)?;
                if !fits {
                    return Err("next continuation arguments exceed response budget".to_owned());
                }
                let arg_len = scratch.bytes.len();
                max_next_len =
                    max_next_len.max(NEXT_SCHEMA.len() + 1 + "graph_explore|".len() + arg_len + 1);
            }
        }
    }
    Ok(max_next_len)
}

fn emit_compact_section(
    out: &mut String,
    schema: &str,
    sec: Section,
    has_emitted: &mut bool,
    arena: &[u8],
    admitted_records: &[EncodedRow],
) -> Result<(), String> {
    let mut sec_has_rows = false;
    for r in admitted_records.iter().filter(|r| r.section == sec) {
        if !sec_has_rows {
            if *has_emitted {
                out.push('\n');
            }
            out.push_str(schema);
            *has_emitted = true;
            sec_has_rows = true;
        }
        out.push('\n');
        let end = r.start.saturating_add(r.len);
        let slice = arena
            .get(r.start..end)
            .ok_or_else(|| "arena slice out of bounds".to_owned())?;
        let row_str = std::str::from_utf8(slice).map_err(|e| e.to_string())?;
        out.push_str(row_str);
    }
    Ok(())
}

fn emit_compact_omitted_counts(
    out: &mut String,
    result: &ExploreResult,
    admitted_records: &[EncodedRow],
) -> Result<(), String> {
    let count_fm_omitted = result.file_matches.len().saturating_sub(
        admitted_records
            .iter()
            .filter(|r| r.section == Section::FileMatches)
            .count(),
    );
    let count_ps_omitted = result.primary_symbols.len().saturating_sub(
        admitted_records
            .iter()
            .filter(|r| r.section == Section::PrimarySymbols)
            .count(),
    );
    let count_dr_omitted = result.direct_relations.len().saturating_sub(
        admitted_records
            .iter()
            .filter(|r| r.section == Section::DirectRelations)
            .count(),
    );
    let count_tc_omitted = result.transitive_consumers.len().saturating_sub(
        admitted_records
            .iter()
            .filter(|r| r.section == Section::TransitiveConsumers)
            .count(),
    );

    if count_fm_omitted > 0 {
        write!(out, "\nfile_matches|{count_fm_omitted}").map_err(|e| e.to_string())?;
    }
    if count_ps_omitted > 0 {
        write!(out, "\nprimary_symbols|{count_ps_omitted}").map_err(|e| e.to_string())?;
    }
    if count_dr_omitted > 0 {
        write!(out, "\ndirect_relations|{count_dr_omitted}").map_err(|e| e.to_string())?;
    }
    if count_tc_omitted > 0 {
        write!(out, "\ntransitive_consumers|{count_tc_omitted}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn build_compact_string(
    result: &ExploreResult,
    limit: usize,
    arena: &[u8],
    admitted_records: &[EncodedRow],
) -> Result<String, String> {
    let is_admitted = |sec: Section, idx: usize| -> bool {
        admitted_records
            .iter()
            .any(|r| r.section == sec && r.index == idx)
    };
    let next_call = find_next_call(result, &COMPACT_PRIORITY, is_admitted);

    let mut out = String::new();
    let mut has_emitted = false;

    emit_compact_section(
        &mut out,
        FILE_MATCH_SCHEMA,
        Section::FileMatches,
        &mut has_emitted,
        arena,
        admitted_records,
    )?;

    let ps_admitted = admitted_records
        .iter()
        .any(|r| r.section == Section::PrimarySymbols);
    if ps_admitted {
        emit_compact_section(
            &mut out,
            SYMBOL_SCHEMA,
            Section::PrimarySymbols,
            &mut has_emitted,
            arena,
            admitted_records,
        )?;
    } else if !has_emitted && result.file_matches.is_empty() && !result.primary_symbols.is_empty() {
        out.push_str(SYMBOL_SCHEMA);
        has_emitted = true;
    }

    emit_compact_section(
        &mut out,
        RELATION_SCHEMA,
        Section::DirectRelations,
        &mut has_emitted,
        arena,
        admitted_records,
    )?;
    emit_compact_section(
        &mut out,
        EXPLORE_IMPACT_SCHEMA,
        Section::TransitiveConsumers,
        &mut has_emitted,
        arena,
        admitted_records,
    )?;

    let separator = if out.is_empty() { "" } else { "\n" };
    write!(
        out,
        "{separator}{PARTIAL_SCHEMA}\ntrue|{limit}\n{OMITTED_SCHEMA}"
    )
    .map_err(|e| e.to_string())?;

    emit_compact_omitted_counts(&mut out, result, admitted_records)?;

    if let Some(next) = &next_call {
        let mut scratch = BudgetBuffer::new(limit);
        let fits = encode_compact_next_arguments(next, &mut scratch)?;
        if !fits {
            return Err("next continuation arguments exceed response budget".to_owned());
        }
        let arguments = std::str::from_utf8(&scratch.bytes).map_err(|e| e.to_string())?;
        write!(out, "\n{NEXT_SCHEMA}\ngraph_explore|{arguments}").map_err(|e| e.to_string())?;
    }

    Ok(out)
}

fn partial_compact_output(result: &ExploreResult, limit: usize) -> Result<String, String> {
    let max_next_len = compact_max_next_len(result, limit)?;

    let reserved_metadata = 1
        + PARTIAL_SCHEMA.len()
        + 1
        + "true|".len()
        + 10
        + 1
        + OMITTED_SCHEMA.len()
        + 1
        + "file_matches|".len()
        + 10
        + 1
        + "primary_symbols|".len()
        + 10
        + 1
        + "direct_relations|".len()
        + 10
        + 1
        + "transitive_consumers|".len()
        + 10
        + max_next_len;

    let candidate_budget = limit.saturating_sub(reserved_metadata);

    let mut scratch = BudgetBuffer::new(limit);
    let mut arena: Vec<u8> = Vec::with_capacity(candidate_budget);
    let mut admitted_records: Vec<EncodedRow> = Vec::new();
    let mut admitted_lens_by_sec = [0usize; 4]; // FM, PS, DR, TC
    let mut total_admitted_len = 0;

    for &sec in &COMPACT_PRIORITY {
        let sec_idx = match sec {
            Section::FileMatches => 0,
            Section::PrimarySymbols => 1,
            Section::DirectRelations => 2,
            Section::TransitiveConsumers => 3,
            _ => continue,
        };

        let count = section_item_count(result, sec);
        for i in 0..count {
            scratch.reset(limit);
            if let Err(err) = write_compact_record(result, sec, i, &mut scratch) {
                if !scratch.overflowed {
                    return Err(err);
                }
                continue;
            }

            let is_first_in_sec = admitted_lens_by_sec.get(sec_idx).copied().unwrap_or(0) == 0;
            let schema_overhead = if is_first_in_sec {
                match sec {
                    Section::FileMatches => FILE_MATCH_SCHEMA.len() + 1,
                    Section::PrimarySymbols => SYMBOL_SCHEMA.len() + 1,
                    Section::DirectRelations => RELATION_SCHEMA.len() + 1,
                    Section::TransitiveConsumers => EXPLORE_IMPACT_SCHEMA.len() + 1,
                    _ => 0,
                }
            } else {
                0
            };

            let row_cost = scratch.bytes.len() + 1 + schema_overhead;
            if total_admitted_len + row_cost <= candidate_budget {
                total_admitted_len += row_cost;
                if let Some(slot) = admitted_lens_by_sec.get_mut(sec_idx) {
                    *slot += 1;
                }
                let start = arena.len();
                let len = scratch.bytes.len();
                arena.extend_from_slice(&scratch.bytes);
                admitted_records.push(EncodedRow {
                    section: sec,
                    index: i,
                    start,
                    len,
                });
            }
        }
    }

    loop {
        let out = build_compact_string(result, limit, &arena, &admitted_records)?;
        if out.len() <= limit || admitted_records.is_empty() {
            return Ok(out);
        }
        admitted_records.pop();
    }
}

fn serialize_record_json(
    result: &ExploreResult,
    sec: Section,
    index: usize,
    scratch: &mut BudgetBuffer,
) -> Result<(), String> {
    match sec {
        Section::FileMatches => {
            let item = result
                .file_matches
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::PrimarySymbols => {
            let item = result
                .primary_symbols
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::Flows => {
            let item = result
                .flows
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::Sources => {
            let item = result
                .sources
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::CallFlows => {
            let item = result
                .call_flows
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::EntryPoints => {
            let item = result
                .entry_points
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::DirectRelations => {
            let item = result
                .direct_relations
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::TransitiveConsumers => {
            let item = result
                .transitive_consumers
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
        Section::NotShown => {
            let item = result
                .not_shown
                .get(index)
                .ok_or_else(|| "index out of bounds".to_owned())?;
            serde_json::to_writer(scratch, item).map_err(|e| e.to_string())
        }
    }
}
fn encode_json_next(nc: &NextCall, scratch: &mut BudgetBuffer) -> Result<bool, String> {
    if let Err(err) = serde_json::to_writer(&mut *scratch, nc) {
        if scratch.overflowed {
            return Ok(false);
        }
        return Err(err.to_string());
    }
    Ok(!scratch.overflowed)
}

fn json_max_next_len(result: &ExploreResult, limit: usize) -> Result<usize, String> {
    let mut max_next_len = 0;
    let mut scratch = BudgetBuffer::new(limit);
    for &sec in &JSON_PRIORITY {
        let count = section_item_count(result, sec);
        for i in 0..count {
            if let Some(nc) = next_for_record(result, sec, i) {
                scratch.reset(limit);
                let fits = encode_json_next(&nc, &mut scratch)?;
                if !fits {
                    return Err("next continuation payload exceeds response budget".to_owned());
                }
                let payload_len = scratch.bytes.len();
                max_next_len = max_next_len.max(r#","next":"#.len() + payload_len);
            }
        }
    }
    Ok(max_next_len)
}

fn encode_scalar_json<T: serde::Serialize>(
    value: &T,
    raw_len: usize,
    budget_limit: usize,
    scratch: &mut BudgetBuffer,
) -> Result<Option<Vec<u8>>, String> {
    if raw_len > budget_limit {
        return Ok(None);
    }
    scratch.reset(budget_limit);
    if let Err(err) = serde_json::to_writer(&mut *scratch, value) {
        if scratch.overflowed {
            return Ok(None);
        }
        return Err(err.to_string());
    }
    if scratch.overflowed {
        Ok(None)
    } else {
        Ok(Some(scratch.bytes.clone()))
    }
}

fn build_json_output_meta(
    result: &ExploreResult,
    limit: usize,
    admitted_records: &[EncodedRow],
    omit_query: bool,
    omit_impact: bool,
    next_call: Option<&NextCall>,
) -> Result<String, String> {
    let mut omitted_map = serde_json::Map::new();
    for &sec in &JSON_PRIORITY {
        let total = section_item_count(result, sec);
        let admitted_count = admitted_records.iter().filter(|r| r.section == sec).count();
        let omitted_count = total.saturating_sub(admitted_count);
        if omitted_count > 0 {
            omitted_map.insert(
                sec.json_key().to_owned(),
                serde_json::Value::Number(serde_json::Number::from(omitted_count)),
            );
        }
    }

    let mut omitted_fields = Vec::new();
    if omit_query {
        omitted_fields.push("query");
    }
    if omit_impact {
        omitted_fields.push("impact_summary");
    }

    let mut output_map = serde_json::Map::new();
    output_map.insert("partial".to_owned(), serde_json::Value::Bool(true));
    output_map.insert(
        "budget_bytes".to_owned(),
        serde_json::Value::Number(serde_json::Number::from(limit)),
    );
    output_map.insert("omitted".to_owned(), serde_json::Value::Object(omitted_map));
    output_map.insert(
        "omitted_fields".to_owned(),
        serde_json::Value::Array(
            omitted_fields
                .into_iter()
                .map(|s| serde_json::Value::String(s.to_owned()))
                .collect(),
        ),
    );

    if let Some(next) = next_call {
        let mut next_map = serde_json::Map::new();
        next_map.insert(
            "tool".to_owned(),
            serde_json::Value::String(next.tool.to_owned()),
        );
        next_map.insert("arguments".to_owned(), next.arguments.clone());
        output_map.insert("next".to_owned(), serde_json::Value::Object(next_map));
    }

    serde_json::to_string(&serde_json::Value::Object(output_map)).map_err(|e| e.to_string())
}

fn emit_json_section(
    out: &mut String,
    key: &str,
    sec: Section,
    arena: &[u8],
    admitted_records: &[EncodedRow],
) -> Result<(), String> {
    out.push_str(",\"");
    out.push_str(key);
    out.push_str("\":[");
    let mut first = true;
    for r in admitted_records.iter().filter(|r| r.section == sec) {
        if !first {
            out.push(',');
        }
        first = false;
        let end = r.start.saturating_add(r.len);
        let slice = arena
            .get(r.start..end)
            .ok_or_else(|| "arena slice out of bounds".to_owned())?;
        let row_str = std::str::from_utf8(slice).map_err(|e| e.to_string())?;
        out.push_str(row_str);
    }
    out.push(']');
    Ok(())
}

struct JsonScalarState<'a> {
    query_encoded: Option<&'a [u8]>,
    impact_encoded: Option<&'a [u8]>,
    omit_query: bool,
    omit_impact: bool,
}

fn build_json_string(
    result: &ExploreResult,
    limit: usize,
    arena: &[u8],
    admitted_records: &[EncodedRow],
    scalars: &JsonScalarState<'_>,
) -> Result<String, String> {
    let is_admitted = |sec: Section, idx: usize| -> bool {
        admitted_records
            .iter()
            .any(|r| r.section == sec && r.index == idx)
    };
    let next_call = find_next_call(result, &JSON_PRIORITY, is_admitted);
    let out_meta_json = build_json_output_meta(
        result,
        limit,
        admitted_records,
        scalars.omit_query,
        scalars.omit_impact,
        next_call.as_ref(),
    )?;

    let mut out = String::new();
    out.push('{');

    out.push_str("\"query\":");
    if scalars.omit_query || scalars.query_encoded.is_none() {
        out.push_str("\"\"");
    } else if let Some(q_bytes) = scalars.query_encoded {
        let q_str = std::str::from_utf8(q_bytes).map_err(|e| e.to_string())?;
        out.push_str(q_str);
    }

    if !result.file_matches.is_empty() {
        emit_json_section(
            &mut out,
            "file_matches",
            Section::FileMatches,
            arena,
            admitted_records,
        )?;
    }
    emit_json_section(
        &mut out,
        "primary_symbols",
        Section::PrimarySymbols,
        arena,
        admitted_records,
    )?;
    emit_json_section(
        &mut out,
        "call_flows",
        Section::CallFlows,
        arena,
        admitted_records,
    )?;

    out.push_str(",\"impact_summary\":");
    if scalars.omit_impact || scalars.impact_encoded.is_none() {
        out.push_str("null");
    } else if let Some(i_bytes) = scalars.impact_encoded {
        let i_str = std::str::from_utf8(i_bytes).map_err(|e| e.to_string())?;
        out.push_str(i_str);
    }

    emit_json_section(
        &mut out,
        "direct_relations",
        Section::DirectRelations,
        arena,
        admitted_records,
    )?;
    emit_json_section(
        &mut out,
        "entry_points",
        Section::EntryPoints,
        arena,
        admitted_records,
    )?;
    emit_json_section(
        &mut out,
        "transitive_consumers",
        Section::TransitiveConsumers,
        arena,
        admitted_records,
    )?;
    emit_json_section(
        &mut out,
        "sources",
        Section::Sources,
        arena,
        admitted_records,
    )?;
    emit_json_section(&mut out, "flows", Section::Flows, arena, admitted_records)?;
    emit_json_section(
        &mut out,
        "not_shown",
        Section::NotShown,
        arena,
        admitted_records,
    )?;

    out.push_str(",\"output\":");
    out.push_str(&out_meta_json);
    out.push('}');
    Ok(out)
}

fn compute_json_candidate_budget(
    result: &ExploreResult,
    limit: usize,
    query_encoded: Option<&[u8]>,
    impact_encoded: Option<&[u8]>,
    omit_query: &mut bool,
    omit_impact: &mut bool,
) -> Result<usize, String> {
    let max_next_len = json_max_next_len(result, limit)?;

    let fixed_json_skeleton = r#"{"query":,"primary_symbols":[],"call_flows":[],"impact_summary":,"direct_relations":[],"entry_points":[],"transitive_consumers":[],"sources":[],"flows":[],"not_shown":[],"output":{"partial":true,"budget_bytes":,"omitted":{},"omitted_fields":[]}}"#.len()
        + 10
        + if !result.file_matches.is_empty() { r#","file_matches":[]"#.len() } else { 0 };

    let worst_case_omitted_map_len = 9 * 35;
    let omitted_fields_max_len = 35;

    let query_bytes_len = query_encoded.as_ref().map_or(r#""""#.len(), |b| b.len());
    let impact_bytes_len = impact_encoded.as_ref().map_or("null".len(), |b| b.len());

    let mut reserved_metadata = fixed_json_skeleton
        + worst_case_omitted_map_len
        + omitted_fields_max_len
        + max_next_len
        + if *omit_query {
            r#""""#.len()
        } else {
            query_bytes_len
        }
        + if *omit_impact {
            "null".len()
        } else {
            impact_bytes_len
        };

    if reserved_metadata > limit {
        if !*omit_query && query_bytes_len > 64 {
            *omit_query = true;
            reserved_metadata = reserved_metadata
                .saturating_sub(query_bytes_len)
                .saturating_add(r#""""#.len());
        }
        if reserved_metadata > limit && !*omit_impact && impact_bytes_len > 64 {
            *omit_impact = true;
            reserved_metadata = reserved_metadata
                .saturating_sub(impact_bytes_len)
                .saturating_add("null".len());
        }
    }

    Ok(limit.saturating_sub(reserved_metadata))
}

fn admit_json_records(
    result: &ExploreResult,
    candidate_budget: usize,
    limit: usize,
    arena: &mut Vec<u8>,
    admitted_records: &mut Vec<EncodedRow>,
    scratch: &mut BudgetBuffer,
) -> Result<(), String> {
    let mut total_admitted_len = 0;
    for &sec in &JSON_PRIORITY {
        let count = section_item_count(result, sec);
        for i in 0..count {
            scratch.reset(limit);
            if let Err(err) = serialize_record_json(result, sec, i, scratch) {
                if !scratch.overflowed {
                    return Err(err);
                }
                continue;
            }

            let row_cost = scratch.bytes.len() + 1;
            if total_admitted_len + row_cost <= candidate_budget {
                total_admitted_len += row_cost;
                let start = arena.len();
                let len = scratch.bytes.len();
                arena.extend_from_slice(&scratch.bytes);
                admitted_records.push(EncodedRow {
                    section: sec,
                    index: i,
                    start,
                    len,
                });
            }
        }
    }
    Ok(())
}

fn partial_json_output(result: &ExploreResult, limit: usize) -> Result<String, String> {
    let mut scratch = BudgetBuffer::new(limit);

    let query_encoded = encode_scalar_json(&result.query, result.query.len(), limit, &mut scratch)?;
    let impact_encoded = match &result.impact_summary {
        Some(summary) => encode_scalar_json(summary, summary.len(), limit, &mut scratch)?,
        None => None,
    };

    let mut omit_query = query_encoded.is_none() && !result.query.is_empty();
    let mut omit_impact = impact_encoded.is_none() && result.impact_summary.is_some();

    let candidate_budget = compute_json_candidate_budget(
        result,
        limit,
        query_encoded.as_deref(),
        impact_encoded.as_deref(),
        &mut omit_query,
        &mut omit_impact,
    )?;

    let mut arena: Vec<u8> = Vec::with_capacity(candidate_budget);
    let mut admitted_records: Vec<EncodedRow> = Vec::new();

    admit_json_records(
        result,
        candidate_budget,
        limit,
        &mut arena,
        &mut admitted_records,
        &mut scratch,
    )?;

    let mut scalars = JsonScalarState {
        query_encoded: query_encoded.as_deref(),
        impact_encoded: impact_encoded.as_deref(),
        omit_query,
        omit_impact,
    };

    loop {
        let out = build_json_string(result, limit, &arena, &admitted_records, &scalars)?;

        if out.len() <= limit {
            return Ok(out);
        }

        if !admitted_records.is_empty() {
            admitted_records.pop();
            continue;
        }

        if !scalars.omit_query && !result.query.is_empty() {
            scalars.omit_query = true;
            continue;
        }
        if !scalars.omit_impact && result.impact_summary.is_some() {
            scalars.omit_impact = true;
            continue;
        }

        return Ok(out);
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/mcp/explore_output.rs"]
mod tests;
