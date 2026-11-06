<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar memory`

Inspect or delete the hall memory database (`.ivar/memory.db`)


## `ivar memory status`

Show the hall memory database: path, schema version, application id, size, observation counts per event, the capture spool, the last drain and the retention sweep


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

Show what a feature wrote to memory, its gaps and the pages its diff may have made stale, plus the last review. Read-only: confirm with the memory_review MCP tool

| argument | required | description |
| --- | --- | --- |
| `feature` | no | Feature to review. Defaults to the session's feature |


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

Record one session hook event into the capture spool. Always exits 0 and prints nothing

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | Provider whose hook payload arrives on stdin; without one nothing is recorded |

