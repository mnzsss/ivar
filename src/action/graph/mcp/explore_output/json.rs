use crate::domain::graph::ExploreResult;

use super::json_build::{JsonScalarState, build_json_string};
use super::json_encode::{encode_scalar_json, json_max_next_len, serialize_record_json};
use super::{BudgetBuffer, EncodedRow, JSON_PRIORITY, section_item_count};

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
