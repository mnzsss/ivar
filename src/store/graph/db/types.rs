//! Error types, record rows, and conversion helpers for GraphDb.

use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::graph::{EdgeKind, Provenance, SymbolKind};

/// Graph database errors.
#[derive(Debug, thiserror::Error)]
pub enum GraphDbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database migration error: {0}")]
    Migration(String),
    #[error("{0}")]
    Message(String),
}

pub type Result<T, E = GraphDbError> = std::result::Result<T, E>;

/// A record representing repository metadata in the `repos` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRow {
    pub id: String,
    pub root_path: String,
    pub default_branch: String,
    pub last_indexed_commit: Option<String>,
    pub indexed_at: i64,
}

/// A record representing an indexed file in the `files` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: i64,
    pub repo: String,
    pub path: String,
    pub content_hash: String,
    pub mtime_ns: i64,
    pub size_bytes: i64,
    pub indexed_at: i64,
}

pub(super) fn now_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn symbol_kind_to_str<'a>(kind: &'a SymbolKind) -> Cow<'a, str> {
    match kind {
        SymbolKind::Fn => "fn".into(),
        SymbolKind::Method => "method".into(),
        SymbolKind::Struct => "struct".into(),
        SymbolKind::Class => "class".into(),
        SymbolKind::Trait => "trait".into(),
        SymbolKind::Interface => "interface".into(),
        SymbolKind::Enum => "enum".into(),
        SymbolKind::Mod => "mod".into(),
        SymbolKind::Const => "const".into(),
        SymbolKind::Other(s) => s.as_str().into(),
    }
}

pub fn parse_symbol_kind(s: &str) -> SymbolKind {
    match s.to_ascii_lowercase().as_str() {
        "fn" => SymbolKind::Fn,
        "method" => SymbolKind::Method,
        "struct" => SymbolKind::Struct,
        "class" => SymbolKind::Class,
        "trait" => SymbolKind::Trait,
        "interface" => SymbolKind::Interface,
        "enum" => SymbolKind::Enum,
        "mod" => SymbolKind::Mod,
        "const" => SymbolKind::Const,
        other => SymbolKind::Other(other.to_owned()),
    }
}

pub fn edge_kind_to_str<'a>(kind: &'a EdgeKind) -> Cow<'a, str> {
    match kind {
        EdgeKind::Calls => "CALLS".into(),
        EdgeKind::Imports => "IMPORTS".into(),
        EdgeKind::Implements => "IMPLEMENTS".into(),
        EdgeKind::CrossImports => "CROSS_IMPORTS".into(),
        EdgeKind::CrossExecutes => "CROSS_EXECUTES".into(),
        EdgeKind::CrossCallsHttp => "CROSS_CALLS_HTTP".into(),
        EdgeKind::Other(s) => s.as_str().into(),
    }
}

pub fn parse_edge_kind(s: &str) -> EdgeKind {
    match s {
        "CALLS" | "calls" => EdgeKind::Calls,
        "IMPORTS" | "imports" => EdgeKind::Imports,
        "IMPLEMENTS" | "implements" => EdgeKind::Implements,
        "CROSS_IMPORTS" | "cross_imports" => EdgeKind::CrossImports,
        "CROSS_EXECUTES" | "cross_executes" => EdgeKind::CrossExecutes,
        "CROSS_CALLS_HTTP" | "cross_calls_http" => EdgeKind::CrossCallsHttp,
        other => EdgeKind::Other(other.to_owned()),
    }
}

pub fn provenance_to_str(p: &Provenance) -> &'static str {
    match p {
        Provenance::Extracted => "EXTRACTED",
        Provenance::Inferred => "INFERRED",
        Provenance::Ambiguous => "AMBIGUOUS",
    }
}

pub fn parse_provenance(s: &str) -> Provenance {
    match s {
        "EXTRACTED" | "extracted" => Provenance::Extracted,
        "INFERRED" | "inferred" => Provenance::Inferred,
        "AMBIGUOUS" | "ambiguous" => Provenance::Ambiguous,
        _ => Provenance::Extracted,
    }
}
