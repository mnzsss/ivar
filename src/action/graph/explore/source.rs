use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::action::graph::query::QueryError;
use crate::domain::graph::{SourceExcerpt, SourceFile};
use crate::infra::hash;
use crate::store::graph::db::GraphDb;

use super::error::ExploreError;

/// Files up to this many lines are returned whole: an agent shown a slice of a
/// small file reads the whole file anyway, which costs more than sending it once.
const WHOLE_FILE_MAX_LINES: usize = 250;
const EXCERPT_MERGE_GAP: usize = 8;

pub(crate) struct FileSpans {
    pub(crate) repo: String,
    pub(crate) file_path: String,
    pub(crate) absolute_path: PathBuf,
    pub(crate) spans: Vec<(usize, usize)>,
}

pub(crate) struct CachedFile {
    pub(crate) lines: Vec<String>,
    pub(crate) content_hash: String,
}

/// Files an answer shows source for; the rest are named for the next call.
/// CodeGraph sends 4 files on repositories under 150 files and 5 under 500, and
/// with it agents stopped reading files.
pub(crate) fn max_source_files(db: &GraphDb) -> Result<usize, ExploreError> {
    let files: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM visible_files", [], |row| row.get(0))
        .map_err(QueryError::from)?;
    Ok(match files {
        ..150 => 4,
        150..500 => 5,
        _ => 6,
    })
}

pub(crate) fn collect_sources(
    db: &GraphDb,
    cache: &HashMap<PathBuf, CachedFile>,
    files: Vec<FileSpans>,
) -> Result<Vec<SourceFile>, ExploreError> {
    let mut sources = Vec::with_capacity(files.len());
    for file in files {
        let Some(cached) = cache
            .get(&file.absolute_path)
            .filter(|cached| !cached.lines.is_empty())
        else {
            continue;
        };
        let lines = &cached.lines;
        // Spans come from the last index. Once the file changed they can cut a
        // function in half, so a changed file is served whole.
        let changed_since_index = db
            .get_file(&file.repo, &file.file_path)?
            .is_some_and(|row| row.content_hash != cached.content_hash);
        let ranges = if changed_since_index || lines.len() <= WHOLE_FILE_MAX_LINES {
            vec![(1, lines.len())]
        } else {
            merge_spans(file.spans)
        };
        let excerpts = ranges
            .into_iter()
            .map(|(start, end)| (start.max(1), end.min(lines.len())))
            .filter(|(start, end)| start <= end)
            .map(|(start, end)| SourceExcerpt {
                start_line: start,
                end_line: end,
                code: number_lines(lines, start, end),
            })
            .collect();
        sources.push(SourceFile {
            repo: file.repo,
            file_path: file.file_path,
            line_count: lines.len(),
            excerpts,
            changed_since_index,
        });
    }
    Ok(sources)
}

fn merge_spans(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 + EXCERPT_MERGE_GAP + 1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// Reads source lines `[start_line, end_line]` (1-indexed, inclusive) and formats with line numbers.
pub(crate) fn get_source_snippet(
    cache: &mut HashMap<PathBuf, CachedFile>,
    file_path: &Path,
    start_line: usize,
    end_line: usize,
) -> Result<String, ExploreError> {
    let cached = match cache.get(file_path) {
        Some(cached) => cached,
        None => {
            let content = std::fs::read_to_string(file_path).map_err(|err| ExploreError::Io {
                path: file_path.to_path_buf(),
                source: err,
            })?;
            cache.entry(file_path.to_path_buf()).or_insert(CachedFile {
                lines: content.lines().map(str::to_owned).collect(),
                content_hash: hash::text(&content),
            })
        }
    };

    Ok(number_lines(&cached.lines, start_line, end_line))
}

pub(crate) fn number_lines(lines: &[String], start_line: usize, end_line: usize) -> String {
    let start = start_line.max(1);
    let end = end_line.max(start);
    (start..=end)
        .filter_map(|n| {
            n.checked_sub(1)
                .and_then(|idx| lines.get(idx))
                .map(|line| format!("{n}\t{line}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
