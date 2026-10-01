use crate::action::graph::compact::ExploreRecord;
use crate::domain::graph::ExploreResult;

use super::next::next_for_record;
use super::{BudgetBuffer, COMPACT_PRIORITY, NEXT_SCHEMA, NextCall, Section, section_item_count};

pub(super) fn write_compact_record(
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

pub(super) struct PipeEscapingWriter<'a, W: std::io::Write> {
    pub(super) inner: &'a mut W,
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

pub(super) fn encode_compact_next_arguments(
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

pub(super) fn compact_max_next_len(result: &ExploreResult, limit: usize) -> Result<usize, String> {
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
