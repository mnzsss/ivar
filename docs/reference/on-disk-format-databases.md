# On-disk format: databases

Part of the [on-disk format](on-disk-format.md) contract: the SQLite files under
`.ivar/`, what they hold and who may delete them.

## Graph database (`.ivar/memory.db`)

The codebase graph is stored locally in SQLite at `.ivar/memory.db`.

Hall paths are stored under the repo id `hall`, with hall-relative file paths.
The id carries no `/` because every base view selects base rows with
`repo NOT LIKE '%/%'`, the convention that marks feature layers. A bare
`ivar graph index` (and `ivar sync`) also drops repos absent from `ivar.json`,
layers of features whose directory is gone, and watch scopes with nothing
behind them, and lists them under `removed` in its outcome.

### Graph schema v10

Schema version 10 introduces the dedicated `file_content_fts` table and `files.content_truncated`
column for full-text search across all tracked UTF-8 text files up to 256 KiB per file.

When migrating from older schema versions to v10, existing indexed repository metadata is invalidated
so that the next `ivar graph index` performs a clean, complete rebuild of all tracked files rather than
serving partial or incomplete text-search results. Active Feature Session layers join through
`visible_files` to ensure layer shadowing and tombstones remain authoritative.

### Graph schema v11

Schema version 11 adds `watch_scopes`, one row per scope the graph watcher keeps fresh:
`base:<repo>` for a default-branch worktree, `layer:<feature>:<repo>` for a feature layer.

| Column | Meaning |
| --- | --- |
| `observed` | Bursts of filesystem events seen for the scope (monotonic) |
| `indexed` | Highest `observed` value whose reindex finished |
| `needs_catchup` | `1` until the leader re-ran the git incremental path for the scope |
| `error` | Last reindex failure, cleared by the next success |
| `updated_at` | Unix seconds of the last change |

A scope is settled when `indexed >= observed`, `needs_catchup = 0` and `error` is null.
Migrating from v10 keeps the existing index.

### Watcher lease (`.ivar/graph-watch.lock`)

Every `ivar graph mcp` process tries a non-blocking exclusive lock on this file on each tool call.
The holder is the leader: it watches the hall and is the only process that reindexes from events.
The file holds the leader's pid. The kernel releases the lock when the leader exits, and the next
tool call from any other server takes over, runs the git catch-up and starts watching.
