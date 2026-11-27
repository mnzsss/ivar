<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar memory`

Inspect, query and maintain the hall memory: pages, handoffs and the session hooks


## `ivar memory status`

Show the hall memory database: path, schema version, application id, size, observation counts per event, the capture spool, the last drain and the retention sweep. Drains the spool, consolidates and reindexes pages, so it writes under `.ivar/`

| flag | value | default | description |
| --- | --- | --- | --- |
| `--read-only` |  |  | Only read: take no lock, drain nothing, migrate, consolidate and reindex nothing, so it works where `.ivar/` is not writable. The spool is reported as pending (`drained: false`) and `pages.invalid` as not verified |


## `ivar memory clean`

Delete the hall memory database after confirmation. Nothing else deletes it


## `ivar memory query`

Search the pages under docs/memory. When no page matches and no filter is set, retained observations answer instead

| argument | required | description |
| --- | --- | --- |
| `text` | yes | Words to search for |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--kind` | `<KIND>` |  | Keep only pages of this kind (repeatable) |
| `--tier` | `<TIER>` |  | Keep only pages of this tier: semantic, procedural, episodic or working |
| `--tag` | `<TAG>` |  | Keep only pages carrying this exact tag |
| `--limit` | `<LIMIT>` | `10` | Most results to return (at most 20) |
| `--include-expired` |  |  | Also return pages whose expires_at has passed |
| `--explain` |  |  | Show how each source ranked every result |


## `ivar memory reindex`

Rebuild the page index from docs/memory. Observations are not touched


## `ivar memory lint`

Check the pages under docs/memory against the page rules. Exits non-zero when any rule is broken


## `ivar memory review`

Show what a feature wrote to memory, its gaps and the pages its diff may have made stale, plus the last review. Drains the capture spool first, unless `--read-only`; `--confirm` records the review that `ivar feature deliver` requires, the way the memory_review MCP tool does

| argument | required | description |
| --- | --- | --- |
| `feature` | no | Feature to review. Defaults to the session's feature |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--confirm` |  |  | Record the review as done, with the same checks as the memory_review MCP tool's `confirm` |
| `--stale` | `<PATH=VERDICT>` |  | A verdict on a possibly stale page, `<path>=updated` or `<path>=still_valid` (repeatable). Needs `--confirm` |
| `--not-required` | `<REASON>` |  | Confirm a feature that wrote no page, and say why none was needed. Needs `--confirm` |
| `--read-only` |  |  | Only read: take no lock, drain nothing and migrate nothing, so it works where `.ivar/` is not writable. Observations still in the capture spool are not counted, and a warning says so |


## `ivar memory eval`

Measure retrieval quality against a file of labelled queries


### `ivar memory eval sample`

Export distinct queries from the usage log (never hook ones) as a JSONL label file: each line has `query` and an empty `expected`, for a human to fill with the paths of the relevant pages. Read-only

| flag | value | default | description |
| --- | --- | --- | --- |
| `--days` | `<DAYS>` | `30` | Only queries made in the last N days |
| `--limit` | `<LIMIT>` | `100` | Most queries to export, newest first |
| `--out` | `<OUT>` |  | Write the JSONL here instead of stdout |


### `ivar memory eval run`

Run each labelled query through the ranking `ivar memory query` uses and report hit@k, recall@k and MRR, per query and overall. Lines with an empty `expected` are skipped and counted. Records nothing in the usage log

| argument | required | description |
| --- | --- | --- |
| `file` | yes | JSONL label file: `{"query": "...", "expected": ["docs/memory/..."]}` per line |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--k` | `<K>` | `3` | How many top results count |


## `ivar memory handoff`

Open, list or cancel a handoff for the next session. Accepting one and messages are MCP-only


### `ivar memory handoff begin`

Open a handoff in this session's feature queue, or the hall's outside a feature

| flag | value | default | description |
| --- | --- | --- | --- |
| `--summary` | `<SUMMARY>` |  | Where the work stopped, in markdown |
| `--pending` | `<PENDING>` |  | A task the next session should pick up (repeatable) |
| `--decision` | `<DECISION>` |  | A decision still open (repeatable) |
| `--file` | `<FILE>` |  | A file the work touched (repeatable) |
| `--feature` | `<FEATURE>` |  | Open it in this feature's queue instead |
| `--hall` |  |  | Open it in the hall queue |
| `--to-provider` | `<TO_PROVIDER>` |  | Only sessions of this provider see it |
| `--to-session` | `<TO_SESSION>` |  | Only this ivar session sees it |
| `--session` | `<SESSION>` |  | Open it on behalf of this ivar session, which becomes its owner |


### `ivar memory handoff list`

List open handoffs. Read-only

| flag | value | default | description |
| --- | --- | --- | --- |
| `--feature` | `<FEATURE>` |  | Only this feature's queue |
| `--hall` |  |  | Only the hall queue |


### `ivar memory handoff cancel`

Cancel an open handoff, whoever owns it

| argument | required | description |
| --- | --- | --- |
| `id` | yes | The handoff id, as `list` shows it |


## `ivar memory capture`

Record one session hook event into the capture spool. Always exits 0 and prints nothing. When a session with no feature ends, it also starts `ivar hall sync` in a process of its own, logging to `.ivar/hall-sync.log`, and does not wait for it; `auto_commit: false` in `ivar.json` turns that off

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | Provider whose hook payload arrives on stdin; without one nothing is recorded |


## `ivar memory brief`

Print the open handoffs and pending messages this session can see, for a session-start hook, and one line when the hall's sync is blocked. Always exits 0; prints nothing when there is nothing to show

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | Provider whose hook runs this; used when the session is outside `ivar session` |

