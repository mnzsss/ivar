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
