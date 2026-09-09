//! Deterministic plan fingerprint calculation.
//!
//! Canonicalizes task checkbox state (e.g. `[ ]`, `[x]`, `[X]`) across markdown
//! task list items and table cells so checking off tasks during wave execution
//! does not alter the plan fingerprint. Any other modifications (instructions,
//! task descriptions, headings, structure) alter the fingerprint and trigger
//! divergence detection.

use camino::Utf8Path;

use crate::error::Failure;
use crate::infra::{fs, hash};

/// Canonicalize checkbox states in markdown text to unchecked `[ ]`.
///
/// Handles:
/// - Indented and unindented list task items starting with `- [ ]`, `- [x]`, `- [X]`
/// - Indented and unindented list task items starting with `* [ ]`, `* [x]`, `* [X]`
/// - Table cells containing `| [ ] |`, `| [x] |`, `| [X] |`
///
/// Preserves exact content, indentation, and newlines outside checkbox markers.
pub(crate) fn normalize_checkboxes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let ends_with_newline = text.ends_with('\n');
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let indent_len = line.len() - trimmed.len();
        let indent = &line[..indent_len];

        if let Some(rest) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
            .or_else(|| trimmed.strip_prefix("- [ ] "))
        {
            out.push_str(indent);
            out.push_str("- [ ] ");
            out.push_str(rest);
        } else if let Some(rest) = trimmed
            .strip_prefix("* [x] ")
            .or_else(|| trimmed.strip_prefix("* [X] "))
            .or_else(|| trimmed.strip_prefix("* [ ] "))
        {
            out.push_str(indent);
            out.push_str("* [ ] ");
            out.push_str(rest);
        } else if line.contains("| [x] |") || line.contains("| [X] |") {
            out.push_str(
                &line
                    .replace("| [x] |", "| [ ] |")
                    .replace("| [X] |", "| [ ] |"),
            );
        } else {
            out.push_str(line);
        }

        if lines.peek().is_some() || ends_with_newline {
            out.push('\n');
        }
    }

    if text.is_empty() {
        return String::new();
    }

    out
}

/// Computes the SHA-256 fingerprint of `path` after canonicalizing checkbox states.
pub(crate) fn normalized_plan_fingerprint(path: &Utf8Path) -> Result<String, Failure> {
    let content = fs::read_text(path)?.ok_or_else(|| {
        Failure::blocked(
            "execute.plan_missing",
            format!("plan file `{path}` does not exist"),
        )
    })?;
    let normalized = normalize_checkboxes(&content);
    Ok(hash::text(&normalized))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/plan_fingerprint.rs"]
mod tests;
