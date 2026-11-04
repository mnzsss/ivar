<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar review`

Review a feature's local changes


## `ivar review comment`

Manage line-range review comments


### `ivar review comment add`

Add a comment on a line range of a file in one of the feature's repos

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature under review |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | The repo the file belongs to |
| `--file` | `<FILE>` |  | The file path, relative to the repo root |
| `--lines` | `<LINES>` |  | `<n>` or `<n>-<m>`, 1-based and inclusive |
| `--body` | `<BODY>` |  | The comment text |


### `ivar review comment list`

List a feature's review comments

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature under review |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--repo` | `<REPO>` |  | Only comments on this repo |
| `--status` | `<STATUS>` |  | Only comments with this status |


### `ivar review comment resolve`

Mark a review comment resolved

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature under review |
| `id` | yes | The comment id, e.g. `c1` |

