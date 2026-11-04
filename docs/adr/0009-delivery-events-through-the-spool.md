# ADR-0009 — Delivery events through the spool

- **Status:** accepted
- **Date:** 2026-11-04

## Context

Hall memory records what happens inside sessions. It does not record what the binary
does to a feature: pushes, pull requests, rebases, merges, cleanup. Those milestones
are the part of a feature's history that outlives its sessions.

Delivery commands are the verbs that matter most, so recording must not change their
outcome. A locked or corrupt `memory.db` must not fail a push, and the same merge
can be seen by more than one command.

Session observations also grew without bound, and the delivery events would add to them.

## Decisions

### D1 — A sink on `Ctx`, default `Silent`

Actions record through a `DeliverySink` carried on `Ctx`, the same shape as `Progress`.
The default is `Silent`, so tests observe an action only through what it returns. The
process installs `SpoolSink`. Eleven events exist: `delivery.refused`, `pushed`, `pr`,
`landed`, `checks`, `rebased`, `integrated`, `merged`, `cleaned`, `closed` and `deleted`.

### D2 — Spool first, drained by the existing drain

`SpoolSink` sanitises the event and appends it to `.ivar/spool/memory/delivery.jsonl`.
The existing drain inserts it. No delivery verb opens SQLite, takes `memory.lock` or waits
on a drain. Bodies are bounded to 10 list items and 200 characters per text.

### D3 — A deterministic `event_id` for merges

Most events take an id from the attempt, the event name, the repo and a sequence number.
A `delivery.merged` id derives from the repo, the pull request URL and the merge commit,
so a merge observed by `cleanup` and again by `integrate` is one row, and the unique
`event_id` absorbs the repeat.

### D4 — The sweep groups `session.*` by session, in the leader

The retention sweep deletes the rows of a session, keyed by `session_id` and
`native_session_id`, when its newest observation is older than 30 days. A session is kept
or deleted whole. The event names it may delete are a closed list of the four `session.*`
names, so `delivery.*`, `memory.*` and `handoff.*` are never touched. It runs in the
`ivar graph mcp` leader once at start and then hourly, under a non-waiting `memory.lock`,
deletes in batches of 1,000 and records its state in `_sweep.json`. It never creates
`memory.db`.

### D5 — A failed write prints one stderr line and keeps exit code 0

When the sanitiser, the spool size cap or the filesystem refuses an event, the sink prints
`ivar: memory.delivery_event_not_recorded: <reason>` on stderr. The command's outcome and
exit code do not change. Nothing appears under `--json`.

## Consequences

- A delivery event appears in `memory.db` only after a drain.
- A hall without a running `ivar graph mcp` leader never sweeps. `ivar doctor` reports
  `memory.sweep_failed` after a failed sweep and `memory.retention_overdue` when a session
  observation is older than 37 days; `ivar memory status` shows the retention window, the
  last sweep and the count deleted.
- `delivery.*` observations are kept. `ivar memory clean` stays the only way to delete them.
- Events the binary does not run through a verb (the `deliver` preview, `feature rename`,
  `feature status`) record nothing; see
  [limitations of delivery](../reference/limitations-delivery.md).
