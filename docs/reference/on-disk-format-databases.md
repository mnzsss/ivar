# On-disk format: databases

Part of the [on-disk format](on-disk-format.md) contract: the SQLite files under
`.ivar/`, what they hold and who may delete them.

## Graph database (`.ivar/graph.db`)

The codebase graph is stored locally in SQLite at `.ivar/graph.db`. It is derived:
`ivar graph clean` and `ivar graph index` may drop and rebuild any of it, and
writers serialise on `.ivar/graph.lock`. Halls written before the split kept it at
`.ivar/memory.db`; see [Memory database](#memory-database-ivarmemorydb) for how it moves.

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

## Memory database (`.ivar/memory.db`)

The hall's memory lives in its own SQLite file, separate from the graph, because
the graph is derived and disposable and the memory is not. It is one file per hall,
shared by every session, in WAL mode, with writers serialised on `.ivar/memory.lock`.

| Field | Value |
| --- | --- |
| `PRAGMA application_id` | `0x69766d31` (`ivm1`) |
| Schema version | the single row of `memory_schema(version)`, mirrored in `PRAGMA user_version` |
| Current schema | 2 — `memory_schema` and `observations` |

### Observations (schema 2)

`observations` holds the session events captured by the provider hooks and the delivery
events the binary records. Rows are appended by draining the [capture spool](on-disk-format.md);
nothing else writes them. Delivery events (`delivery.refused`, `delivery.pushed`,
`delivery.pr`, `delivery.landed`, `delivery.checks`, `delivery.rebased`,
`delivery.integrated`, `delivery.merged`, `delivery.cleaned`, `delivery.closed`,
`delivery.deleted`) travel through the same spool under the key `delivery`.

| Column | Meaning |
| --- | --- |
| `id` | integer primary key |
| `event_id` | unique; a replayed spool file inserts nothing twice |
| `source_event` | `session.started`, `session.prompted`, `session.tool`, `session.ended`, or one of the eleven `delivery.*` names |
| `extension` | owner of the event, `'ivar'` by default |
| `occurred_at` | RFC 3339 time of the event |
| `session_id` | ivar session id, empty outside a view dir |
| `native_session_id` | the provider's own session id |
| `provider`, `ivar_version` | harness that fired the hook and the binary that recorded it |
| `feature`, `parent_feature`, `repo`, `origin` | context resolved from the working directory, empty when unknown |
| `importance` | integer rank of the event |
| `attempt_id` | execution attempt, empty outside one |
| `body` | sanitised event text |

Indexes: `observations_session (session_id, native_session_id, occurred_at)`,
`observations_source_event (source_event, occurred_at)` and `observations_time (occurred_at)`.
The trigger `observations_no_update` aborts every `UPDATE`, so a row never changes after
it is written. `DELETE` stays allowed: the retention sweep deletes a session's `session.*` rows when its
last observation is older than 30 days, and never deletes `delivery.*`, `memory.*` or
`handoff.*`.

The version lives in a table because an older `ivar` that opens this file runs the
graph migrations on it and overwrites `user_version`. A file whose `memory_schema`
version is newer than the binary is refused and left untouched.

Only `ivar memory clean` deletes it, and only after confirmation. It finishes a
pending legacy move first, and never creates a `memory.db` only to delete it. `ivar graph clean`
(any flag), the graph's reconcile and prune, `ivar repo remove`, `ivar cleanup`
and `ivar feature rename` never touch it.

### Moving a legacy graph

Before the split the graph lived at `.ivar/memory.db`. The first command that opens
the graph with a newer `ivar` finds a `memory.db` carrying the `files` and `symbols`
tables and no memory `application_id`, and moves it. `graph_misses` is not
required, because a graph last written by v0.10.0 does not have it:

1. it takes `.ivar/memory.lock`, then `.ivar/graph.lock`;
2. it folds the WAL into the file (`PRAGMA wal_checkpoint(TRUNCATE)`, then
   `PRAGMA journal_mode = DELETE`);
3. it renames `memory.db` to `graph.db` in one step;
4. it creates the new `memory.db`. A leftover `memory.db-shm` or `memory.db-journal`, or an empty
   `memory.db-wal`, is removed first; a `memory.db-wal` that still holds frames
   stops the move, because SQLite would replay it into the new file. The new
   file is built as `memory.db.new` and renamed into place once its schema is
   committed, its WAL folded in (`PRAGMA wal_checkpoint(TRUNCATE)`) and its
   connection closed, so a crash mid-creation leaves only `memory.db.new`, which
   the next open discards.

The move is a rename, not a copy, and an interrupted move finishes on the next open.
When another process still has the legacy file open (typically an MCP server of an
older `ivar`), the journal mode change is refused, nothing is moved, and the command
fails with `memory.split_busy`. A legacy file locked by another connection while
it is being recognised fails the same way. The probe only sees connections that have run a
statement; every `ivar` open path runs one right after opening, so an older `ivar`
is always seen. Opening a session finishes the move too, and turns a busy legacy
file into a warning. Best-effort hook writes never move it; they skip recording
until the move has happened.

### No downgrade

An older `ivar` that opens the new `memory.db` creates graph tables inside it. No data
is lost: `ivar doctor` reports `memory.db_polluted`. Stop the older `ivar`, then run
`ivar memory status`, which reopens the file for writing and drops those tables.
Going back to an older `ivar` for good means deleting the new `memory.db` with
`ivar memory clean` first.
