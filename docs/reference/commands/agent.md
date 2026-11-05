<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar agent`

Schedule and run unattended agents in containers


## `ivar agent sync`

Install, update and remove the systemd timers for agents/*.md


## `ivar agent status`

List agents with schedule, next run, last outcome and timer state


## `ivar agent run`

Run an agent now, through the same path its timer uses

| argument | required | description |
| --- | --- | --- |
| `name` | yes | Agent name (the file stem under agents/) |


## `ivar agent logs`

Print the output of the latest run, or of --run <id>

| argument | required | description |
| --- | --- | --- |
| `name` | yes | Agent name (the file stem under agents/) |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--run` | `<RUN>` |  | Run id to print instead of the latest run |


## `ivar agent feedback`

Record your verdict on a run

| argument | required | description |
| --- | --- | --- |
| `run_id` | yes | Run id to rate |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--rating` | `<RATING>` |  | Your verdict on the run |
| `--comment` | `<COMMENT>` |  | Why, in a sentence |


## `ivar agent notify`

Set up where agents send their run notifications


### `ivar agent notify telegram`

Notifications through a Telegram bot


#### `ivar agent notify telegram setup`

Save the bot token, find the chat, send a test message and set `notify` on agents

