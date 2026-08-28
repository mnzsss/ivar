//! Generic filesystem I/O: reads, writes, directories, metadata, and
//! removal — everything that is not a symlink and not the read-only guard.
//! See `super::mod` for the shared `Error`.

/// Read a file as UTF-8 text. `Ok(None)` if the file does not exist.
use camino::{Utf8Path, Utf8PathBuf};

use super::{Error, is_not_found, not_utf8, sibling_temp_path};

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

/// Remove the empty parent directories `path` left behind, walking up until
/// `boundary` (exclusive) or the first directory that is not empty.
///
/// A nested name — a worktree on branch `feat/login` under
/// `.ivar/repos/<repo>/` — creates the intermediate directories on the way
/// down, but removing the leaf takes only the leaf. Without this, `feat/`
/// survives as an empty orphan that nothing owns and nothing will reclaim.
///
/// Best-effort by design: `remove_dir` refusing is the stop condition — the
/// directory still holds a sibling worktree — and a directory left standing
/// is cosmetic, never a reason to fail the teardown that called this. A
/// `path` outside `boundary` prunes nothing.
pub fn prune_empty_parents(path: &Utf8Path, boundary: &Utf8Path) {
    if !path.starts_with(boundary) {
        return;
    }
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        if dir == boundary || !dir.starts_with(boundary) {
            return;
        }
        // Non-recursive: only an already-empty directory goes, so a sibling
        // worktree under the same prefix stops the walk rather than being
        // swept up with it.
        if fs_err::remove_dir(dir.as_std_path()).is_err() {
            return;
        }
        cursor = dir.parent();
    }
}

/// A temporary directory created on disk that is recursively deleted on drop.
#[derive(Debug)]
pub struct TempDir {
    path: Utf8PathBuf,
}

impl TempDir {
    /// Create a new temporary directory under `std::env::temp_dir()`.
    pub fn new() -> Result<Self, Error> {
        let id = uuid::Uuid::new_v4();
        let path_std = std::env::temp_dir().join(format!("ivar-tmp-{id}"));
        let path = Utf8PathBuf::from_path_buf(path_std)
            .map_err(|p| Error::NotUtf8 { display: p.display().to_string() })?;
        ensure_dir(&path)?;
        Ok(Self { path })
    }

    /// The path of the temporary directory.
    #[must_use]
    pub fn path(&self) -> &Utf8Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = remove_path(&self.path);
    }
}

/// Recursively copy directory `src` to `dst`.
pub fn copy_dir(src: &Utf8Path, dst: &Utf8Path) -> Result<(), Error> {
    ensure_dir(dst)?;
    for entry in walkdir::WalkDir::new(src.as_std_path()) {
        let entry = entry.map_err(|e| Error::Read {
            path: src.to_owned(),
            source: e.into(),
        })?;
        let rel_path = entry.path().strip_prefix(src.as_std_path()).map_err(|_| Error::Read {
            path: src.to_owned(),
            source: std::io::Error::other("strip_prefix failed"),
        })?;
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        let rel_utf8 = Utf8Path::from_path(rel_path).ok_or_else(|| not_utf8(entry.path().to_path_buf()))?;
        let target_path = dst.join(rel_utf8);

        let file_type = entry.file_type();
        if file_type.is_dir() {
            ensure_dir(&target_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                ensure_dir(parent)?;
            }
            fs_err::copy(entry.path(), target_path.as_std_path()).map_err(|source| Error::Write {
                path: target_path,
                source,
            })?;
        } else if file_type.is_symlink() {
            if let Some(parent) = target_path.parent() {
                ensure_dir(parent)?;
            }
            let link_target = fs_err::read_link(entry.path()).map_err(|source| Error::Read {
                path: Utf8PathBuf::from_path_buf(entry.path().to_path_buf()).unwrap_or_default(),
                source,
            })?;
            let link_target_utf8 = Utf8Path::from_path(&link_target).ok_or_else(|| not_utf8(link_target.clone()))?;
            super::create_symlink(&target_path, link_target_utf8)?;
        }
    }
    Ok(())
}
