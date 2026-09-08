//! Types, errors, and path filter predicates for codebase graph indexing.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::infra::graph::parser::SupportedLanguage;
use crate::store::graph::db::GraphDbError;
use crate::store::graph::extractor::ExtractorError;

/// Error encountered during repository indexing.
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("Git error: {0}")]
    Git(#[from] crate::git::Error),
    #[error("Database error: {0}")]
    Db(#[from] GraphDbError),
    #[error("AST extractor error in `{path}`: {source}")]
    Extractor {
        path: String,
        source: ExtractorError,
    },
    #[error("IO error in `{path}`: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
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
}

pub fn is_supported_file(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    if path_str.ends_with(".min.js")
        || path_str.ends_with(".min.ts")
        || path_str.ends_with(".bundle.js")
        || path_str.ends_with(".d.ts.map")
    {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    SupportedLanguage::from_extension(ext).is_some()
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
