# ADR-0010 — Memory pages in `docs/memory`

- **Status:** accepted
- **Date:** 2026-11-05

## Context

Hall memory held observations only. The knowledge worth keeping, product behaviour and
delivery records, lived as numbered files in `docs/product/` and `docs/updates/`, which
nothing indexed. An agent starting a plan could not ask "what do we know about this" and
had to read `HALL.md` and guess. ADR-0005 removed per-feature documents from `docs/`, so
the folders held only what `feature cleanup` wrote.

## Decisions

### D1 — Two units of work, binary first

First the `ivar` feature that understands `docs/memory/`, delivered and installed.
Then, directly on the hall's `main`, the `git mv` of the existing pages, the rewrite of
their links and of the `paths` in cleanup records, and `HALL.md`. Moving first would
break `feature cleanup`, which validates records under the new path. Between the two,
`ivar doctor` reports `memory.legacy_docs`.

Rejected: a `migrate` verb for other halls. Only this hall has records; another hall
that has `docs/updates/` gets the diagnostic and runs the `git mv` it prints.

### D2 — The page model is pure domain

`domain::memory_page` defines `PageKind`, `Tier` (derived from the kind), `PageMeta` and
the validation. YAML stays behind `infra::frontmatter`; `store::memory::page_source` joins
the two, and the reindex and `ivar doctor` share that one function. A file without front
matter is valid, as the pages of today have none.

Rejected: a tier field in the file. It would let the file and the kind disagree.

### D3 — Four index tables in schema 3

`pages`, `pages_fts` and `page_links` index the pages; `observations_fts` is an
external-content index over `observations`, kept by insert and delete triggers.
The page body lives only in `pages_fts`. The page reindex never touches observations.

Rejected: external-content FTS for pages, because the body would exist twice or need a
second table; a plain FTS copy of observations, which duplicates up to 4 KiB per row; a
markdown crate for links, because three patterns do not justify a dependency.

### D4 — The index follows the disk without a new watcher

The incremental reindex compares size and mtime, confirms by SHA-256, and runs before
`memory query` and `memory status`, in `ivar sync`, and every 5 s in the MCP leader
under `try_lock`. An uncommitted page is searchable at once. `ivar memory reindex`
rebuilds everything, and a page deleted from disk leaves the index.

Rejected: a file watcher for pages, because the on-read refresh already gives
instant visibility; a `Warning` from `sync` on reindex failure, because it would turn
its exit code to 1. A failure goes to `_reindex.json` and `ivar doctor` instead.

### D5 — The query fuses two sources

`ivar memory query` fuses page FTS (bm25 weighted toward title and tags) and the
one-hop link neighbourhood by reciprocal-rank fusion (k = 60), then applies a bounded
tier multiplier and a `pinned` bonus. Every term of the user's text is quoted, so it never
reaches the FTS5 parser as syntax. Link expansion is skipped under any filter, and link
neighbours rank behind their seeds. With no page hit and no filter, observations answer
in a separate section.

Rejected: AND-first with an OR fallback, as extra code for no tested need.

## Consequences

- `Layout` swaps `docs_updates_dir()` for `docs_memory_dir()`, `page_dir(kind)` and
  `update_pages_dir()`.
- `feature cleanup` requires records under `docs/memory/update/` and documentation
  paths under `docs/memory/` or `docs/repo-relations/`. The failure codes keep their
  names because scripts read `--json`.
- Files outside `<kind>/`, or nested, are not indexed and are reported, so they
  are not lost silently.
- Schema 3 is not readable by an older `ivar`; see the
  [on-disk format](../reference/on-disk-format-versions.md).
