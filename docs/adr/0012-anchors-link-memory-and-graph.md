# ADR-0012: Anchors link memory and graph

- **Status:** accepted
- **Date:** 2026-11-05

## Context

A memory page names files and symbols, and the graph knows them. Nothing joined the two.
An agent that read a page could not tell whether the code it cites still exists, and an agent
that explored the code could not see the pages written about it. The memory index lives in
`memory.db` and the graph in `graph.db`, two files with different lifetimes (ADR-0007).

## Decisions

### D1: Anchors are resolved in the memory reindex through a read-only `ATTACH`

The reindex runs `ATTACH 'file:<graph.db>?mode=ro' AS graph` on its `memory.db` connection and
resolves only pages that are new, changed or pending. SQLite refuses `ATTACH` inside an open
transaction, so the order is attach, begin, commit, detach, kept in one helper. Anchors are
stored as text keys (`repo`, `path`, `symbol`), never graph row ids.

Rejected: letting the graph leader push anchors into `memory.db`. It makes the graph aware of
memory, and the graph does not know a page's hash.

### D2: `graph_explore` reads memory through its own read-only connection

`explore_within` opens `.ivar/memory.db` read-only once per call to fill `related_memory`. The
CLI and the MCP server share it. A missing or unreadable file leaves the section empty and never
fails the answer.

Rejected: `ATTACH` on the long-lived graph connection. It changes the state of a connection that
has session layers mounted, and a `DETACH` can fail while a statement is open.

### D3: `docs/memory` leaves the graph content index but stays in files

For the `hall` repo, the indexer writes no `file_content_fts` row for paths under `docs/memory/`.
The `files` row stays, so `graph_explore` `paths` still returns a page whole. Graph schema 12
deletes the rows already indexed, because unchanged files are skipped by hash and would never be
rewritten. A page is then found once, through `memory_query`.

Rejected: dropping the files from `files`, which breaks exact-path lookup; filtering at query
time, which leaves the content in the index.

### D4: Pending anchors are a column on `pages`

`pages.anchors_pending` is set when a page is indexed while `graph.db` is missing or unreadable.
The next reindex with a graph resolves those pages. The flag belongs to the indexed row and is
dropped with it.

Rejected: a marker in a spool JSON like `_consolidate.json`. It drifts from the rows it describes.

## Consequences

- A change in the graph never re-anchors a page. Only a page edit or `ivar memory reindex` does.
- `lint.anchor_broken` is a warning, because a graph that has not indexed a file yet looks the
  same as a deleted file.
- `graph_explore` shows up to 3 related pages and drops them first when the answer is too long.
- Memory schema is 4 and graph schema is 12.
- See [MCP tools](../reference/mcp-tools.md) and
  [memory limitations](../reference/limitations-memory.md).
