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
    for edge in &data.edges {
        elements.push(serde_json::json!({
            "data": {
                "id": format!("e:{}->{}", edge.from, edge.to),
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

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Codebase Graph Visualizer</title>
<style>
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: #0b0e14; color: #e1e7ec; height: 100vh; overflow: hidden; display: flex; flex-direction: column; }}
header {{ height: 52px; background: #161d27; border-bottom: 1px solid #263342; display: flex; align-items: center; justify-content: space-between; padding: 0 16px; gap: 16px; flex-shrink: 0; z-index: 10; }}
.header-left {{ display: flex; align-items: center; gap: 16px; }}
h1 {{ font-size: 16px; font-weight: 600; color: #f0f6fc; letter-spacing: 0.5px; white-space: nowrap; }}
.stats-badge {{ font-size: 12px; color: #8b949e; background: #21262d; border: 1px solid #30363d; padding: 3px 8px; border-radius: 12px; }}
.controls {{ display: flex; align-items: center; gap: 12px; }}
input[type="text"], select, button.btn {{ background: #0d1117; border: 1px solid #30363d; color: #c9d1d9; padding: 6px 12px; border-radius: 6px; font-size: 13px; outline: none; }}
input[type="text"]:focus, select:focus {{ border-color: #58a6ff; }}
button.btn {{ cursor: pointer; }}
button.btn:hover {{ background: #21262d; color: #f0f6fc; }}
#main-container {{ flex: 1; position: relative; width: 100%; height: calc(100vh - 52px); display: flex; }}
#cy {{ flex: 1; width: 100%; height: 100%; background: #0b0e14; }}
#cy canvas {{ background: transparent !important; }}
#sidebar {{ width: 340px; background: #161d27; border-left: 1px solid #263342; padding: 20px; overflow-y: auto; display: none; flex-direction: column; gap: 16px; position: absolute; right: 0; top: 0; bottom: 0; box-shadow: -4px 0 16px rgba(0,0,0,0.4); z-index: 20; }}
#sidebar.active {{ display: flex; }}
.sidebar-header {{ display: flex; justify-content: space-between; align-items: flex-start; border-bottom: 1px solid #30363d; padding-bottom: 12px; }}
.sidebar-title {{ font-size: 16px; font-weight: 600; color: #58a6ff; word-break: break-all; }}
.close-btn {{ background: none; border: none; color: #8b949e; font-size: 20px; cursor: pointer; padding: 0 4px; line-height: 1; }}
.close-btn:hover {{ color: #f0f6fc; }}
.prop-group {{ display: flex; flex-direction: column; gap: 4px; }}
.prop-label {{ font-size: 11px; text-transform: uppercase; color: #8b949e; font-weight: 600; letter-spacing: 0.5px; }}
.prop-val {{ font-size: 13px; color: #c9d1d9; word-break: break-all; font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace; }}
.badge {{ display: inline-block; padding: 2px 6px; border-radius: 4px; font-size: 11px; font-weight: 600; text-transform: uppercase; }}
.badge-exported {{ background: rgba(35, 134, 54, 0.2); color: #3fb950; border: 1px solid #238636; }}
.badge-private {{ background: rgba(110, 118, 129, 0.2); color: #8b949e; border: 1px solid #6e7681; }}
.edge-list {{ list-style: none; display: flex; flex-direction: column; gap: 6px; }}
.edge-item {{ font-size: 12px; padding: 6px 8px; background: #0d1117; border: 1px solid #30363d; border-radius: 4px; display: flex; justify-content: space-between; align-items: center; cursor: pointer; }}
.edge-item:hover {{ border-color: #58a6ff; }}
.instructions {{ position: absolute; left: 16px; bottom: 16px; background: rgba(22, 29, 39, 0.85); border: 1px solid #263342; border-radius: 6px; padding: 8px 12px; font-size: 11px; color: #8b949e; pointer-events: none; z-index: 5; }}
</style>
<script>{cytoscape_js}</script>
</head>
<body>
<header>
  <div class="header-left">
    <h1>Codebase Graph</h1>
    <span class="stats-badge" id="stats-badge">0 symbols &bull; 0 edges</span>
  </div>
  <div class="controls">
    <select id="kind-filter">
      <option value="">All Kinds</option>
    </select>
    <input type="text" id="search-input" placeholder="Search symbols..." />
    <button class="btn" id="btn-fit" title="Fit to screen">Fit</button>
    <button class="btn" id="btn-reset" title="Reset view">Reset</button>
  </div>
</header>
<div id="main-container">
  <div id="cy"></div>
  <div class="instructions">Scroll: Zoom &bull; Drag: Pan &bull; Click Node: Neighborhood & Details</div>
  <div id="sidebar">
    <div class="sidebar-header">
      <div class="sidebar-title" id="node-name">-</div>
      <button class="close-btn" id="close-sidebar">&times;</button>
    </div>
    <div class="prop-group">
      <div class="prop-label">Kind</div>
      <div class="prop-val" id="node-kind">-</div>
    </div>
    <div class="prop-group">
      <div class="prop-label">Visibility</div>
      <div class="prop-val" id="node-visibility">-</div>
    </div>
    <div class="prop-group">
      <div class="prop-label">Repository</div>
      <div class="prop-val" id="node-repo">-</div>
    </div>
    <div class="prop-group">
      <div class="prop-label">Location</div>
      <div class="prop-val" id="node-location">-</div>
    </div>
    <div class="prop-group">
      <div class="prop-label">Complexity</div>
      <div class="prop-val" id="node-complexity">-</div>
    </div>
    <div class="prop-group">
      <div class="prop-label" id="incoming-header">Incoming References (0)</div>
      <ul class="edge-list" id="incoming-edges"></ul>
    </div>
    <div class="prop-group">
      <div class="prop-label" id="outgoing-header">Outgoing References (0)</div>
      <ul class="edge-list" id="outgoing-edges"></ul>
    </div>
  </div>
</div>

<script>
const ELEMENTS = {json_elements};

(function() {{
  const searchInput = document.getElementById('search-input');
  const kindFilter = document.getElementById('kind-filter');
  const statsBadge = document.getElementById('stats-badge');
  const sidebar = document.getElementById('sidebar');
  const closeSidebarBtn = document.getElementById('close-sidebar');
  const btnFit = document.getElementById('btn-fit');
  const btnReset = document.getElementById('btn-reset');

  const KIND_COLORS = {{
    'Function': '#38bdf8',
    'Method': '#60a5fa',
    'Class': '#f59e0b',
    'Struct': '#fbbf24',
    'Interface': '#34d399',
    'Trait': '#10b981',
    'Enum': '#a78bfa',
    'Constant': '#f472b6',
    'Variable': '#94a3b8',
    'Module': '#cbd5e1'
  }};

  const symbolElements = ELEMENTS.filter(el => el.data.type === 'symbol');
  const edgeElements = ELEMENTS.filter(el => !el.data.type && el.data.source);

  const kinds = new Set();
  symbolElements.forEach(el => {{
    if (el.data.kind) kinds.add(el.data.kind);
  }});

  kinds.forEach(k => {{
    const opt = document.createElement('option');
    opt.value = k;
    opt.textContent = k;
    kindFilter.appendChild(opt);
  }});

  statsBadge.textContent = `${{symbolElements.length}} symbols \u2022 ${{edgeElements.length}} edges`;

  const cy = cytoscape({{
    container: document.getElementById('cy'),
    elements: ELEMENTS,
    style: [
      {{
        selector: 'node',
        style: {{
          'background-color': '#475569',
          'label': 'data(label)',
          'color': '#cbd5e1',
          'font-size': '11px',
          'font-family': 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
          'text-valign': 'center',
          'text-halign': 'center'
        }}
      }},
      {{
        selector: 'node[type = "repo"]',
        style: {{
          'background-color': '#111827',
          'background-opacity': 0.6,
          'border-width': 1,
          'border-color': '#374151',
          'border-style': 'solid',
          'shape': 'roundrectangle',
          'text-valign': 'top',
          'text-halign': 'center',
          'color': '#9ca3af',
          'font-weight': 'bold',
          'font-size': '13px',
          'padding': '16px'
        }}
      }},
      {{
        selector: 'node[type = "file"]',
        style: {{
          'background-color': '#1f2937',
          'background-opacity': 0.5,
          'border-width': 1,
          'border-color': '#4b5563',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'text-valign': 'top',
          'text-halign': 'center',
          'color': '#9ca3af',
          'font-size': '11px',
          'padding': '12px'
        }}
      }},
      {{
        selector: 'node[type = "symbol"]',
        style: {{
          'width': 28,
          'height': 28,
          'shape': 'ellipse',
          'background-color': function(ele) {{
            const k = ele.data('kind');
            return KIND_COLORS[k] || '#38bdf8';
          }},
          'color': '#f3f4f6',
          'text-valign': 'bottom',
          'text-margin-y': 4,
          'text-background-color': '#0b0e14',
          'text-background-opacity': 0.7,
          'text-background-padding': '2px',
          'text-background-shape': 'roundrectangle'
        }}
      }},
      {{
        selector: 'edge',
        style: {{
          'width': 1.5,
          'line-color': '#334155',
          'target-arrow-color': '#334155',
          'target-arrow-shape': 'triangle',
          'curve-style': 'bezier',
          'arrow-scale': 0.8
        }}
      }},
      {{
        selector: '.highlighted',
        style: {{
          'background-color': '#38bdf8',
          'line-color': '#38bdf8',
          'target-arrow-color': '#38bdf8',
          'z-index': 999,
          'border-width': 2,
          'border-color': '#93c5fd'
        }}
      }},
      {{
        selector: '.faded',
        style: {{
          'opacity': 0.15
        }}
      }}
    ],
    layout: {{
      name: 'cose',
      animate: false,
      nodeDimensionsIncludeLabels: true,
      padding: 30
    }}
  }});

  function inspectNode(node) {{
    if (!node || node.data('type') !== 'symbol') {{
      sidebar.classList.remove('active');
      cy.elements().removeClass('highlighted faded');
      return;
    }}

    const d = node.data();
    document.getElementById('node-name').textContent = d.label || '-';
    document.getElementById('node-kind').textContent = d.kind || '-';
    
    const visElem = document.getElementById('node-visibility');
    if (d.is_exported) {{
      visElem.innerHTML = '<span class="badge badge-exported">Public (Exported)</span>';
    }} else {{
      visElem.innerHTML = '<span class="badge badge-private">Private / Internal</span>';
    }}

    document.getElementById('node-repo').textContent = d.repo || '-';
    document.getElementById('node-location').textContent = `${{d.file || '-'}}:${{d.line || 1}}`;
    document.getElementById('node-complexity').textContent = d.complexity != null ? d.complexity : 'N/A';

    const inEdges = node.incomers('edge');
    const outEdges = node.outgoers('edge');

    document.getElementById('incoming-header').textContent = `Incoming References (${{inEdges.length}})`;
    const inList = document.getElementById('incoming-edges');
    inList.innerHTML = '';
    inEdges.forEach(e => {{
      const src = e.source();
      const li = document.createElement('li');
      li.className = 'edge-item';
      li.innerHTML = `<span>${{src.data('label') || src.id()}}</span><span style="color:#8b949e;font-size:10px">${{e.data('kind') || ''}}</span>`;
      li.addEventListener('click', () => {{
        inspectNode(src);
        cy.center(src);
      }});
      inList.appendChild(li);
    }});

    document.getElementById('outgoing-header').textContent = `Outgoing References (${{outEdges.length}})`;
    const outList = document.getElementById('outgoing-edges');
    outList.innerHTML = '';
    outEdges.forEach(e => {{
      const tgt = e.target();
      const li = document.createElement('li');
      li.className = 'edge-item';
      li.innerHTML = `<span>${{tgt.data('label') || tgt.id()}}</span><span style="color:#8b949e;font-size:10px">${{e.data('kind') || ''}}</span>`;
      li.addEventListener('click', () => {{
        inspectNode(tgt);
        cy.center(tgt);
      }});
      outList.appendChild(li);
    }});

    // Highlight neighborhood
    cy.elements().addClass('faded');
    node.removeClass('faded').addClass('highlighted');
    const neighborhood = node.neighborhood();
    neighborhood.removeClass('faded');
    neighborhood.nodes().addClass('highlighted');
    neighborhood.edges().addClass('highlighted');

    sidebar.classList.add('active');
  }}

  cy.on('tap', 'node', function(evt) {{
    inspectNode(evt.target);
  }});

  cy.on('tap', function(evt) {{
    if (evt.target === cy) {{
      inspectNode(null);
    }}
  }});

  closeSidebarBtn.addEventListener('click', () => {{
    inspectNode(null);
  }});

  btnFit.addEventListener('click', () => {{
    cy.fit(undefined, 30);
  }});

  btnReset.addEventListener('click', () => {{
    cy.reset();
    cy.fit(undefined, 30);
  }});

  function applyFilter() {{
    const term = searchInput.value.toLowerCase().trim();
    const kind = kindFilter.value;

    cy.batch(() => {{
      cy.nodes('[type = "symbol"]').forEach(n => {{
        const name = (n.data('label') || '').toLowerCase();
        const nKind = n.data('kind') || '';
        const matchText = !term || name.includes(term);
        const matchKind = !kind || nKind === kind;

        if (matchText && matchKind) {{
          n.style('display', 'element');
        }} else {{
          n.style('display', 'none');
        }}
      }});
    }});
  }}

  searchInput.addEventListener('input', applyFilter);
  kindFilter.addEventListener('change', applyFilter);
}})();
</script>
</body>
</html>
"#,
        cytoscape_js = cytoscape_js,
        json_elements = json_elements
    );

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
