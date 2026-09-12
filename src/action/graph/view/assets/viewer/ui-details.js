/* Detail, Search, and Filter Rendering */
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
