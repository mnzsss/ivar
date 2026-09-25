//! Types, errors, and path filter predicates for codebase graph indexing.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::store::graph::db::{GraphDbError, MAX_INDEXED_CONTENT_BYTES};

/// Error encountered during repository indexing.
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("Git error: {0}")]
    Git(#[from] crate::git::Error),
    #[error("Database error: {0}")]
    Db(#[from] GraphDbError),
}

/// Statistics and outcome of an indexing run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexOutcome {
    pub repo: String,
    pub files_indexed: usize,
    pub files_deleted: usize,
    pub symbols_indexed: usize,
    pub edges_indexed: usize,
    pub duration_ms: u64,
    pub skipped_up_to_date: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_failed: Vec<FileFailure>,
}

/// Details of a single file indexing failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileFailure {
    pub path: String,
    pub reason: String,
}

/// A repository whose indexing failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RepoFailure {
    pub repo: String,
    pub reason: String,
}

/// Every base repository indexed across a hall.
#[derive(Debug, Default)]
pub struct HallIndex {
    pub repos: Vec<IndexOutcome>,
    pub repos_failed: Vec<RepoFailure>,
}

impl FileFailure {
    pub fn new(path: &str, reason: impl std::fmt::Display) -> Self {
        Self {
            path: path.to_owned(),
            reason: reason.to_string(),
        }
    }
}

pub fn is_ignored_dir(name: &std::ffi::OsStr) -> bool {
    let s = name.to_string_lossy();
    matches!(
        s.as_ref(),
        ".git"
            | ".ivar"
            | "target"
            | "node_modules"
            | "dist"
            | "build"
            | "out"
            | ".next"
            | ".turbo"
            | ".nuxt"
            | ".output"
            | ".cache"
            | ".parcel-cache"
            | "coverage"
            | ".svelte-kit"
            | ".astro"
            | "vendor"
            | "__pycache__"
            | ".venv"
            | "venv"
            | "env"
            | ".tox"
            | ".pytest_cache"
            | "bundle"
            | "pkg"
            | "Pods"
            | ".pnpm-store"
            | "storybook-static"
    )
}

pub fn is_ignored_path(path: &Path) -> bool {
    for comp in path.components() {
        if let std::path::Component::Normal(c) = comp
            && is_ignored_dir(c)
        {
            return true;
        }
    }
    false
}

pub fn is_ignored_file_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.ends_with(".min.js")
        || path_str.ends_with(".min.ts")
        || path_str.ends_with(".bundle.js")
        || path_str.ends_with(".d.ts.map")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextClassification {
    Binary,
    Text {
        content: String,
        indexed_len: usize,
        truncated: bool,
    },
}

pub(crate) fn classify_text(bytes: Vec<u8>) -> TextClassification {
    let probe_len = bytes.len().min(8192);
    if bytes.get(..probe_len).is_some_and(|slice| slice.contains(&b'\0')) {
        return TextClassification::Binary;
    }

    let Ok(content) = String::from_utf8(bytes) else {
        return TextClassification::Binary;
    };

    if content.len() <= MAX_INDEXED_CONTENT_BYTES {
        let len = content.len();
        TextClassification::Text {
            content,
            indexed_len: len,
            truncated: false,
        }
    } else {
        let indexed_len = content.floor_char_boundary(MAX_INDEXED_CONTENT_BYTES);
        TextClassification::Text {
            content,
            indexed_len,
            truncated: true,
        }
    }
}
