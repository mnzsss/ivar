//! Standalone zero-dependency HTML code graph visualizer generator.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::store::graph::db::GraphDb;

/// Errors that can occur during graph visualization generation.
#[derive(Debug, Error)]
pub enum VizError {
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Canonical JSON error: {0}")]
    JsonInfra(#[from] crate::infra::json::Error),
}

/// A node in the visualization graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VizNode {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub repo: String,
    pub line: usize,
    pub complexity: Option<u32>,
    pub is_exported: bool,
}

/// A directed edge in the visualization graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VizEdge {
    pub from: i64,
    pub to: i64,
    pub kind: String,
}

/// Collected data payload for graph visualization.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VizData {
    pub nodes: Vec<VizNode>,
    pub edges: Vec<VizEdge>,
}

/// Collects symbols and their connecting edges from `db`, optionally filtering by repository name.
pub fn collect_viz_data(db: &GraphDb, repo: Option<&str>) -> Result<VizData, VizError> {
    let conn = db.conn();

    let (symbol_query, edge_query) = match repo {
        Some(_) => (
            "SELECT s.id, s.name, s.kind, f.path, s.repo, s.start_line, s.complexity, s.is_exported \
             FROM symbols s \
             JOIN files f ON s.file_id = f.id \
             WHERE s.repo = ?1 \
             ORDER BY s.id ASC",
            "SELECT from_symbol_id, to_symbol_id, kind \
             FROM edges \
             WHERE repo = ?1 AND from_symbol_id IS NOT NULL AND to_symbol_id IS NOT NULL",
        ),
        None => (
            "SELECT s.id, s.name, s.kind, f.path, s.repo, s.start_line, s.complexity, s.is_exported \
             FROM symbols s \
             JOIN files f ON s.file_id = f.id \
             ORDER BY s.id ASC",
            "SELECT from_symbol_id, to_symbol_id, kind \
             FROM edges \
             WHERE from_symbol_id IS NOT NULL AND to_symbol_id IS NOT NULL",
        ),
    };

    let mut nodes = Vec::new();
    let mut node_ids = HashSet::new();

    let mut stmt = conn.prepare(symbol_query)?;
    let mut rows = match repo {
        Some(r) => stmt.query(params![r])?,
        None => stmt.query([])?,
    };

    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let kind: String = row.get(2)?;
        let file: String = row.get(3)?;
        let repo_val: String = row.get(4)?;
        let line: i64 = row.get(5)?;
        let complexity: Option<u32> = row.get::<_, Option<i64>>(6)?.map(|c| c as u32);
        let is_exported_int: i64 = row.get(7)?;
        let is_exported = is_exported_int != 0;

        node_ids.insert(id);
        nodes.push(VizNode {
            id,
            name,
            kind,
            file,
            repo: repo_val,
            line: line.max(0) as usize,
            complexity,
            is_exported,
        });
    }

    let mut edges = Vec::new();
    let mut edge_stmt = conn.prepare(edge_query)?;
    let mut edge_rows = match repo {
        Some(r) => edge_stmt.query(params![r])?,
        None => edge_stmt.query([])?,
    };

    while let Some(row) = edge_rows.next()? {
        let from_id: i64 = row.get(0)?;
        let to_id: i64 = row.get(1)?;
        let kind: String = row.get(2)?;

        if node_ids.contains(&from_id) && node_ids.contains(&to_id) {
            edges.push(VizEdge {
                from: from_id,
                to: to_id,
                kind,
            });
        }
    }

    Ok(VizData { nodes, edges })
}

/// Generates a 100% self-contained, air-gapped HTML5 visualizer string for `data` using Cytoscape.js.
pub fn generate_html(data: &VizData) -> Result<String, VizError> {
    use std::collections::BTreeSet;

    // Collect unique repos and files for compound nodes
    let mut repos = BTreeSet::new();
    let mut files = BTreeSet::new();

    for node in &data.nodes {
        repos.insert(node.repo.clone());
        files.insert((node.repo.clone(), node.file.clone()));
    }

    let mut elements = Vec::new();

    // 1. Repo compound nodes
    for repo in &repos {
        elements.push(serde_json::json!({
            "data": {
                "id": format!("repo:{repo}"),
                "label": repo,
                "type": "repo"
            }
        }));
    }

    // 2. File compound nodes
    for (repo, file) in &files {
        elements.push(serde_json::json!({
            "data": {
                "id": format!("file:{repo}:{file}"),
                "label": file,
                "parent": format!("repo:{repo}"),
                "type": "file"
            }
        }));
    }

    // 3. Symbol leaf nodes
    for node in &data.nodes {
        elements.push(serde_json::json!({
            "data": {
                "id": format!("sym:{}", node.id),
                "label": node.name,
                "parent": format!("file:{}:{}", node.repo, node.file),
                "kind": node.kind,
                "repo": node.repo,
                "file": node.file,
                "line": node.line,
                "complexity": node.complexity,
                "is_exported": node.is_exported,
                "type": "symbol"
            }
        }));
    }

    // 4. Edges
    for (idx, edge) in data.edges.iter().enumerate() {
        elements.push(serde_json::json!({
            "data": {
                "id": format!("e:{}->{}:{}", edge.from, edge.to, idx),
                "source": format!("sym:{}", edge.from),
                "target": format!("sym:{}", edge.to),
                "kind": edge.kind
            }
        }));
    }

    let raw_json = crate::infra::json::to_canonical_string(&elements)?;
    // Escape closing tags and characters that could prematurely terminate HTML script blocks
    let json_elements = raw_json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");

    let cytoscape_js = crate::action::graph::view::assets::CYTOSCAPE_JS;

    let template = crate::action::graph::view::assets::STANDALONE_HTML;
    let html = template
        .replace("__CYTOSCAPE_JS__", cytoscape_js)
        .replace("__JSON_ELEMENTS__", &json_elements);
    Ok(html)
}

/// Collects data from `db`, generates the standalone HTML visualizer, and writes it to `output_path`.
/// Returns the collected `VizData` and the canonicalized (or resolved) path to the written file.
pub fn execute_viz(
    db: &GraphDb,
    output_path: &Path,
    repo: Option<&str>,
) -> Result<(VizData, PathBuf), VizError> {
    let data = collect_viz_data(db, repo)?;
    let html = generate_html(&data)?;

    if let Some(parent) = output_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(output_path, html)?;

    let resolved_path = output_path
        .canonicalize()
        .unwrap_or_else(|_| output_path.to_path_buf());

    Ok((data, resolved_path))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/viz.rs"]
mod tests;
