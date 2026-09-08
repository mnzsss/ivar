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

/// Generates a 100% self-contained, air-gapped HTML5 visualizer string for `data`.
pub fn generate_html(data: &VizData) -> Result<String, VizError> {
    let raw_json = serde_json::to_string(data)?;
    // Escape closing tags and characters that could prematurely terminate HTML script blocks
    let json_data = raw_json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Codebase Graph Visualizer</title>
<style>
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: #0f141c; color: #e1e7ec; height: 100vh; overflow: hidden; display: flex; flex-direction: column; }}
header {{ height: 52px; background: #161d27; border-bottom: 1px solid #263342; display: flex; align-items: center; justify-content: space-between; padding: 0 16px; gap: 16px; flex-shrink: 0; }}
.header-left {{ display: flex; align-items: center; gap: 16px; }}
h1 {{ font-size: 16px; font-weight: 600; color: #f0f6fc; letter-spacing: 0.5px; white-space: nowrap; }}
.stats-badge {{ font-size: 12px; color: #8b949e; background: #21262d; border: 1px solid #30363d; padding: 3px 8px; border-radius: 12px; }}
.controls {{ display: flex; align-items: center; gap: 12px; }}
input[type="text"], select {{ background: #0d1117; border: 1px solid #30363d; color: #c9d1d9; padding: 6px 12px; border-radius: 6px; font-size: 13px; outline: none; }}
input[type="text"]:focus, select:focus {{ border-color: #58a6ff; }}
#main-container {{ flex: 1; position: relative; width: 100%; height: calc(100vh - 52px); display: flex; }}
canvas {{ flex: 1; width: 100%; height: 100%; background: #0b0e14; cursor: grab; }}
canvas:active {{ cursor: grabbing; }}
#sidebar {{ width: 340px; background: #161d27; border-left: 1px solid #263342; padding: 20px; overflow-y: auto; display: none; flex-direction: column; gap: 16px; position: absolute; right: 0; top: 0; bottom: 0; box-shadow: -4px 0 16px rgba(0,0,0,0.4); z-index: 10; }}
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
.instructions {{ position: absolute; left: 16px; bottom: 16px; background: rgba(22, 29, 39, 0.85); border: 1px solid #263342; border-radius: 6px; padding: 8px 12px; font-size: 11px; color: #8b949e; pointer-events: none; }}
</style>
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
  </div>
</header>
<div id="main-container">
  <canvas id="graph-canvas"></canvas>
  <div class="instructions">Scroll: Zoom &bull; Drag: Pan &bull; Click Node: Inspect</div>
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
const GRAPH_DATA = {json_data};

(function() {{
  const canvas = document.getElementById('graph-canvas');
  const ctx = canvas.getContext('2d');
  const searchInput = document.getElementById('search-input');
  const kindFilter = document.getElementById('kind-filter');
  const statsBadge = document.getElementById('stats-badge');
  const sidebar = document.getElementById('sidebar');
  const closeSidebarBtn = document.getElementById('close-sidebar');

  let width = canvas.clientWidth;
  let height = canvas.clientHeight;
  canvas.width = width;
  canvas.height = height;

  window.addEventListener('resize', () => {{
    width = canvas.clientWidth;
    height = canvas.clientHeight;
    canvas.width = width;
    canvas.height = height;
  }});

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

  const nodes = GRAPH_DATA.nodes || [];
  const edges = GRAPH_DATA.edges || [];
  const nodeMap = new Map();

  const kinds = new Set();
  nodes.forEach((n, idx) => {{
    kinds.add(n.kind);
    const angle = (idx / Math.max(1, nodes.length)) * Math.PI * 2;
    const radius = Math.min(width, height) * 0.35 * Math.sqrt(Math.random());
    const simNode = {{
      ...n,
      x: width / 2 + Math.cos(angle) * radius,
      y: height / 2 + Math.sin(angle) * radius,
      vx: (Math.random() - 0.5) * 2,
      vy: (Math.random() - 0.5) * 2,
      radius: Math.max(5, Math.min(18, 6 + (n.complexity || 1) * 0.5))
    }};
    nodeMap.set(n.id, simNode);
  }});

  kinds.forEach(k => {{
    const opt = document.createElement('option');
    opt.value = k;
    opt.textContent = k;
    kindFilter.appendChild(opt);
  }});

  statsBadge.textContent = `${{nodes.length}} symbols \u2022 ${{edges.length}} edges`;

  const simEdges = edges.map(e => ({{
    source: nodeMap.get(e.from),
    target: nodeMap.get(e.to),
    kind: e.kind
  }})).filter(e => e.source && e.target);

  let transform = {{ x: 0, y: 0, k: 1 }};
  let selectedNode = null;
  let hoveredNode = null;
  let isDragging = false;
  let dragNode = null;
  let dragStart = {{ x: 0, y: 0 }};

  function simulate() {{
    const simNodes = Array.from(nodeMap.values());
    const alpha = 0.05;

    for (let i = 0; i < simNodes.length; i++) {{
      for (let j = i + 1; j < simNodes.length; j++) {{
        const n1 = simNodes[i];
        const n2 = simNodes[j];
        const dx = n2.x - n1.x;
        const dy = n2.y - n1.y;
        const distSq = dx * dx + dy * dy + 0.01;
        const dist = Math.sqrt(distSq);
        if (dist < 300) {{
          const force = (800 / distSq) * alpha;
          const fx = (dx / dist) * force;
          const fy = (dy / dist) * force;
          n1.vx -= fx;
          n1.vy -= fy;
          n2.vx += fx;
          n2.vy += fy;
        }}
      }}
    }}

    for (const edge of simEdges) {{
      const n1 = edge.source;
      const n2 = edge.target;
      const dx = n2.x - n1.x;
      const dy = n2.y - n1.y;
      const dist = Math.sqrt(dx * dx + dy * dy) + 0.001;
      const targetDist = 70;
      const force = (dist - targetDist) * 0.008 * alpha;
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      n1.vx += fx;
      n1.vy += fy;
      n2.vx -= fx;
      n2.vy -= fy;
    }}

    const cx = width / 2;
    const cy = height / 2;
    for (const n of simNodes) {{
      n.vx += (cx - n.x) * 0.0005;
      n.vy += (cy - n.y) * 0.0005;

      n.vx *= 0.88;
      n.vy *= 0.88;

      if (n !== dragNode) {{
        n.x += n.vx;
        n.y += n.vy;
      }}
    }}
  }}

  function render() {{
    simulate();

    ctx.clearRect(0, 0, width, height);
    ctx.save();
    ctx.translate(transform.x, transform.y);
    ctx.scale(transform.k, transform.k);

    const query = searchInput.value.trim().toLowerCase();
    const selectedKind = kindFilter.value;

    ctx.lineWidth = 1;
    for (const edge of simEdges) {{
      const isConnected = selectedNode && (edge.source.id === selectedNode.id || edge.target.id === selectedNode.id);
      ctx.beginPath();
      ctx.moveTo(edge.source.x, edge.source.y);
      ctx.lineTo(edge.target.x, edge.target.y);
      if (isConnected) {{
        ctx.strokeStyle = '#58a6ff';
        ctx.lineWidth = 2;
      }} else {{
        ctx.strokeStyle = '#21262d';
        ctx.lineWidth = 1;
      }}
      ctx.stroke();
    }}

    const simNodes = Array.from(nodeMap.values());
    for (const n of simNodes) {{
      const matchesSearch = !query || n.name.toLowerCase().includes(query) || n.file.toLowerCase().includes(query);
      const matchesKind = !selectedKind || n.kind === selectedKind;
      const isMatch = matchesSearch && matchesKind;

      const isSelected = selectedNode && selectedNode.id === n.id;
      const isHovered = hoveredNode && hoveredNode.id === n.id;

      ctx.beginPath();
      ctx.arc(n.x, n.y, n.radius, 0, Math.PI * 2);

      const color = KIND_COLORS[n.kind] || '#94a3b8';
      ctx.fillStyle = isMatch ? color : '#30363d';
      ctx.globalAlpha = isMatch ? 1.0 : 0.2;
      ctx.fill();

      if (isSelected || isHovered) {{
        ctx.strokeStyle = '#f0f6fc';
        ctx.lineWidth = isSelected ? 3 : 2;
        ctx.stroke();
      }}

      ctx.globalAlpha = 1.0;

      if (isMatch && (transform.k > 0.8 || isSelected || isHovered)) {{
        ctx.font = '11px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
        ctx.fillStyle = isSelected ? '#58a6ff' : '#c9d1d9';
        ctx.textAlign = 'center';
        ctx.fillText(n.name, n.x, n.y + n.radius + 12);
      }}
    }}

    ctx.restore();
    requestAnimationFrame(render);
  }}

  function screenToWorld(sx, sy) {{
    return {{
      x: (sx - transform.x) / transform.k,
      y: (sy - transform.y) / transform.k
    }};
  }}

  function getNodeAt(x, y) {{
    const simNodes = Array.from(nodeMap.values());
    for (let i = simNodes.length - 1; i >= 0; i--) {{
      const n = simNodes[i];
      const dx = n.x - x;
      const dy = n.y - y;
      if (dx * dx + dy * dy <= (n.radius + 4) * (n.radius + 4)) {{
        return n;
      }}
    }}
    return null;
  }}

  function inspectNode(node) {{
    selectedNode = node;
    if (!node) {{
      sidebar.classList.remove('active');
      return;
    }}

    document.getElementById('node-name').textContent = node.name;
    document.getElementById('node-kind').textContent = node.kind;
    document.getElementById('node-visibility').innerHTML = node.is_exported
      ? '<span class="badge badge-exported">Exported</span>'
      : '<span class="badge badge-private">Internal</span>';
    document.getElementById('node-repo').textContent = node.repo;
    document.getElementById('node-location').textContent = `${{node.file}}:${{node.line}}`;
    document.getElementById('node-complexity').textContent = node.complexity !== null && node.complexity !== undefined ? node.complexity : 'N/A';

    const incoming = edges.filter(e => e.to === node.id);
    const outgoing = edges.filter(e => e.from === node.id);

    const incList = document.getElementById('incoming-edges');
    const outList = document.getElementById('outgoing-edges');
    document.getElementById('incoming-header').textContent = `Incoming References (${{incoming.length}})`;
    document.getElementById('outgoing-header').textContent = `Outgoing References (${{outgoing.length}})`;

    incList.innerHTML = '';
    outgoing.length;
    incoming.forEach(e => {{
      const fromNode = nodeMap.get(e.from);
      const li = document.createElement('li');
      li.className = 'edge-item';
      li.innerHTML = `<span>${{fromNode ? fromNode.name : '#' + e.from}}</span><span style="color:#8b949e">${{e.kind}}</span>`;
      if (fromNode) {{
        li.addEventListener('click', () => inspectNode(fromNode));
      }}
      incList.appendChild(li);
    }});

    outList.innerHTML = '';
    outgoing.forEach(e => {{
      const toNode = nodeMap.get(e.to);
      const li = document.createElement('li');
      li.className = 'edge-item';
      li.innerHTML = `<span>${{toNode ? toNode.name : '#' + e.to}}</span><span style="color:#8b949e">${{e.kind}}</span>`;
      if (toNode) {{
        li.addEventListener('click', () => inspectNode(toNode));
      }}
      outList.appendChild(li);
    }});

    sidebar.classList.add('active');
  }}

  canvas.addEventListener('mousedown', e => {{
    const rect = canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const wpos = screenToWorld(sx, sy);
    const hit = getNodeAt(wpos.x, wpos.y);

    if (hit) {{
      dragNode = hit;
      dragNode.vx = 0;
      dragNode.vy = 0;
    }} else {{
      isDragging = true;
      dragStart = {{ x: sx - transform.x, y: sy - transform.y }};
    }}
  }});

  canvas.addEventListener('mousemove', e => {{
    const rect = canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const wpos = screenToWorld(sx, sy);

    if (dragNode) {{
      dragNode.x = wpos.x;
      dragNode.y = wpos.y;
      dragNode.vx = 0;
      dragNode.vy = 0;
    }} else if (isDragging) {{
      transform.x = sx - dragStart.x;
      transform.y = sy - dragStart.y;
    }} else {{
      hoveredNode = getNodeAt(wpos.x, wpos.y);
    }}
  }});

  window.addEventListener('mouseup', () => {{
    dragNode = null;
    isDragging = false;
  }});

  canvas.addEventListener('click', e => {{
    const rect = canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const wpos = screenToWorld(sx, sy);
    const hit = getNodeAt(wpos.x, wpos.y);
    inspectNode(hit);
  }});

  canvas.addEventListener('wheel', e => {{
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const zoomFactor = e.deltaY < 0 ? 1.1 : 0.9;
    const newK = Math.max(0.1, Math.min(5, transform.k * zoomFactor));

    transform.x = sx - (sx - transform.x) * (newK / transform.k);
    transform.y = sy - (sy - transform.y) * (newK / transform.k);
    transform.k = newK;
  }}, {{ passive: false }});

  closeSidebarBtn.addEventListener('click', () => {{
    inspectNode(null);
  }});

  requestAnimationFrame(render);
}})();
</script>
</body>
</html>
"#,
        json_data = json_data
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
