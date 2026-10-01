use crate::domain::graph::ExploreResult;

use super::next::find_next_call;
use super::{EncodedRow, JSON_PRIORITY, NextCall, Section, section_item_count};

pub(super) fn build_json_output_meta(
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

pub(super) fn emit_json_section(
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

pub(super) struct JsonScalarState<'a> {
    pub(super) query_encoded: Option<&'a [u8]>,
    pub(super) impact_encoded: Option<&'a [u8]>,
    pub(super) omit_query: bool,
    pub(super) omit_impact: bool,
}

pub(super) fn build_json_string(
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
