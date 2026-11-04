<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar repo`

Manage repos


## `ivar repo list`

List the repos in ivar.json and their state


## `ivar repo add`

Declare a repo in ivar.json, clone it bare, and materialise its default-branch worktree

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The repo's name — one path segment, unique within the hall |
| `url` | yes | The git remote URL to clone from |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--default-branch` | `<DEFAULT_BRANCH>` |  | The branch a fresh worktree defaults to. Defaults to `main` |
| `--reuse` |  |  | Reuse a bare clone already present at the expected path |
| `--fresh` |  |  | Delete an existing bare clone (and its worktree) and clone anew |


## `ivar repo create`

Create a brand-new repo — stored in the hall's origin (`--local`) or on GitHub (`--remote`) — and declare it in ivar.json

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The repo's name — one path segment, unique within the hall |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--local` |  |  | Store the repo under `refs/heads/repos/<name>/` in the hall's origin |
| `--remote` |  |  | Create the repo on GitHub with `gh` |
| `--public` |  |  | Make the GitHub repo public. Only with `--remote` |
| `--default-branch` | `<DEFAULT_BRANCH>` |  | The branch the initial commit lands on. Defaults to `main` |


## `ivar repo remove`

Remove a repo from ivar.json and tear down its files. Refuses while the repo is promoted in a feature or referenced by a live session; `--force` lifts both gates and cascades

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The repo's name, as declared in ivar.json |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--force` |  |  | Tear down even while the repo is promoted in a feature or referenced by a live session. Cascades: removes its worktrees, scrubs its promotion records, repairs view-dir symlinks, and regenerates the providers' config |


## `ivar repo pull`

Refresh one or all repos' default branches from their remotes

| argument | required | description |
| --- | --- | --- |
| `repo` | no | The repo to fetch. Fetches every repo when omitted |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--diagnose` |  |  | When a repo cannot fast-forward, report the divergence in detail — the local and remote commits each side has. Read-only |
| `--resolve` |  |  | Automatically reconcile a diverged default branch when it is safe: reset it to the remote tip when every local commit is a duplicate of work already upstream (same patch-id). Never touches a branch with genuine local work, and implies `--diagnose` for the repos it cannot resolve |


## `ivar repo setup`

Run the setup script for one repo

| argument | required | description |
| --- | --- | --- |
| `repo` | no | The repo whose setup script to run. Runs every repo's setup when omitted |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--force-setup` |  |  | Ignore the receipt and run the setup script even if unchanged |


## `ivar repo view`

Open an interactive multi-shell view over the hall's repos — one shell per repo, each running in its default-branch worktree. An inspection view: those worktrees are read-only while a session guards them, and `ivar repo pull` is what refreshes them

| argument | required | description |
| --- | --- | --- |
| `repos` | no | Which declared repos to open; opens every declared repo when omitted |


## `ivar repo upstream`

Manage remote upstream for a repo

| argument | required | description |
| --- | --- | --- |
| `repo` | yes | The repo to manage |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--url` | `<URL>` |  | The upstream remote URL to set (or remove with `--remove`) |
| `--remove` |  |  | Remove the upstream remote entirely |

