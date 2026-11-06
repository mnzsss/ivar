# Command reference

Every verb `ivar` has, what it does, and what it takes.

The verb pages linked below — flags, arguments, defaults — are **generated from the binary's
own argument parser**, so they cannot drift from what your `ivar` actually
accepts. The prose around them is written by hand, because when to reach for a
verb is a judgement call and a generator has no opinion about it.

If you are looking for the shape of the tool rather than its surface, read
[Concepts](../concepts.md) first. This page is a lookup table, not a tour.

## How to read this

- **`ivar <verb>`** — the hall-level verbs. The hall is the root of the surface:
  the verb is `ivar sync`, with no `hall` group in front of it.
- **`ivar <group> <verb>`** — `repo`, `feature`, `session`, `provider`, `plan`
  and `skill` group their verbs, because each has more of them than a flat root
  could carry.
- **`--json`** works on every verb and prints exactly the value the verb
  computed — the human text you see is a rendering of that same value, never a
  separately written summary. Script against `--json`.

## Exit codes

| code | meaning |
| --- | --- |
| `0` | done, with nothing needing attention |
| `1` | done, but something needs attention — the warnings are on stderr |
| `2` | refused before starting, or broke mid-flight |

Exit `1` is the one worth designing around: a verb that crosses eight repos and
hits a problem in one of them reports seven successes and a warning, and that is
a `1`, not a failure. Treating `1` as fatal in a script will make `ivar` look
broken when it is telling you something.

<!-- BEGIN GENERATED COMMANDS -->
<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

### `ivar`

Multi-repo worktrees for coding agents: one feature across repos, on one branch, one PR per repo.

| flag | value | default | description |
| --- | --- | --- | --- |
| `--json` |  |  | Emit machine-readable output. Prints exactly the value the command computed. The human-readable text is a rendering of that same value, so the two can never tell you different things — script against this. |
| `--compact` |  |  | Output token-optimized compact pipe-delimited records with schema header. Only `ivar graph` verbs support it; any other verb refuses it with a usage error (exit 2) rather than printing the human output. |
| `--color` | `<COLOR>` | `auto` | When to colour output. `auto` follows `NO_COLOR`, then `FORCE_COLOR`, then whether the stream is a terminal — a pipe or a redirect gets none. `always` and `never` override all of that. Only labels are ever coloured; values never are, so `--json` is unaffected either way. |
| `-C` | `<dir>` |  | Run as if `ivar` was started in `<dir>`. Applied before the hall or session is looked up, so every verb sees `<dir>` as its working directory — like `git -C`. Repeated `-C` options compose: each relative one is resolved against the one before it. |

| verb | what it does |
| --- | --- |
| [`ivar init`](commands/init.md) | Create a hall: `ivar.json`, `.ivar/`, and the hall's `.gitignore` lines |
| [`ivar sync`](commands/sync.md) | Bring the local hall in line with `ivar.json`: clone missing repos, materialise harness config, run setup scripts |
| [`ivar status`](commands/status.md) | Report hall health |
| [`ivar doctor`](commands/doctor.md) | Diagnose problems and suggest fixes |
| [`ivar cleanup`](commands/cleanup.md) | Reconcile stale state (interactive; asks before deleting) |
| [`ivar migrate`](commands/migrate.md) | Advance `ivar.json`'s schema version (interactive; shows the change, then asks) |
| [`ivar repo`](commands/repo.md) | Manage repos |
| [`ivar feature`](commands/feature.md) | Manage features |
| [`ivar session`](commands/session.md) | Manage sessions |
| [`ivar provider`](commands/provider.md) | Manage providers |
| [`ivar discovery`](commands/discovery.md) | Manage discovery docs: a unit of work's working brief |
| [`ivar plan`](commands/plan.md) | Manage SPDD plans |
| [`ivar review`](commands/review.md) | Review a feature's local changes |
| [`ivar skill`](commands/skill.md) | Manage skills |
| [`ivar mcp`](commands/mcp.md) | Authenticate the hall's declared MCP servers |
| [`ivar graph`](commands/graph.md) | Query and index the codebase dependency graph |
| [`ivar memory`](commands/memory.md) | Inspect, query and maintain the hall memory: pages, handoffs and the session hooks |
| [`ivar feedback`](commands/feedback.md) | Record bugs, proposals, and submit issues |
| [`ivar agent`](commands/agent.md) | Schedule and run unattended agents in containers |
| [`ivar guard`](commands/guard.md) | Guard: evaluate a tool request against the session's writable set |
| [`ivar upgrade`](commands/upgrade.md) | Upgrade ivar through the channel that installed it |
<!-- END GENERATED COMMANDS -->

## Notes the generator cannot give you

**`ivar migrate` is not part of a normal week.** It exists for one situation:
you upgraded `ivar`, the new build's `ivar.json` format is newer than the one
your hall was written with, and something refused to write until a human agreed
to advance it. See [On-disk format](on-disk-format.md).

**`ivar cleanup` and `ivar migrate` both ask, and both refuse to act when nobody
is there to answer.** Run either with output piped and they print what they
*would* do and change nothing. That is deliberate: neither deletion nor a
rewrite of a committed file should be reachable by a script that nobody is
watching. There is no `--yes`.

**`ivar graph index` indexes all tracked text files and declares the graph MCP server.**
The graph index covers every Git-tracked UTF-8 textual file across all declared repositories
(including configuration, workflows, documentation, schema files, and dotfiles) regardless of
extension, up to 256 KiB of searchable content per file. Files with recognized tree-sitter grammars
receive structural symbols and call edges; other plain-text files are indexed for path, basename, and
full-text content search without synthetic AST symbols. Binary files (containing NUL bytes or invalid
UTF-8), untracked files, Git-ignored files, and ignored build/dependency directories are excluded.
When no `mcp` entry in `ivar.json` already runs `ivar graph mcp`, it adds
`{"name":"ivar","type":"local","command":"ivar","args":["graph","mcp"]}` and
reports `mcp_registration` (`registered`, `already_declared`, `name_clash`,
`needs_migration`, `failed` or `skipped`), with `next_command: "ivar sync"`
after a registration. It never runs `ivar sync`: run it yourself so providers
pick the server up. A different server already named `ivar` is left alone
with a warning; an `ivar.json` that still needs `ivar migrate` is left alone
without one, so indexing still exits cleanly; `--no-mcp` skips registration
entirely.

**`ivar graph explore` unifies structural and textual retrieval with bounded output.**
Retrieval applies lexicographic ranking precedence: unambiguous repository/path constraints, exact
symbol or basename matches, path-pinned matches, structured symbol names, content FTS, and loose
symbol FTS. Total candidates are capped at 20 across symbols and files (`MAX_EXPLORE_CANDIDATES = 20`),
reserving slots for pinned and exact matches first. Multi-repo queries consume all named visible repositories.
Human-readable CLI relation sections (entry points, callers/callees, transitive consumers) are capped at 40 items (`MAX_LIST_ITEMS`)
with `…and N more` and an 18,000 character output budget. Specific file slices can be requested with
`path:START-END` line ranges, and block truncation notices specify the exact continuation slice `path:<n+1>-<last>`.

**`graph_explore` MCP structured output has a byte budget.** Compact and JSON
responses are limited to 18,000 UTF-8 bytes of MCP text, or 24,000 for explicitly
requested files. Schemas, escaped values, separators, and continuation metadata
count toward the limit. These byte limits are not guarantees about a host's
model-token limit. Markdown retains its existing source-range behavior.

A complete structured response keeps its existing representation. An oversized
JSON response keeps the existing result fields and adds `output` with
`partial: true`, `budget_bytes`, per-category `omitted` record counts,
`omitted_fields` for oversized scalar values, and `next` when omitted located
evidence can be inspected, plus `truncated` when source text was cut. Rows are
omitted whole, except source text, which is cut at a line boundary; a partial
result does not claim that omitted callers, consumers, or source do not exist.

An oversized compact response retains its normal schemas and complete rows and
adds `#SCHEMA: partial|budget_bytes`,
`#SCHEMA: omitted_category|omitted_records`, and, when available,
`#SCHEMA: next_tool|next_arguments`. The next_arguments cell is single-line JSON;
a literal pipe in its values is encoded as `\u007c` so it stays one cell.

Source comes before relations: the requested source (symbol, file, or `paths`
range) is admitted first and callers, entry points, relations and consumers get
what is left. A source larger than the room is cut at a line boundary and listed
in `output.truncated`; `next` then names `path:start-end` from the following
line to the end of the range, so repeating the call advances without gaps or
overlap. See `docs/product/012-graph-explore-precision.md`.

Follow the returned `graph_explore` arguments: they name an actual repository and
source interval using Markdown, rather than repeating the broad query. This is
targeted evidence inspection, not cursor pagination of every graph relation.
Oversized source-less scalar text can be omitted without inventing a source
follow-up. An unrepresentable evidence identity is an explicit rendering error,
not silent truncation. Machine-readable `ivar graph explore --json` and
`--compact` keep their complete output contract; only MCP structured responses
use this budget.

**`ivar graph misses` shows where the graph fell short.** The guard records a
`skipped` miss when a session greps (Grep, Glob, or an `rg`/`grep` Bash
command) without having asked the graph anything, and a `followup` miss for the
first grep within two minutes of a graph call, carrying that call's query. The
`graph_feedback` MCP tool, advertised under both `--tools` surfaces, lets the
agent record a `feedback` miss with the query it asked and why the answer did
not help. `graph_explore` itself records a `weak` miss, with the query and the reason
`weak`, when no candidate covers enough of the query's words (see
[graph_explore precision](../product/012-graph-explore-precision.md)).
`--kind skipped|followup|feedback|weak` filters by kind; `--since` takes a
Unix timestamp or `Nm`/`Nh`/`Nd`. `--limit <N>` (default 10, 0 for all) bounds
the returned rows. `--json` prints a bare array of misses, newest first, not the usual report object.
Misses and usage rows are kept for 30 days, so `ivar graph stats` reports a 30-day window; `ivar graph misses`,
`ivar graph stats`, and the MCP server at startup prune older rows.

**`ivar doctor` reports graph and memory database findings by code.** Findings and their fixes:

| Code | Meaning | Fix |
| --- | --- | --- |
| `graph.repo_not_in_manifest` | an indexed repo `ivar.json` no longer declares | `ivar graph clean --repo <name>` or `ivar graph index` |
| `graph.layer_orphaned` | a layer of a feature that no longer exists | `ivar graph index` |
| `graph.watch_scope_orphaned` | a watcher scope with no repo, feature or path behind it | `ivar graph index` |
| `graph.watch_scope_failed` | a scope with a live target the watcher could not reindex | `ivar graph index`; a leader retries once the scope is watched again |
| `graph.repo_stale` | a repo indexed behind its worktree HEAD | `ivar graph index --repo <name>` |
| `graph.hall_paths_unset` | the hall has `docs`, `brand` or `.ivar/skills` with files and `graph.paths` is empty | `ivar graph index --path <dir>` lists the directory in `graph.paths` and indexes it |
| `graph.schema_mismatch` | database schema older or newer than the binary | `ivar graph index` or `ivar upgrade` |
| `graph.open_failed` | `.ivar/graph.db` exists but cannot be opened | remove it, then `ivar graph index` rebuilds it |
| `graph.watch_catchup_pending` | no watcher leader is running and a scope has changes nobody indexed | `ivar graph index`, or start `ivar graph mcp` to catch up |
| `graph.db_missing` | the graph MCP server is registered but `.ivar/graph.db` is gone | `ivar graph index` |
| `memory.db_missing` | `.ivar/graph.db` exists without `.ivar/memory.db` | `ivar memory status` creates it |
| `memory.db_corrupt` | `.ivar/memory.db` fails to open, fails `quick_check`, or carries another `application_id` | `ivar memory clean`, then `ivar memory status` |
| `memory.schema_newer` | memory schema newer than the binary | `ivar upgrade` |
| `memory.db_polluted` | an older `ivar` created graph tables inside `.ivar/memory.db` | stop the older `ivar`, then `ivar memory status` repairs it |
| `memory.split_busy` | a legacy graph at `.ivar/memory.db` is open in another process, so it was not moved | stop sessions and MCP servers of an older `ivar`, then `ivar doctor` |
| `memory.split_failed` | moving the legacy graph failed for another reason | check `.ivar/` permissions and free space, then `ivar doctor` |
| `memory.spool_orphan` | a capture spool file has waited over an hour to be drained | `ivar memory status` drains it |
| `memory.spool_dropped` | events were dropped because a session spool reached 8 MiB, and the last drop was within 7 days | `ivar memory status`, or keep `ivar graph mcp` running, so a drain acknowledges them; the finding clears after 7 days without new drops |
| `memory.drain_failed` | the last drain of the capture spool failed | `ivar memory status`; if the error repeats, read the database findings of `ivar doctor` |
| `memory.sweep_failed` | the last retention sweep failed | `ivar memory status`; if the error repeats, read the database findings of `ivar doctor` |
| `memory.consolidate_failed` | the last consolidation of session pages failed | `ivar memory status`; if the error repeats, read the database findings of `ivar doctor` |
| `memory.retention_overdue` | session observations older than 37 days are still stored | keep `ivar graph mcp` running; its leader deletes expired session observations hourly |
| `provider.capture_hook_missing` | `memory.capture` is on and a provider has no capture hook | `ivar sync`, or set `memory.capture` to false in `ivar.json` |

**`ivar memory query` searches the pages under `docs/memory/`.** `ivar memory query <TEXT>`
takes `--kind` (repeatable), `--tier`, `--tag`, `--limit` (default 10, at most 20),
`--include-expired` and `--explain`, and `--json`. Results carry kind, tier, path, snippet
and `pinned`. With no page hit and no filter it returns retained observations instead.
`ivar memory reindex` rebuilds the index from disk and never touches observations. The
page format is in [on-disk format: pages](on-disk-format-pages.md).

Page findings of `ivar doctor`:

| Code | Meaning | Fix |
| --- | --- | --- |
| `memory.legacy_docs` | `docs/product` or `docs/updates` still holds at least one Markdown page (any depth, any extension case); an empty folder, or one with no `.md`, is not reported | the commands it prints: `git mv` (or `mv` in a hall without git) on the folder itself, or on its contents (`git mv docs/updates/* docs/memory/update/`, `mv -n …` without git) when the destination already exists. Nothing is overwritten: a page that already exists at the destination stays where it is, so resolve those by hand. Hidden entries are skipped |
| `memory.page_invalid` | front matter does not parse, `kind` differs from the directory, or `expires_at` is not `YYYY-MM-DDTHH:MM:SSZ` | edit the page |
| `memory.page_kind_unknown` | a directory under `docs/memory/` or `.ivar/memory/` is not a kind of that root | rename or move it |
| `memory.page_misplaced` | a page is outside `<kind>/`, nested, has a non-kebab slug, or does not end in lowercase `.md` (or no `NNN-` for `product` and `update`) | move or rename it |
| `memory.page_field_unknown` | front matter has a key the format does not know (warning) | remove it or ignore it |
| `memory.page_uncommitted` | page files under `docs/memory` have uncommitted changes | commit them |
| `memory.index_failed` | the last page reindex failed | `ivar memory reindex` |

**`ivar session start` is the one verb that takes over your terminal.** It opens
a TUI. Everything else prints and exits.

**`ivar feature execute …` manages the Run Receipt lifecycle for an approved
plan.** The provider coordinates guided wave execution via the `ivar-execute` skill; Ivar records the execution boundary and Run Receipt evidence.
Read [Planning and execution](../guides/planning-and-execution.md) before using
it directly.
