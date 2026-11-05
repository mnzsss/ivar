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


## `ivar memory capture`

Record one session hook event into the capture spool. Always exits 0 and prints nothing

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | Provider whose hook payload arrives on stdin; without one nothing is recorded |

