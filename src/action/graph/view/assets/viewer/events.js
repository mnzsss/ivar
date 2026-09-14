/* Cytoscape Initialization and Event Binding */
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
    style: cyStylesheet,
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
