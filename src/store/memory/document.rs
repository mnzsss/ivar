//! Read, write, and delete canonical topic markdown documents.

use crate::domain::memory::ScopeName;
use crate::domain::memory::topic::{MemoryTopic, TopicMetadata};
use crate::error::{Failure, FixAction};
use crate::infra::frontmatter;
use crate::infra::fs::{self, SymlinkTarget};
use crate::store::layout::Layout;

/// Read a canonical topic from disk.
pub fn read_topic(
    layout: &Layout,
    scope: &ScopeName,
    slug: &str,
) -> Result<MemoryTopic, Failure> {
    validate_slug(slug)?;
    let path = layout.memory_topic(scope, slug);

    match fs::read_symlink(&path) {
        Ok(SymlinkTarget::Target(target)) => {
            return Err(
                Failure::blocked(
                    "memory_topic.symlink_rejected",
                    format!("memory topic at '{path}' is a symlink to '{target}'"),
                )
                .fix(FixAction::unsafe_(
                    "replace_symlink",
                    "Replace the symlink with a canonical regular file.",
                )),
            );
        }
        Ok(SymlinkTarget::Absent) => {
            return Err(
                Failure::blocked(
                    "memory_topic.not_found",
                    format!("memory topic '{slug}' does not exist in scope '{scope}'"),
                )
                .fix(FixAction::safe(
                    "create_topic",
                    "Create the topic before reading.",
                )),
            );
        }
        Ok(SymlinkTarget::NotASymlink) => {}
        Err(err) => {
            return Err(Failure::failed(
                "memory_topic.read_error",
                format!("failed to check topic symlink at '{path}': {err}"),
            ));
        }
    }

    let text = match fs::read_text(&path) {
        Ok(Some(text)) => text,
        Ok(None) => {
            return Err(
                Failure::blocked(
                    "memory_topic.not_found",
                    format!("memory topic '{slug}' does not exist in scope '{scope}'"),
                )
                .fix(FixAction::safe(
                    "create_topic",
                    "Create the topic before reading.",
                )),
            );
        }
        Err(err) => {
            return Err(Failure::failed(
                "memory_topic.read_error",
                format!("failed to read topic at '{path}': {err}"),
            ));
        }
    };

    let split = frontmatter::split(&text).map_err(|err| {
        Failure::blocked(
            "memory_topic.invalid_frontmatter",
            format!("invalid frontmatter in topic '{path}': {err}"),
        )
        .fix(FixAction::safe(
            "fix_frontmatter",
            "Fix YAML frontmatter syntax.",
        ))
    })?;

    if split.frontmatter.is_none() {
        return Err(
            Failure::blocked(
                "memory_topic.missing_frontmatter",
                format!("missing frontmatter in topic '{path}'"),
            )
            .fix(FixAction::safe(
                "add_frontmatter",
                "Add frontmatter header delimited by '---'.",
            )),
        );
    }

    let metadata: TopicMetadata = frontmatter::parse(&text).map_err(|err| {
        Failure::blocked(
            "memory_topic.invalid_metadata",
            format!("invalid topic metadata in '{path}': {err}"),
        )
        .fix(FixAction::safe(
            "correct_metadata",
            "Correct topic metadata fields in frontmatter.",
        ))
    })?;

    let body = split.body.to_string();

    Ok(MemoryTopic {
        metadata,
        content: body,
    })
}

/// Write a canonical topic to disk atomically.
pub fn write_topic(
    layout: &Layout,
    scope: &ScopeName,
    slug: &str,
    topic: &MemoryTopic,
) -> Result<(), Failure> {
    validate_slug(slug)?;
    let path = layout.memory_topic(scope, slug);

    match fs::read_symlink(&path) {
        Ok(SymlinkTarget::Target(target)) => {
            return Err(
                Failure::blocked(
                    "memory_topic.symlink_rejected",
                    format!("memory topic at '{path}' is a symlink to '{target}'"),
                )
                .fix(FixAction::unsafe_(
                    "refuse_symlink",
                    "Refusing to write to a symlink.",
                )),
            );
        }
        Ok(SymlinkTarget::Absent | SymlinkTarget::NotASymlink) => {}
        Err(err) => {
            return Err(Failure::failed(
                "memory_topic.write_error",
                format!("failed to inspect topic path '{path}': {err}"),
            ));
        }
    }

    let scope_dir = layout.memory_scope_dir(scope);
    fs::ensure_dir(&scope_dir).map_err(|err| {
        Failure::failed(
            "memory_topic.directory_creation_failed",
            format!("failed to create scope directory '{scope_dir}': {err}"),
        )
    })?;

    let rendered = frontmatter::replace(&topic.content, &topic.metadata).map_err(|err| {
        Failure::failed(
            "memory_topic.serialization_failed",
            format!("failed to serialize topic frontmatter: {err}"),
        )
    })?;

    fs::write_atomic(&path, rendered.as_bytes()).map_err(|err| {
        Failure::failed(
            "memory_topic.atomic_write_failed",
            format!("failed to atomically write topic at '{path}': {err}"),
        )
    })?;

    Ok(())
}

/// Delete a canonical topic from disk.
pub fn delete_topic(
    layout: &Layout,
    scope: &ScopeName,
    slug: &str,
) -> Result<(), Failure> {
    validate_slug(slug)?;
    let path = layout.memory_topic(scope, slug);

    match fs::read_symlink(&path) {
        Ok(SymlinkTarget::Target(target)) => {
            return Err(
                Failure::blocked(
                    "memory_topic.symlink_rejected",
                    format!("memory topic at '{path}' is a symlink to '{target}'"),
                )
                .fix(FixAction::unsafe_(
                    "refuse_symlink_delete",
                    "Refusing to delete a symlink blindly.",
                )),
            );
        }
        Ok(SymlinkTarget::Absent) => {
            // Deleting a non-existent file is success
            return Ok(());
        }
        Ok(SymlinkTarget::NotASymlink) => {}
        Err(err) => {
            return Err(Failure::failed(
                "memory_topic.delete_error",
                format!("failed to inspect topic path '{path}': {err}"),
            ));
        }
    }

    fs::remove_file(&path).map_err(|err| {
        Failure::failed(
            "memory_topic.delete_failed",
            format!("failed to remove topic file at '{path}': {err}"),
        )
    })?;

    Ok(())
}

fn validate_slug(slug: &str) -> Result<(), Failure> {
    if slug.is_empty() {
        return Err(Failure::blocked(
            "memory_topic.empty_slug",
            "topic slug must not be empty",
        ));
    }
    if slug.contains("..") || slug.contains('/') || slug.contains('\\') || slug.starts_with('/') {
        return Err(Failure::blocked(
            "memory_topic.invalid_slug",
            format!("invalid topic slug '{slug}': path traversal characters are forbidden"),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/document.rs"]
mod tests;
