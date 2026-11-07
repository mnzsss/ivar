<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar automation`

Find the requests users keep repeating, as candidates to automate


## `ivar automation scan`

Report the requests users keep repeating, as candidates to automate: groups of similar prompts typed in at least 3 sessions on at least 2 days of the last 90. Read-only apart from draining the capture spool; the samples are memory text, evidence only. Run it from a session that can write `.ivar/` or with `--no-drain` where it cannot

| flag | value | default | description |
| --- | --- | --- | --- |
| `--feature` | `<FEATURE>` |  | Keep only the groups with a prompt in a session of this feature or one of its descendants |
| `--no-drain` |  |  | Do not drain the capture spool first. Nothing is written, so it works when `.ivar/` is not writable; prompts still in the spool are missed |

