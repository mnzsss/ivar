use crate::domain::graph::ExploreResult;

use super::{NextCall, Section, section_item_count};

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

pub(super) fn next_for_record(
    result: &ExploreResult,
    section: Section,
    index: usize,
) -> Option<NextCall> {
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

pub(super) fn find_next_call(
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
