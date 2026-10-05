//! Storage, parsing, and rendering for feedback docs in `.ivar/feedback/`.

use crate::domain::feedback::{FeedbackEntry, FeedbackStatus, Frontmatter, entry_id, next_seq};
use crate::error::Failure;
use crate::infra::{frontmatter, fs};
use crate::store::layout::Layout;

/// Parse a feedback document. Never fails: unparseable frontmatter becomes
/// status Unknown with the full source retained as body.
#[must_use]
pub fn parse(id: &str, source: &str) -> FeedbackEntry {
    let unknown = || FeedbackEntry {
        id: id.to_owned(),
        frontmatter: Frontmatter {
            title: id.to_owned(),
            kind: Default::default(),
            status: FeedbackStatus::Unknown,
            created_at: String::new(),
            ivar_version: String::new(),
            os: String::new(),
            arch: String::new(),
            provider: None,
            session: None,
            feature: None,
            published_url: None,
            extra: Default::default(),
        },
        body: source.to_owned(),
    };

    let Ok(split) = frontmatter::split(source) else {
        return unknown();
    };
    if split.frontmatter.is_none() {
        return unknown();
    }
    let Ok(parsed) = frontmatter::parse::<Frontmatter>(source) else {
        return unknown();
    };
    if parsed.status == FeedbackStatus::Unknown {
        return unknown();
    }

    FeedbackEntry {
        id: id.to_owned(),
        frontmatter: parsed,
        body: split.body.to_owned(),
    }
}

/// Render a feedback entry back to markdown with frontmatter.
/// # Errors
///
/// Returns [`Failure`] if the entry status is Unknown or YAML serialisation fails.
pub fn render(entry: &FeedbackEntry) -> Result<String, Failure> {
    if !entry.is_writable() {
        return Err(Failure::blocked(
            "feedback.unwritable",
            "cannot re-render a feedback doc whose front matter failed to parse",
        ));
    }

    frontmatter::replace(&entry.body, &entry.frontmatter).map_err(Into::into)
}

/// List all feedback entries in the hall, newest first by ID sequence descending.
/// # Errors
///
/// Returns [`Failure`] on I/O read failure.
pub fn list(layout: &Layout) -> Result<Vec<FeedbackEntry>, Failure> {
    let dir = layout.feedback_dir();
    if !fs::exists(&dir)? {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&dir)?;
    let mut docs = Vec::new();

    for path in entries {
        if path.extension() == Some("md")
            && let Some(stem) = path.file_stem()
            && let Some(content) = fs::read_text(&path)?
        {
            docs.push(parse(stem, &content));
        }
    }

    docs.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(docs)
}

/// Read a single feedback entry by ID.
/// # Errors
///
/// Returns [`Failure`] on I/O read failure.
pub fn read(layout: &Layout, id: &str) -> Result<Option<FeedbackEntry>, Failure> {
    let doc_path = layout.feedback_doc(id);
    match fs::read_text(&doc_path)? {
        Some(content) => Ok(Some(parse(id, &content))),
        None => Ok(None),
    }
}

/// Create a new feedback entry atomically using create_new semantics.
///
/// Allocates the next numeric sequence, calls `build` with the assigned ID,
/// and retries with the next sequence number if collision occurs.
/// # Errors
///
/// Returns [`Failure`] on I/O creation or rendering failure.
pub fn create(
    layout: &Layout,
    title: &str,
    build: impl Fn(&str) -> FeedbackEntry,
) -> Result<FeedbackEntry, Failure> {
    let dir = layout.feedback_dir();
    fs::ensure_dir(&dir)?;

    let existing = list(layout)?;
    let mut seq = next_seq(existing.iter().map(|e| e.id.as_str()));

    loop {
        let id = entry_id(seq, title);
        let entry = build(&id);
        let content = render(&entry)?;
        let doc_path = layout.feedback_doc(&id);

        match fs::create_new_text(&doc_path, &content) {
            Ok(()) => return Ok(entry),
            Err(fs::Error::Write { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                seq += 1;
                continue;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

/// Atomically write an updated feedback entry to disk.
/// # Errors
///
/// Returns [`Failure`] on serialization or atomic write failure.
pub fn write(layout: &Layout, entry: &FeedbackEntry) -> Result<(), Failure> {
    let content = render(entry)?;
    let doc_path = layout.feedback_doc(&entry.id);
    fs::write_atomic(&doc_path, content.as_bytes()).map_err(Into::into)
}

#[cfg(test)]
#[path = "../../tests/unit/store/feedback.rs"]
mod tests;
