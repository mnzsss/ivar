<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar doctor`

Diagnose problems and suggest fixes. Each orphan worktree is classified: safe to remove (clean, nothing unpushed, its work already on the default branch) or needing a look.

| flag | value | default | description |
| --- | --- | --- | --- |
| `--github` |  |  | Ask GitHub, through `gh`, whether each orphan branch's pull request merged or closed. A network call, so only on request. Without `gh`, or offline, the classification falls back to comparing the branch's files with the default branch locally. |
| `--fix` |  |  | Remove the orphan worktrees classified safe, and only those. Never touches a worktree with uncommitted changes, with commits not pushed to its upstream, or whose work is not already integrated. |

