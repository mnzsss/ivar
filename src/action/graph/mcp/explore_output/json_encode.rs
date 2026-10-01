use crate::domain::graph::ExploreResult;

use super::next::next_for_record;
use super::{BudgetBuffer, JSON_PRIORITY, NextCall, Section, section_item_count};

pub(super) fn serialize_record_json(
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

pub(super) fn encode_json_next(nc: &NextCall, scratch: &mut BudgetBuffer) -> Result<bool, String> {
    if let Err(err) = serde_json::to_writer(&mut *scratch, nc) {
        if scratch.overflowed {
            return Ok(false);
        }
        return Err(err.to_string());
    }
    Ok(!scratch.overflowed)
}

pub(super) fn json_max_next_len(result: &ExploreResult, limit: usize) -> Result<usize, String> {
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

pub(super) fn encode_scalar_json<T: serde::Serialize>(
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
