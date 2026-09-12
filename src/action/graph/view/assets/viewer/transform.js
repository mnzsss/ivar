/* Graph Data Transformation */
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

function removeElements(elementIds) {
  if (!elementIds || !elementIds.length) return;
  elementIds.forEach(function(id) {
    state.nodes.delete(id);
  });
  if (cy) {
    elementIds.forEach(function(id) {
      var el = cy.getElementById('sym:' + id);
      if (el && el.length > 0) {
        cy.remove(el);
      }
    });
    updateStats();
    updateFilterOptions();
  }
}
