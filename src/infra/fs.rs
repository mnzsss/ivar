//! The filesystem primitive set. Nothing else in the crate touches `std::fs`.
//!
//! This exists for the same reason its TypeScript predecessor did: filesystem
//! errors that do not name the path are almost useless in a tool that touches
//! hundreds of paths per run. `fs-err` supplies that; this module supplies the
//! rest of the vocabulary and the UTF-8 path discipline.
//!
//! # Contract
//!
//! Every path is a [`camino::Utf8Path`] — never `std::path::Path`. A path that is
//! not valid UTF-8 is rejected at the edge, once, rather than producing an
//! `Option` in every signature downstream.
//!
//! Primitives, mirroring what the orchestrator actually needs:
//!
//! - read/write text, read/write bytes
//! - `ensure_dir` (recursive, idempotent), `read_dir` (sorted, so callers are
//!   deterministic)
//! - `exists`, `is_file`, `is_dir`, `stat`
//! - `chmod`
//! - symlinks: `create_symlink`; `replace_symlink` (rename-based replace: a
//!   reader can never observe the link *missing*, though see its doc comment
//!   for a real platform race a concurrent reader can still hit *resolving*
//!   it); `replace_symlink_if_changed` (skips the replace, and the race
//!   window, when the target already matches — this is the one to reach for
//!   day to day, since the view dir is rebuilt on every session connect and
//!   that rebuild is usually a complete no-op); `read_symlink` (returns
//!   [`SymlinkTarget`], not `Option` — see its doc comment for why "present
//!   but not a symlink" needs to be a third, permanent answer rather than an
//!   error or a fold into "absent")
//! - `remove_file`, `remove_path` (recursive)
//! - `write_atomic` (temp sibling + rename), used by every state writer
//!
//! # The distinction that matters
//!
//! "Genuinely absent" is not an error. A missing optional config file returns
//! `Ok(None)`; a file that exists but cannot be read is a hard error. Discriminate
//! on [`std::io::ErrorKind::NotFound`] specifically — never by checking `exists`
//! first, which races.
//!
//! That same discrimination is applied to every *read-shaped* primitive here
//! (`read_text`, `read_bytes`, `stat`): absent is `Ok(None)`, broken is `Err`.
//! `read_symlink` applies it too, but through [`SymlinkTarget::Absent`] rather
//! than `Option`, because it has a third state to represent. `remove_file` and
//! `remove_path` go the other way — removing something already gone is
//! treated as success, because a caller tearing down a view dir should not
//! have to check existence first either.
//!
//! # Judgment calls not spelled out above
//!
//! - `write_text`/`write_bytes` are plain, non-atomic writes (they exist for
//!   things that do not need crash-safety, e.g. logs). `write_atomic` is the
//!   separate, deliberate primitive for state.
//! - Errors from a read-shaped operation (once past the "absent" check) render as
//!   [`crate::error::Status::Blocked`] — nothing was mutated, so retrying once the
//!   cause (usually permissions) is fixed is safe. Errors from a write-shaped
//!   operation render as [`crate::error::Status::Failed`].
//! - `chmod` takes raw Unix mode bits. There is no Windows target (the view dir is
//!   built from symlinks, which need Developer Mode there; see `ARCHITECTURE.md`),
//!   so this module does not carry a Windows fallback.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::error::{Failure, FixAction};

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
        source: io::Error,
    },
    #[error("could not write {path}")]
    Write {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not create directory {path}")]
    CreateDir {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read directory {path}")]
    ReadDir {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read metadata for {path}")]
    Metadata {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not remove {path}")]
    Remove {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not rename {from} to {to}")]
    Rename {
        from: Utf8PathBuf,
        to: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not create symlink {link} -> {target}")]
    Symlink {
        link: Utf8PathBuf,
        target: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read symlink {path}")]
    ReadSymlink {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not change permissions on {path}")]
    Chmod {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
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

fn is_not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
}

/// Whether `error` is the transient APFS race documented on [`read_symlink`]:
/// `readlink` returning `EINVAL` immediately after `lstat` already confirmed
/// the entry is a symlink.
///
/// `io::ErrorKind::InvalidInput` is the portable stand-in for `EINVAL` here
/// (Rust does not expose a dedicated `ErrorKind` for it). Calling this is only
/// sound *after* `lstat` has ruled out the permanent cause of `EINVAL` — "this
/// path is not a symlink" — which is why [`read_symlink`] gates on `lstat`
/// first rather than calling this from a bare `readlink` result. An embedded
/// NUL byte in the path would also produce this `ErrorKind`, but `lstat`
/// already rejects that before this function is ever reached.
fn is_transient_not_a_symlink(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::InvalidInput
}

fn not_utf8(path: std::path::PathBuf) -> Error {
    Error::NotUtf8 {
        display: path.to_string_lossy().into_owned(),
    }
}

/// A unique sibling path in the same directory as `path`, for the
/// write-to-temp-then-rename dance. Never collides across concurrent callers —
/// suffixed with a fresh UUID, not a counter — and never leaves the containing
/// directory, so the final `rename` is guaranteed to stay on one filesystem.
fn sibling_temp_path(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("file");
    let unique = uuid::Uuid::new_v4();
    let temp_name = format!(".{file_name}.{unique}.tmp");
    match path.parent() {
        Some(parent) => parent.join(temp_name),
        None => Utf8PathBuf::from(temp_name),
    }
}

/// Read a file as UTF-8 text. `Ok(None)` if the file does not exist.
pub fn read_text(path: &Utf8Path) -> Result<Option<String>, Error> {
    match fs_err::read_to_string(path.as_std_path()) {
        Ok(contents) => Ok(Some(contents)),
        Err(source) if is_not_found(&source) => Ok(None),
        Err(source) => Err(Error::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Read a file as raw bytes. `Ok(None)` if the file does not exist.
pub fn read_bytes(path: &Utf8Path) -> Result<Option<Vec<u8>>, Error> {
    match fs_err::read(path.as_std_path()) {
        Ok(contents) => Ok(Some(contents)),
        Err(source) if is_not_found(&source) => Ok(None),
        Err(source) => Err(Error::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Write text to a file, plain — not crash-safe. Use [`write_atomic`] for state.
pub fn write_text(path: &Utf8Path, contents: &str) -> Result<(), Error> {
    fs_err::write(path.as_std_path(), contents).map_err(|source| Error::Write {
        path: path.to_owned(),
        source,
    })
}

/// Write raw bytes to a file, plain — not crash-safe. Use [`write_atomic`] for
/// state.
pub fn write_bytes(path: &Utf8Path, contents: &[u8]) -> Result<(), Error> {
    fs_err::write(path.as_std_path(), contents).map_err(|source| Error::Write {
        path: path.to_owned(),
        source,
    })
}

/// Write bytes to `path` atomically: write to a fresh temp sibling, then
/// `rename` it over `path`. A crash or a concurrent reader never observes a
/// half-written file — `rename` within one directory is atomic on every
/// platform this crate ships for.
pub fn write_atomic(path: &Utf8Path, contents: &[u8]) -> Result<(), Error> {
    let temp = sibling_temp_path(path);
    fs_err::write(temp.as_std_path(), contents).map_err(|source| Error::Write {
        path: temp.clone(),
        source,
    })?;
    fs_err::rename(temp.as_std_path(), path.as_std_path()).map_err(|source| Error::Rename {
        from: temp,
        to: path.to_owned(),
        source,
    })
}

/// Create a directory and all missing ancestors. Idempotent — succeeds if the
/// directory already exists.
pub fn ensure_dir(path: &Utf8Path) -> Result<(), Error> {
    fs_err::create_dir_all(path.as_std_path()).map_err(|source| Error::CreateDir {
        path: path.to_owned(),
        source,
    })
}

/// Move `from` to `to` — a file or a directory, within one filesystem.
///
/// A plain `rename`, not a replace: `to` must not already exist. The one use
/// in this crate is `session convert` moving a View Dir from `.ivar/sessions/`
/// to `.ivar/features/<feature>/sessions/` — both under the same `.ivar/`
/// tree, so the same-filesystem requirement is structural.
pub fn rename(from: &Utf8Path, to: &Utf8Path) -> Result<(), Error> {
    fs_err::rename(from.as_std_path(), to.as_std_path()).map_err(|source| Error::Rename {
        from: from.to_owned(),
        to: to.to_owned(),
        source,
    })
}

/// List the entries of a directory, sorted, so callers get a deterministic
/// order regardless of what the OS handed back.
pub fn read_dir(path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
    let entries = fs_err::read_dir(path.as_std_path()).map_err(|source| Error::ReadDir {
        path: path.to_owned(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::ReadDir {
            path: path.to_owned(),
            source,
        })?;
        let utf8 = Utf8PathBuf::from_path_buf(entry.path()).map_err(not_utf8)?;
        paths.push(utf8);
    }
    paths.sort();
    Ok(paths)
}

/// Whether something exists at `path` (following symlinks).
pub fn exists(path: &Utf8Path) -> Result<bool, Error> {
    match fs_err::metadata(path.as_std_path()) {
        Ok(_) => Ok(true),
        Err(source) if is_not_found(&source) => Ok(false),
        Err(source) => Err(Error::Metadata {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Whether `path` is a regular file (following symlinks). `false` if absent.
pub fn is_file(path: &Utf8Path) -> Result<bool, Error> {
    match fs_err::metadata(path.as_std_path()) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(source) if is_not_found(&source) => Ok(false),
        Err(source) => Err(Error::Metadata {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Whether `path` is a directory (following symlinks). `false` if absent.
pub fn is_dir(path: &Utf8Path) -> Result<bool, Error> {
    match fs_err::metadata(path.as_std_path()) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(source) if is_not_found(&source) => Ok(false),
        Err(source) => Err(Error::Metadata {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Full metadata for `path` (following symlinks). `Ok(None)` if absent.
pub fn stat(path: &Utf8Path) -> Result<Option<std::fs::Metadata>, Error> {
    match fs_err::metadata(path.as_std_path()) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(source) if is_not_found(&source) => Ok(None),
        Err(source) => Err(Error::Metadata {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Set `path`'s permission bits to raw Unix `mode` bits (e.g. clearing the
/// write bits with `mode & !0o222` for a read-only worktree).
#[cfg(unix)]
pub fn chmod(path: &Utf8Path, mode: u32) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(mode);
    fs_err::set_permissions(path.as_std_path(), permissions).map_err(|source| Error::Chmod {
        path: path.to_owned(),
        source,
    })
}

/// The raw Unix mode bits of `path` (following symlinks). `Ok(None)` if the
/// path does not exist.
///
/// The one read every read-only-guard decision starts from: whether the write
/// bits are present, and — for a temporary lift — what to restore them to.
#[cfg(unix)]
pub fn unix_mode(path: &Utf8Path) -> Result<Option<u32>, Error> {
    Ok(stat(path)?.map(|metadata| metadata.permissions().mode()))
}

/// Clear every write bit on `path` — the read-only guard ivar applies to the
/// default-branch worktrees a session does not promote. Idempotent: a path
/// with no write bits is left untouched, syscall included.
#[cfg(unix)]
pub fn clear_write_bits(path: &Utf8Path) -> Result<(), Error> {
    let Some(mode) = unix_mode(path)? else {
        return Ok(());
    };
    if mode & 0o222 != 0 {
        chmod(path, mode & !0o222)?;
    }
    Ok(())
}

/// Restore write bits on `path`, undoing [`clear_write_bits`].
///
/// This is how a git mutation temporarily lifts a read-only guard: git cannot
/// create files in a write-bit-cleared directory, and a checkout that fails
/// mid-merge would leave the branch advanced but the files missing. Idempotent:
/// a path that already has write bits is left untouched.
#[cfg(unix)]
pub fn restore_write_bits(path: &Utf8Path) -> Result<(), Error> {
    let Some(mode) = unix_mode(path)? else {
        return Ok(());
    };
    if mode & 0o222 == 0 {
        chmod(path, mode | 0o222)?;
    }
    Ok(())
}

/// Create a symlink at `link` pointing to `target`. Fails if `link` already
/// exists — see [`replace_symlink`] for the atomic-replace version.
#[cfg(unix)]
pub fn create_symlink(target: &Utf8Path, link: &Utf8Path) -> Result<(), Error> {
    fs_err::os::unix::fs::symlink(target.as_std_path(), link.as_std_path()).map_err(|source| {
        Error::Symlink {
            link: link.to_owned(),
            target: target.to_owned(),
            source,
        }
    })
}

/// Point `link` at `target`. Builds the new symlink at a temp sibling and
/// `rename`s it over `link`.
///
/// # What this guarantees, and what it does not
///
/// `rename()` never leaves `link` momentarily *absent* — there is no
/// unlink-then-create window, so a concurrent [`read_symlink`] never sees
/// `Ok(None)` (`ENOENT`) mid-replace. That part is a real guarantee, not an
/// aspiration.
///
/// What is **not** true, measured directly on macOS/APFS: a concurrent reader
/// resolving the same link can still hit a transient error. Measured
/// independently of this crate — plain `std::fs`, no `fs-err` — across three
/// runs of 300 concurrent replaces each against one reader thread:
///
/// ```text
/// reads=1466 readlink_err=34 lstat_err=0 stat_err=22 open_err=14
/// ```
///
/// `readlink` and `stat` and `open`-through-the-link can all transiently fail;
/// only `lstat` never did. This is dentry-level APFS behaviour under rapid
/// replacement, not a bug in this function's temp-then-rename scheme, and
/// nothing written here closes that window — [`read_symlink`] retries its own
/// `readlink` call specifically (see its doc comment, which separately
/// verified that retry clears the `readlink` case completely over 300k+
/// iterations), but a `stat` or an `open` elsewhere in the crate, or in a
/// harness process, has no such retry and can still observe a transient
/// failure. Untested on Linux — do not assume either outcome there.
///
/// The real mitigation is not retrying harder, it is replacing less: see
/// [`replace_symlink_if_changed`], which skips the rename — and the window —
/// whenever the link already points at the right place.
#[cfg(unix)]
pub fn replace_symlink(target: &Utf8Path, link: &Utf8Path) -> Result<(), Error> {
    let temp = sibling_temp_path(link);
    fs_err::os::unix::fs::symlink(target.as_std_path(), temp.as_std_path()).map_err(|source| {
        Error::Symlink {
            link: temp.clone(),
            target: target.to_owned(),
            source,
        }
    })?;
    fs_err::rename(temp.as_std_path(), link.as_std_path()).map_err(|source| Error::Rename {
        from: temp,
        to: link.to_owned(),
        source,
    })
}

/// Point `link` at `target`, replacing it only if it does not already point
/// there. **This is the primitive callers should reach for** — the view dir
/// is rebuilt on every session connect, that rebuild is documented as
/// idempotent, and in practice the overwhelming majority of those rebuilds
/// leave every link exactly where it was. Every call to plain
/// [`replace_symlink`] opens the transient-error window described on its doc
/// comment; skipping the rename when nothing changed closes that window for
/// free, for the common case, rather than retrying harder after opening it.
#[cfg(unix)]
pub fn replace_symlink_if_changed(target: &Utf8Path, link: &Utf8Path) -> Result<(), Error> {
    if let SymlinkTarget::Target(current) = read_symlink(link)?
        && current == target
    {
        return Ok(());
    }
    replace_symlink(target, link)
}

/// What [`read_symlink`] found at a path: genuinely absent, present but not a
/// symlink, or a symlink pointing somewhere.
///
/// This is three states, not the `Option` every other read-shaped primitive
/// in this module returns, because "present but not a symlink" is a distinct,
/// permanent answer callers need as data — not folded into `Absent`, and not
/// an [`Error`] either. The sync planner's `CONFLICT (target is not a
/// symlink)` case and `doctor`'s "did someone replace this view-dir entry
/// with a real directory?" check both need to tell "gone" apart from
/// "something else is there now" without treating either as a fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymlinkTarget {
    /// Nothing exists at the path.
    Absent,
    /// Something exists at the path, but it is not a symlink.
    NotASymlink,
    /// A symlink exists at the path, pointing at this target.
    Target(Utf8PathBuf),
}

/// Resolve whatever is at `path`. See [`SymlinkTarget`] for the three
/// possible answers.
///
/// # Why `lstat` is the gate, not the retry
///
/// `replace_symlink` promises a concurrent reader never observes `link`
/// missing mid-replace, because `rename()` is atomic at the directory-entry
/// level. That promise holds — but on macOS/APFS, a `readlink(2)` racing a
/// rapid stream of such renames has been observed to transiently fail with
/// `EINVAL` ("the named file is not a symbolic link"), even though the entry
/// is a symlink both immediately before and immediately after.
///
/// The trap: `EINVAL` from `readlink` is *also* the normal, **permanent**
/// answer to "this path exists and is genuinely not a symlink" — a real file
/// or directory sitting where a symlink used to be. Those two causes are
/// indistinguishable from `readlink`'s return value alone, so retrying
/// blindly on `EINVAL` would turn a legitimate, instant answer
/// ([`SymlinkTarget::NotASymlink`]) into dozens of wasted syscalls followed by
/// a misleading I/O error.
///
/// The fix is to never ask `readlink` a question it cannot answer alone.
/// `symlink_metadata` (`lstat`) was measured, in the same stress run that
/// found the `readlink` race, to never fail even once — so it is the gate:
///
/// 1. `lstat` the path. `NotFound` → [`SymlinkTarget::Absent`].
/// 2. `lstat` succeeds and says the entry is **not** a symlink →
///    [`SymlinkTarget::NotASymlink`], immediately, in one syscall, no
///    `readlink` call and no retry. This is the permanent case.
/// 3. `lstat` succeeds and says it **is** a symlink → only now call
///    `readlink`. An `EINVAL` here, immediately after `lstat` already
///    confirmed the entry is a symlink, can only be the transient race —
///    so only this path retries, and the bound can be small and honest
///    about how rare it actually is once `lstat` has ruled out the
///    permanent cause.
pub fn read_symlink(path: &Utf8Path) -> Result<SymlinkTarget, Error> {
    const MAX_RETRIES: u32 = 8;

    let metadata = match fs_err::symlink_metadata(path.as_std_path()) {
        Ok(metadata) => metadata,
        Err(source) if is_not_found(&source) => return Ok(SymlinkTarget::Absent),
        Err(source) => {
            return Err(Error::Metadata {
                path: path.to_owned(),
                source,
            });
        }
    };

    if !metadata.file_type().is_symlink() {
        return Ok(SymlinkTarget::NotASymlink);
    }

    let mut attempts = 0;
    loop {
        match fs_err::read_link(path.as_std_path()) {
            Ok(target) => {
                return Ok(SymlinkTarget::Target(
                    Utf8PathBuf::from_path_buf(target).map_err(not_utf8)?,
                ));
            }
            // The entry vanished between the `lstat` above and this
            // `readlink` — a real, if rare, race of its own. Absent is the
            // honest answer: whatever `lstat` saw is no longer there.
            Err(source) if is_not_found(&source) => return Ok(SymlinkTarget::Absent),
            Err(source) if attempts < MAX_RETRIES && is_transient_not_a_symlink(&source) => {
                attempts += 1;
            }
            Err(source) => {
                return Err(Error::ReadSymlink {
                    path: path.to_owned(),
                    source,
                });
            }
        }
    }
}

/// Remove a file (or symlink, unlinked rather than followed). Removing
/// something already gone is success, not an error.
pub fn remove_file(path: &Utf8Path) -> Result<(), Error> {
    match fs_err::remove_file(path.as_std_path()) {
        Ok(()) => Ok(()),
        Err(source) if is_not_found(&source) => Ok(()),
        Err(source) => Err(Error::Remove {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Remove whatever is at `path` — file, symlink, or directory tree —
/// recursively. Removing something already gone is success, not an error.
pub fn remove_path(path: &Utf8Path) -> Result<(), Error> {
    // `symlink_metadata` (not `metadata`) so a symlink is unlinked as itself,
    // never followed into whatever directory it might point at.
    match fs_err::symlink_metadata(path.as_std_path()) {
        Ok(metadata) if metadata.is_dir() => {
            fs_err::remove_dir_all(path.as_std_path()).map_err(|source| Error::Remove {
                path: path.to_owned(),
                source,
            })
        }
        Ok(_) => fs_err::remove_file(path.as_std_path()).map_err(|source| Error::Remove {
            path: path.to_owned(),
            source,
        }),
        Err(source) if is_not_found(&source) => Ok(()),
        Err(source) => Err(Error::Remove {
            path: path.to_owned(),
            source,
        }),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/infra/fs.rs"]
mod tests;
