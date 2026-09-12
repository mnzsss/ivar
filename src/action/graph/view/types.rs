use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::action::graph::query::types::{CalleeInfo, CallerInfo};
use crate::domain::graph::{EdgeKind, Provenance, Span, SymbolKind};

pub const MAX_DEPTH: usize = 2;
pub const MAX_NODES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum ViewSeed {
    Default,
    Repo(String),
    Symbol(String),
    File(String),
    Impact(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewerNode {
    pub id: i64,
    pub repo: String,
    pub name: String,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub file: String,
    pub span: Span,
    pub exported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewerEdge {
    pub from: i64,
    pub to: i64,
    pub kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewerGraph {
    pub nodes: Vec<ViewerNode>,
    pub edges: Vec<ViewerEdge>,
    pub truncated: bool,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewerNodeDetails {
    pub node: ViewerNode,
    pub callers: Vec<CallerInfo>,
    pub callees: Vec<CalleeInfo>,
}

#[derive(Debug, Error)]
pub enum ViewError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("query error: {0}")]
    Query(String),
    #[error("seed not found: {0}")]
    SeedNotFound(String),
    #[error("invalid parameter: {0}")]
    InvalidParam(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("security error: {0}")]
    Security(String),
    #[error("bind error: {0}")]
    Bind(String),
    #[error("not found")]
    NotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
