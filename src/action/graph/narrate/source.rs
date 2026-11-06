use super::explore::FileMatches;
use super::*;

/// Emits verbatim source, one block per file, within the output budget, and
/// returns the files that did not fit.
pub(super) fn narrate_source(
    out: &mut String,
    res: &ExploreResult,
    files: &[FileMatches<'_>],
    max_chars: usize,
) -> Vec<FileMention> {
    out.push_str("**Source**\n\n");
    out.push_str(
        "> The code below is the verbatim, current on-disk source, line-numbered like the Read \
         tool. Treat each block as a Read you have already performed: do not Read a file shown \
         here.\n\n",
    );

    let limit = max_chars
        .max(out.len() + MIN_SOURCE_CHARS)
        .min(MAX_REQUESTED_OUTPUT_CHARS - NOT_SHOWN_RESERVE_CHARS);
    let mut emitted = false;
    let mut left_out = Vec::new();
    for file in files {
        let source = res
            .sources
            .iter()
            .find(|s| s.repo == file.repo && s.file_path == file.path);
        let block = file_block(file, source);
        if out.len() + block.len() <= limit {
            out.push_str(&block);
            emitted = true;
        } else if !emitted {
            // The rest to ask for ends where this answer would have ended, not
            // at the end of the file: a ranged request must not grow past its range.
            let last_line = source
                .and_then(|s| s.excerpts.last().map(|e| e.end_line))
                .unwrap_or(0);
            out.push_str(&truncate_block(
                &block,
                limit.saturating_sub(out.len() + 160),
                file.path,
                last_line,
            ));
            emitted = true;
        } else {
            left_out.push(FileMention {
                repo: file.repo.to_owned(),
                file_path: file.path.to_owned(),
                symbols: file
                    .symbols
                    .iter()
                    .map(|s| MentionedSymbol {
                        name: s.symbol.name.clone(),
                        line: s.symbol.span.start_line,
                    })
                    .collect(),
            });
        }
    }
    left_out
}

/// Names the matching files that got no source, so the next step is another
/// `graph_explore` with those names instead of a Read.
pub(super) fn narrate_not_shown(
    out: &mut String,
    left_out: &[FileMention],
    not_shown: &[FileMention],
) {
    let files: Vec<&FileMention> = left_out.iter().chain(not_shown).collect();
    if files.is_empty() {
        return;
    }
    out.push_str(
        "**Not shown above — call `graph_explore` with these paths or names for their source**\n\n",
    );
    for file in files.iter().take(MAX_NOT_SHOWN_FILES) {
        let names: Vec<String> = file
            .symbols
            .iter()
            .take(MAX_NAMES_PER_FILE)
            .map(|s| format!("{}:{}", s.name, s.line))
            .collect();
        let _ = write!(out, "- {}: {}", file.file_path, names.join(", "));
        if file.symbols.len() > MAX_NAMES_PER_FILE {
            let _ = write!(out, ", +{} more", file.symbols.len() - MAX_NAMES_PER_FILE);
        }
        out.push('\n');
    }
    if files.len() > MAX_NOT_SHOWN_FILES {
        let _ = writeln!(
            out,
            "- …and {} more files",
            files.len() - MAX_NOT_SHOWN_FILES
        );
    }
    narrate_next_call(out, &files);
}

/// Agents Read left-out files one per turn, each turn repeating the whole
/// prompt, unless the answer spells out the single call that returns them.
pub(super) fn narrate_next_call(out: &mut String, files: &[&FileMention]) {
    let batch: Vec<&str> = files
        .iter()
        .take(MAX_REQUESTED_FILES)
        .map(|file| file.file_path.as_str())
        .collect();
    let args = serde_json::json!({ "paths": batch });
    let _ = write!(
        out,
        "\nNext: call `graph_explore` with {args} for their full source, one call instead of {} Reads.",
        batch.len()
    );
    let rest = files.len() - batch.len();
    if rest > 0 {
        let _ = write!(out, " Then call it again for the {rest} files after them.");
    }
    out.push('\n');
}

fn file_block(file: &FileMatches<'_>, source: Option<&SourceFile>) -> String {
    let mut block = String::new();
    let _ = write!(block, "`{}` ({}", file.path, file.repo);
    match source {
        Some(source) if source.changed_since_index => {
            let _ = write!(
                block,
                " · whole file, {} line{}, changed since the last index so the symbol lines \
                 below may have moved",
                source.line_count,
                plural(source.line_count)
            );
        }
        Some(source) if is_whole_file(source) => {
            let _ = write!(
                block,
                " · whole file, {} line{}",
                source.line_count,
                plural(source.line_count)
            );
        }
        Some(source) => {
            let ranges: Vec<String> = source
                .excerpts
                .iter()
                .map(|e| format!("{}-{}", e.start_line, e.end_line))
                .collect();
            let _ = write!(
                block,
                " · lines {} of {}",
                ranges.join(", "),
                source.line_count
            );
        }
        None => block.push_str(" · source unavailable"),
    }
    let names: Vec<String> = file
        .symbols
        .iter()
        .map(|s| format!("`{}` {}", s.symbol.name, s.symbol.span.start_line))
        .collect();
    if !names.is_empty() {
        let _ = writeln!(block, ") · {}\n", names.join(", "));
    } else if !file.file_matches.is_empty() {
        let _ = writeln!(block, ") · matched file\n");
    } else {
        let _ = writeln!(block, ")\n");
    }

    let language = fence_language(file.path);
    for excerpt in source.map(|s| s.excerpts.as_slice()).unwrap_or_default() {
        let _ = writeln!(block, "```{language}\n{}\n```\n", excerpt.code.trim_end());
    }
    block
}

fn fence_language(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("js" | "mjs" | "cjs" | "jsx") => "javascript",
        Some("rs") => "rust",
        _ => "",
    }
}

fn is_whole_file(source: &SourceFile) -> bool {
    matches!(source.excerpts.as_slice(), [only] if only.start_line == 1 && only.end_line == source.line_count)
}

fn truncate_block(block: &str, max_len: usize, path: &str, range_end: usize) -> String {
    let mut cut = String::new();
    let mut last_line = None;
    for line in block.lines() {
        if cut.len() + line.len() + 1 > max_len {
            break;
        }
        cut.push_str(line);
        cut.push('\n');
        if let Some(n) = line
            .split_once('\t')
            .and_then(|(n, _)| n.parse::<usize>().ok())
        {
            last_line = Some(n);
        }
    }
    if cut.matches("```").count() % 2 == 1 {
        cut.push_str("```\n");
    }
    if let Some(n) = last_line {
        let next_start = n + 1;
        let last_target = range_end.max(next_start);
        let _ = writeln!(
            cut,
            "\n…truncated after line {n}. Call `graph_explore` with paths: [\"{path}:{next_start}-{last_target}\"] for the rest; do not Read it."
        );
    }
    cut
}

pub(super) fn push_remainder(out: &mut String, total: usize) {
    if total > MAX_LIST_ITEMS {
        let _ = writeln!(out, "- …and {} more", total - MAX_LIST_ITEMS);
    }
}
