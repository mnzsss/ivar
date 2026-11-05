# ADR-0011: One MCP server for the graph and the memory

- **Status:** accepted
- **Date:** 2026-11-05

## Context

`ivar graph mcp` served two graph tools under the name `graph`. The memory pages of
ADR-0010 were readable only through `ivar memory query`, and an agent could not write one.
A second server for memory would need a second process, a second manifest entry and a
second approval in every provider. It would also stay stale when the graph index fails.

## Decisions

### D1: One server, named `ivar`

The manifest entry changes from `graph` to `ivar`, and a provider shows `<hall>-ivar`. The
command stays `ivar graph mcp`, so `runs_graph_mcp` still recognises the server by command.
`serverInfo.name` becomes `ivar`. `register_graph_mcp` registers `ivar` in new halls.

Rejected: a `ivar mcp` verb, because it adds a second way to start the same process; a
separate `ivar-memory` server, for the reasons in the context.

### D2: `ivar sync` renames the entry, once

`ivar sync` renames an unedited `graph` entry to `ivar` through `Manifest::write`, then
materialises the provider configs, which replace the whole MCP map. An edited entry, or
another server named `ivar`, gives `graph.mcp_name_clash` and nothing is touched. It is the
first time `sync` writes `ivar.json`, and the write is limited to this rename. There is no
alias: a session opened earlier keeps the old name until it restarts. The session guard
accepts both tool-name suffix families until 0.20.0.

Rejected: an alias that serves both names, because every provider would list the tools twice.

### D3: Memory tools sit on the default surface and route before the graph refresh

`tools/list` returns 11 tools by default. A `memory_*` call is dispatched before
`call_tool_in_session`: it does not need the graph index, so a failed graph layer cannot
break it. The tools open `memory.db` per call, refresh the page index before they read, and
write nothing to `usage`.

Rejected: listing the memory tools only under `--tools all`, because the agent would not
see them by default.

### D4: The tools confine their own writes

The server runs outside the session write guard, which sees the call and not the files it
touches. So `memory_write_page`, `memory_patch_page` and `memory_delete_page` accept only
`docs/memory/<durable kind>/<slug>.md`, refuse `..`, absolute paths and symlinks, write at
the hall root, validate the page before they publish, and publish by `hard_link` so a page
created first is never replaced. They never commit.

Rejected: teaching the guard about MCP calls. It would have to parse tool arguments to
learn which file a call touches, and it would not cover another host.

#### Amendment (2026-11-05): session pages

`memory_patch_page` also accepts `.ivar/memory/session/<id>.md`, with the same symlink
refusal, and changes only `pinned` and `expires_at` there. The consolidation owns the
body of a `session` page; the two fields are how a person keeps one past its 30 days or
expires it early. `memory_write_page` and `memory_delete_page` are unchanged.

## Consequences

- A user rule `mcp__<hall>-graph__*` must change to `mcp__<hall>-ivar__*`; see
  [MCP tools](../reference/mcp-tools.md).
- Approval is per server. One `<hall>-ivar` approval covers read, write and delete tools.
- A page written through the tools is uncommitted until a person or a skill commits it.
- The default tool surface grows from 2 to 11, which costs prompt space on hosts that list
  every tool.
