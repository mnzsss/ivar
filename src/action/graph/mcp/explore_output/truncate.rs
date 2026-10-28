//! Line-boundary truncation of source-bearing rows.
//!
//! A source or symbol larger than the response budget used to be dropped whole,
//! so the budget went to relations and the requested text never arrived. Here the
//! row keeps the longest line prefix that fits, and the [`Truncation`] records
//! where the rest starts so `output.next` can continue from the following line.

use serde::Serialize;

use crate::domain::graph::{SourceExcerpt, SourceFile, SymbolSnippet};

use super::next::make_exact_range_next_call;
use super::{BudgetBuffer, NextCall, Section};

/// A truncated row must keep at least this many bytes to be worth sending: a
/// stub of a line or two costs a round trip and shows nothing.
pub(super) const MIN_TRUNCATED_BYTES: usize = 400;

/// Where the text a truncated row left out begins and ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Truncation {
    pub(super) section: Section,
    pub(super) repo: String,
    pub(super) file_path: String,
    pub(super) next_start: usize,
    pub(super) next_end: usize,
}

impl Truncation {
    pub(super) fn next_call(&self) -> NextCall {
        make_exact_range_next_call(&self.repo, &self.file_path, self.next_start, self.next_end)
    }

    pub(super) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "section": self.section.json_key(),
            "repo": self.repo,
            "file_path": self.file_path,
            "next_start": self.next_start,
            "next_end": self.next_end,
        })
    }
}

pub(super) struct TruncatedRow {
    pub(super) bytes: Vec<u8>,
    pub(super) truncation: Truncation,
}

/// Encoded length of `value`, or `None` once it passes `max_len`.
fn serialized_len<T: Serialize>(value: &T, max_len: usize) -> Option<usize> {
    let mut buffer = BudgetBuffer::new(max_len);
    serde_json::to_writer(&mut buffer, value).ok()?;
    (!buffer.overflowed).then_some(buffer.bytes.len())
}

/// Largest `n <= total` for which `fits(n)` holds, assuming `fits` is monotone
/// (more lines never shrink the row). Zero when not even one line fits.
fn largest_fit(total: usize, fits: impl Fn(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, total);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if fits(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

/// Keeps the longest prefix of `source` (counted in lines across its excerpts)
/// whose encoding is at most `max_len` bytes.
///
/// `None` when not even [`MIN_TRUNCATED_BYTES`] of source fits, or when the
/// whole row already fits and there is nothing to cut.
pub(super) fn truncate_source(
    source: &SourceFile,
    max_len: usize,
) -> Result<Option<TruncatedRow>, String> {
    let lines: Vec<Vec<&str>> = source
        .excerpts
        .iter()
        .map(|excerpt| excerpt.code.split('\n').collect())
        .collect();
    let total: usize = lines.iter().map(Vec::len).sum();
    let prefix = |count: usize| -> SourceFile {
        let mut remaining = count;
        let mut excerpts = Vec::new();
        for (excerpt, excerpt_lines) in source.excerpts.iter().zip(&lines) {
            let take = remaining.min(excerpt_lines.len());
            if take == 0 {
                break;
            }
            if take == excerpt_lines.len() {
                excerpts.push(excerpt.clone());
            } else {
                excerpts.push(SourceExcerpt {
                    start_line: excerpt.start_line,
                    end_line: excerpt.start_line + take - 1,
                    code: excerpt_lines
                        .iter()
                        .take(take)
                        .copied()
                        .collect::<Vec<_>>()
                        .join("\n"),
                });
            }
            remaining -= take;
        }
        SourceFile {
            excerpts,
            ..source.clone()
        }
    };

    let kept = largest_fit(total, |count| {
        serialized_len(&prefix(count), max_len).is_some()
    });
    if kept == 0 || kept >= total {
        return Ok(None);
    }
    let kept_source = prefix(kept);
    let bytes = serde_json::to_vec(&kept_source).map_err(|e| e.to_string())?;
    if bytes.len() < MIN_TRUNCATED_BYTES {
        return Ok(None);
    }
    let Some(last_returned) = kept_source.excerpts.last().map(|e| e.end_line) else {
        return Ok(None);
    };
    let next_start = last_returned + 1;
    let next_end = source
        .excerpts
        .last()
        .map_or(next_start, |e| e.end_line)
        .max(next_start);
    Ok(Some(TruncatedRow {
        bytes,
        truncation: Truncation {
            section: Section::Sources,
            repo: source.repo.clone(),
            file_path: source.file_path.clone(),
            next_start,
            next_end,
        },
    }))
}

/// Keeps the longest line prefix of a symbol's `code` that fits in `max_len`
/// bytes, moving `end_line` to the last line kept.
pub(super) fn truncate_primary_symbol(
    snippet: &SymbolSnippet,
    max_len: usize,
) -> Result<Option<TruncatedRow>, String> {
    let lines: Vec<&str> = snippet.code.split('\n').collect();
    let start = if snippet.start_line > 0 {
        snippet.start_line
    } else {
        snippet.symbol.span.start_line.max(1)
    };
    let prefix = |count: usize| -> SymbolSnippet {
        SymbolSnippet {
            code: lines
                .iter()
                .take(count)
                .copied()
                .collect::<Vec<_>>()
                .join("\n"),
            start_line: start,
            end_line: start + count.saturating_sub(1),
            ..snippet.clone()
        }
    };

    let kept = largest_fit(lines.len(), |count| {
        serialized_len(&prefix(count), max_len).is_some()
    });
    if kept == 0 || kept >= lines.len() {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(&prefix(kept)).map_err(|e| e.to_string())?;
    if bytes.len() < MIN_TRUNCATED_BYTES {
        return Ok(None);
    }
    let next_start = start + kept;
    let known_end = if snippet.end_line > 0 {
        snippet.end_line
    } else {
        snippet.symbol.span.end_line
    };
    let next_end = known_end.max(start + lines.len() - 1).max(next_start);
    Ok(Some(TruncatedRow {
        bytes,
        truncation: Truncation {
            section: Section::PrimarySymbols,
            repo: snippet.symbol.repo.clone(),
            file_path: snippet.file_path.clone(),
            next_start,
            next_end,
        },
    }))
}
