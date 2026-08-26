# Command reference

Every verb `ivar` has, what it does, and what it takes.

The tables below — flags, arguments, defaults — are **generated from the binary's
own argument parser**, so they cannot drift from what your `ivar` actually
accepts. The prose around them is written by hand, because when to reach for a
verb is a judgement call and a generator has no opinion about it.

If you are looking for the shape of the tool rather than its surface, read
[Concepts](../concepts.md) first. This page is a lookup table, not a tour.

## How to read this

- **`ivar <verb>`** — the hall-level verbs. The hall is the root of the surface:
  `ivar sync`, not `ivar hall sync`.
- **`ivar <group> <verb>`** — `repo`, `feature`, `session`, `provider`, `plan`
  and `skill` group their verbs, because each has more of them than a flat root
  could carry.
- **`--json`** works on every verb and prints exactly the value the verb
  computed — the human text you see is a rendering of that same value, never a
  separately written summary. Script against `--json`.

## Exit codes

| code | meaning |
| --- | --- |
| `0` | done, with nothing needing attention |
| `1` | done, but something needs attention — the warnings are on stderr |
| `2` | refused before starting, or broke mid-flight |

Exit `1` is the one worth designing around: a verb that crosses eight repos and
hits a problem in one of them reports seven successes and a warning, and that is
a `1`, not a failure. Treating `1` as fatal in a script will make `ivar` look
broken when it is telling you something.

<!-- BEGIN GENERATED COMMANDS -->
<!-- Generated from clap by tests/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test docs_reference`. -->

### `ivar`

Mount the repos a feature spans into one directory, on one branch, for one agent session.

| flag | value | default | description |
| --- | --- | --- | --- |
| `--json` |  |  | Emit machine-readable output. Prints exactly the value the command computed. The human-readable text is a rendering of that same value, so the two can never tell you different things — script against this. |
| `--color` | `<COLOR>` | `auto` | When to colour output. `auto` follows `NO_COLOR`, then `FORCE_COLOR`, then whether the stream is a terminal — a pipe or a redirect gets none. `always` and `never` override all of that. Only labels are ever coloured; values never are, so `--json` is unaffected either way. |


#### `ivar init`

Create a hall: `ivar.json`, `.ivar/`, and the hall's `.gitignore` lines

| argument | required | description |
| --- | --- | --- |
| `path` | no | Directory to create the hall in. Defaults to the current directory |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--name` | `<NAME>` |  | The hall's name. Defaults to the target directory's name |
| `--provider` | `<PROVIDER>` |  | The provider to record as the hall's sole available (and default) provider. Defaults to `claude-code` |


#### `ivar sync`

Bring the local hall in line with `ivar.json`: clone missing repos, materialise harness config, run setup scripts

| flag | value | default | description |
| --- | --- | --- | --- |
| `--force-setup` |  |  | Run every repo's setup script even if it has already run for this version of the script. For when a script's effect was undone outside `ivar` — a deleted `node_modules`, a dropped database |


#### `ivar status`

Report hall health


#### `ivar doctor`

Diagnose problems and suggest fixes


#### `ivar cleanup`

Reconcile stale state (interactive; asks before deleting)


#### `ivar migrate`

Advance `ivar.json`'s schema version (interactive; shows the change, then asks). Only ever needed after upgrading `ivar` to a build whose format is newer than the one your hall was written with. Local state migrates itself; `ivar.json` is committed, so advancing it is a decision you make and then commit.


#### `ivar repo`

Manage repos


##### `ivar repo list`

List the repos in ivar.json and their state


##### `ivar repo add`

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


##### `ivar repo remove`

Remove a repo from ivar.json and tear down its files. Refuses while the repo is promoted in a feature or referenced by a live session; `--force` lifts both gates and cascades

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The repo's name, as declared in ivar.json |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--force` |  |  | Tear down even while the repo is promoted in a feature or referenced by a live session. Cascades: removes its worktrees, scrubs its promotion records, repairs view-dir symlinks, and regenerates the providers' config |


##### `ivar repo pull`

Refresh one or all repos' default branches from their remotes

| argument | required | description |
| --- | --- | --- |
| `repo` | no | The repo to fetch. Fetches every repo when omitted |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--diagnose` |  |  | When a repo cannot fast-forward, report the divergence in detail — the local and remote commits each side has. Read-only |
| `--resolve` |  |  | Automatically reconcile a diverged default branch when it is safe: reset it to the remote tip when every local commit is a duplicate of work already upstream (same patch-id). Never touches a branch with genuine local work, and implies `--diagnose` for the repos it cannot resolve |


##### `ivar repo setup`

Run the setup script for one repo

| argument | required | description |
| --- | --- | --- |
| `repo` | no | The repo whose setup script to run. Runs every repo's setup when omitted |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--force-setup` |  |  | Ignore the receipt and run the setup script even if unchanged |


##### `ivar repo upstream`

Manage remote upstream for a repo

| argument | required | description |
| --- | --- | --- |
| `repo` | yes | The repo to manage |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--url` | `<URL>` |  | The upstream remote URL to set (or remove with `--remove`) |
| `--remove` |  |  | Remove the upstream remote entirely |


#### `ivar feature`

Manage features


##### `ivar feature create`

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


##### `ivar feature list`

List features and how far each got


##### `ivar feature promote`

Promote a repo onto a feature's branch and materialise its worktree. A branch that already exists is adopted as-is; one that does not is created off the repo's effective base

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature to promote into |
| `repo` | yes | The repo to promote onto the feature's branch |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--base` | `<BASE>` |  | Override the branch a new worktree starts from, for this repo only. Defaults to the feature's declared base, or the repo's default branch |


##### `ivar feature demote`

Remove a repo from a feature. Its worktree stays on disk

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature to demote from |
| `repo` | yes | The repo to demote |


##### `ivar feature status`

Show one feature in detail: every promoted repo and its state, and — with `--recursive` — its whole subtree's health

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature to inspect |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--recursive` |  |  | Render the feature's whole subtree — itself and every descendant, in deterministic pre-order — with each feature's derived state, repos, and blockers |


##### `ivar feature integrate`

Integrate a child into its immediate parent, leaves first: each promoted repo's work lands on the parent's branch, durably and resumably. `--via`/`--strategy` override the resolved policy for the run; after the first receipt the policy is frozen

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The child feature to integrate |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--via` | `<VIA>` |  | The via override for this run: `pr` or `local`. Ignored once the first receipt froze the policy |
| `--strategy` | `<STRATEGY>` |  | The strategy override for this run: `squash`, `merge`, or `rebase`. Ignored once the first receipt froze the policy |


##### `ivar feature reparent`

Move a still-pristine child under a different parent, updating its parent and derived base in one record write. Refused once any promotion, plan, execution, session, receipt, close record, or descendant exists

| argument | required | description |
| --- | --- | --- |
| `child` | yes | The child feature to move |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--parent` | `<PARENT>` |  | The new parent feature. The child's `base` is rewritten to the new parent's branch in the same record write |


##### `ivar feature execute`

Manage a feature's Run Receipt lifecycle


###### `ivar feature execute start`

Start a new run, resume a blocked run, or restart a non-terminal run

| argument | required | description |
| --- | --- | --- |
| `feature` | yes |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--plan` | `<PLAN>` |  |  |
| `--resume` |  |  |  |
| `--restart` |  |  |  |


###### `ivar feature execute finish`

Record a coordinator's structured completion report

| argument | required | description |
| --- | --- | --- |
| `feature` | yes |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--plan` | `<PLAN>` |  |  |
| `--report-json` | `<REPORT_JSON>` |  |  |
| `--outcome` | `<OUTCOME>` |  |  |


###### `ivar feature execute status`

Show the current receipt, a receipt by id, or complete history

| argument | required | description |
| --- | --- | --- |
| `feature` | yes |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--history` |  |  |  |
| `--run` | `<RUN>` |  |  |


###### `ivar feature execute accept-revision`

Accept an approved plan revision for a diverged run

| argument | required | description |
| --- | --- | --- |
| `feature` | yes |  |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--plan` | `<PLAN>` |  |  |


##### `ivar feature deliver`

Preview, then push, a feature's promoted repos. `--preview` prints the side-effect-free summary (with its fingerprint) and pushes nothing; applying with `--fingerprint` is refused if the state has drifted

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The feature to deliver |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--preview` |  |  | Print the delivery preview and push nothing |
| `--fingerprint` | `<FINGERPRINT>` |  | The fingerprint from the preview the human approved; required to apply. Apply recomputes the preview and refuses when the fingerprint differs — the state has drifted since the preview |


##### `ivar feature close`

Close a feature: stop its executor sessions, remove its execution state, and record the outcome on plan.md's frontmatter. Idempotent — closing an already-closed feature is a no-op

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The feature to close |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--outcome` | `<OUTCOME>` |  | How the feature ended: `delivered` or `abandoned` |


##### `ivar feature delete`

Delete a feature: its worktrees, its directory under `.ivar/`, and its plans. Refuses if anything under the feature directory is not removable, and preserves the feature record for retry if a teardown step fails

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The feature to delete |


##### `ivar feature rebase`

Rebase every promoted repo's worktree onto its effective base. A dirty worktree is skipped; a conflict is aborted and reported

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The feature to rebase |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--onto` | `<ONTO>` |  | Collapse the base: rebase every promoted repo onto this branch, and record it as the declared base for each repo that lands there. The verb for once a feature's own base has landed |


##### `ivar feature review`

Write a VSCode workspace opening the feature: promoted repos on the feature branch, everyone else on their default branch

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The feature to open |


##### `ivar feature view`

Open an interactive multi-shell view over the feature's promoted repos — one shell per repo, each running in its worktree

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The feature to view |


##### `ivar feature prune`

Delete features whose branches have been merged into their default branches


#### `ivar session`

Manage sessions


##### `ivar session start`

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


##### `ivar session connect`

Re-bind to an existing live session: locate it, re-materialise its view dir, and emit the binding as `IVAR_*` env vars

| argument | required | description |
| --- | --- | --- |
| `session_id` | no | The session id, or a unique prefix of one |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--feature` | `<FEATURE>` |  | Narrow the search to sessions bound to this feature |


##### `ivar session convert`

Bind a discovery session to a feature (one-way), moving its view dir into the feature's session tree

| argument | required | description |
| --- | --- | --- |
| `session_id` | yes | The discovery session's id, or a unique prefix of one |
| `feature` | yes | The feature to bind the session to. Must already exist |


##### `ivar session stop`

Stop a session — tear down its view dir and end any running harness. Omitting the session stops *every* session in the hall

| argument | required | description |
| --- | --- | --- |
| `session` | no | The session to stop — its id, or a unique prefix of one. Omitting it stops **every** session in the hall: every discovery session and every feature's sessions, not just this feature's and not just the most recent. Pass `$IVAR_SESSION_ID` to stop only your own. |


##### `ivar session prune`

Remove dead sessions: view dirs that exist but hold no readable `state.json`. A session with a readable record is never touched


##### `ivar session relay`

Relay session info: four-line output contract for external consumers

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature to relay a session for |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | The provider to relay to. Required — relay must switch providers |


#### `ivar provider`

Manage providers


##### `ivar provider list`

List the hall's providers and the default one


##### `ivar provider add`

Register a new provider by name

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The provider's name (e.g. `claude-code`, `opencode`) |


#### `ivar plan`

Manage SPDD plans


##### `ivar plan create`

Scaffold a feature's SPDD artifacts (requirements, analysis, plan), or only the ones named. With a subset, writes what is missing and leaves what is already there untouched

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature to scaffold plans for |
| `artifacts` | no | Which artifacts to scaffold (`requirements`, `analysis`, `plan`); scaffolds all three when omitted |


##### `ivar plan list`

List which features have plans, and how complete


##### `ivar plan show`

Print one feature's SPDD artifact

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature whose artifact to show |
| `artifact` | yes | Which artifact: `requirements`, `analysis`, or `plan` |


##### `ivar plan approve`

Approve one of a feature's SPDD gates: requirements, analysis, plan. Requires every gate upstream of it to be either approved or never written — an artifact that exists still has to be approved, even though an absent one is skipped — and records a fingerprint of the artifact's content

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature whose gate to approve |
| `gate` | yes | The gate: `requirements`, `analysis`, or `plan` |


##### `ivar plan invalidate`

Declare a revision of an approved gate, marking it — and every gate downstream — as needing revision

| argument | required | description |
| --- | --- | --- |
| `feature` | yes | The feature whose gate to invalidate |
| `gate` | yes | The gate: `requirements`, `analysis`, or `plan` |


##### `ivar plan status`

Show approval gate status for a plan file. Omits a gate that has no artifact and was never approved; a gate that was approved and whose artifact then vanished is still shown, as needs-revision

| argument | required | description |
| --- | --- | --- |
| `plan_path` | yes | Path to the plan file (plan.md or similar) |


#### `ivar skill`

Manage skills


##### `ivar skill list`

List the skills in the hall's shared skills directory


##### `ivar skill create`

Scaffold a new skill: a folder with a SKILL.md

| argument | required | description |
| --- | --- | --- |
| `id` | yes | The skill's id — one path segment, unique within the skills dir |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--description` | `<DESCRIPTION>` |  | The skill's description, for the SKILL.md frontmatter |


##### `ivar skill add`

Install an external skill from a git repo

| argument | required | description |
| --- | --- | --- |
| `repo` | yes | The git repo URL or path to install the skill from |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--path` | `<PATH>` |  | A sub-path inside the repo that holds the skill folder |
| `--ref` | `<REF>` |  | A git ref (branch, tag, or sha) to pin the skill to |


##### `ivar skill update`

Update external skills to their tracked ref

| argument | required | description |
| --- | --- | --- |
| `skills` | no | Which external skills to update; updates all when omitted |


##### `ivar skill remove`

Remove a skill from the hall's shared skills directory

| argument | required | description |
| --- | --- | --- |
| `skill` | yes | The skill's id to remove |


##### `ivar skill detach`

Convert an external skill into an authored (local) skill

| argument | required | description |
| --- | --- | --- |
| `skill` | yes | The external skill's id to convert into an authored skill |


##### `ivar skill sync`

Materialise hall skills to native targets for other tools


##### `ivar skill status`

Show skill installation state — which are external, authored, or stale


##### `ivar skill doctor`

Health diagnostics for skills: find broken links, missing refs, and suggest fix_actions


#### `ivar mcp`

Authenticate the hall's declared MCP servers


##### `ivar mcp auth`

Authenticate one MCP server. Resolves the server from `ivar.json`'s `mcp` array and the provider from the hall's default, `--provider`, or — with `--all-providers` — every provider the hall lists, run one at a time; where a provider's own dynamic client registration is known to be rejected by the server (Figma on OpenCode, today), pre-registers a client first for that provider — a registration is not an authentication, and is reported separately, per provider. Then hands off to each provider's own login command (`claude mcp login <name>` or `opencode mcp auth <name>`), which owns the terminal from that point on: it prints a URL and waits on a browser. With `--all-providers`, every provider is attempted even after an earlier one fails, and the command reports which succeeded and which failed rather than stopping at the first problem

| argument | required | description |
| --- | --- | --- |
| `server` | yes | The server's name, as declared in `ivar.json`'s `mcp` array |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | The provider to authenticate against. Defaults to the hall's default provider. Conflicts with `--all-providers` |
| `--all-providers` |  |  | Authenticate every provider the hall lists (`providers.available`), one at a time — never concurrently, since each provider's login command takes over the terminal and waits on a browser. Every provider is attempted even if an earlier one fails; the run is reported as needing attention (not a clean success) the moment any of them does. Conflicts with `--provider` |

<!-- END GENERATED COMMANDS -->

## Notes the generator cannot give you

**`ivar migrate` is not part of a normal week.** It exists for one situation:
you upgraded `ivar`, the new build's `ivar.json` format is newer than the one
your hall was written with, and something refused to write until a human agreed
to advance it. See [On-disk format](on-disk-format.md).

**`ivar cleanup` and `ivar migrate` both ask, and both refuse to act when nobody
is there to answer.** Run either with output piped and they print what they
*would* do and change nothing. That is deliberate: neither deletion nor a
rewrite of a committed file should be reachable by a script that nobody is
watching. There is no `--yes`.

**`ivar session start` is the one verb that takes over your terminal.** It opens
a TUI. Everything else prints and exits.

**`ivar feature execute …` manages the Run Receipt lifecycle for an approved
plan.** The provider coordinates the work; Ivar records the execution boundary.
Read [Planning and execution](../guides/planning-and-execution.md) before using
it directly.
