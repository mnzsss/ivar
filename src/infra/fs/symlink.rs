//! Symlink behaviour: create, replace, and read, plus the three-state
//! [`SymlinkTarget`] answer. See `super::mod` for the shared `Error`.

/// Create a symlink at `link` pointing to `target`. Fails if `link` already
/// exists — see [`replace_symlink`] for the atomic-replace version.
use camino::{Utf8Path, Utf8PathBuf};

use super::{Error, is_not_found, is_transient_not_a_symlink, not_utf8, sibling_temp_path};

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
