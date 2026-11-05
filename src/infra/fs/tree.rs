//! Whole-tree helpers: a self-deleting temporary directory and a recursive copy.

use camino::{Utf8Path, Utf8PathBuf};

use super::{Error, ensure_dir, not_utf8, remove_path};

/// A temporary directory created on disk that is recursively deleted on drop.
#[derive(Debug)]
pub struct TempDir {
    path: Utf8PathBuf,
}

impl TempDir {
    /// Create a new temporary directory under `std::env::temp_dir()`.
    /// # Errors
    ///
    /// Returns [`Error`] if the temp directory's path is not UTF-8 or it
    /// cannot be created.
    pub fn new() -> Result<Self, Error> {
        let id = uuid::Uuid::new_v4();
        let path_std = std::env::temp_dir().join(format!("ivar-tmp-{id}"));
        let path = Utf8PathBuf::from_path_buf(path_std).map_err(|p| Error::NotUtf8 {
            display: p.display().to_string(),
        })?;
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
/// # Errors
///
/// Returns [`Error`] if a directory entry cannot be read, a path is not
/// UTF-8, or a file/directory/symlink cannot be created or copied.
pub fn copy_dir(src: &Utf8Path, dst: &Utf8Path) -> Result<(), Error> {
    ensure_dir(dst)?;
    for entry in walkdir::WalkDir::new(src.as_std_path()) {
        let entry = entry.map_err(|e| Error::Read {
            path: src.to_owned(),
            source: e.into(),
        })?;
        let rel_path = entry
            .path()
            .strip_prefix(src.as_std_path())
            .map_err(|_| Error::Read {
                path: src.to_owned(),
                source: std::io::Error::other("strip_prefix failed"),
            })?;
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        let rel_utf8 = Utf8Path::from_path(rel_path).ok_or_else(|| not_utf8(entry.path()))?;
        let target_path = dst.join(rel_utf8);

        let file_type = entry.file_type();
        if file_type.is_dir() {
            ensure_dir(&target_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                ensure_dir(parent)?;
            }
            fs_err::copy(entry.path(), target_path.as_std_path()).map_err(|source| {
                Error::Write {
                    path: target_path,
                    source,
                }
            })?;
        } else if file_type.is_symlink() {
            if let Some(parent) = target_path.parent() {
                ensure_dir(parent)?;
            }
            let link_target = fs_err::read_link(entry.path()).map_err(|source| Error::Read {
                path: Utf8PathBuf::from_path_buf(entry.path().to_path_buf()).unwrap_or_default(),
                source,
            })?;
            let link_target_utf8 =
                Utf8Path::from_path(&link_target).ok_or_else(|| not_utf8(&link_target.clone()))?;
            let resolved = entry
                .path()
                .parent()
                .map(|parent| lexically_normalise(&parent.join(&link_target)))
                .unwrap_or_default();
            let src_normalised = lexically_normalise(src.as_std_path());
            if link_target.is_relative() && resolved.starts_with(&src_normalised) {
                super::create_symlink(link_target_utf8, &target_path)?;
            } else if src_normalised.starts_with(&resolved) {
                return Err(Error::Read {
                    path: Utf8PathBuf::from_path_buf(entry.path().to_path_buf())
                        .unwrap_or_default(),
                    source: std::io::Error::other(
                        "symlink points at an ancestor of the copied tree",
                    ),
                });
            } else if resolved.is_dir() {
                let resolved_utf8 =
                    Utf8Path::from_path(&resolved).ok_or_else(|| not_utf8(&resolved.clone()))?;
                copy_dir(resolved_utf8, &target_path)?;
            } else if resolved.is_file() {
                fs_err::copy(&resolved, target_path.as_std_path()).map_err(|source| {
                    Error::Write {
                        path: target_path,
                        source,
                    }
                })?;
            }
        }
    }
    Ok(())
}

fn lexically_normalise(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut normalised = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalised.pop();
            }
            other => normalised.push(other),
        }
    }
    normalised
}
