//! Renderers for skill materialisation — symlinks and copies.
//!
//! This module owns the I/O operations that turn a sync plan [`Step`] into
//! actual filesystem changes. Each renderer is a pure function from step →
//! result, with no side effects beyond what the step requests.
//!
//! # Contract
//!
//! - [`render_create`] — materialise a skill at its target path using the
//!   mode declared in the step (symlink or copy).
//! - [`render_remove`] — tear down a materialised skill at its target path.
//! - [`verify_status`] — inspect what is currently at a target path and
//!   return a [`MaterialStatus`]. Used by the planner before deciding
//!   which action to take.

use std::error::Error as StdError;

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::skill::RenderMode;
use crate::domain::skill_sync::MaterialStatus;
use crate::error::Failure;
use crate::infra::fs;

/// Everything that can go wrong during rendering.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A symlink operation failed.
    #[error("could not create symlink")]
    Symlink {
        link: Utf8PathBuf,
        target: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A file/directory copy operation failed.
    #[error("could not copy files")]
    Copy {
        source: Utf8PathBuf,
        target: Utf8PathBuf,
        #[source]
        source_io: std::io::Error,
    },

    /// A removal operation failed.
    #[error("could not remove path")]
    Remove {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Symlink {
                link,
                target,
                source,
            } => Failure::failed(
                "render.symlink_failed",
                format!("could not create symlink {link} -> {target}: {source}"),
            )
            .actual(source.to_string()),
            Error::Copy {
                source,
                target,
                source_io,
            } => Failure::failed(
                "render.copy_failed",
                format!("could not copy {source} -> {target}: {source_io}"),
            )
            .actual(source_io.to_string()),
            Error::Remove { path, source } => Failure::failed(
                "render.remove_failed",
                format!("could not remove {path}: {source}"),
            )
            .actual(source.to_string()),
        }
    }
}

/// Create or update a skill materialisation at `step.target` pointing at
/// `step.source`.
///
/// For [`RenderMode::Symlink`], creates a symlink (or replaces an existing one).
/// For [`RenderMode::Copy`], copies the source directory contents to the target.
///
/// Returns `Ok(())` on success. The caller should already have verified that
/// the target does not exist (for Create) or that an Update is safe to apply.
pub fn render(step: &crate::domain::skill_sync::Step) -> Result<(), Error> {
    match step.mode {
        RenderMode::Symlink => render_symlink(step),
        RenderMode::Copy => render_copy(step),
    }
}

/// The `io::Error` underlying an `fs::Error` — every variant with an I/O
/// source carries one. `NotUtf8` has none, so the message is used instead.
fn io_of(error: &fs::Error) -> std::io::Error {
    error
        .source()
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .map(|io| std::io::Error::new(io.kind(), io.to_string()))
        .unwrap_or_else(|| std::io::Error::other(error.to_string()))
}

/// Remove a skill materialisation at `step.target`.
///
/// Works for both symlink and copy targets — it removes whatever is at the
/// path without checking how it was created.
pub fn remove(step: &crate::domain::skill_sync::Step) -> Result<(), Error> {
    fs::remove_path(&step.target).map_err(|e| {
        let io_error = io_of(&e);
        Error::Remove {
            path: step.target.clone(),
            source: io_error,
        }
    })?;
    Ok(())
}

/// Inspect what is at `path` and return the current [`MaterialStatus`].
///
/// This is the planner's eyes on disk: it answers "should I create, update,
/// or is everything fine?" without mutating anything.
pub fn verify_status(path: &Utf8Path, expected_target: &Utf8Path) -> MaterialStatus {
    // Check if path exists at all.
    if !fs::exists(path).unwrap_or(false) {
        return MaterialStatus::Missing;
    }

    // Path exists — check if it's a symlink.
    let symlink_target = match fs::read_symlink(path) {
        Ok(target) => target,
        Err(_) => return MaterialStatus::NotLink,
    };

    match symlink_target {
        fs::SymlinkTarget::Absent => MaterialStatus::Missing,
        fs::SymlinkTarget::NotASymlink => MaterialStatus::NotLink,
        fs::SymlinkTarget::Target(link_target) => {
            // Symlink exists — check if it points to the right place.
            if link_target == *expected_target {
                MaterialStatus::Ok
            } else {
                // Check if the link is broken (target doesn't exist).
                if !fs::exists(&link_target).unwrap_or(false) {
                    MaterialStatus::BrokenSymlink
                } else {
                    MaterialStatus::WrongLink
                }
            }
        }
    }
}

fn render_symlink(step: &crate::domain::skill_sync::Step) -> Result<(), Error> {
    // Ensure parent directory exists.
    if let Some(parent) = step.target.parent() {
        fs::ensure_dir(parent).map_err(|e| {
            let io_error = io_of(&e);
            Error::Symlink {
                link: step.target.clone(),
                target: step.source.clone(),
                source: io_error,
            }
        })?;
    }

    fs::replace_symlink(&step.source, &step.target).map_err(|e| {
        let io_error = io_of(&e);
        Error::Symlink {
            link: step.target.clone(),
            target: step.source.clone(),
            source: io_error,
        }
    })
}

fn render_copy(step: &crate::domain::skill_sync::Step) -> Result<(), Error> {
    use walkdir::WalkDir;

    // Ensure parent directory exists.
    if let Some(parent) = step.target.parent() {
        fs::ensure_dir(parent).map_err(|e| {
            let io_error = io_of(&e);
            Error::Copy {
                source: step.source.clone(),
                target: step.target.clone(),
                source_io: io_error,
            }
        })?;
    }

    // Walk the source directory and copy each file.
    let entries = WalkDir::new(&step.source)
        .into_iter()
        .filter_map(|e| e.ok());

    for entry in entries {
        let entry_path = entry.path();
        let relative = entry_path.strip_prefix(&step.source).unwrap_or(entry_path);
        let dest = step.target.join(
            Utf8Path::from_path(relative)
                .unwrap_or_else(|| Utf8Path::new(relative.to_str().unwrap_or("unknown"))),
        );

        if let Some(parent) = dest.parent() {
            let _ = fs::ensure_dir(parent);
        }

        if entry.file_type().is_dir() {
            continue;
        }

        let Some(utf8_entry) = Utf8Path::from_path(entry_path).map(Utf8Path::to_path_buf) else {
            return Err(Error::Copy {
                source: step.source.clone(),
                target: step.target.clone(),
                source_io: std::io::Error::other("path is not valid UTF-8"),
            });
        };
        if let Ok(Some(contents)) = fs::read_bytes(&utf8_entry) {
            if let Err(e) = fs::write_bytes(&dest, &contents) {
                let io_error = io_of(&e);
                return Err(Error::Copy {
                    source: step.source.clone(),
                    target: step.target.clone(),
                    source_io: io_error,
                });
            }
        }
    }

    Ok(())
}
