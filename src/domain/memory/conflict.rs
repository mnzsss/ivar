//! Conflict preservation logic for shared memory topics.

use crate::domain::memory::ScopeName;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use camino::Utf8PathBuf;

/// Result of topic conflict preservation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictResolution {
    /// Files preserved/created as part of conflict resolution.
    pub preserved_files: Vec<Utf8PathBuf>,
    /// Whether user action is required to resolve conflicting topic files.
    pub requires_user_action: bool,
}

/// Preserve topic conflict if incoming content differs from existing topic content.
///
/// If `topic_path` does not exist:
/// - Write `incoming_content` to `topic_path`.
/// - Return `ConflictResolution { preserved_files: vec![topic_path], requires_user_action: false }`.
///
/// If `topic_path` exists:
/// - If existing text equals `incoming_content`, return `requires_user_action: false`.
/// - If contents differ, write `incoming_content` to `<slug>.conflict-<timestamp>.md` and return `requires_user_action: true`.
pub fn preserve_topic_conflict(
    layout: &Layout,
    scope: &ScopeName,
    slug: &str,
    incoming_content: &str,
) -> Result<ConflictResolution, Failure> {
    let topic_path = layout.memory_topic(scope, slug);
    let scope_dir = layout.memory_scope_dir(scope);
    fs::ensure_dir(&scope_dir)?;

    if !fs::exists(&topic_path)? {
        fs::write_text(&topic_path, incoming_content)?;
        return Ok(ConflictResolution {
            preserved_files: vec![topic_path],
            requires_user_action: false,
        });
    }

    let existing = fs::read_text(&topic_path)?.unwrap_or_default();
    if existing == incoming_content {
        return Ok(ConflictResolution {
            preserved_files: vec![topic_path],
            requires_user_action: false,
        });
    }

    let ts = crate::domain::session::rfc3339_now().replace(':', "-");
    let clean_slug = slug.strip_suffix(".md").unwrap_or(slug);
    let conflict_filename = format!("{clean_slug}.conflict-{ts}.md");
    let conflict_path = scope_dir.join(conflict_filename);

    fs::write_text(&conflict_path, incoming_content)?;

    Ok(ConflictResolution {
        preserved_files: vec![topic_path, conflict_path],
        requires_user_action: true,
    })
}

/// Scan `layout.memory_dir()` for any pending conflict files (`*.conflict-*.md`).
pub fn list_pending_conflicts(layout: &Layout) -> Result<Vec<Utf8PathBuf>, Failure> {
    let memory_dir = layout.memory_dir();
    if !fs::exists(&memory_dir)? {
        return Ok(Vec::new());
    }

    let mut conflicts = Vec::new();
    let entries = fs::read_dir(&memory_dir)?;
    for entry in entries {
        if fs::is_dir(&entry)? {
            let sub_entries = fs::read_dir(&entry)?;
            for sub in sub_entries {
                let file_name = sub.file_name().unwrap_or_default();
                if file_name.contains(".conflict-") && file_name.ends_with(".md") {
                    conflicts.push(sub);
                }
            }
        } else {
            let file_name = entry.file_name().unwrap_or_default();
            if file_name.contains(".conflict-") && file_name.ends_with(".md") {
                conflicts.push(entry);
            }
        }
    }

    conflicts.sort();
    Ok(conflicts)
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/conflict.rs"]
mod tests;
