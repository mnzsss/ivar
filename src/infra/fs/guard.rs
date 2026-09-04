//! The read-only guard: chmod, mode reads, and the write-bit clear/restore
//! pair. See `super::mod` for the shared `Error`.

/// Set `path`'s permission bits to raw Unix `mode` bits (e.g. clearing the
/// write bits with `mode & !0o222` for a read-only worktree).
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, FixAction};

use super::{Error, stat};

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
///
/// Exactly the path given, never the tree under it. A worktree is full of
/// files hardlinked out of a package manager's content-addressed store (pnpm,
/// bun), and `chmod` acts on the inode rather than the link: recursing would
/// change permissions inside that store and inside every other checkout
/// sharing it. What the root-only guard buys, and what it does not, is
/// `docs/reference/limitations.md`.
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

/// Restore the owner's write bit on `path`, undoing [`clear_write_bits`].
///
/// This is how a git mutation or a setup run temporarily lifts a read-only
/// guard: neither git nor a setup script can create files in a
/// write-bit-cleared directory, and a checkout that fails mid-merge would leave
/// the branch advanced but the files missing. Idempotent: a path that already
/// has write bits is left untouched.
///
/// Only `u+w` comes back. Restoring `mode | 0o222` would hand a 755 worktree
/// back as **777** — a lift widening what it was asked to restore, and leaving
/// it world-writable if the process died before the re-guard. The guard does
/// not record the bits it cleared, so a group-writable path returns
/// owner-writable; that is the direction to err in, and `ivar` runs as the
/// owner either way.
#[cfg(unix)]
pub fn restore_write_bits(path: &Utf8Path) -> Result<(), Error> {
    let Some(mode) = unix_mode(path)? else {
        return Ok(());
    };
    if mode & 0o222 == 0 {
        chmod(path, mode | 0o200)?;
    }
    Ok(())
}

/// A lifted read-only guard that puts itself back.
///
/// [`clear_write_bits`] and [`restore_write_bits`] are a pair, and the window
/// between them is where a worktree is writable. Holding that window open with
/// a value rather than a pair of statements is what makes the restore
/// unconditional: it happens on the success path, on every early return, and
/// on unwind, because it happens in `Drop`.
///
/// The alternative — restoring with a statement at the end of the function —
/// is correct only while nothing between the two returns early, which is a
/// property every later edit has to re-establish by reading the whole function.
///
/// A path that was already writable is not recorded and not touched on drop:
/// lifting a guard that does not exist must not create one.
#[cfg(unix)]
#[derive(Debug)]
pub struct LiftedGuard {
    lifted: Vec<(Utf8PathBuf, u32)>,
}

#[cfg(unix)]
impl LiftedGuard {
    /// Lift the read-only guard on each of `paths` that has one, recording the
    /// exact mode to restore.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] when a path's mode cannot be read, or when its
    /// write bits cannot be restored. Paths lifted before the failure are
    /// re-guarded by the drop of the partially built guard.
    pub fn lift(paths: &[&Utf8Path]) -> Result<Self, Failure> {
        let mut guard = Self { lifted: Vec::new() };
        for &path in paths {
            match unix_mode(path) {
                Ok(Some(mode)) if mode & 0o222 == 0 => {
                    if let Err(e) = restore_write_bits(path) {
                        return Err(Failure::failed(
                            "fs.lift_write_bits_failed",
                            format!("could not lift write permissions on `{path}`: {e}"),
                        )
                        .expected(format!("write permissions to be lifted on `{path}`"))
                        .actual(format!("chmod failed: {e}"))
                        .fix(FixAction::safe(
                            "fs.check_permissions",
                            format!(
                                "Ensure user has permission to modify permissions on `{path}`."
                            ),
                        )));
                    }
                    guard.lifted.push((path.to_path_buf(), mode));
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(Failure::failed(
                        "fs.read_mode_failed",
                        format!("could not inspect permissions on `{path}`: {e}"),
                    )
                    .expected(format!("path `{path}` to exist and be readable"))
                    .actual(format!("fs error: {e}"))
                    .fix(FixAction::safe(
                        "fs.check_path",
                        format!("Ensure `{path}` exists and is accessible."),
                    )));
                }
            }
        }
        Ok(guard)
    }
}

#[cfg(unix)]
impl Drop for LiftedGuard {
    fn drop(&mut self) {
        for (path, mode) in &self.lifted {
            let _ = chmod(path, *mode);
        }
    }
}
