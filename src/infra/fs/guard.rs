//! The read-only guard: chmod, mode reads, and the write-bit clear/restore
//! pair. See `super::mod` for the shared `Error`.

/// Set `path`'s permission bits to raw Unix `mode` bits (e.g. clearing the
/// write bits with `mode & !0o222` for a read-only worktree).
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use camino::Utf8Path;

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
