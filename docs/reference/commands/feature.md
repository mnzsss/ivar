<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar feature`

Manage features


## `ivar feature create`

Create a feature: name, branch, no repos promoted yet. A subfeature is created with `--parent <feature>`, which derives its base from the parent's branch; `--via`/`--strategy` persist the feature's own integration-policy override

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The feature's name — one path segment, unique within the hall |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--branch` | `<BRANCH>` |  | The branch to work on. Defaults to the feature's name. Use it to adopt a branch a feature name cannot spell, such as `feat/login` |
| `--base` | `<BASE>` |  | The branch new promotions should start from, per repo. Defaults to each repo's own default branch. Conflicts with `--parent`: a child's base is always derived from its immediate parent's branch |
| `--parent` | `<PARENT>` |  | The parent feature this subfeature integrates into. Conflicts with `--base`: the child's base is derived from the parent's branch |
| `--via` | `<VIA>` |  | This feature's integration via override: `pr` or `local`. Omitted, the hall default (or the embedded `local`) applies. Persisted at creation; there is no policy-configure command |
| `--strategy` | `<STRATEGY>` |  | This feature's integration strategy override: `squash`, `merge`, or `rebase`. Omitted, the hall default (or the embedded `squash`) applies. Persisted at creation |


## `ivar feature list`

List features and how far each got


## `ivar feature promote`

Promote a repo onto a feature's branch and materialise its worktree. A branch that already exists is adopted as-is; one that does not is created off the repo's effective base

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to promote into |
| `repo` | yes | The repo to promote onto the feature's branch |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--base` | `<BASE>` |  | Override the branch a new worktree starts from, for this repo only. Defaults to the feature's declared base, or the repo's default branch |


## `ivar feature demote`

Remove a repo from a feature. Its worktree stays on disk unless `--remove-worktree` is passed

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to demote from |
| `repo` | yes | The repo to demote |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--remove-worktree` |  |  | Also remove the repo's worktree and local branch. Refuses when the worktree is dirty, the branch has commits beyond its base, or it is published on the remote |


## `ivar feature status`

Show one feature in detail: every promoted repo and its state, and — with `--recursive` — its whole subtree's health

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to inspect |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--recursive` |  |  | Render the feature's whole subtree — itself and every descendant, in deterministic pre-order — with each feature's derived state, repos, and blockers |


## `ivar feature integrate`

Integrate a child into its immediate parent, leaves first: each promoted repo's work lands on the parent's branch, durably and resumably. `--via`/`--strategy` override the resolved policy for the run; after the first receipt the policy is frozen

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The child feature to integrate |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--via` | `<VIA>` |  | The via override for this run: `pr` or `local`. Ignored once the first receipt froze the policy |
| `--strategy` | `<STRATEGY>` |  | The strategy override for this run: `squash`, `merge`, or `rebase`. Ignored once the first receipt froze the policy |
| `--name` | `<NAME>` |  | The integration title, e.g. `feat: add checkout tax`: the squash or merge commit message on the parent, and with `--via pr` the PR title and merge subject. Defaults to `feat: integrate <child>` |


## `ivar feature reparent`

Move a still-pristine child under a different parent, updating its parent and derived base in one record write. Refused once any promotion, plan, execution, session, receipt, close record, or descendant exists

| argument | required | description |
| --- | --- | --- |
| `child` | no | The child feature to move |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--parent` | `<PARENT>` |  | The new parent feature. The child's `base` is rewritten to the new parent's branch in the same record write |


## `ivar feature rename`

Rename a feature, its branch, or both — the one allowed identity transition. Every promoted repo's local branch and worktree, its remote branch when published, direct children, live sessions, and the feature directory all move together, durably and resumably: a mid-flight failure automatically reverses whatever already landed, and an interruption resumes on the next `ivar feature rename` invocation naming the same feature

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to rename |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--name` | `<NAME>` |  | The feature's new name. Requires at least one of `--name`/`--branch` to differ from the current value |
| `--branch` | `<BRANCH>` |  | The feature's new branch. Requires at least one of `--name`/`--branch` to differ from the current value |


## `ivar feature execute`

Manage a feature's Run Receipt lifecycle


### `ivar feature execute start`

Start a new run, resume a blocked run, or restart a non-terminal run

| argument | required | description |
| --- | --- | --- |
| `feature` | no |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--plan` | `<PLAN>` |  | Plan file; defaults to `.ivar/features/<feature>/plan.md` |
| `--resume` |  |  |  |
| `--restart` |  |  |  |
| `--mode` | `<MODE>` |  | Execution mode: `default` keeps every human gate; `goal` runs to the delivery gate without stopping. Omitted on `--resume`, the run keeps its recorded mode; omitted otherwise, a new run is `default` |


### `ivar feature execute finish`

Record a coordinator's structured completion report (see `--print-schema`)

| argument | required | description |
| --- | --- | --- |
| `feature` | no |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--plan` | `<PLAN>` |  | Plan file; defaults to `.ivar/features/<feature>/plan.md` |
| `--report-json` | `<REPORT_JSON>` |  | Path to the coordinator report JSON. Run with `--print-schema` for its shape |
| `--outcome` | `<OUTCOME>` |  | How the run ended: succeeded, failed or blocked |
| `--print-schema` |  |  | Print the coordinator report JSON schema and the accepted `--outcome` values, then exit |


### `ivar feature execute status`

Show the current or most recent receipt, a receipt by id, or complete history

| argument | required | description |
| --- | --- | --- |
| `feature` | no |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--plan` | `<PLAN>` |  | Plan file; defaults to `.ivar/features/<feature>/plan.md` |
| `--history` |  |  | Show every receipt of the feature, newest first |
| `--run` | `<RUN>` |  | Show the receipt with this run id |


### `ivar feature execute accept-revision`

Accept an approved plan revision for a diverged run

| argument | required | description |
| --- | --- | --- |
| `feature` | no |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--plan` | `<PLAN>` |  | Plan file; defaults to `.ivar/features/<feature>/plan.md` |


### `ivar feature execute checkpoint`

Record an approved wave on the active run without editing the plan

| argument | required | description |
| --- | --- | --- |
| `feature` | no |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--wave` | `<WAVE>` |  | The wave number exactly as `plan.md` numbers it (a plan may start at Wave 0 or Wave 1) |
| `--summary` | `<SUMMARY>` |  | Completed tasks, satisfied exit criteria, and deferred validation failures |


### `ivar feature execute interrupt`

Abandon an active or blocked run, transitioning it to interrupted

| argument | required | description |
| --- | --- | --- |
| `feature` | no |  |


## `ivar feature deliver`

Preview, then push, a feature's promoted repos. `--preview` prints the side-effect-free summary (with its fingerprint) and pushes nothing; applying with `--fingerprint` is refused if the state has drifted

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to deliver. |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--preview` |  |  | Print the delivery preview and push nothing. |
| `--land` |  |  | Land feature branches into default branches (fast-forward only), then push each default branch to its remote. |
| `--fingerprint` | `<FINGERPRINT>` |  | The fingerprint from the preview the human approved; required to apply. It covers `--name`, `--body`, `--draft` and `--only`, so apply with the same values the preview used. Apply recomputes the preview and refuses when the fingerprint differs — the state has drifted since the preview. |
| `--name` | `<TITLE>` |  | Pull request title. If placed before any `--repo`, applies globally; if placed after a `--repo`, applies to that repo. Part of the delivery fingerprint: pass the same value to the preview and the apply. |
| `--body` | `<BODY>` |  | Pull request body text, or a path to a `.md` / `.txt` file — either `./relative` or absolute. If placed before any `--repo`, applies globally; if placed after a `--repo`, applies to that repo. Part of the delivery fingerprint: pass the same value to the preview and the apply. |
| `--repo` | `<REPO>` |  | Scope following `--name`, `--body`, and `--draft` flags to this promoted repository. Does not select which repositories are delivered; use `--only` for that. |
| `--only` | `<REPO>` |  | Deliver only this promoted repository; repeat to select several. Without it every promoted repository is delivered. Part of the delivery fingerprint: pass the same values to the preview and the apply. |
| `--draft` |  |  | Create or convert a pull request to a draft. If placed before any `--repo`, applies globally to all repos; if placed after a `--repo`, applies only to that repo. Incompatible with `--land`. |
| `--skip-memory-review` | `<REASON>` |  | Apply without the memory review the delivery requires, and record `memory.review_skipped` with REASON. Apply only: it lifts no other blocker and is not part of the fingerprint. |


## `ivar feature close`

Close a feature: stop its executor sessions, remove its execution state, and record the outcome on plan.md's frontmatter. Idempotent — closing an already-closed feature is a no-op

| argument | required | description |
| --- | --- | --- |
| `name` | no | The feature to close |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--outcome` | `<OUTCOME>` |  | How the feature ended: `delivered` or `abandoned` |


## `ivar feature delete`

Delete a feature: its worktrees, its directory under `.ivar/`, and its plans. Refuses if anything under the feature directory is not removable, and preserves the feature record for retry if a teardown step fails

| argument | required | description |
| --- | --- | --- |
| `name` | no | The feature to delete |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--force` |  |  | Delete even with a live session or uncommitted or untracked changes in a promoted worktree, discarding them |
| `--yes` |  |  | Consent to deleting the feature together with every subfeature, without asking |


## `ivar feature rebase`

Rebase promoted repos' worktrees onto the remote tip of their base (`--offline`: the local ref); `--repo` selects repos. A dirty worktree is skipped; a conflict is aborted and reported

| argument | required | description |
| --- | --- | --- |
| `name` | no | The feature to rebase |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--onto` | `<ONTO>` |  | Collapse the base: rebase every selected repo onto this branch, and record it as the declared base for each repo that lands there. The verb for once a feature's own base has landed |
| `--repo` | `<REPO>` |  | Rebase only this promoted repo; repeat to select several. Without it every promoted repo is rebased |
| `--offline` |  |  | Make no network call: rebase onto the local base ref instead of the remote tip `deliver` checks |


## `ivar feature view`

Open an interactive multi-shell view over the feature's promoted repos — one shell per repo, each running in its worktree

| argument | required | description |
| --- | --- | --- |
| `name` | no | The feature to view |


## `ivar feature prune`

Delete features whose branches have been merged into their default branches


## `ivar feature cleanup`

Authorize and perform a feature's local teardown

| argument | required | description |
| --- | --- | --- |
| `name` | no | The feature to clean up |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--preview` |  |  | Preview only: compute and print the summary, teardown nothing |
| `--record` | `<RECORD>` |  | Path to the approved cleanup record; required to perform teardown |


## `ivar feature workspace`

Generate a multi-root VSCode workspace (.code-workspace) for a feature, opening promoted repos writable and every context repo read-only

| argument | required | description |
| --- | --- | --- |
| `feature` | no | The feature to generate a workspace for |
| `repos` | no | Which declared repos to include; includes all when omitted |

