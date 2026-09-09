/*
 * ivar graph view - Cytoscape.js Graph Visualizer with Compound Hierarchy & Dual Layouts.
 */
(function() {
  'use strict';

  /* State Model */
  var state = {
    nodes: new Map(),
    edges: new Map(),
    selectedNodeId: null,
    highlightNodeIds: new Set(),
    highlightEdgeKeys: new Set(),
    depth: 1,
    currentLayout: 'dagre',
    filters: { repo: '', kind: '', provenance: '', search: '' },
    tracing: { active: false, sourceNode: null, targetNode: null },
    simulation: { alpha: 0.0, settled: true, iterations: 0 }
  };

  var cy = null;

  function normalizeKind(k) {
    if (!k) return '';
    if (typeof k === 'string') return k;
    if (typeof k === 'object' && k.other) return String(k.other);
    return String(k);
  }

  /* DOM Elements */
  var cyContainer = document.getElementById('cy');
  var searchInput = document.getElementById('search-input');
  var searchResults = document.getElementById('search-results');
  var repoFilter = document.getElementById('repo-filter');
  var kindFilter = document.getElementById('kind-filter');
  var provenanceFilter = document.getElementById('provenance-filter');
  var depthSelect = document.getElementById('depth-select');
  var layoutSelect = document.getElementById('layout-select');
  var traceBtn = document.getElementById('trace-btn');
  var fitBtn = document.getElementById('fit-btn');
  var resetBtn = document.getElementById('reset-btn');
  var expandBtn = document.getElementById('expand-btn');
  /* Sidebar Elements */
  var detailName = document.getElementById('detail-name');
  var detailKind = document.getElementById('detail-kind');
  var detailRepo = document.getElementById('detail-repo');
  var detailLocation = document.getElementById('detail-location');
  var detailMeta = document.getElementById('detail-meta');
  var detailSignature = document.getElementById('detail-signature');
  var callersLabel = document.getElementById('callers-label');
  var callersList = document.getElementById('callers-list');
  var calleesLabel = document.getElementById('callees-label');
  var calleesList = document.getElementById('callees-list');

  /* Status elements */
  var statNodes = document.getElementById('stat-nodes');
  var statEdges = document.getElementById('stat-edges');
  var statDepth = document.getElementById('stat-depth');

  /* API Client Layer */
  function apiFetch(endpoint) {
    return fetch(endpoint).then(function(res) {
      if (!res.ok) throw new Error('API error: ' + res.status);
      return res.json();
    });
  }

  function loadInitialGraph() {
    var depth = state.depth || 1;
    var base = '/api/subgraph';
    apiFetch(base + '?depth=' + encodeURIComponent(depth)).then(function(graph) {
      applyGraphData(graph);
    }).catch(function(err) {
      /* Fallback to /api/graph if /api/subgraph fails */
      apiFetch('/api/graph').then(function(graph) {
        applyGraphData(graph);
      }).catch(function(e) {
        console.error('Failed to load initial graph:', e);
      });
    });
  }

  function fetchNodeDetails(id) {
    var base = '/api/node';
    var endpoint = base + '?id=' + encodeURIComponent(id);
    apiFetch(endpoint).then(function(details) {
      renderDetails(details);
    }).catch(function(err) {
      console.error('Failed to load node details:', err);
    });
  }

  function fetchExpansion(id) {
    var base = '/api/expand';
    apiFetch(base + '?id=' + encodeURIComponent(id)).then(function(graph) {
      mergeGraphData(graph);
    }).catch(function(err) {
      console.error('Failed to expand node:', err);
    });
  }

  function fetchPath(fromId, toId) {
    var base = '/api/path';
    var endpoint = base + '?from=' + encodeURIComponent(fromId) + '&to=' + encodeURIComponent(toId);
    apiFetch(endpoint).then(function(pathNodes) {
      state.highlightNodeIds = new Set(pathNodes);
      updateHighlights();
    }).catch(function(err) {
      console.error('Path search failed:', err);
    });
  }

  function fetchImpact(id, direction, depth) {
    var base = '/api/impact';
    var endpoint = base + '?id=' + encodeURIComponent(id) +
      '&direction=' + encodeURIComponent(direction || 'both') +
      '&depth=' + encodeURIComponent(depth || 2);
    apiFetch(endpoint).then(function(impactNodes) {
      state.highlightNodeIds = new Set(impactNodes);
      updateHighlights();
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
    var base = '/api/search';
    var endpoint = base + '?q=' + encodeURIComponent(query) +
      '&repo=' + encodeURIComponent(state.filters.repo) +
      '&kind=' + encodeURIComponent(state.filters.kind) +
      '&limit=20';
    apiFetch(endpoint).then(function(results) {
      renderSearchResults(results);
    }).catch(function(err) {
      console.error('Search failed:', err);
    });
  }

  /* Cytoscape Initialization & Styling */
  function initCytoscape() {
    if (!window.cytoscape) {
      console.error('Cytoscape library not loaded');
      return;
    }

    window._cyCore = cy = window.cytoscape({
      container: cyContainer,
      boxSelectionEnabled: false,
      autounselectify: false,
      minZoom: 0.05,
      maxZoom: 3.5,
      wheelSensitivity: 0.3,
      style: [
        /* Base compound parent styling (Repo & File) */
        {
          selector: ':parent',
          style: {
            'background-color': '#1a1a1a',
            'background-opacity': 0.7,
            'border-width': 1,
            'border-color': '#2e2e2e',
            'border-opacity': 0.9,
            'shape': 'roundrectangle',
            'corner-radius': '6px',
            'padding': '14px',
            'color': '#c8c0b2',
            'font-family': 'Fira Code, monospace',
            'font-size': '11px',
            'text-valign': 'top',
            'text-halign': 'center',
            'text-margin-y': -4
          }
        },
        /* Repo compound container */
        {
          selector: 'node[type = "repo"]',
          style: {
            'background-color': '#171717',
            'border-color': '#3a3a3a',
            'border-width': 1.5,
            'color': '#f2ebdd',
            'font-weight': 'bold',
            'font-size': '12px'
          }
        },
        /* File compound container */
        {
          selector: 'node[type = "file"]',
          style: {
            'background-color': '#202020',
            'border-color': '#2a2a2a',
            'border-width': 1,
            'color': '#a0988c',
            'font-size': '10px'
          }
        },
        /* Symbol leaf nodes */
        {
          selector: 'node[type = "symbol"]',
          style: {
            'width': 28,
            'height': 28,
            'background-color': '#2a2a2a',
            'border-width': 2,
            'border-color': '#8ba6ff',
            'label': 'data(label)',
            'color': '#f2ebdd',
            'font-family': 'Fira Code, monospace',
            'font-size': '11px',
            'text-valign': 'bottom',
            'text-halign': 'center',
            'text-margin-y': 4,
            'text-background-color': '#151515',
            'text-background-opacity': 0.8,
            'text-background-padding': '2px',
            'text-background-shape': 'roundrectangle',
            'transition-property': 'background-color, border-color, width, height',
            'transition-duration': '0.15s'
          }
        },
        /* Symbol exported / private styling */
        {
          selector: 'node[type = "symbol"][?exported]',
          style: {
            'border-color': '#8ba6ff',
            'border-width': 2.5
          }
        },
        {
          selector: 'node[type = "symbol"][!exported]',
          style: {
            'border-color': '#5a554c',
            'border-width': 1.5,
            'border-style': 'dashed'
          }
        },
        /* Symbol kind color variations */
        {
          selector: 'node[kind = "function"], node[kind = "method"]',
          style: {
            'background-color': '#232b38',
            'shape': 'ellipse'
          }
        },
        {
          selector: 'node[kind = "struct"], node[kind = "class"], node[kind = "type"]',
          style: {
            'background-color': '#2b2620',
            'shape': 'round-rectangle',
            'border-color': '#d97735'
          }
        },
        {
          selector: 'node[kind = "trait"], node[kind = "interface"]',
          style: {
            'background-color': '#26222e',
            'shape': 'diamond',
            'border-color': '#c084fc'
          }
        },
        {
          selector: 'node[kind = "enum"], node[kind = "constant"], node[kind = "module"]',
          style: {
            'background-color': '#1f2d27',
            'shape': 'hexagon',
            'border-color': '#4ade80'
          }
        },
        /* Selection & Highlight & Dim & Path States */
        {
          selector: 'node:selected, node.selected',
          style: {
            'border-color': '#f2ebdd',
            'border-width': 3,
            'background-color': '#3e4c6b',
            'shadow-blur': 12,
            'shadow-color': '#8ba6ff',
            'shadow-opacity': 0.6
          }
        },
        {
          selector: 'node.highlighted',
          style: {
            'border-color': '#d97735',
            'border-width': 3,
            'background-color': '#4a2e1d',
            'shadow-blur': 10,
            'shadow-color': '#d97735',
            'shadow-opacity': 0.5,
            'opacity': 1.0
          }
        },
        {
          selector: 'node.dimmed',
          style: {
            'opacity': 0.15
          }
        },
        {
          selector: 'node.path-highlight',
          style: {
            'border-color': '#38bdf8',
            'border-width': 4,
            'background-color': '#0369a1',
            'shadow-blur': 14,
            'shadow-color': '#38bdf8',
            'shadow-opacity': 0.8,
            'opacity': 1.0,
            'z-index': 9999
          }
        },
        /* Edge Styling */
        {
          selector: 'edge',
          style: {
            'width': 1.5,
            'line-color': '#4a4844',
            'target-arrow-color': '#4a4844',
            'target-arrow-shape': 'triangle',
            'arrow-scale': 0.8,
            'curve-style': 'bezier',
            'opacity': 0.8
          }
        },
        /* Edge Provenance Semantics */
        {
          selector: 'edge[provenance = "extracted"]',
          style: {
            'line-style': 'solid',
            'line-color': '#5e5a52',
            'target-arrow-color': '#5e5a52'
          }
        },
        {
          selector: 'edge[provenance = "inferred"]',
          style: {
            'line-style': 'dashed',
            'line-dash-pattern': [6, 4],
            'line-color': '#7c6f50',
            'target-arrow-color': '#7c6f50'
          }
        },
        {
          selector: 'edge[provenance = "ambiguous"]',
          style: {
            'line-style': 'dashed',
            'line-dash-pattern': [4, 4],
            'line-color': '#d97735',
            'target-arrow-color': '#d97735',
            'opacity': 0.9
          }
        },
        {
          selector: 'edge.highlighted',
          style: {
            'width': 3,
            'line-color': '#8ba6ff',
            'target-arrow-color': '#8ba6ff',
            'opacity': 1.0,
            'z-index': 999
          }
        },
        {
          selector: 'edge.dimmed',
          style: {
            'opacity': 0.15
          }
        },
        {
          selector: 'edge.path-highlight',
          style: {
            'width': 4,
            'line-color': '#38bdf8',
            'target-arrow-color': '#38bdf8',
            'opacity': 1.0,
            'z-index': 9999
          }
        }
      ],
      elements: []
    });
    window.cy = cy;
    window.state = state;

    cy.on('tap', 'node[type = "symbol"]', function(evt) {
      var node = evt.target;
      var rawId = node.data('rawId');
      if (rawId === undefined || rawId === null) return;

      if (state.tracing.active) {
        handlePathTracingTap(node);
      } else {
        selectNode(rawId);
      }
    });

    cy.on('tap', function(evt) {
      if (evt.target === cy) {
        clearSelection();
      }
    });
  }

  function getLayoutConfig(layoutName) {
    var name = layoutName || state.currentLayout;
    if (name === 'dagre') {
      return {
        name: 'dagre',
        rankDir: 'TB',
        nodeSep: 50,
        rankSep: 70,
        edgeSep: 20,
        animate: false,
        fit: true,
        padding: 40
      };
    } else {
      return {
        name: 'cose',
        animate: false,
        randomize: true,
        fit: true,
        padding: 40,
        componentSpacing: 60,
        nodeRepulsion: function(node) {
          return node.isParent() ? 1000 : 2048;
        },
        nodeOverlap: 4,
        idealEdgeLength: function(edge) { return 32; },
        edgeElasticity: function(edge) { return 32; },
        nestingFactor: 1.2,
        gravity: 1.5,
        numIter: 400,
        initialTemp: 200,
        coolingFactor: 0.95,
        minTemp: 1.0
      };
    }
  }

  function runLayout(layoutName) {
    if (!cy) return;
    var config = getLayoutConfig(layoutName);
    var layout = cy.layout(config);
    layout.run();
  }

  function transformToCytoscapeElements(nodesMap, edgesMap) {
    var elements = [];
    var reposSeen = new Set();
    var filesSeen = new Set();

    nodesMap.forEach(function(node) {
      var repoId = 'repo:' + node.repo;
      if (node.repo && !reposSeen.has(repoId)) {
        reposSeen.add(repoId);
        elements.push({
          group: 'nodes',
          data: {
            id: repoId,
            label: node.repo,
            type: 'repo'
          }
        });
      }

      var fileParent = repoId;
      var fileId = 'file:' + node.repo + ':' + (node.file || 'unknown');
      if (node.file && !filesSeen.has(fileId)) {
        filesSeen.add(fileId);
        var shortPath = node.file.split('/').pop() || node.file;
        elements.push({
          group: 'nodes',
          data: {
            id: fileId,
            label: shortPath,
            type: 'file',
            parent: node.repo ? repoId : undefined
          }
        });
      }

      var symbolId = 'sym:' + node.id;
      var parentId = filesSeen.has(fileId) ? fileId : (reposSeen.has(repoId) ? repoId : undefined);

      elements.push({
        group: 'nodes',
        data: {
          id: symbolId,
          rawId: node.id,
          label: node.name,
          kind: normalizeKind(node.kind),
          exported: Boolean(node.exported),
          complexity: node.complexity,
          type: 'symbol',
          parent: parentId
        }
      });
    });

    edgesMap.forEach(function(edge) {
      var sourceId = 'sym:' + edge.from;
      var targetId = 'sym:' + edge.to;
      var edgeId = 'e:' + edge.from + '->' + edge.to + ':' + edge.kind;
      elements.push({
        group: 'edges',
        data: {
          id: edgeId,
          source: sourceId,
          target: targetId,
          kind: edge.kind,
          provenance: edge.provenance,
          confidence: edge.confidence
        }
      });
    });

    return elements;
  }

  function applyGraphData(graph) {
    state.nodes.clear();
    state.edges.clear();
    if (!graph || !graph.nodes) return;

    graph.nodes.forEach(function(n) {
      state.nodes.set(n.id, n);
    });
    if (graph.edges) {
      graph.edges.forEach(function(e) {
        var key = e.from + '->' + e.to + ':' + e.kind;
        state.edges.set(key, e);
      });
    }

    if (cy) {
      var elements = transformToCytoscapeElements(state.nodes, state.edges);
      cy.elements().remove();
      cy.add(elements);
      applyFilterVisibility();
      cy.resize();
      runLayout(state.currentLayout);
    }

    updateStats();
    updateFilterOptions();
  }

  function mergeGraphData(graph) {
    if (!graph || !graph.nodes) return;

    var newNodes = 0;
    graph.nodes.forEach(function(n) {
      if (!state.nodes.has(n.id)) {
        state.nodes.set(n.id, n);
        newNodes++;
      }
    });

    if (graph.edges) {
      graph.edges.forEach(function(e) {
        var key = e.from + '->' + e.to + ':' + e.kind;
        state.edges.set(key, e);
      });
    }

    if (cy) {
      var elements = transformToCytoscapeElements(state.nodes, state.edges);
      cy.elements().remove();
      cy.add(elements);
      applyFilterVisibility();
      runLayout(state.currentLayout);
    }

    updateStats();
    updateFilterOptions();
  }

  function applyFilterVisibility() {
    if (!cy) return;
    var repoF = state.filters.repo;
    var kindF = state.filters.kind;
    var provF = state.filters.provenance;
    var searchQ = (state.filters.search || '').toLowerCase().trim();

    cy.batch(function() {
      cy.nodes('[type = "symbol"]').forEach(function(node) {
        var rawId = node.data('rawId');
        var rawNode = state.nodes.get(rawId);
        var visible = true;

        if (rawNode) {
          if (repoF && rawNode.repo !== repoF) visible = false;
          if (kindF && normalizeKind(rawNode.kind) !== kindF) visible = false;
          if (searchQ) {
            var nameMatch = rawNode.name && rawNode.name.toLowerCase().indexOf(searchQ) !== -1;
            var fileMatch = rawNode.file && rawNode.file.toLowerCase().indexOf(searchQ) !== -1;
            if (!nameMatch && !fileMatch) {
              visible = false;
            }
          }
        }

        if (visible) {
          node.style('display', 'element');
        } else {
          node.style('display', 'none');
        }
      });

      cy.edges().forEach(function(edge) {
        var prov = edge.data('provenance');
        if (provF && prov !== provF) {
          edge.style('display', 'none');
        } else {
          edge.style('display', 'element');
        }
      });
    });
  }

  function updateHighlights() {
    if (!cy) return;
    cy.batch(function() {
      cy.elements().removeClass('highlighted');
      state.highlightNodeIds.forEach(function(id) {
        var symNode = cy.getElementById('sym:' + id);
        if (symNode && symNode.length > 0) {
          symNode.addClass('highlighted');
        }
      });
    });
  }

  function isolateNeighborhood(node) {
    if (!cy || !node) return;
    var neighborhood = node.closedNeighborhood();
    cy.batch(function() {
      cy.elements().removeClass('highlighted dimmed path-highlight');
      cy.nodes('[type = "symbol"]').not(neighborhood).addClass('dimmed');
      cy.edges().not(neighborhood).addClass('dimmed');
      neighborhood.addClass('highlighted');
    });
  }

  function handlePathTracingTap(node) {
    if (!cy || !node) return;
    if (!state.tracing.sourceNode) {
      state.tracing.sourceNode = node;
      cy.batch(function() {
        cy.elements().removeClass('highlighted dimmed path-highlight');
        node.addClass('path-highlight');
      });
    } else {
      state.tracing.targetNode = node;
      var dijkstra = cy.elements().dijkstra({
        root: state.tracing.sourceNode,
        directed: true
      });
      var path = dijkstra.pathTo(node);
      cy.batch(function() {
        cy.elements().removeClass('highlighted dimmed path-highlight');
        if (path && path.length > 0) {
          cy.nodes('[type = "symbol"]').not(path).addClass('dimmed');
          cy.edges().not(path).addClass('dimmed');
          path.addClass('path-highlight');
          cy.fit(path, 50);
        } else {
          state.tracing.sourceNode.addClass('path-highlight');
          node.addClass('path-highlight');
        }
      });
      /* Reset tracing source/target after trace completes */
      state.tracing.sourceNode = null;
      state.tracing.targetNode = null;
    }
  }

  function togglePathTracing() {
    state.tracing.active = !state.tracing.active;
    state.tracing.sourceNode = null;
    state.tracing.targetNode = null;
    if (traceBtn) {
      if (state.tracing.active) {
        traceBtn.classList.add('active');
        traceBtn.textContent = 'Tracing...';
      } else {
        traceBtn.classList.remove('active');
        traceBtn.textContent = 'Trace Path';
      }
    }
    clearHighlights();
  }

  function clearHighlights() {
    if (!cy) return;
    cy.batch(function() {
      cy.elements().removeClass('highlighted dimmed path-highlight');
    });
  }

  function selectNode(id) {
    state.selectedNodeId = id;
    expandBtn.disabled = false;

    if (cy) {
      var symNode = cy.getElementById('sym:' + id);
      if (symNode && symNode.length > 0) {
        cy.nodes().removeClass('selected');
        symNode.addClass('selected');
        isolateNeighborhood(symNode);
        var neighborhood = symNode.closedNeighborhood();
        cy.animate({
          fit: {
            eles: neighborhood,
            padding: 80
          },
          duration: 300
        });
      }
    }
    fetchNodeDetails(id);
  }

  function clearSelection() {
    state.selectedNodeId = null;
    expandBtn.disabled = true;
    if (cy) {
      cy.nodes().removeClass('selected');
      clearHighlights();
    }
    detailName.textContent = 'No selection';
    detailKind.textContent = '-';
    detailRepo.textContent = '-';
    detailLocation.textContent = '-';
    detailMeta.textContent = '-';
    detailSignature.textContent = '-';
    callersLabel.textContent = 'Callers (0)';
    callersList.innerHTML = '';
    calleesLabel.textContent = 'Callees (0)';
    calleesList.innerHTML = '';
  }

  function fitGraphToView() {
    if (cy) {
      cy.resize();
      if (state.selectedNodeId !== null) {
        var sel = cy.getElementById('sym:' + state.selectedNodeId);
        if (sel && sel.length > 0) {
          cy.fit(sel.closedNeighborhood(), 80);
          return;
        }
      }
      cy.fit(undefined, 40);
    }
  }

  function resetView() {
    if (cy) {
      clearSelection();
      cy.resize();
      cy.reset();
      runLayout(state.currentLayout);
    }
  }
  function renderDetails(details) {
    if (!details || !details.node) return;
    var node = details.node;
    detailName.textContent = node.name;
    detailKind.textContent = normalizeKind(node.kind);
    detailRepo.textContent = node.repo;
    detailLocation.textContent = node.file + (node.span ? ':' + node.span.start_line : '');
    detailMeta.textContent = (node.exported ? 'Exported' : 'Private') +
      (node.complexity ? ' · Complexity ' + node.complexity : '');
    detailSignature.textContent = node.signature || '-';

    callersList.innerHTML = '';
    var callers = details.callers || [];
    callersLabel.textContent = 'Callers (' + callers.length + ')';
    callers.forEach(function(c) {
      var sym = c.caller || c.caller_symbol || c;
      var name = sym.name || c.caller_name || 'unknown';
      var repo = sym.repo || c.caller_repo || c.repo || '';
      var id = sym.id || c.caller_id || (sym !== c ? sym.id : null);
      var li = document.createElement('li');
      li.className = 'ref-item';
      li.textContent = name + (repo ? ' (' + repo + ')' : '');
      if (id) {
        li.style.cursor = 'pointer';
        li.addEventListener('click', function() { selectNode(id); });
      } else {
        li.style.opacity = '0.6';
        li.style.cursor = 'default';
      }
      callersList.appendChild(li);
    });

    calleesList.innerHTML = '';
    var callees = details.callees || [];
    calleesLabel.textContent = 'Callees (' + callees.length + ')';
    callees.forEach(function(c) {
      var sym = c.callee_symbol || c.callee || c;
      var name = sym.name || c.callee_name || 'unknown';
      var repo = sym.repo || c.callee_repo || c.repo || '';
      var id = sym.id || c.callee_id || (sym !== c ? sym.id : null);
      var li = document.createElement('li');
      li.className = 'ref-item';
      li.textContent = name + (repo ? ' (' + repo + ')' : '');
      if (id) {
        li.style.cursor = 'pointer';
        li.addEventListener('click', function() { selectNode(id); });
      } else {
        li.style.opacity = '0.6';
        li.style.cursor = 'default';
      }
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
      var s = item.symbol || item;
      var div = document.createElement('div');
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
    var repos = new Set();
    var kinds = new Set();
    state.nodes.forEach(function(n) {
      if (n.repo) repos.add(n.repo);
      if (n.kind) kinds.add(normalizeKind(n.kind));
    });
    repoFilter.innerHTML = '<option value="">All Repos</option>';
    repos.forEach(function(r) {
      var opt = document.createElement('option');
      opt.value = r;
      opt.textContent = r;
      if (state.filters.repo === r) opt.selected = true;
      repoFilter.appendChild(opt);
    });
    kindFilter.innerHTML = '<option value="">All Kinds</option>';
    kinds.forEach(function(k) {
      var opt = document.createElement('option');
      opt.value = k;
      opt.textContent = k;
      if (state.filters.kind === k) opt.selected = true;
      kindFilter.appendChild(opt);
    });
  }

  /* Interaction & Event Handlers */
  function setupEvents() {
    if (traceBtn) {
      traceBtn.addEventListener('click', togglePathTracing);
    }
    fitBtn.addEventListener('click', fitGraphToView);
    resetBtn.addEventListener('click', resetView);
    expandBtn.addEventListener('click', function() {
      if (state.selectedNodeId !== null) {
        fetchExpansion(state.selectedNodeId);
      }
    });

    if (layoutSelect) {
      layoutSelect.addEventListener('change', function(e) {
        state.currentLayout = e.target.value;
        runLayout(state.currentLayout);
      });
    }

    var searchDebounce = null;
    searchInput.addEventListener('input', function(e) {
      state.filters.search = e.target.value;
      applyFilterVisibility();
      clearTimeout(searchDebounce);
      searchDebounce = setTimeout(function() {
        performSearch(e.target.value);
      }, 150);
    });

    repoFilter.addEventListener('change', function(e) {
      state.filters.repo = e.target.value;
      applyFilterVisibility();
    });
    kindFilter.addEventListener('change', function(e) {
      state.filters.kind = e.target.value;
      applyFilterVisibility();
    });
    provenanceFilter.addEventListener('change', function(e) {
      state.filters.provenance = e.target.value;
      applyFilterVisibility();
    });
    depthSelect.addEventListener('change', function(e) {
      state.depth = parseInt(e.target.value, 10) || 1;
      statDepth.textContent = 'depth ' + state.depth;
      loadInitialGraph();
    });

    window.addEventListener('resize', function() {
      if (cy) {
        cy.resize();
      }
    });
  }

  /* Initialization */
  function init() {
    initCytoscape();
    setupEvents();
    loadInitialGraph();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
