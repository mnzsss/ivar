use crate::domain::graph::ExploreResult;

use super::json_build::{JsonScalarState, build_json_string};
use super::json_encode::{encode_scalar_json, json_max_next_len, serialize_record_json};
use super::truncate::{Truncation, truncate_primary_symbol, truncate_source};
use super::{BudgetBuffer, EncodedRow, JSON_PRIORITY, Section, section_item_count};

/// Worst case for `output.truncated`: at most one truncated source and one
/// truncated symbol, each naming its file and a line range.
fn truncated_metadata_len(result: &ExploreResult) -> usize {
    let longest = result
        .sources
        .iter()
        .map(|s| (s.repo.len(), s.file_path.len()))
        .chain(
            result
                .primary_symbols
                .iter()
                .map(|s| (s.symbol.repo.len(), s.file_path.len())),
        )
        .map(|(repo, path)| repo + path)
        .max();
    longest.map_or(0, |len| {
        // `,"truncated":[` + `]` and two entries; JSON escaping can double a string.
        const ENTRY_OVERHEAD: usize = 140;
        16 + 2 * (ENTRY_OVERHEAD + 2 * len + 1)
    })
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
        + truncated_metadata_len(result)
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

/// Share of the record budget kept for symbols while source is admitted, so a
/// large source cannot erase the symbols that locate it.
const SYMBOL_RESERVE_DIVISOR: usize = 4;

/// Encodes one record into `scratch`; `None` when it alone exceeds the limit.
fn encode_row(
    result: &ExploreResult,
    sec: Section,
    index: usize,
    limit: usize,
    scratch: &mut BudgetBuffer,
) -> Result<Option<usize>, String> {
    scratch.reset(limit);
    match serialize_record_json(result, sec, index, scratch) {
        Ok(()) => Ok(Some(scratch.bytes.len())),
        Err(_) if scratch.overflowed => Ok(None),
        Err(err) => Err(err),
    }
}

/// Bytes held back from source for the primary symbols: what they cost, up to a
/// quarter of the record budget.
fn symbol_reserve(
    result: &ExploreResult,
    candidate_budget: usize,
    limit: usize,
    scratch: &mut BudgetBuffer,
) -> Result<usize, String> {
    if result.sources.is_empty() {
        return Ok(0);
    }
    let cap = candidate_budget / SYMBOL_RESERVE_DIVISOR;
    let mut cost = 0usize;
    for i in 0..result.primary_symbols.len() {
        let row = encode_row(result, Section::PrimarySymbols, i, limit, scratch)?;
        cost = cost.saturating_add(row.map_or(cap, |len| len + 1));
        if cost >= cap {
            return Ok(cap);
        }
    }
    Ok(cost)
}

/// Whether an admitted source row already shows the file of this primary
/// symbol, in which case a truncated copy of the symbol would only repeat it.
fn source_shows_file(result: &ExploreResult, admitted: &[EncodedRow], sym_index: usize) -> bool {
    let Some(sym) = result.primary_symbols.get(sym_index) else {
        return false;
    };
    admitted
        .iter()
        .filter(|r| r.section == Section::Sources)
        .filter_map(|r| result.sources.get(r.index))
        .any(|src| src.repo == sym.symbol.repo && src.file_path == sym.file_path)
}

fn push_row(
    arena: &mut Vec<u8>,
    admitted: &mut Vec<EncodedRow>,
    section: Section,
    index: usize,
    bytes: &[u8],
    truncation: Option<Truncation>,
) {
    let start = arena.len();
    arena.extend_from_slice(bytes);
    admitted.push(EncodedRow {
        section,
        index,
        start,
        len: bytes.len(),
        truncation,
    });
}

fn admit_json_records(
    result: &ExploreResult,
    candidate_budget: usize,
    limit: usize,
    arena: &mut Vec<u8>,
    admitted_records: &mut Vec<EncodedRow>,
    scratch: &mut BudgetBuffer,
) -> Result<(), String> {
    let reserve = symbol_reserve(result, candidate_budget, limit, scratch)?;
    let mut total_admitted_len = 0;
    for &sec in &JSON_PRIORITY {
        let budget = if sec == Section::Sources {
            candidate_budget.saturating_sub(reserve)
        } else {
            candidate_budget
        };
        let count = section_item_count(result, sec);
        let mut first_symbol_misfit = None;
        let mut source_truncated = false;
        for i in 0..count {
            let row_len = encode_row(result, sec, i, limit, scratch)?;
            if let Some(len) = row_len
                && total_admitted_len + len < budget
            {
                total_admitted_len += len + 1;
                push_row(arena, admitted_records, sec, i, &scratch.bytes, None);
                continue;
            }
            match sec {
                // The requested source is cut where it stops fitting, in place,
                // so it comes before anything later in the section.
                Section::Sources if !source_truncated => {
                    source_truncated = true;
                    let room = budget.saturating_sub(total_admitted_len + 1);
                    let cut = result
                        .sources
                        .get(i)
                        .map(|source| truncate_source(source, room))
                        .transpose()?
                        .flatten();
                    if let Some(row) = cut {
                        total_admitted_len += row.bytes.len() + 1;
                        push_row(
                            arena,
                            admitted_records,
                            sec,
                            i,
                            &row.bytes,
                            Some(row.truncation),
                        );
                    }
                }
                Section::PrimarySymbols if first_symbol_misfit.is_none() => {
                    first_symbol_misfit = Some(i);
                }
                _ => {}
            }
        }
        // Symbols that fit whole come first; the first one that did not gets
        // what remains, unless a source already shows that file.
        if let Some(i) = first_symbol_misfit
            && !source_shows_file(result, admitted_records, i)
        {
            let room = budget.saturating_sub(total_admitted_len + 1);
            let cut = result
                .primary_symbols
                .get(i)
                .map(|snippet| truncate_primary_symbol(snippet, room))
                .transpose()?
                .flatten();
            if let Some(row) = cut {
                total_admitted_len += row.bytes.len() + 1;
                push_row(
                    arena,
                    admitted_records,
                    sec,
                    i,
                    &row.bytes,
                    Some(row.truncation),
                );
            }
        }
    }
    // Rows were appended in admission order; the response lists each section by index.
    admitted_records.sort_by_key(|row| {
        (
            JSON_PRIORITY.iter().position(|&s| s == row.section),
            row.index,
        )
    });
    Ok(())
}

pub(super) fn partial_json_output(result: &ExploreResult, limit: usize) -> Result<String, String> {
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
