/* UI & Interaction Rendering */
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

