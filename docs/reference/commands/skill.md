<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar skill`

Manage skills


## `ivar skill list`

List the skills in the hall's shared skills directory


## `ivar skill create`

Scaffold a new skill: a folder with a SKILL.md

| argument | required | description |
| --- | --- | --- |
| `id` | yes | The skill's id — one path segment, unique across both skills roots |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--description` | `<DESCRIPTION>` |  | The skill's description, for the SKILL.md frontmatter |
| `--hall` |  |  | Create it in the hall's committed skills directory, shared with everyone who clones the hall. Without this it stays personal to you |


## `ivar skill add`

Install an external skill from a git repo

| argument | required | description |
| --- | --- | --- |
| `repo` | yes | The skill source: owner/repo, https://github.com/owner/repo, or https://github.com/owner/repo/tree/<ref>/<path> |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--path` | `<PATH>` |  | A sub-path inside the repo that holds the skill folder |
| `--ref` | `<REF>` |  | A git ref (branch, tag, or sha) to pin the skill to |
| `--hall` |  |  | Install into the hall's committed skills directory, shared with everyone who clones the hall. Without this it stays personal to you |


## `ivar skill update`

Update external skills to their tracked ref

| argument | required | description |
| --- | --- | --- |
| `skills` | no | Which external skills to update; updates all when omitted |


## `ivar skill remove`

Remove a skill from the hall's shared skills directory

| argument | required | description |
| --- | --- | --- |
| `skill` | yes | The skill's id to remove |


## `ivar skill detach`

Convert an external skill into an authored (local) skill

| argument | required | description |
| --- | --- | --- |
| `skill` | yes | The external skill's id to convert into an authored skill |


## `ivar skill enable`

Turn a skill back on and materialise it for the hall's providers

| argument | required | description |
| --- | --- | --- |
| `skill` | yes | The skill's id, from either skills root |


## `ivar skill disable`

Turn a skill off without deleting it: its provider targets are removed and `ivar skill sync` leaves it out until it is enabled again. The choice is personal (`.ivar/skills-local/disabled.json`)

| argument | required | description |
| --- | --- | --- |
| `skill` | yes | The skill's id, from either skills root |


## `ivar skill sync`

Materialise hall skills for the hall's providers and remove the ones left behind by providers it no longer lists. When something changed, `.ivar/skills/state.json` and the plan's destinations are committed as `chore(skills): Sync hall skills`, then every other pending hall change as `chore(hall): Sync pending hall changes`, and the branch is pushed; `.ivar/skills-local/` is never committed. `auto_commit: false` in `ivar.json` turns all of that off and `auto_push: false` only the push. A hall sync that cannot finish is a warning and never changes the exit code


## `ivar skill status`

Show skill installation state — which are external, authored, or stale


## `ivar skill doctor`

Health diagnostics for skills: find broken links, missing refs, and suggest fix_actions

