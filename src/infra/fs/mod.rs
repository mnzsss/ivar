//! Filesystem primitives: reads, writes, directories, symlinks, and the
//! read-only guard. Everything below this facade is implementation detail —
//! callers use the reexports here.
//!
//! One rule: **only files under `infra/fs/` touch `std::fs`** for managed
//! filesystem operations. The three concerns live in three files — generic
//! I/O in `io`, symlink behaviour in `symlink`, and the read-only guard in
//! `guard` — and this facade owns the shared `Error` and its `Failure`
//! conversion.

mod guard;
mod io;
mod symlink;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, FixAction};

pub use guard::{chmod, clear_write_bits, restore_write_bits, unix_mode};
pub use io::{
    ensure_dir, exists, is_dir, is_file, prune_empty_parents, read_bytes, read_dir, read_text,
    remove_file, remove_path, rename, stat, write_atomic, write_bytes, write_text,
};
pub use symlink::{
    SymlinkTarget, create_symlink, read_symlink, replace_symlink, replace_symlink_if_changed,
};

/// Everything that can go wrong touching the filesystem.
///
/// Variants are named after what broke, not wrapped in a single opaque
/// catch-all — see `ARCHITECTURE.md` on the error model.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A path (or a directory entry, or a symlink target) was not valid UTF-8.
    #[error("path is not valid UTF-8: {display}")]
    NotUtf8 {
        /// Lossy rendering of the offending path, for the human message only.
        display: String,
    },
    #[error("could not read {path}")]
    Read {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {path}")]
    Write {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not create directory {path}")]
    CreateDir {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read directory {path}")]
    ReadDir {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read metadata for {path}")]
    Metadata {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not remove {path}")]
    Remove {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not rename {from} to {to}")]
    Rename {
        from: Utf8PathBuf,
        to: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not create symlink {link} -> {target}")]
    Symlink {
        link: Utf8PathBuf,
        target: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read symlink {path}")]
    ReadSymlink {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not change permissions on {path}")]
    Chmod {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::NotUtf8 { display } => {
                Failure::blocked("fs.not_utf8", format!("path is not valid UTF-8: {display}")).fix(
                    FixAction::unsafe_("fs.rename_to_utf8", "rename the path to valid UTF-8"),
                )
            }
            Error::Read { path, source } => {
                Failure::blocked("fs.read_failed", format!("could not read {path}: {source}"))
                    .expected("a readable file")
                    .actual(source.to_string())
            }
            Error::ReadDir { path, source } => Failure::blocked(
                "fs.read_dir_failed",
                format!("could not read directory {path}: {source}"),
            )
            .actual(source.to_string()),
            Error::Metadata { path, source } => Failure::blocked(
                "fs.metadata_failed",
                format!("could not read metadata for {path}: {source}"),
            )
            .actual(source.to_string()),
            Error::ReadSymlink { path, source } => Failure::blocked(
                "fs.read_symlink_failed",
                format!("could not read symlink {path}: {source}"),
            )
            .actual(source.to_string()),
            Error::Write { path, source } => Failure::failed(
                "fs.write_failed",
                format!("could not write {path}: {source}"),
            )
            .actual(source.to_string()),
            Error::CreateDir { path, source } => Failure::failed(
                "fs.create_dir_failed",
                format!("could not create directory {path}: {source}"),
            )
            .actual(source.to_string()),
            Error::Remove { path, source } => Failure::failed(
                "fs.remove_failed",
                format!("could not remove {path}: {source}"),
            )
            .actual(source.to_string()),
            Error::Rename { from, to, source } => Failure::failed(
                "fs.rename_failed",
                format!("could not rename {from} to {to}: {source}"),
            )
            .actual(source.to_string()),
            Error::Symlink {
                link,
                target,
                source,
            } => Failure::failed(
                "fs.symlink_failed",
                format!("could not create symlink {link} -> {target}: {source}"),
            )
            .actual(source.to_string()),
            Error::Chmod { path, source } => Failure::failed(
                "fs.chmod_failed",
                format!("could not change permissions on {path}: {source}"),
            )
            .actual(source.to_string()),
        }
    }
}

pub(super) fn is_not_found(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

/// Whether `error` is the transient APFS race documented on [`read_symlink`]:
/// `readlink` returning `EINVAL` immediately after `lstat` already confirmed
/// the entry is a symlink.
///
/// `std::io::ErrorKind::InvalidInput` is the portable stand-in for `EINVAL` here
/// (Rust does not expose a dedicated `ErrorKind` for it). Calling this is only
/// sound *after* `lstat` has ruled out the permanent cause of `EINVAL` — "this
/// path is not a symlink" — which is why [`read_symlink`] gates on `lstat`
/// first rather than calling this from a bare `readlink` result. An embedded
/// NUL byte in the path would also produce this `ErrorKind`, but `lstat`
/// already rejects that before this function is ever reached.
pub(super) fn is_transient_not_a_symlink(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::InvalidInput
}

pub(super) fn not_utf8(path: std::path::PathBuf) -> Error {
    Error::NotUtf8 {
        display: path.to_string_lossy().into_owned(),
    }
}

/// A unique sibling path in the same directory as `path`, for the
/// write-to-temp-then-rename dance. Never collides across concurrent callers —
/// suffixed with a fresh UUID, not a counter — and never leaves the containing
/// directory, so the final `rename` is guaranteed to stay on one filesystem.
pub(super) fn sibling_temp_path(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("file");
    let unique = uuid::Uuid::new_v4();
    let temp_name = format!(".{file_name}.{unique}.tmp");
    match path.parent() {
        Some(parent) => parent.join(temp_name),
        None => Utf8PathBuf::from(temp_name),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/infra/fs.rs"]
mod tests;
