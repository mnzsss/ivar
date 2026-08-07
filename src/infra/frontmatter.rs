//! Markdown frontmatter: split, parse, emit.
//!
//! This is the **only** module in the crate that touches YAML. That is
//! deliberate: the YAML crate choice carries the most residual risk in the
//! dependency set (`serde_yaml` is abandoned; the replacement is young), so it is
//! quarantined behind a two-function surface. Swapping the parser must be a
//! one-file change. No `serde_saphyr` type appears in any signature here — only
//! `&str` in, `T: Serialize` / `T: DeserializeOwned` across the boundary.
//!
//! # Where frontmatter appears
//!
//! - skill definitions — read
//! - `plan.md`, `requirements.md`, `analysis.md` — read, and **written back**:
//!   closing a feature records `outcome` and `closed_at`. Round-trip is required,
//!   not optional.
//!
//! # Contract
//!
//! - `split(source)` — separate the frontmatter block from the body. A leading
//!   `---` fence, the block, a closing `---` fence, then the body. No frontmatter
//!   is a normal case, not an error: the whole input is body.
//! - `parse::<T>(source)` — split, then deserialize the block into `T`. No
//!   frontmatter and an empty frontmatter block both deserialize as an empty
//!   YAML mapping — that only succeeds if `T` tolerates one (e.g. every field has
//!   a default).
//! - `replace(source, &new_frontmatter)` — re-emit with the frontmatter replaced
//!   and **the body untouched, byte-for-byte**. A human wrote that body; do not
//!   reflow it, do not renormalise its line endings.
//!
//! # Details worth getting right the first time
//!
//! Splitting is hand-rolled — finding a `---` fence does not need a crate. Watch:
//! a fence must be at the very start of the input; `---` inside the body is not a
//! terminator once the block has closed; `\r\n` inputs exist and the body must
//! survive them unchanged; an unterminated opening fence is a hard error naming
//! the line, not a silent "no frontmatter".
//!
//! A fence line must be *exactly* `---` (with an optional trailing `\r`) — a line
//! with trailing text or extra dashes does not count, and is treated as body.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Failure;

/// The fence line every frontmatter block opens and closes with.
const FENCE: &str = "---";

/// Something that went wrong turning frontmatter text into data, or back.
#[derive(Debug, thiserror::Error)]
pub enum FrontmatterError {
    /// The document opens a `---` fence but the matching closing fence never
    /// appears before end of input.
    #[error(
        "unterminated frontmatter: opening `---` fence at line {line} has no matching closing `---`"
    )]
    UnterminatedFence {
        /// The line the opening fence is on. Always 1, since a fence must sit at
        /// the very start of the input — kept as a field rather than a constant
        /// so the message stays honest if that constraint ever relaxes.
        line: usize,
    },

    /// The frontmatter block is not valid YAML.
    #[error("frontmatter is not valid YAML: {0}")]
    Invalid(#[source] serde_saphyr::DeserializeError),

    /// A value could not be rendered as YAML frontmatter.
    #[error("frontmatter could not be rendered as YAML: {0}")]
    Unrenderable(#[source] serde_saphyr::SerializeError),
}

impl From<FrontmatterError> for Failure {
    fn from(error: FrontmatterError) -> Self {
        match error {
            FrontmatterError::UnterminatedFence { line } => {
                Failure::failed("frontmatter.unterminated_fence", error.to_string())
                    .expected("a closing `---` fence")
                    .actual(format!("no closing fence after line {line}"))
            }
            FrontmatterError::Invalid(_) => {
                Failure::failed("frontmatter.invalid_yaml", error.to_string())
            }
            FrontmatterError::Unrenderable(_) => {
                Failure::failed("frontmatter.unrenderable", error.to_string())
            }
        }
    }
}

/// The two parts of a document: the frontmatter block, if any, and the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split<'a> {
    /// The text between the fences, not including either `---` line or its
    /// terminator. `None` when the input has no opening fence at all.
    pub frontmatter: Option<&'a str>,
    /// Everything after the closing fence's line terminator, or the entire
    /// input when there is no frontmatter. Byte-for-byte identical to the
    /// corresponding slice of `source` — never reflowed, never renormalised.
    pub body: &'a str,
}

/// Separate the frontmatter block from the body.
///
/// No frontmatter is normal, not an error: `Split { frontmatter: None, body:
/// source }`. An opening fence with no matching close is a hard error.
pub fn split(source: &str) -> Result<Split<'_>, FrontmatterError> {
    let Some((first_line, after_first_line)) = next_line(source, 0) else {
        // Empty input: nothing to open, so it is all (empty) body.
        return Ok(Split {
            frontmatter: None,
            body: source,
        });
    };

    if first_line != FENCE {
        return Ok(Split {
            frontmatter: None,
            body: source,
        });
    }

    let block_start = after_first_line;
    let mut cursor = block_start;
    loop {
        let Some((line, after_line)) = next_line(source, cursor) else {
            return Err(FrontmatterError::UnterminatedFence { line: 1 });
        };

        if line == FENCE {
            let frontmatter = source.get(block_start..cursor).unwrap_or_default();
            let body = source.get(after_line..).unwrap_or_default();
            return Ok(Split {
                frontmatter: Some(frontmatter),
                body,
            });
        }

        cursor = after_line;
    }
}

/// Split, then deserialize the frontmatter block into `T`.
///
/// No frontmatter and an empty frontmatter block are indistinguishable here —
/// both feed an empty YAML document to the deserializer, which succeeds only if
/// `T` accepts an empty mapping (every field defaultable, or `T` is itself
/// something like a map type).
pub fn parse<T: DeserializeOwned>(source: &str) -> Result<T, FrontmatterError> {
    let split = split(source)?;
    let block = split.frontmatter.unwrap_or_default();
    serde_saphyr::from_str(block).map_err(FrontmatterError::Invalid)
}

/// Re-emit `source` with its frontmatter replaced by `new_frontmatter`.
///
/// The body is untouched, byte-for-byte: whatever followed the original closing
/// fence (or the entire input, if there was no frontmatter) is copied through
/// unchanged, including its line endings and trailing-newline state.
pub fn replace<T: Serialize>(
    source: &str,
    new_frontmatter: &T,
) -> Result<String, FrontmatterError> {
    let split = split(source)?;
    let yaml = serde_saphyr::to_string(new_frontmatter).map_err(FrontmatterError::Unrenderable)?;

    let mut out = String::with_capacity(yaml.len() + split.body.len() + 8);
    out.push_str(FENCE);
    out.push('\n');
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(FENCE);
    out.push('\n');
    out.push_str(split.body);
    Ok(out)
}

/// Returns `(line, offset_of_next_line)` for the line starting at byte offset
/// `from`, where `line` has its terminator (`\n`, or `\r\n`) stripped, and
/// `offset_of_next_line` points just past that terminator — or past the line
/// itself, if it is the last line and has no terminator.
///
/// `None` means `from` is at or past the end of `source`: there is no line
/// there.
fn next_line(source: &str, from: usize) -> Option<(&str, usize)> {
    let rest = source.get(from..)?;
    if rest.is_empty() {
        return None;
    }

    match rest.find('\n') {
        Some(newline_index) => {
            let raw = rest.get(..newline_index)?;
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            Some((line, from + newline_index + 1))
        }
        None => Some((rest, from + rest.len())),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
    #[serde(default)]
    struct Doc {
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        closed_at: Option<String>,
    }

    #[test]
    fn no_frontmatter_is_normal_not_an_error() {
        let source = "just a plain body\nwith two lines\n";
        let split = split(source).unwrap();
        assert_eq!(split.frontmatter, None);
        assert_eq!(split.body, source);
    }

    #[test]
    fn empty_input_is_normal_too() {
        let split = split("").unwrap();
        assert_eq!(split.frontmatter, None);
        assert_eq!(split.body, "");
    }

    #[test]
    fn fence_must_be_at_the_very_start() {
        // A `---` line that isn't the first line of the document is body, not a
        // fence — nothing opened it.
        let source = "intro\n---\nkey: value\n---\nbody\n";
        let split = split(source).unwrap();
        assert_eq!(split.frontmatter, None);
        assert_eq!(split.body, source);
    }

    #[test]
    fn a_line_with_extra_dashes_is_not_a_fence() {
        let source = "----\nnot frontmatter\n";
        let split = split(source).unwrap();
        assert_eq!(split.frontmatter, None);
        assert_eq!(split.body, source);
    }

    #[test]
    fn splits_frontmatter_from_body() {
        let source = "---\noutcome: shipped\n---\nbody line one\nbody line two\n";
        let split = split(source).unwrap();
        assert_eq!(split.frontmatter, Some("outcome: shipped\n"));
        assert_eq!(split.body, "body line one\nbody line two\n");
    }

    #[test]
    fn dashes_inside_the_body_are_not_a_terminator_once_closed() {
        let source = "---\noutcome: shipped\n---\nbody\n---\nmore body\n";
        let split = split(source).unwrap();
        assert_eq!(split.frontmatter, Some("outcome: shipped\n"));
        assert_eq!(split.body, "body\n---\nmore body\n");
    }

    #[test]
    fn crlf_input_survives_with_body_unchanged() {
        let source = "---\r\noutcome: shipped\r\n---\r\nbody line\r\nsecond line\r\n";
        let split = split(source).unwrap();
        assert_eq!(split.frontmatter, Some("outcome: shipped\r\n"));
        assert_eq!(split.body, "body line\r\nsecond line\r\n");
    }

    #[test]
    fn unterminated_opening_fence_is_a_hard_error_naming_the_line() {
        let source = "---\noutcome: shipped\nno closing fence\n";
        let error = split(source).unwrap_err();
        match error {
            FrontmatterError::UnterminatedFence { line } => assert_eq!(line, 1),
            other => panic!("expected UnterminatedFence, got {other:?}"),
        }
    }

    #[test]
    fn unterminated_fence_with_nothing_after_it_is_also_an_error() {
        let error = split("---\n").unwrap_err();
        assert!(matches!(error, FrontmatterError::UnterminatedFence { .. }));

        let error = split("---").unwrap_err();
        assert!(matches!(error, FrontmatterError::UnterminatedFence { .. }));
    }

    #[test]
    fn empty_frontmatter_block_is_valid_and_deserializes_as_empty_mapping() {
        let split = split("---\n---\nbody\n").unwrap();
        assert_eq!(split.frontmatter, Some(""));
        assert_eq!(split.body, "body\n");

        let doc: Doc = parse("---\n---\nbody\n").unwrap();
        assert_eq!(doc, Doc::default());
    }

    #[test]
    fn no_frontmatter_also_deserializes_as_empty_mapping() {
        let doc: Doc = parse("no frontmatter here\n").unwrap();
        assert_eq!(doc, Doc::default());
    }

    #[test]
    fn parse_deserializes_the_block() {
        let source = "---\noutcome: shipped\nclosed_at: '2026-08-06'\n---\nbody\n";
        let doc: Doc = parse(source).unwrap();
        assert_eq!(
            doc,
            Doc {
                outcome: Some("shipped".to_owned()),
                closed_at: Some("2026-08-06".to_owned()),
            }
        );
    }

    #[test]
    fn parse_surfaces_invalid_yaml() {
        // An unclosed flow mapping is invalid YAML, not a Rust-level shape
        // mismatch, so this should come back through the `Invalid` variant.
        let source = "---\noutcome: [shipped\n---\nbody\n";
        let error = parse::<Doc>(source).unwrap_err();
        assert!(matches!(error, FrontmatterError::Invalid(_)));
    }

    #[test]
    fn replace_swaps_frontmatter_and_leaves_the_body_byte_for_byte() {
        let source = "---\noutcome: pending\n---\nA human wrote this.\nDo not touch it.\n";
        let updated = Doc {
            outcome: Some("shipped".to_owned()),
            closed_at: Some("2026-08-06".to_owned()),
        };

        let result = replace(source, &updated).unwrap();

        let split_before = split(source).unwrap();
        let split_after = split(&result).unwrap();
        assert_eq!(split_after.body, split_before.body);
        assert_eq!(split_after.body, "A human wrote this.\nDo not touch it.\n");

        let round_tripped: Doc = parse(&result).unwrap();
        assert_eq!(round_tripped, updated);
    }

    #[test]
    fn replace_on_a_document_with_no_frontmatter_prepends_a_fresh_block() {
        let source = "no frontmatter at all\njust body\n";
        let updated = Doc {
            outcome: Some("shipped".to_owned()),
            closed_at: None,
        };

        let result = replace(source, &updated).unwrap();
        let split = split(&result).unwrap();
        assert_eq!(split.body, source);

        let round_tripped: Doc = parse(&result).unwrap();
        assert_eq!(round_tripped, updated);
    }

    #[test]
    fn replace_preserves_a_body_with_no_trailing_newline() {
        let source = "---\noutcome: pending\n---\nbody with no trailing newline";
        let updated = Doc::default();

        let result = replace(source, &updated).unwrap();
        let split = split(&result).unwrap();
        assert_eq!(split.body, "body with no trailing newline");
    }

    #[test]
    fn replace_preserves_a_crlf_body() {
        let source = "---\r\noutcome: pending\r\n---\r\nline one\r\nline two\r\n";
        let updated = Doc {
            outcome: Some("shipped".to_owned()),
            closed_at: None,
        };

        let result = replace(source, &updated).unwrap();
        let split = split(&result).unwrap();
        assert_eq!(split.body, "line one\r\nline two\r\n");
    }

    #[test]
    fn replace_surfaces_unterminated_fences_in_the_source() {
        let source = "---\nno closing fence\n";
        let error = replace(source, &Doc::default()).unwrap_err();
        assert!(matches!(error, FrontmatterError::UnterminatedFence { .. }));
    }
}
