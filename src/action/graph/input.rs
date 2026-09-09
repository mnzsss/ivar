//! Input structures for graph action commands.

use crate::action::graph::view::types::ViewSeed;

#[derive(Debug, Clone)]
pub struct ExploreInput {
    pub query: String,
    pub repo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AffectedInput {
    pub files: Vec<String>,
    pub stdin: bool,
    pub repo: Option<String>,
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PathInput {
    pub from: String,
    pub to: String,
    pub max_hops: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct FindInput {
    pub query: String,
    pub repo: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct CallersInput {
    pub symbol: String,
    pub repo: Option<String>,
    pub cross_repo: bool,
    pub min_confidence: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CalleesInput {
    pub symbol_id: i64,
}

#[derive(Debug, Clone)]
pub struct FileInput {
    pub repo: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct IndexInput {
    pub repo: Option<String>,
    pub full: bool,
}

#[derive(Debug, Clone)]
pub struct ImpactInput {
    pub symbol_id: i64,
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct DeadCodeInput {
    pub repo: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ComplexityInput {
    pub threshold: u32,
    pub repo: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct HierarchyInput {
    pub symbol: String,
    pub repo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VizInput {
    pub output: String,
    pub repo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphViewInput {
    pub seed: ViewSeed,
    pub depth: usize,
    pub limit: usize,
    pub no_open: bool,
    pub port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct CleanInput {
    pub repo: Option<String>,
    pub all: bool,
}
