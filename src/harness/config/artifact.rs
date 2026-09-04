use camino::Utf8Path;

use crate::infra::fs;

use super::{Change, Error};

/// Materialise an exact-byte managed file at `path`.
///
/// Returns [`Change::Created`] if newly created, [`Change::Updated`] if bytes changed,
/// or [`Change::Unchanged`] if on-disk contents already match.
pub fn reconcile_managed_artifact(path: &Utf8Path, contents: &str) -> Result<Change, Error> {
    let existing = fs::read_text(path).map_err(|source| Error::Artifact {
        path: path.to_owned(),
        source,
    })?;

    if existing.as_deref() == Some(contents) {
        return Ok(Change::Unchanged);
    }

    if let Some(parent) = path.parent() {
        fs::ensure_dir(parent).map_err(|source| Error::Artifact {
            path: parent.to_owned(),
            source,
        })?;
    }

    fs::write_text(path, contents).map_err(|source| Error::Artifact {
        path: path.to_owned(),
        source,
    })?;

    if existing.is_some() {
        Ok(Change::Updated)
    } else {
        Ok(Change::Created)
    }
}

/// Remove a managed artifact at `path`. Absent file is [`Change::Unchanged`].
pub fn remove_managed_artifact(path: &Utf8Path) -> Result<Change, Error> {
    if !path.exists() {
        return Ok(Change::Unchanged);
    }

    fs::remove_file(path).map_err(|source| Error::Artifact {
        path: path.to_owned(),
        source,
    })?;

    Ok(Change::Removed)
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config/artifact.rs"]
mod tests;
