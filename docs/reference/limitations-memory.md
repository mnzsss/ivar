# Limitations of session capture and memory

What session capture does not record, and what it costs. See
[Limitations and failure modes](limitations.md) for the rest of `ivar`.

Capture records four events per session into `.ivar/memory.db`: `session.started`,
`session.prompted`, `session.tool` and `session.ended`. It records an event only when
the provider's hook exposes it.

## Hook cost on every tool call

Each captured event is one `ivar memory capture` process. On Claude Code the hook
adds an entry beside the guard on every tool call. On OMP and OpenCode the plugin
spawns one process per event. The process reads at most 8 MiB of the payload, cuts
the event body to 32 KiB before redaction and 4 KiB after, and only appends a line to a file under
`.ivar/spool/memory/`; it never opens SQLite. Measured on an optimised binary,
the hook's p95 was about 15.8 ms per event. A slow disk or a debug build costs more.

## Events a provider does not expose

| Provider | Gap |
| --- | --- |
| OpenCode | No session end, so no `session.ended` is recorded. Its `tool.execute.after` reports `failed: false`, so no OpenCode `session.tool` is recorded as a failure and none gets importance 6. OpenCode has no brief: its plugins cannot inject context at session start, so the MCP instructions ask for `memory_handoff_list` and `memory_message_list`. |
| T3 Code | `session.ended` is recorded when the thread is closed (archived) with reason `other`, not when a turn finishes, so an idle open thread has no end yet. Deleting a thread or quitting the app was not tested. T3 hosts Claude Code, so it uses the Claude Code hooks. |

The payload fixtures under `tests/cli/capture_payloads` are written by hand from each
provider's documented hook format. They were not recorded from live sessions, so a
provider that changes its payload shape is caught by `ivar doctor`
(`provider.capture_hook_missing`) only when the hook disappears, not when it changes.

## Events lost or delayed

- A session spool is capped at 8 MiB. Past the cap, events are counted in
  `<key>.dropped` and discarded. `ivar doctor` reports them as `memory.spool_dropped`
  until a drain acknowledges the count, and for 7 days after the last drop.
- Events reach `memory.db` only when a drain runs. `ivar memory status` drains on demand,
  and the `ivar graph mcp` leader drains while it runs. The leader does not drain when
  `GraphDb::open` fails, so with a broken graph database only `ivar memory status`
  drains.
- A hook that opened `<key>.jsonl` just before a drain renamed it can append to the
  renamed file after the drain read it. The window is microseconds and the event is
  lost. This is accepted.
- Sanitising redacts known secret patterns before an event reaches disk. A secret
  in a shape no pattern matches is stored as written.

## Retention

The leader of `ivar graph mcp` deletes `session.*` observations of sessions whose last
observation is older than 30 days, once at start and then every hour. A hall without a
running leader never sweeps, `ivar doctor` reports `memory.retention_overdue` after
37 days, and a broken graph database stops the loop. `delivery.*`, `handoff.*` and
`message.*` observations are kept; `ivar memory clean` deletes the whole database.

The same sweep marks handoffs and messages past `memory.handoff_ttl_days` as expired, and
deletes accepted, popped, cancelled and expired rows 30 days after they ended. The read
tools skip an expired row on their own, so a late sweep only delays the deletion.

## Session pages

`ivar` writes one `session` page per provider conversation in
`.ivar/memory/session/<id>.md`, by template, with no model call. A conversation is
consolidated once it ends (`session.ended`) or after 30 minutes without observations,
and `ivar session stop` consolidates the conversations of that session right away.
The MCP leader consolidates every 5 seconds. `memory_query`, `ivar memory query`,
`memory_recent` and `memory_read_session_observations` consolidate before they read,
and skip the pass when another process holds `memory.lock`. `ivar memory status` and
`ivar session stop` wait for the lock.

- A conversation with no observation of importance 4 or more gets no page.
- OpenCode has no session end, so its pages appear after 30 idle minutes or on stop.
- A page expires 30 days after the conversation's last observation. The hourly sweep
  deletes it, so an expired page can still be returned for up to an hour.
- `pinned: true` keeps a page past 30 days; an explicit `expires_at` always applies.
  Only `memory_patch_page` sets them, and regeneration keeps both.
- The page is regenerated only when the conversation has new observations; hand edits
  to its body are lost on the next regeneration.
- A failed pass is recorded in `.ivar/spool/memory/_consolidate.json`. `ivar doctor`
  reports it as `memory.consolidate_failed` and `ivar memory status` prints it.

## The page index

`ivar memory query` searches the pages under `docs/memory/` through an index in
`memory.db`; see [memory pages](on-disk-format-pages.md).

- The index follows the disk, not a watcher. It is refreshed by `ivar memory query`,
  `ivar memory status` and `ivar sync`, and every 5 s by the `ivar graph mcp` leader. A
  hall without a leader reindexes only on those reads. `ivar sync` never waits for
  `memory.lock`: when another process holds it, sync leaves the pass to that process.
  An uncommitted page is indexed too.
- A page that fails validation is not indexed and `ivar doctor` names it
  (`memory.page_invalid`, `memory.page_kind_unknown`, `memory.page_misplaced`). A
  failed reindex, including one that could not take `memory.lock`, shows as
  `memory.index_failed`, and `ivar memory status` prints the refresh error. A pass is
  one transaction, so the last good index keeps answering, also after a failed
  `ivar memory reindex`.
- A pass that indexes and removes nothing leaves `_reindex.json` as it is, so
  `last_reindex_at` is the last pass that changed the index, not the last tick.
- Wikilinks resolve by slug. Two pages with the same slug in different kinds make a
  bare `[[slug]]` ambiguous; write `[[kind/slug]]`.
- Ranking constants (the fusion constant, the tier multipliers, the pinned bonus) belong
  to the binary and are not configurable.
- Link expansion adds the neighbours of the ten best matches, ranked behind them. It is
  skipped when `--kind`, `--tier` or `--tag` is set.
- Anchors are absent: a page is found as a whole, never at a heading, and the lint has no
  anchor check.
- `docs/memory/` is indexed twice until the retrieval slice: `ivar graph index` still
  reads `docs/` as text, so the same page can also answer `graph_explore`.
- Observations answer only when no page matches and no filter is set, and only the ones
  the retention sweep has not deleted.

## Anchors and usage

- Anchors resolve against the base layer only, never a feature session's layer.
- A change in the graph never re-anchors a page. A file indexed after the page was anchored joins
  only when the page changes or `ivar memory reindex` runs.
- An ambiguous path or symbol becomes an anchor only when the page also cites a resolved path in
  the same repo or file; otherwise it is dropped.
- The guard counts a memory read from a `Read` target, a Grep or Glob `path`, or a Bash search
  command. A non-search Bash read such as `cat docs/memory/x.md` is not counted.
- Usage and misses are written with a 50 ms busy timeout and dropped when the database is busy.

## The memory MCP tools

The tools are in [MCP tools](mcp-tools.md).

- The tools never commit. A page they write stays uncommitted until you commit it, and
  `ivar doctor` reports `memory.page_uncommitted`.
- The MCP server runs outside the session write guard. The write tools confine themselves
  to `docs/memory/<durable kind>/`, plus `.ivar/memory/session/` for `memory_patch_page`,
  and no hook checks the files a call touches.
- Claude Code approves the server, not single tools. Approving `<hall>-ivar` approves
  `memory_write_page`, `memory_patch_page` and `memory_delete_page` with the read tools.
- Title similarity is a word overlap of 0.8 or more within one kind. A reworded title can
  slip past it, and a short title can match too easily. A match never blocks a write.
- `memory_lint` and `ivar memory lint` check markdown links only. A wikilink or a
  `relations` target without a page is not an error, so a planned stub stays valid.
- Two writers numbering `product` or `update` pages at once are serialised by
  `memory.lock`. A filesystem without hard links cannot publish a page without `replace`.

## The memory review

- A captured write counts as a written page only when the provider's hook exposes its
  path (`file_path`, `filePath` or `path`). A body cut at 4 KiB keeps the path in most
  cases, because the path is searched in the raw text too.
- A stale page comes from the feature's diff against its merge base, uncommitted and
  untracked files included. Deliver refuses an uncommitted worktree, so at apply the
  set is `merge-base..HEAD`.
- OpenCode reports every tool as `failed: false`, so the "failed tool without a
  gotcha" gap never fires for OpenCode sessions.
- "Analysis approved without a query" uses the captured `ivar plan approve <feature> analysis`.
  When that command was not captured, any captured `memory_query` in the feature's
  sessions counts.
- A page removed or renamed by a captured Bash command (`rm`, `mv`, `sed -i`) is not
  seen, so it stays in `written`. A cut body is searched as raw text, so a write that
  quotes a `docs/memory/` path in its content can count as that page.
- A stale page is found only through its anchors. A page that cites the changed code in
  prose without an anchor is not listed.
- `memory.reviewed` keeps at most 50 verdicts, 50 written and 50 stale paths in its
  body. A body past 4 KiB is cut and `last` loses its lists, but the heads survive.
- A broken anchor counts wherever its file is, so a page broken before the feature
  is listed too and needs a verdict.

## Handoffs and messages

The tools are in [MCP tools](mcp-tools.md#handoffs-and-messages).

- Outside an `ivar session` the owner is `<provider>:mcp-<uuid>`, one uuid per MCP server
  process. It ends with the conversation, so afterwards only
  `ivar memory handoff cancel <id>` or the expiry clears its open handoff.
- A `clientInfo.name` the provider matrix does not know resolves to provider `unknown`.
  Outside an `ivar session` such a caller sees only unaddressed items.
- `ivar memory brief` runs at session start on Claude Code and T3 (`SessionStart`) and
  on OMP (`before_agent_start`, once per process). A reload of the OMP hook resets its
  module flag, so the brief shows again. OpenCode has none.
- The brief prints at most 5 handoff headers, opened by a header that marks them as
  untrusted data. Each summary is cut to 120 characters and quoted on one `> ` line.
  Messages appear only as a count, never by body.
- The brief reads `memory.db` read-only and never drains the spool. It prints nothing on
  any failure and always exits 0. Measured with 1000 handoffs, its p95 was 13.46 ms.
- `ivar session stop` warns `session.handoff_missing` when the session worked and opened
  no handoff. It does not block the stop.

## Older `ivar`

A hall that sets `memory.capture` or `memory.handoff_ttl_days` in `ivar.json` cannot be
read by an `ivar` that predates the key. See [On-disk format: versions](on-disk-format-versions.md).
