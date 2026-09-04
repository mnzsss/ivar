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

pub use guard::{LiftedGuard, chmod, clear_write_bits, restore_write_bits, unix_mode};
pub use io::{
    TempDir, copy_dir, ensure_dir, exists, is_dir, is_file, prune_empty_parents, read_bytes,
    read_dir, read_text, remove_file, remove_path, rename, stat, write_atomic, write_bytes,
    write_sensitive_atomic, write_text,
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

/// Resolve the base directory for ivar's persistent data, following the
/// platform convention the provider uses for the same directory:
///
/// - Linux: `$XDG_DATA_HOME` (absolute) or `$HOME/.local/share`.
/// - macOS: `$XDG_DATA_HOME` (absolute) or `$HOME/Library/Application Support`.
/// - Windows: `$XDG_DATA_HOME` (absolute) or `%APPDATA%` or `%LOCALAPPDATA%`.
///
/// A relative or empty variable is treated as unset, not as an error — the
/// cascade falls through rather than failing outright. This never returns a
/// guessed path: if nothing resolves, the returned [`Failure`] names every
/// variable it looked for.
pub fn data_dir() -> Result<Utf8PathBuf, Failure> {
    data_dir_from(
        std::env::var("XDG_DATA_HOME").ok(),
        std::env::var("HOME").ok(),
        std::env::var("APPDATA").ok(),
        std::env::var("LOCALAPPDATA").ok(),
        std::env::consts::OS,
    )
}

/// The resolution cascade as a pure function, so the fallthrough rules and
/// the no-path failure are testable without mutating the process environment
/// (which races across concurrently-run tests). `os` is `std::env::consts::OS`.
fn data_dir_from(
    xdg_data_home: Option<String>,
    home: Option<String>,
    appdata: Option<String>,
    localappdata: Option<String>,
    os: &str,
) -> Result<Utf8PathBuf, Failure> {
    // `$XDG_DATA_HOME`, when set to an absolute path, wins on every platform —
    // it is the one variable the caller can use to override the convention.
    if let Some(path) = absolute_non_empty(xdg_data_home) {
        return Ok(path);
    }

    let home = home.filter(|h| !h.is_empty());
    match os {
        "windows" => {
            if let Some(path) = absolute_non_empty(appdata) {
                return Ok(path);
            }
            if let Some(path) = absolute_non_empty(localappdata) {
                return Ok(path);
            }
            Err(data_dir_failure())
        }
        "macos" => match home {
            Some(home) => Ok(Utf8PathBuf::from(home)
                .join("Library")
                .join("Application Support")),
            None => Err(data_dir_failure()),
        },
        _ => match home {
            Some(home) => Ok(Utf8PathBuf::from(home).join(".local").join("share")),
            None => Err(data_dir_failure()),
        },
    }
}

/// `value`, when it names a non-empty absolute path.
fn absolute_non_empty(value: Option<String>) -> Option<Utf8PathBuf> {
    let value = value?;
    let path = Utf8PathBuf::from(value);
    if path.as_str().is_empty() || !path.is_absolute() {
        return None;
    }
    Some(path)
}

/// The failure for "no platform data-directory variable resolved".
fn data_dir_failure() -> Failure {
    Failure::failed("fs.data_dir", "could not resolve a data directory")
        .expected(
            "$XDG_DATA_HOME set to an absolute path, $HOME set, or (on Windows) \
             $APPDATA/$LOCALAPPDATA set",
        )
        .actual("no platform data-directory variable resolved to an absolute path")
        .fix(FixAction::safe(
            "fs.set_data_dir",
            "Set $XDG_DATA_HOME (any platform), $HOME (Unix/macOS), or \
             $APPDATA/$LOCALAPPDATA (Windows) to an absolute path.",
        ))
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
