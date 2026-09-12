/* State Model & DOM Element Cache */
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
