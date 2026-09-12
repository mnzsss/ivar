/* Layout & Simulation */
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

function stepSimulation() {
  if (state.simulation.settled) return;
  state.simulation.alpha *= 0.95;
  state.simulation.iterations++;
  if (state.simulation.alpha < 0.001 || state.simulation.iterations > 300) {
    state.simulation.settled = true;
    state.simulation.alpha = 0.0;
  }
}
