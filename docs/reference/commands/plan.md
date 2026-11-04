<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar plan`

Manage SPDD plans


## `ivar plan create`

Scaffold a feature's SPDD artifacts (requirements, analysis, plan), or only the ones named. With a subset, writes what is missing and leaves what is already there untouched

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to scaffold plans for |
| `artifacts` | no | Which artifacts to scaffold (`requirements`, `analysis`, `plan`); scaffolds all three when omitted |


## `ivar plan list`

List which features have plans, and how complete


## `ivar plan show`

Print one feature's SPDD artifact

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature whose artifact to show |
| `artifact` | yes | Which artifact: `requirements`, `analysis`, or `plan` |


## `ivar plan approve`

Approve one of a feature's SPDD gates: requirements, analysis, plan. Requires every gate upstream of it to be either approved or never written — an artifact that exists still has to be approved, even though an absent one is skipped — and records a fingerprint of the artifact's content

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature whose gate to approve |
| `gate` | yes | The gate: `requirements`, `analysis`, or `plan` |


## `ivar plan invalidate`

Declare a revision of an approved gate, marking it — and every gate downstream — as needing revision

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature whose gate to invalidate |
| `gate` | yes | The gate: `requirements`, `analysis`, or `plan` |


## `ivar plan status`

Show approval gate status for a plan file. Omits a gate that has no artifact and was never approved; a gate that was approved and whose artifact then vanished is still shown, as needs-revision

| argument | required | description |
| --- | --- | --- |
| `target` | no | The feature, or a path to a file under its feature directory (e.g. its plan.md) |

