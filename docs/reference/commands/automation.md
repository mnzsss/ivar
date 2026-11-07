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


## `ivar automation decide`

Record the answer to a proposal to automate a candidate: `--accepted` (the automation exists as `--artifact`), `--declined` (do not propose it again while it keeps recurring) or `--deferred` (not before `--until`, 30 days by default). The group is the one a scan reported as `group_ref`; its current texts are stored, so the answer survives the group gaining new ones. `--same-as` adds the group to an existing declined or deferred decision instead

| argument | required | description |
| --- | --- | --- |
| `group_ref` | yes | The `group_ref` of a candidate: the id of one of its request fingerprints |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--accepted` |  |  | The automation exists. Needs `--intent`, `--form` and `--artifact` |
| `--declined` |  |  | Do not propose this request again while it keeps recurring. Needs `--intent` |
| `--deferred` |  |  | Do not propose this request before `--until`. Needs `--intent` |
| `--same-as` | `<DECISION_ID>` |  | Add this group's texts to an existing declined or deferred decision, which keeps its kind and `until` |
| `--intent` | `<INTENT>` |  | A short name for what the request is |
| `--form` | `<FORM>` |  | What the request became |
| `--artifact` | `<ARTIFACT>` |  | The automation's path, relative to the hall |
| `--until` | `<UNTIL>` |  | The last day of the deferral, `YYYY-MM-DD`. Defaults to 30 days from now |


## `ivar automation renew`

Mark an accepted decision as still wanted: rewrites its `updated_at`

| argument | required | description |
| --- | --- | --- |
| `decision_id` | yes | The decision's id, as `ivar automation decisions` lists it |


## `ivar automation decisions`

List the automation decisions. Read-only


## `ivar automation undo`

Delete a decision: its group is a candidate again once it recurs enough

| argument | required | description |
| --- | --- | --- |
| `decision_id` | yes | The decision's id, as `ivar automation decisions` lists it |

