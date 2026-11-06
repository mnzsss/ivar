<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar graph`

Query and index the codebase dependency graph


## `ivar graph explore`

Hero query synthesizing symbol discovery, source snippet, callers, and impact

| argument | required | description |
| --- | --- | --- |
| `query` | yes | Symbol name or query pattern to explore |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Limit exploration to a specific repository |


## `ivar graph affected`

Find reverse-dependent test files for changed files

| argument | required | description |
| --- | --- | --- |
| `files` | no | Changed file paths to find reverse dependencies for |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--stdin` |  |  | Read changed file paths from stdin (one per line) |
| `--repo` | `<REPO>` |  | Restrict search to a specific repository |
| `--max-depth` | `<MAX_DEPTH>` |  | Maximum search depth hops |


## `ivar graph path`

Find shortest path between two symbols or files

| argument | required | description |
| --- | --- | --- |
| `from` | yes | Starting symbol name or file path |
| `to` | yes | Target symbol name or file path |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--max-hops` | `<MAX_HOPS>` |  | Maximum traversal hops |


## `ivar graph find`

Find symbols matching a query name pattern

| argument | required | description |
| --- | --- | --- |
| `query` | yes | Symbol name query pattern |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Restrict search to a specific repository |
| `--limit` | `<LIMIT>` |  | Maximum number of matching symbols to return |


## `ivar graph callers`

List all callers of a symbol

| argument | required | description |
| --- | --- | --- |
| `symbol` | yes | Symbol name to find callers for |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Restrict search to a specific repository |
| `--cross-repo` |  |  | Search cross-repo callers |
| `--min-confidence` | `<MIN_CONFIDENCE>` |  | Minimum edge confidence threshold (0.0 - 1.0) |


## `ivar graph callees`

List all outgoing calls (callees) from a symbol

| argument | required | description |
| --- | --- | --- |
| `symbol_id` | yes | Symbol ID to find outgoing callees for |


## `ivar graph file`

Show file outline with all defined symbols

| argument | required | description |
| --- | --- | --- |
| `repo` | yes | Target repository name |
| `path` | yes | Target file path within the repository |


## `ivar graph index`

Incrementally update or build the codebase graph index

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Specific repository to index (indexes all declared repos if omitted) |
| `--full` |  |  | Force full reindex regardless of last indexed commit |
| `--no-mcp` |  |  | Do not declare the graph MCP server in ivar.json |
| `--path` | `<DIR>` |  | Hall-root directory to index as well (repeatable). Saved to `graph.paths` in `ivar.json`. A run with `--path` indexes hall paths only |


## `ivar graph stats`

Show overall graph statistics


## `ivar graph impact`

Compute transitive blast-radius impact analysis for a symbol

| argument | required | description |
| --- | --- | --- |
| `symbol_id` | yes | Symbol ID to compute blast radius impact for |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--max-depth` | `<MAX_DEPTH>` |  | Maximum traversal depth |


## `ivar graph mcp`

Run graph MCP server

| flag | value | default | description |
| --- | --- | --- | --- |
| `--tools` | `<TOOLS>` | `explore` | Tools to advertise: `explore` lists `graph_explore`, `graph_feedback`, the fourteen memory tools including `memory_lint`, `memory_review` and the handoff tools (16 tools in total), `all` lists every graph tool as well |


## `ivar graph dead-code`

Find unreferenced private symbols and dead code

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Limit search to a specific repository |
| `--limit` | `<LIMIT>` |  | Maximum number of dead code items to return |


## `ivar graph complexity`

Find functions and methods ranked descending by cyclomatic complexity

| flag | value | default | description |
| --- | --- | --- | --- |
| `--threshold` | `<THRESHOLD>` | `10` | Minimum cyclomatic complexity threshold |
| `--repo` | `<REPO>` |  | Limit search to a specific repository |
| `--limit` | `<LIMIT>` |  | Maximum number of items to return |


## `ivar graph hierarchy`

Analyze class, struct, and trait inheritance/implementation hierarchy

| argument | required | description |
| --- | --- | --- |
| `symbol` | yes | Symbol name to analyze inheritance/implementation hierarchy for |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Limit search to a specific repository |


## `ivar graph viz`

Generate a standalone zero-dependency HTML interactive graph visualizer

| flag | value | default | description |
| --- | --- | --- | --- |
| `--output` / `-o` | `<OUTPUT>` | `graph.html` | Output file path for the standalone HTML visualizer |
| `--repo` | `<REPO>` |  | Limit visualization to a specific repository |


## `ivar graph view`

Interactive browser-based codebase graph viewer

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Start graph visualization focused on a specific repository |
| `--symbol` | `<SYMBOL>` |  | Start graph visualization focused on a specific symbol |
| `--file` | `<FILE>` |  | Start graph visualization focused on a specific file path |
| `--impact` | `<IMPACT>` |  | Start graph visualization focused on transitive impact of a symbol |
| `--depth` | `<DEPTH>` |  | Maximum neighborhood depth hops (1..=2) |
| `--limit` | `<LIMIT>` |  | Maximum number of nodes to load initially (1..=500) |
| `--no-open` |  |  | Start the server without opening the browser automatically |
| `--port` | `<PORT>` |  | Optional port to listen on (defaults to an ephemeral loopback port) |


## `ivar graph clean`

Remove indexed repository data or clean the entire graph database

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Specific repository to remove from the graph index |
| `--feature` | `<FEATURE>` |  | Specific feature whose layers should be removed from the graph index |
| `--all` |  |  | Remove all repositories and data from the graph database |
| `--path` | `<DIR>` |  | Remove one hall directory from the index and from `graph.paths` in `ivar.json` |


## `ivar graph misses`

List recorded graph misses (skipped/follow-up searches, feedback)

| flag | value | default | description |
| --- | --- | --- | --- |
| `--kind` | `<KIND>` |  | Filter by miss kind: `skipped`, `followup`, `feedback`, or `weak` |
| `--since` | `<SINCE>` |  | Only include misses at or after this time: a Unix timestamp, or a relative duration like `30m`, `24h`, or `7d` |
| `--limit` | `<LIMIT>` | `10` | Maximum number of misses to display (0 for unlimited) |

