<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar memory`

Inspect or delete the hall memory database (`.ivar/memory.db`)


## `ivar memory status`

Show the hall memory database: path, schema version, application id and size


## `ivar memory clean`

Delete the hall memory database after confirmation. Nothing else deletes it


## `ivar memory capture`

Record one session hook event into the capture spool. Always exits 0 and prints nothing

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | Provider whose hook payload arrives on stdin; without one nothing is recorded |

