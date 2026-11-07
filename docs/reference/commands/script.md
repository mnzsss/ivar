<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar script`

List the hall's scripts, and approve or check their content


## `ivar script list`

List the hall's scripts: name, scope (personal or team), description, usage and env


## `ivar script approve`

Approve a script's exact content, by sha256, so an agent may run it without asking. Refuses (and records nothing) when the content matches a secret pattern

| argument | required | description |
| --- | --- | --- |
| `path` | yes | The script, relative to the working directory or absolute; it must lie inside the hall (e.g. `.ivar/scripts/deploy.sh`) |


## `ivar script check`

Report whether a script's current content is approved. Exits 0 when approved and 3 when unreviewed (never approved, or changed since)

| argument | required | description |
| --- | --- | --- |
| `path` | yes | The script, relative to the working directory or absolute; it must lie inside the hall (e.g. `.ivar/scripts/deploy.sh`) |

