use std::fmt::Write as _;

use crate::action::graph::compact::{
    EXPLORE_IMPACT_SCHEMA, FILE_MATCH_SCHEMA, RELATION_SCHEMA, SYMBOL_SCHEMA,
};
use crate::domain::graph::ExploreResult;

use super::compact_row::{
    compact_max_next_len, encode_compact_next_arguments, write_compact_record,
};
use super::next::find_next_call;
use super::{
    BudgetBuffer, COMPACT_PRIORITY, EncodedRow, NEXT_SCHEMA, OMITTED_SCHEMA, PARTIAL_SCHEMA,
    Section, section_item_count,
};

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

pub(super) fn partial_compact_output(
    result: &ExploreResult,
    limit: usize,
) -> Result<String, String> {
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
