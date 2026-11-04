# Limitations of the codebase graph

What the graph index covers and where it stops. See [Limitations and failure modes](limitations.md) for the rest of `ivar`.

## Codebase graph indexing and plain-text bounds

The codebase graph indexes all Git-tracked UTF-8 text files, but applies clear operational bounds:

- **Searchable content bound:** Each file is indexed up to a maximum of 256 KiB of UTF-8 content.
  Content beyond this bound is not indexed for full-text search, and the file record marks
  `content_truncated: true`. Exact requested-file retrieval still reads the full current file from disk.
- **Binary and unparseable files:** Files containing NUL bytes or invalid UTF-8 within the content probe
  are rejected as binary and skipped without failing the repository index.
- **Excluded files:** Untracked files, Git-ignored files, and ignored build/dependency directories
  (such as `target/`, `node_modules/`, `.git/`, `.ivar/`) are not indexed.
- **Hall files:** Directories listed in `graph.paths` (including the hall's committed
  `.ivar/skills`) are indexed from the filesystem under the repo id `hall`, tracked or not, with
  hall-relative paths. Other `.ivar` state is never indexed.
- **Path queries stay in their repo:** A path in a query resolves by exact path first, then as the tail
  of an indexed path. A shorter indexed path matches only behind a workspace prefix that names its own
  repo (`services/api/src/a.ts`, `.ivar/repos/api/main/src/a.ts`), so `brand/DESIGN.md` never returns a
  `DESIGN.md` from another repo. A query that names only paths the index lacks returns no symbols
  instead of unrelated fuzzy matches. The miss says which paths are missing, and when the first directory
  of the path exists in the hall but is not indexed it points at `graph.paths` (`ivar graph index --path <dir>`).
  A basename without a directory still matches across repos, and every match names its repo.
- **Plain-text files without grammars:** Text files lacking tree-sitter grammars participate in path,
  basename, and full-text search, but do not produce synthetic AST symbols, callers, callees, or
  call graph edges.
- **Response budgets and truncation:** Exploration outputs enforce deterministic budgets (18,000 characters
  for standard exploration; 24,000 for explicit file requests). When files or sources exceed budgets,
  omitted files are listed under `not_shown` with an exact JSON `paths` continuation call.
- **Relevance floor:** `graph_explore` compares the words of an intent query with each candidate file's
  path, symbol names and content. A file that covers less than half of the query's distinct words (after
  generic words are dropped) is `weak`, unless the query names it by path, basename or exact symbol. When
  every candidate is weak the answer says `confidence: none` and lists them as locations without source;
  see [graph_explore precision](../product/012-graph-explore-precision.md). Matching is by whole word
  (plurals folded), so a query whose words are synonyms of the code's names can still miss.
