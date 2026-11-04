<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar provider`

Manage providers


## `ivar provider list`

List the hall's providers and the default one


## `ivar provider add`

Register a new provider by name and materialise its config and skills

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The provider's name (e.g. `claude-code`, `opencode`, `omp`, `t3`) |


## `ivar provider remove`

Unregister a provider and tear down its hall config and skills

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The provider's name (e.g. `claude-code`, `opencode`, `omp`, `t3`) |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--default` | `<DEFAULT>` |  | The provider to make the default. Required when removing the current default |


## `ivar provider default`

Make a registered provider the hall's default — the one `ivar session start` and `ivar mcp auth` use when none is named. Sessions already running keep their provider

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The provider's name (e.g. `claude-code`, `opencode`, `omp`, `t3`) |

