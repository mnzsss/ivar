/* Cytoscape Stylesheet Definitions */
var cyStylesheet = [
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
];
