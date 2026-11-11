<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar session`

Manage sessions


## `ivar session start`

Open a session: view dir over a feature's promoted repos, agent running in it, TUI on top

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to open a session for. Omit for a discovery session: no feature bound, every repo read-only on its default branch |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--resume` |  |  | Resume an existing session, where the harness supports it |
| `--provider` | `<PROVIDER>` |  | The provider to run. Defaults to the hall's default provider |
| `--detached` |  |  | Create the session without launching a provider. The view dir persists after this command returns, until an explicit stop |
| `--relay` |  |  | Relay: a fresh session on the same feature under a different provider than the feature's most recent session. Requires `--provider` |
| `--prompt` | `<PROMPT>` |  | The agent's first message: the session opens already working on it. Passed as one argument, never through a shell, and never persisted |


## `ivar session list`

List every live session in the hall — discovery and feature sessions — oldest first. Read-only


## `ivar session connect`

Re-bind to an existing live session: locate it, re-materialise its view dir, and emit the binding as `IVAR_*` env vars

| argument | required | description |
| --- | --- | --- |
| `session_id` | no | The session id, or a unique prefix of one |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--feature` | `<FEATURE>` |  | Narrow the search to sessions bound to this feature |
| `--create` |  |  | Attach or create: take the feature's most recent session that no harness is running in, and start a fresh detached one when every candidate is busy or none exist. Needs `--feature` |


## `ivar session convert`

Promote a discovery session to a feature session, keeping its name

| argument | required | description |
| --- | --- | --- |
| `session_id` | yes | The discovery session's id, or a unique prefix of one |


## `ivar session stop`

Stop a session — tear down its view dir and end any running harness. Omitting the session stops `$IVAR_SESSION_ID`; `--all` stops every session in the hall. Stopping a discovery session (one with no feature) also commits the pages it wrote under `docs/memory/` as `docs(memory): Record discovery session pages`, then commits the rest of the hall and pushes, unless `auto_commit` is off in `ivar.json`

| argument | required | description |
| --- | --- | --- |
| `session` | no | The session to stop — its id, or a unique prefix of one. Defaults to `$IVAR_SESSION_ID`; with neither, the command stops nothing and fails |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--all` |  |  | Stop every session in the hall: every discovery session and every feature's sessions |


## `ivar session prune`

Remove dead sessions: view dirs that exist but hold no readable `state.json`. A session with a readable record is never touched


## `ivar session relay`

Relay session info: four-line output contract for external consumers

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to relay a session for. Defaults to the session's feature |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | The provider to relay to. Required — relay must switch providers |


## `ivar session env`

Resolve and output the session environment by walking up from cwd

| flag | value | default | description |
| --- | --- | --- | --- |
| `--cwd` | `<CWD>` |  | Working directory to resolve from. Defaults to current working directory |

