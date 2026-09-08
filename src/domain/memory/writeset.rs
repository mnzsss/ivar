//! Session-recorded memory writeset data model.

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::domain::name::SessionId;
use crate::error::Failure;

/// Set of memory paths modified during a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryWriteSet {
    /// Session identifier.
    pub session: SessionId,
    /// Paths modified during the session.
    pub modified_paths: Vec<Utf8PathBuf>,
}

impl MemoryWriteSet {
    /// Create a new memory writeset.
    #[must_use]
    pub fn new(session: SessionId, modified_paths: Vec<Utf8PathBuf>) -> Self {
        Self {
            session,
            modified_paths,
        }
    }

    /// Check whether the writeset has no modified paths.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modified_paths.is_empty()
    }

    /// Return the number of modified paths in the writeset.
    #[must_use]
    pub fn len(&self) -> usize {
        self.modified_paths.len()
    }

    /// Retain only modified paths that exist on disk relative to a given root directory (or as absolute paths).
    #[must_use]
    pub fn filter_existing(&self, base: &Utf8Path) -> Self {
        let modified_paths = self
            .modified_paths
            .iter()
            .filter(|path| {
                let full: Utf8PathBuf = if path.is_absolute() {
                    (*path).clone()
                } else {
                    base.join(path)
                };
                crate::infra::fs::exists(&full).unwrap_or(false)
            })
            .cloned()
            .collect();

        Self {
            session: self.session.clone(),
            modified_paths,
        }
    }

    /// Serialize writeset to canonical JSON string.
    pub fn to_json(&self) -> Result<String, Failure> {
        crate::infra::json::to_canonical_string(self).map_err(Into::into)
    }

    /// Deserialize writeset from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, Failure> {
        serde_json::from_str(json).map_err(|err| {
            Failure::failed("memory.writeset.parse_failed", format!("invalid writeset JSON: {err}"))
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/writeset.rs"]
mod tests;
