<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar discovery`

Manage discovery docs: a unit of work's working brief


## `ivar discovery create`

Start a unit of work's discovery brief. When run in a discovery session, writes `discovery.md` into the session view dir

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The unit of work's name. Lowercase kebab-case |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--title` | `<TITLE>` |  | A human-readable title. Defaults to the name |


## `ivar discovery list`

List every unit of work with a discovery doc

| flag | value | default | description |
| --- | --- | --- | --- |
| `--status` | `<STATUS>` |  | Show only discoveries in this status |


## `ivar discovery show`

Print one unit of work's memory

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The unit of work's name |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--path` |  |  | Print only the path, not the content |


## `ivar discovery amend`

Add to a unit of work's memory. Appends a dated block by default; `--merge` replaces the whole document and requires `--expected-hash`

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The unit of work's name |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--file` | `<FILE>` |  | Read the content from this file, or from stdin with `-` |
| `--merge` |  |  | Replace the whole document instead of appending to it |
| `--expected-hash` | `<EXPECTED_HASH>` |  | The document's current SHA-256. Required with `--merge` |


## `ivar discovery close`

End a discovery: `converted` when it became a feature, `abandoned` when it did not. The doc is kept either way

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The unit of work's name |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--outcome` | `<OUTCOME>` |  | How it ended |

