<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar feedback`

Record bugs, proposals, and submit issues


## `ivar feedback add`

Add a new feedback entry (bug or proposal)

| flag | value | default | description |
| --- | --- | --- | --- |
| `--title` | `<TITLE>` |  | Title of the feedback entry |
| `--kind` | `<KIND>` | `bug` | Kind of feedback (bug or proposal) |
| `--file` | `<FILE>` |  | File containing feedback description, or - for stdin |


## `ivar feedback list`

List recorded feedback entries

| flag | value | default | description |
| --- | --- | --- | --- |
| `--status` | `<STATUS>` |  | Filter by status: open or published |


## `ivar feedback show`

Show details and body of a feedback entry

| argument | required | description |
| --- | --- | --- |
| `id` | yes | Feedback entry ID (e.g. 001-my-bug) |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--redacted` |  |  | Redact local paths, usernames, and hostnames in displayed output |


## `ivar feedback submit`

Submit a feedback entry as a GitHub issue

| argument | required | description |
| --- | --- | --- |
| `id` | yes | Feedback entry ID (e.g. 001-my-bug) |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | GitHub target repository (defaults to mnzsss/ivar) |
| `--preview` |  |  | Print the redacted issue and its fingerprint; publish nothing. Needs no terminal |
| `--fingerprint` | `<FINGERPRINT>` |  | Publish without a terminal, only if the entry still matches this preview fingerprint |

