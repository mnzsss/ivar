//! A memory page file on disk: where a session page lives, its front matter,
//! and how a page is published.

use std::fmt::Write as _;

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::memory_page::PageMeta;

pub const SESSION_PAGES_DIR: &str = ".ivar/memory/session/";

#[must_use]
pub fn page_rel(native_session_id: &str) -> String {
    format!("{SESSION_PAGES_DIR}{}.md", page_stem(native_session_id))
}

#[must_use]
pub fn page_stem(native_session_id: &str) -> String {
    let safe = !native_session_id.is_empty()
        && native_session_id.len() <= 128
        && !native_session_id.starts_with('.')
        && native_session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if safe {
        native_session_id.to_owned()
    } else {
        crate::infra::hash::text(native_session_id)
            .chars()
            .take(16)
            .collect()
    }
}

fn temp_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(format!(
        ".{}-{:?}.tmp",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// Writes `content` through a temporary file in the same directory.
///
/// # Errors
///
/// When the file cannot be written, or `path` exists and `replace` is false.
pub fn publish(path: &Utf8Path, rel: &str, content: &str, replace: bool) -> Result<(), String> {
    let dir = path.parent().ok_or("page path has no directory")?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let temp = temp_path(dir);
    if let Err(error) = std::fs::write(&temp, content) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.to_string());
    }
    // hard_link fails with AlreadyExists instead of replacing, so a page another writer created first is never clobbered; rename would overwrite it.
    let placed = if replace {
        std::fs::rename(&temp, path)
    } else {
        std::fs::hard_link(&temp, path)
    };
    let _ = std::fs::remove_file(&temp);
    placed.map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            format!("`{rel}` already exists; pass `replace: true` to overwrite it")
        } else if replace {
            error.to_string()
        } else {
            format!("could not write `{rel}` without overwriting; the filesystem may not support hard links: {error}")
        }
    })
}

fn json_scalar(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

#[must_use]
pub fn render_meta(title: Option<&str>, meta: &PageMeta, body: &str) -> String {
    let mut out = String::from("---\n");
    if let Some(title) = title {
        let _ = writeln!(out, "title: {}", json_scalar(title));
    }
    if !meta.tags.is_empty() {
        let tags: Vec<String> = meta.tags.iter().map(|tag| json_scalar(tag)).collect();
        let _ = writeln!(out, "tags: [{}]", tags.join(", "));
    }
    if meta.pinned {
        out.push_str("pinned: true\n");
    }
    if !meta.relations.is_empty() {
        out.push_str("relations:\n");
        for relation in &meta.relations {
            let _ = writeln!(
                out,
                "  - type: {}\n    target: {}",
                json_scalar(&relation.kind),
                json_scalar(&relation.target)
            );
        }
    }
    if let Some(expires_at) = &meta.expires_at {
        let _ = writeln!(out, "expires_at: {}", json_scalar(expires_at));
    }
    if out == "---\n" {
        return body.to_owned();
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/page_file.rs"]
mod tests;
