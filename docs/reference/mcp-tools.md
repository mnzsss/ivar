# MCP tools

Part of the [command reference](commands.md). `ivar graph mcp` serves one MCP server
for the hall. It carries the code graph tools and the hall memory tools.

## Server name

The server is `ivar`, so a provider sees it as `<hall>-ivar`. The command is still
`ivar graph mcp`. `serverInfo.name` is `ivar`.

The default surface lists 11 tools: `graph_explore`, `graph_feedback` and the nine
`memory_*` tools below. `ivar graph mcp --tools all` also lists the remaining graph tools.
`graph_explore` and `memory_query` carry `anthropic/alwaysLoad`.

## Migration from `<hall>-graph`

Earlier versions registered the server as `graph`.

- `ivar sync` renames an unedited `graph` entry in `ivar.json` to `ivar`, then rewrites
  the provider config files. `<hall>-graph` disappears from them. A second `ivar sync`
  writes nothing.
- An edited `graph` entry, or an unrelated server already named `ivar`, stays as it is
  and `ivar sync` reports `graph.mcp_name_clash`. Rename the entry by hand and sync again.
- `ivar doctor` reports `mcp.legacy_name` (the manifest still has `graph`) and
  `mcp.provider_config_stale` (a provider file still has `<hall>-graph`). Both are fixed
  by `ivar sync`.
- A session opened before the sync keeps the old name until it restarts.
- `ivar` does not edit your own configuration. A rule such as `mcp__<hall>-graph__*` in a
  personal `settings.json` points at a server that no longer exists. Change it to
  `mcp__<hall>-ivar__*`.

## Approval

`ivar` approves the server, not single tools. On Claude Code the entry is
`<hall>-ivar` in `enabledMcpjsonServers`, so one approval covers all eleven tools,
including `memory_write_page`, `memory_patch_page` and `memory_delete_page`. Each
provider's permission mode decides whether it asks before a call.

## Memory tools

None of the schemas takes `workspace`, `project`, `cwd`, `scopes` or `global`. The hall
is the scope. The read tools carry `readOnlyHint`. `memory_delete_page` carries
`destructiveHint`. A failure comes back as an MCP result with `isError`.

| Tool | Arguments | Result |
| --- | --- | --- |
| `memory_query` | `query` (required), `kind[]`, `tier`, `tag`, `limit` (default 10, maximum 20), `include_expired` | Pages with title, kind, tier, path, excerpt and `pinned`. Observations appear only as a fallback, in a separate section. |
| `memory_read_page` | `path` | Frontmatter fields and body. Also reads `.ivar/memory/` pages. |
| `memory_write_page` | `kind` and `title` (required), `body`, `slug`, `tags`, `pinned`, `relations[{type,target}]`, `expires_at`, `replace` | `path`, `hash`, `indexed`, `similar`. |
| `memory_patch_page` | `path` (required), `old`, `new`, `tags`, `pinned`, `relations`, `expires_at` | `path`, `hash`, `indexed`. |
| `memory_delete_page` | `path` | `path`, `indexed`. |
| `memory_lint` | none | `issues`: a list of `{path, code, severity, message}`. |
| `memory_status` | none | The same block as `ivar memory status`. |
| `memory_recent` | `limit` (default 20, maximum 100), `source_event` (prefix), `feature` | Observations, newest first, each with a one-line summary. |
| `memory_read_session_observations` | `session` (required), `limit` (default 50, maximum 100), `cursor` | Observations of one session, oldest first, and `next_cursor` while more remain. |

Both observation tools drain the capture spool before they read.

### Writing

- `kind` is one of `product`, `update`, `decision`, `gotcha`, `procedure` or `concept`.
  `session` is refused: only the consolidation writes it. The tool description and the
  server instructions carry the definition of each kind.
- `slug` defaults to the kebab-case form of the title. For `product` and `update` the
  tool adds the `NNN-` prefix: the highest number in the directory plus one, counting
  `*.md` and `*.cleanup.json`. A `slug` that already starts with `NNN-` is refused.
- The page is published with `hard_link` from a temporary file in the same directory.
  `hard_link` fails when the target exists, so a page another writer created first is never
  replaced. Without `replace: true`, an existing page fails with its path. With it, the
  tool uses `rename`. A filesystem without hard links fails the write with that cause.
- A title whose words overlap an existing title of the same kind at 0.8 or more (shared
  words over all words) still writes. The result lists them in `similar` as `{path, title}`.
- `memory_patch_page` replaces one exact piece of the body. `old` must appear once. It
  also changes `tags`, `pinned`, `relations` and `expires_at`. It never changes `kind` or
  `title`.
- `memory_delete_page` refuses `update` pages, which belong to the delivery record.
- Every write parses the result as a page before it publishes, then refreshes the index
  and reports it in `indexed`. The `hash` is the SHA-256 of the file content.
- The tools never commit. A written page stays uncommitted until you commit it, and
  `ivar doctor` keeps reporting `memory.page_uncommitted`.

### Confinement

The server runs outside the session write guard, so the tools confine themselves.
`memory_write_page`, `memory_patch_page` and `memory_delete_page`:

- accept only `docs/memory/<durable kind>/<slug>.md`, and refuse `session`, `.ivar/memory/`
  and any other path;
- refuse an absolute path, `..`, a backslash, a hidden file name and any component that is a
  symlink;
- write at the hall root, never inside a session view, because the index reads the root;
- hold `memory.lock` while they number, check and publish.

`memory_read_page` accepts `docs/memory/<kind>/` and `.ivar/memory/` paths of known kinds. It
refuses a path that resolves outside the hall.

## Lint

`memory_lint` and `ivar memory lint` share one function. It reads the files, not the
index, so it covers uncommitted pages. The verb exits non-zero when it finds an issue and
accepts `--json`. Every issue has severity `error`.

| Code | When |
| --- | --- |
| `lint.kind_unknown` | A directory under `docs/memory/` that is not a durable kind. |
| `lint.frontmatter_invalid` | Unreadable YAML, a field of the wrong type, a `kind` that differs from the directory, or a malformed `expires_at`. |
| `lint.field_unknown` | A field outside the six frontmatter fields. |
| `lint.link_broken` | A markdown link that resolves to a missing `.md` under `docs/memory/`. |
| `lint.misplaced` | A subfolder, a slug that is not kebab-case ASCII, or a `product` or `update` page without `NNN-`. |

## Session guard and the tool names

The guard recognises `graph_explore` calls by name suffix. Claude Code and T3 use
`__graph_explore`. OpenCode and OMP use `-ivar_graph_explore` and `_ivar_graph_explore`.
It also keeps `-graph_graph_explore` and `_graph_graph_explore` for sessions that still run
under the old name. The old suffixes leave in 0.20.0, and a test fails from that version
while they remain. Memory tool calls are not written to `usage`.

Capture classifies `memory_query`, `memory_read_page`, `memory_lint`, `memory_status`,
`memory_recent` and `memory_read_session_observations` as reads. The other tools are writes.
