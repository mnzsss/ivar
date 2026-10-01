use crate::domain::graph::ExploreResult;

use super::next::{find_next_call, next_for_record};
use super::{BudgetBuffer, EncodedRow, JSON_PRIORITY, NextCall, Section, section_item_count};

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
