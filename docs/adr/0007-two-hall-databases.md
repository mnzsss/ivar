# ADR-0007 — Two Hall Databases: `graph.db` and `memory.db`

- **Status:** accepted
- **Date:** 2026-11-03

## Context

The code graph lived in `.ivar/memory.db`: one SQLite file, derived from the
checkouts and freely rebuilt. The hall memory (observations, then page indexes,
handoffs and messages) is primary data that cannot be rebuilt. Keeping both in one
file would let `ivar graph clean`, reconcile and prune delete memory. The watcher
and the capture hooks would also contend for one WAL writer, and the file's name
would keep saying "memory" about the graph.

## Decisions

### D1 — Two files, independent schemas

The graph moves to `.ivar/graph.db` and keeps its schema (version 11), `usage` and
`graph_misses`. The memory gets `.ivar/memory.db`, one per hall, with no layers,
`PRAGMA application_id = 0x69766d31` and its own migrations. Joining the two is done
later by a read-only `ATTACH`, and links across files use stable keys, never row ids.

### D2 — The memory version lives in a table

`memory_schema(version)` is the source of truth, and `user_version` only mirrors it.
An older `ivar` that opens `memory.db` runs the graph migrations, which overwrite
`user_version` with the graph's version. A version kept only there could not tell a
newer memory schema from a polluted file.

### D3 — Move, by checkpoint and one rename

The first command that opens the graph with a newer binary moves a legacy
`memory.db`. Under `memory.lock` and then `graph.lock`, it runs `wal_checkpoint(TRUNCATE)`
and `journal_mode = DELETE`, renames the file once, and creates the new `memory.db`.
Renaming the `-wal` and `-shm` files separately was rejected: separating a WAL from its
database, or renaming a database another process has open, is a documented way to lose
committed transactions. The journal mode change doubles as the "is anyone else using
it" probe. When it is refused, the move stops with `memory.split_busy` and is retried
on the next open. The probe only sees connections that have run a statement, since
that is when a connection starts holding the WAL. A connection opened and never used
goes unseen. Every `ivar` open path (`GraphDb::open`, `open_for_usage`,
`open_read_only`) runs a statement right after opening, so an older `ivar` holding the
file is always detected. A `memory.db-wal` left with frames and no `memory.db` stops
the creation of the new file, so an orphan WAL is never replayed into it. A copy
(`VACUUM INTO`) was rejected because it doubles the disk space for a file that only
needs a new name.

### D4 — Locks follow the files

The graph index lock becomes `.ivar/graph.lock`, and `.ivar/memory.lock` becomes the
memory database's lock. The move takes both, so it also excludes indexers of an older
binary, which still lock `memory.lock`.

### D5 — Only `ivar memory clean` deletes `memory.db`

`ivar graph clean` (any flag), reconcile, prune, `repo remove`, `cleanup` and
`feature rename` never touch it. `memory clean` asks for confirmation and deletes
nothing without it.

## Consequences

- No downgrade. An older `ivar` that opens the new `memory.db` creates graph tables in
  it. Nothing is lost: `ivar doctor` reports `memory.db_polluted`. Stop the older `ivar`,
  then run `ivar memory status`, which reopens the file for writing and drops those
  tables. Returning to an older binary for good requires `ivar memory clean` first.
- A hall whose older MCP server is still running cannot move its graph until that server
  stops. Commands fail with `memory.split_busy`, and `ivar sync` degrades to a warning.
- Hooks never move the file, so the per-tool-call guard stays a file-existence check.
- "There will never be a hall you cannot open" still holds: the move is the migration
  from the one-file layout, and it is never pruned.
