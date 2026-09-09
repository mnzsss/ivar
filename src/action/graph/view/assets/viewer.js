/**
 * ivar graph view - Zero-dependency Vanilla JS Canvas 2D Graph Visualizer.
 */
(function() {
  'use strict';

  /* State Model */
  const state = {
    nodes: new Map(),
    edges: new Map(),
    selectedNodeId: null,
    highlightNodeIds: new Set(),
    highlightEdgeKeys: new Set(),
    transform: { x: 0, y: 0, k: 1 },
    drag: { isDragging: false, startX: 0, startY: 0 },
    simulation: { alpha: 1.0, settled: false, iterations: 0, maxIterations: 180 },
    depth: 1,
    filters: { repo: '', kind: '', provenance: '' }
  };

  function normalizeKind(k) {
    if (!k) return '';
    if (typeof k === 'string') return k;
    if (typeof k === 'object' && k.other) return String(k.other);
    return String(k);
  }

  /* DOM Elements */
  const canvas = document.getElementById('graph-canvas');
  const ctx = canvas.getContext('2d');
  const searchInput = document.getElementById('search-input');
  const searchResults = document.getElementById('search-results');
  const repoFilter = document.getElementById('repo-filter');
  const kindFilter = document.getElementById('kind-filter');
  const provenanceFilter = document.getElementById('provenance-filter');
  const depthSelect = document.getElementById('depth-select');
  const fitBtn = document.getElementById('fit-btn');
  const resetBtn = document.getElementById('reset-btn');
  const expandBtn = document.getElementById('expand-btn');

  /* Sidebar Elements */
  const detailName = document.getElementById('detail-name');
  const detailKind = document.getElementById('detail-kind');
  const detailRepo = document.getElementById('detail-repo');
  const detailLocation = document.getElementById('detail-location');
  const detailMeta = document.getElementById('detail-meta');
  const detailSignature = document.getElementById('detail-signature');
  const callersLabel = document.getElementById('callers-label');
  const callersList = document.getElementById('callers-list');
  const calleesLabel = document.getElementById('callees-label');
  const calleesList = document.getElementById('callees-list');

  /* Status elements */
  const statNodes = document.getElementById('stat-nodes');
  const statEdges = document.getElementById('stat-edges');
  const statDepth = document.getElementById('stat-depth');

  /* API Client Layer */
  function apiFetch(endpoint) {
    return fetch(endpoint).then(function(res) {
      if (!res.ok) throw new Error('API error: ' + res.status);
      return res.json();
    });
  }

  function loadInitialGraph() {
    apiFetch('/api/graph').then(function(graph) {
      mergeGraphData(graph);
      restartSimulation();
      setTimeout(fitGraphToView, 250);
    }).catch(function(err) {
      console.error('Failed to load initial graph:', err);
    });
  }

  function fetchNodeDetails(id) {
    const base = '/api/node';
    const endpoint = base + '?id=' + encodeURIComponent(id);
    apiFetch(endpoint).then(function(details) {
      renderDetails(details);
    }).catch(function(err) {
      console.error('Failed to load node details:', err);
    });
  }

  function fetchExpansion(id) {
    const base = '/api/expand';
    apiFetch(base + '?id=' + encodeURIComponent(id)).then(function(graph) {
      mergeGraphData(graph);
      restartSimulation();
    }).catch(function(err) {
      console.error('Failed to expand node:', err);
    });
  }

  function fetchPath(fromId, toId) {
    const base = '/api/path';
    const endpoint = base + '?from=' + encodeURIComponent(fromId) + '&to=' + encodeURIComponent(toId);
    apiFetch(endpoint).then(function(pathNodes) {
      state.highlightNodeIds = new Set(pathNodes);
      render();
    }).catch(function(err) {
      console.error('Path search failed:', err);
    });
  }

  function fetchImpact(id, direction, depth) {
    const base = '/api/impact';
    const endpoint = base + '?id=' + encodeURIComponent(id) +
      '&direction=' + encodeURIComponent(direction || 'both') +
      '&depth=' + encodeURIComponent(depth || 2);
    apiFetch(endpoint).then(function(impactNodes) {
      state.highlightNodeIds = new Set(impactNodes);
      render();
    }).catch(function(err) {
      console.error('Impact query failed:', err);
    });
  }

  function performSearch(query) {
    if (!query || query.trim().length === 0) {
      searchResults.hidden = true;
      searchResults.innerHTML = '';
      return;
    }
    const base = '/api/search';
    const endpoint = base + '?q=' + encodeURIComponent(query) +
      '&repo=' + encodeURIComponent(state.filters.repo) +
      '&kind=' + encodeURIComponent(state.filters.kind) +
      '&limit=20';
    apiFetch(endpoint).then(function(results) {
      renderSearchResults(results);
    }).catch(function(err) {
      console.error('Search failed:', err);
    });
  }

  /* Graph Merging & Simulation */
  function mergeGraphData(graph) {
    if (!graph || !graph.nodes) return;

    graph.nodes.forEach(function(n) {
      if (!state.nodes.has(n.id)) {
        const angle = (n.id * 137.5) * (Math.PI / 180);
        const radius = 60 + (n.id % 20) * 15;
        state.nodes.set(n.id, {
          id: n.id,
          repo: n.repo,
          name: n.name,
          kind: normalizeKind(n.kind),
          signature: n.signature,
          file: n.file,
          span: n.span,
          exported: n.exported,
          complexity: n.complexity,
          x: Math.cos(angle) * radius + (canvas.width / 2),
          y: Math.sin(angle) * radius + (canvas.height / 2),
          vx: 0,
          vy: 0
        });
      }
    });

    if (graph.edges) {
      graph.edges.forEach(function(e) {
        const key = e.from + '->' + e.to + ':' + e.kind;
        if (!state.edges.has(key)) {
          state.edges.set(key, e);
        }
      });
    }

    updateStats();
    updateFilterOptions();
  }

  function restartSimulation() {
    state.simulation.alpha = 1.0;
    state.simulation.settled = false;
    state.simulation.iterations = 0;
    requestAnimationFrame(animationLoop);
  }

  function stepSimulation() {
    if (state.simulation.settled) return;

    const nodes = Array.from(state.nodes.values());
    const kRepel = 1200;
    const kAttract = 0.04;
    const centerAttract = 0.01;
    const cx = canvas.width > 0 ? canvas.width / 2 : 400;
    const cy = canvas.height > 0 ? canvas.height / 2 : 300;

    for (let i = 0; i < nodes.length; i++) {
      const u = nodes[i];
      for (let j = i + 1; j < nodes.length; j++) {
        const v = nodes[j];
        const dx = v.x - u.x;
        const dy = v.y - u.y;
        const distSq = dx * dx + dy * dy + 1;
        if (distSq > 90000) continue;
        const dist = Math.sqrt(distSq);
        const force = (kRepel / distSq) * state.simulation.alpha;
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        u.vx -= fx;
        u.vy -= fy;
        v.vx += fx;
        v.vy += fy;
      }
    }

    state.edges.forEach(function(edge) {
      const u = state.nodes.get(edge.from);
      const v = state.nodes.get(edge.to);
      if (u && v) {
        const dx = v.x - u.x;
        const dy = v.y - u.y;
        const dist = Math.sqrt(dx * dx + dy * dy) + 0.1;
        const force = (dist - 80) * kAttract * state.simulation.alpha;
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        u.vx += fx;
        u.vy += fy;
        v.vx -= fx;
        v.vy -= fy;
      }
    });

    nodes.forEach(function(u) {
      u.vx += (cx - u.x) * centerAttract * state.simulation.alpha;
      u.vy += (cy - u.y) * centerAttract * state.simulation.alpha;
      const speed = Math.sqrt(u.vx * u.vx + u.vy * u.vy);
      if (speed > 40) {
        u.vx = (u.vx / speed) * 40;
        u.vy = (u.vy / speed) * 40;
      }
      u.x += u.vx * 0.85;
      u.y += u.vy * 0.85;
      u.vx *= 0.5;
      u.vy *= 0.5;
    });

    state.simulation.iterations++;
    state.simulation.alpha *= 0.94;
    if (state.simulation.alpha < 0.005 || state.simulation.iterations >= 90) {
      state.simulation.settled = true;
    }
  }

  /* Rendering Layer */
  function render() {
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.save();
    ctx.translate(state.transform.x, state.transform.y);
    ctx.scale(state.transform.k, state.transform.k);

    /* 1. Regular direct edges (batched) */
    ctx.beginPath();
    ctx.setLineDash([]);
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(242, 235, 221, 0.25)';
    state.edges.forEach(function(edge) {
      const u = state.nodes.get(edge.from);
      const v = state.nodes.get(edge.to);
      if (!u || !v) return;
      const isAmbiguous = edge.provenance === 'ambiguous' || edge.provenance === 'inferred';
      const isSelected = state.selectedNodeId === u.id || state.selectedNodeId === v.id;
      if (isAmbiguous || isSelected) return;

      const uMatch = (!state.filters.repo || u.repo === state.filters.repo) && (!state.filters.kind || u.kind === state.filters.kind);
      const vMatch = (!state.filters.repo || v.repo === state.filters.repo) && (!state.filters.kind || v.kind === state.filters.kind);
      const provMatch = !state.filters.provenance || edge.provenance === state.filters.provenance;
      if (!uMatch || !vMatch || !provMatch) return; /* skip filtered */

      ctx.moveTo(u.x, u.y);
      ctx.lineTo(v.x, v.y);
    });
    ctx.stroke();

    /* 2. Ambiguous / inferred edges (batched dashed) */
    ctx.beginPath();
    ctx.setLineDash([4, 4]);
    ctx.lineWidth = 1;
    ctx.strokeStyle = '#d97735';
    state.edges.forEach(function(edge) {
      const u = state.nodes.get(edge.from);
      const v = state.nodes.get(edge.to);
      if (!u || !v) return;
      const isAmbiguous = edge.provenance === 'ambiguous' || edge.provenance === 'inferred';
      const isSelected = state.selectedNodeId === u.id || state.selectedNodeId === v.id;
      if (!isAmbiguous || isSelected) return;

      const uMatch = (!state.filters.repo || u.repo === state.filters.repo) && (!state.filters.kind || u.kind === state.filters.kind);
      const vMatch = (!state.filters.repo || v.repo === state.filters.repo) && (!state.filters.kind || v.kind === state.filters.kind);
      const provMatch = !state.filters.provenance || edge.provenance === state.filters.provenance;
      if (!uMatch || !vMatch || !provMatch) return;

      ctx.moveTo(u.x, u.y);
      ctx.lineTo(v.x, v.y);
    });
    ctx.stroke();

    /* 3. Highlighted / selected edges */
    if (state.selectedNodeId !== null) {
      ctx.beginPath();
      ctx.setLineDash([]);
      ctx.lineWidth = 2;
      ctx.strokeStyle = '#8ba6ff';
      state.edges.forEach(function(edge) {
        const u = state.nodes.get(edge.from);
        const v = state.nodes.get(edge.to);
        if (!u || !v) return;
        if (state.selectedNodeId === u.id || state.selectedNodeId === v.id) {
          ctx.moveTo(u.x, u.y);
          ctx.lineTo(v.x, v.y);
        }
      });
      ctx.stroke();
    }
    ctx.setLineDash([]);

    /* 4. Primary / default nodes (batched) */
    ctx.fillStyle = '#f2ebdd';
    ctx.beginPath();
    state.nodes.forEach(function(node) {
      const isSelected = state.selectedNodeId === node.id;
      const isImpact = state.highlightNodeIds.has(node.id);
      if (isSelected || isImpact) return;
      const matchesRepo = !state.filters.repo || node.repo === state.filters.repo;
      const matchesKind = !state.filters.kind || node.kind === state.filters.kind;
      if (!matchesRepo || !matchesKind) return;
      ctx.moveTo(node.x + 5, node.y);
      ctx.arc(node.x, node.y, 5, 0, 2 * Math.PI);
    });
    ctx.fill();

    /* 5. Impact nodes (batched) */
    if (state.highlightNodeIds.size > 0) {
      ctx.fillStyle = '#d97735';
      ctx.beginPath();
      state.nodes.forEach(function(node) {
        const isImpact = state.highlightNodeIds.has(node.id);
        const isSelected = state.selectedNodeId === node.id;
        if (!isImpact || isSelected) return;
        ctx.moveTo(node.x + 6, node.y);
        ctx.arc(node.x, node.y, 6, 0, 2 * Math.PI);
      });
      ctx.fill();
    }

    /* 6. Selected node */
    if (state.selectedNodeId !== null) {
      const sel = state.nodes.get(state.selectedNodeId);
      if (sel) {
        ctx.fillStyle = '#8ba6ff';
        ctx.beginPath();
        ctx.arc(sel.x, sel.y, 8, 0, 2 * Math.PI);
        ctx.fill();
      }
    }

    /* 7. Text labels */
    ctx.font = '11px Fira Code';
    state.nodes.forEach(function(node) {
      const isSelected = state.selectedNodeId === node.id;
      const isImpact = state.highlightNodeIds.has(node.id);
      const matchesRepo = !state.filters.repo || node.repo === state.filters.repo;
      const matchesKind = !state.filters.kind || node.kind === state.filters.kind;
      if (!matchesRepo || !matchesKind) return;

      if (state.transform.k >= 0.45 || isSelected || isImpact) {
        ctx.fillStyle = isSelected ? '#8ba6ff' : (isImpact ? '#d97735' : '#f2ebdd');
        ctx.fillText(node.name, node.x + 9, node.y + 4);
      }
    });

    ctx.restore();
  }

  function animationLoop() {
    if (!state.simulation.settled) {
      stepSimulation();
      render();
      requestAnimationFrame(animationLoop);
    } else {
      render();
    }
  }

  /* Interaction & Event Handlers */
  function setupEvents() {
    window.addEventListener('resize', function() {
      canvas.width = canvas.parentElement.clientWidth;
      canvas.height = canvas.parentElement.clientHeight;
      render();
    });

    canvas.addEventListener('mousedown', function(e) {
      const rect = canvas.getBoundingClientRect();
      const clickX = (e.clientX - rect.left - state.transform.x) / state.transform.k;
      const clickY = (e.clientY - rect.top - state.transform.y) / state.transform.k;

      let clickedNode = null;
      state.nodes.forEach(function(node) {
        const dx = node.x - clickX;
        const dy = node.y - clickY;
        if (dx * dx + dy * dy <= 64) {
          clickedNode = node;
        }
      });

      if (clickedNode) {
        selectNode(clickedNode.id);
      } else {
        state.drag.isDragging = true;
        state.drag.startX = e.clientX - state.transform.x;
        state.drag.startY = e.clientY - state.transform.y;
      }
    });

    window.addEventListener('mousemove', function(e) {
      if (state.drag.isDragging) {
        state.transform.x = e.clientX - state.drag.startX;
        state.transform.y = e.clientY - state.drag.startY;
        render();
      }
    });

    window.addEventListener('mouseup', function() {
      state.drag.isDragging = false;
    });

    canvas.addEventListener('wheel', function(e) {
      e.preventDefault();
      const zoomFactor = e.deltaY < 0 ? 1.1 : 0.9;
      state.transform.k = Math.max(0.2, Math.min(4.0, state.transform.k * zoomFactor));
      render();
    }, { passive: false });

    fitBtn.addEventListener('click', fitGraphToView);
    resetBtn.addEventListener('click', resetView);
    expandBtn.addEventListener('click', function() {
      if (state.selectedNodeId !== null) {
        fetchExpansion(state.selectedNodeId);
      }
    });

    let searchDebounce = null;
    searchInput.addEventListener('input', function(e) {
      clearTimeout(searchDebounce);
      searchDebounce = setTimeout(function() {
        performSearch(e.target.value);
      }, 150);
    });

    repoFilter.addEventListener('change', function(e) {
      state.filters.repo = e.target.value;
      restartSimulation();
    });
    kindFilter.addEventListener('change', function(e) {
      state.filters.kind = e.target.value;
      render();
    });
    provenanceFilter.addEventListener('change', function(e) {
      state.filters.provenance = e.target.value;
      render();
    });
    depthSelect.addEventListener('change', function(e) {
      state.depth = parseInt(e.target.value, 10) || 1;
      statDepth.textContent = 'depth ' + state.depth;
    });
  }

  function selectNode(id) {
    state.selectedNodeId = id;
    expandBtn.disabled = false;
    fetchNodeDetails(id);
    render();
  }

  function fitGraphToView() {
    if (state.nodes.size === 0) return;
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    state.nodes.forEach(function(node) {
      if (node.x < minX) minX = node.x;
      if (node.x > maxX) maxX = node.x;
      if (node.y < minY) minY = node.y;
      if (node.y > maxY) maxY = node.y;
    });
    const padding = 60;
    const gw = (maxX - minX) || 100;
    const gh = (maxY - minY) || 100;
    const scale = Math.min((canvas.width - padding * 2) / gw, (canvas.height - padding * 2) / gh, 2.0);
    state.transform.k = Math.max(0.2, scale);
    state.transform.x = (canvas.width - gw * state.transform.k) / 2 - minX * state.transform.k;
    state.transform.y = (canvas.height - gh * state.transform.k) / 2 - minY * state.transform.k;
    render();
  }

  function resetView() {
    state.transform.x = 0;
    state.transform.y = 0;
    state.transform.k = 1;
    restartSimulation();
  }

  function renderDetails(details) {
    if (!details || !details.node) return;
    const node = details.node;
    detailName.textContent = node.name;
    detailKind.textContent = normalizeKind(node.kind);
    detailRepo.textContent = node.repo;
    detailLocation.textContent = node.file + (node.span ? ':' + node.span.start_line : '');
    detailMeta.textContent = (node.exported ? 'Exported' : 'Private') +
      (node.complexity ? ' · Complexity ' + node.complexity : '');
    detailSignature.textContent = node.signature || '-';

    callersList.innerHTML = '';
    const callers = details.callers || [];
    callersLabel.textContent = 'Callers (' + callers.length + ')';
    callers.forEach(function(c) {
      const li = document.createElement('li');
      li.className = 'ref-item';
      li.textContent = c.name + ' (' + c.repo + ')';
      li.addEventListener('click', function() { selectNode(c.id); });
      callersList.appendChild(li);
    });

    calleesList.innerHTML = '';
    const callees = details.callees || [];
    calleesLabel.textContent = 'Callees (' + callees.length + ')';
    callees.forEach(function(c) {
      const li = document.createElement('li');
      li.className = 'ref-item';
      li.textContent = c.name + ' (' + c.repo + ')';
      li.addEventListener('click', function() { selectNode(c.id); });
      calleesList.appendChild(li);
    });
  }

  function renderSearchResults(results) {
    searchResults.innerHTML = '';
    if (!results || results.length === 0) {
      searchResults.hidden = true;
      return;
    }
    results.forEach(function(item) {
      const s = item.symbol || item;
      const div = document.createElement('div');
      div.className = 'search-item';
      div.innerHTML = '<span>' + s.name + '</span><span class="prop-label">' + normalizeKind(s.kind) + '</span>';
      div.addEventListener('click', function() {
        searchResults.hidden = true;
        selectNode(s.id);
      });
      searchResults.appendChild(div);
    });
    searchResults.hidden = false;
  }

  function updateStats() {
    statNodes.textContent = state.nodes.size + ' nodes';
    statEdges.textContent = state.edges.size + ' edges';
    statDepth.textContent = 'depth ' + state.depth;
  }

  function updateFilterOptions() {
    const repos = new Set();
    const kinds = new Set();
    state.nodes.forEach(function(n) {
      if (n.repo) repos.add(n.repo);
      if (n.kind) kinds.add(normalizeKind(n.kind));
    });
    repoFilter.innerHTML = '<option value="">All Repos</option>';
    repos.forEach(function(r) {
      const opt = document.createElement('option');
      opt.value = r;
      opt.textContent = r;
      if (state.filters.repo === r) opt.selected = true;
      repoFilter.appendChild(opt);
    });
    kindFilter.innerHTML = '<option value="">All Kinds</option>';
    kinds.forEach(function(k) {
      const opt = document.createElement('option');
      opt.value = k;
      opt.textContent = k;
      if (state.filters.kind === k) opt.selected = true;
      kindFilter.appendChild(opt);
    });
  }


  /* Initialization */
  function init() {
    const w = (canvas.parentElement && canvas.parentElement.clientWidth) || window.innerWidth || 800;
    const h = (canvas.parentElement && canvas.parentElement.clientHeight) || (window.innerHeight - 48) || 600;
    canvas.width = w;
    canvas.height = h;
    setupEvents();
    loadInitialGraph();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
