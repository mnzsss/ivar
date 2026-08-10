# Glossary

Every term `ivar` uses, in one place.

The five that carry the model — **hall, repo, feature, promote, session** — are
explained properly in [Concepts](concepts.md), with a diagram. They are repeated
here in one line each so this page stands alone, but if you are meeting them for
the first time, read Concepts instead.

## The model

**Hall** — a directory that owns a set of registered repos, features and
sessions, defined by a committed `ivar.json`. One per scope of work: a team, a
product, an initiative. A hall is itself a git repo, which is how a team shares
one.

**Repo** — a git repository registered in `ivar.json` by name, URL and default
branch. Cloned bare into `.ivar/repos/<name>/.bare/`; every checkout, including
the default branch, is a worktree off that bare clone.

**Feature** — a unit of work spanning one or more repos, bound to a branch. The
branch is the feature's name. Survives sessions, agents and reboots, because it
is a file plus branches rather than a conversation.

**Promote** — making a repo writable for a feature: refresh its default branch,
cut a worktree on the feature's branch, run its setup script the first time, and
repoint the view dir. Idempotent. Feature-scoped and shared — every session bound
to that feature writes the same worktree.

**Demote** — the inverse. The repo leaves the feature's promotion list; its
worktree stays on disk, so nothing is lost.

**Session** — a view dir with an agent running in it, optionally bound to one
feature. Cannot switch features mid-session; open another instead. Multiple
sessions may bind the same feature at once and share its worktrees. **Live** while
its view dir exists — liveness is not a process.

**View dir** — the per-session directory of symlinks, one per repo, pointing at
the right worktree: the feature branch for promoted repos, the shared read-only
default branch for the rest. At
`.ivar/features/<name>/sessions/<uuid>/` for feature sessions,
`.ivar/sessions/<uuid>/` for discovery sessions.

**Provider** — the agent harness that runs inside a session: Claude Code or
OpenCode. Chosen at `ivar init`, added later with `ivar provider add`, selected
per session with `--provider`. Each discovers config differently, so hall config
and view dir contents are generated per provider.

## Sessions

**Discovery session** — a session with no bound feature. Every repo is symlinked
to its default branch, read-only, and promotion is disabled. For exploring and
cross-repo reading. May convert exactly once into a feature session.

**Session conversion** — one-way binding of a discovery session to a feature.
Preserves the session's id, provider and start time, and moves its view dir from
`.ivar/sessions/<id>` into the feature's tree with symlinks rebuilt. Resumable if
interrupted. Never called *promote* — that word is reserved for making a repo
writable.

**Session relay** — starting a fresh session on an existing feature under a
*different* provider, because the previous one ended before the work did
(typically token exhaustion). **A relay passes the work, never the thread**: the
branch, worktrees and plan live on disk, the conversation does not. The opposite
axis to conversion — a conversion keeps the provider and changes the binding; a
relay keeps the binding and changes the provider.

**Connect** — re-binding your shell or agent to an existing live session without
creating one. Finds it by id-prefix and/or feature, re-materialises its view dir
(idempotent, and it repairs symlinks and read-only guards left stale by a promote
elsewhere), and emits `IVAR_SESSION_ID`, `IVAR_FEATURE` and `IVAR_SESSION_PATH`.

**Detached session** — one created without launching a provider, so an
already-running agent can bind to it. Persists until an explicit `session stop`,
unlike a session whose provider `ivar` launched, which ends when that provider
exits.

## Repos and refreshing

**Smart fetch** — the default-branch refresh at session start. Every registered
repo, promoted or not, is fetched and fast-forwarded. The "smart" is the safety
envelope, not a skip list: default branch only, fast-forward only, best-effort per
repo. It never touches a feature worktree and never moves a merge-base.

**Pull** — the same fetch-and-fast-forward, on demand and in bulk, across the
hall. Needs no live session. Best-effort: an unreachable remote is reported and
skipped, never aborting the batch.

**Rebase** — rewriting a feature's promoted worktrees onto the current tip of
their default branches. Skips dirty worktrees (no autostash, so uncommitted work
is safe), and on conflict aborts that repo and continues.

**Deregister** — dropping a repo from `ivar.json` and tearing down its whole
`.ivar/repos/<name>/` tree, including the feature worktrees of any feature that
promoted it. Refuses while the repo is promoted or referenced by a live session,
naming the blockers; `--force` lifts both gates and cascades.

**Setup script** — a per-repo bootstrap script at `.ivar/setups/<repo>.sh`,
committed to the hall. It prepares a worktree for work: dependencies, env files,
anything git does not carry (a worktree shares history but not untracked files).
Discovered by file presence, not by `ivar.json`. Runs during `sync` and on a
repo's first promote, gated by a receipt owned by that physical worktree — so it
runs once, and re-runs when the script's own content changes. A failure is
reported, never silently swallowed, and never aborts the other repos.

**Session hook** — a per-repo script at `.ivar/setups/<repo>.session.sh`,
committed to the hall, run on every `session start` in each promoted repo's
worktree. The **Setup script**'s sibling, and the difference is lifetime: the
setup script prepares a worktree once and is receipt-gated, while a hook runs
every session and is not. Per-session daemon state — a database, a compose
project sibling sessions must not share — belongs here. A failure warns; the
session still opens.

**Secrets dir** — `.ivar/secrets/`, handed to setup scripts and session hooks as
`IVAR_SECRETS_DIR`. Created by `sync`, never written to by `ivar`, and gitignored
by the same `.ivar/*` rule that covers the rest of local state. `ivar` stores no
secrets; it points at a directory you maintain.

## Planning

**Requirements · Analysis · Plan** — the three SPDD artifacts, committed under
`plans/<feature>/`. Requirements: what must be true. Analysis: what the code
actually looks like, and the trade-offs. Plan: the design and the concrete
operations that implement it.

**REASONS canvas** — the structure `plan.md` is written in: Requirements,
Entities, Approach, Structure, Operations, Norms, Safeguards. The design sections
reference the standing sources and record only this feature's delta rather than
restating them. *Canvas* names the format, not the artifact.

**Approval gate** — an explicit human decision that moves a feature through the
lifecycle. Four exist: requirements, analysis, plan, execution graph. Each
refuses until the one upstream is approved, and approving records a fingerprint of
the artifact — so editing an approved artifact invalidates it and cascades
downstream. The execution-graph gate is crossed by `ivar feature execute approve`,
not `plan approve`.

**Feature execution board** — persistent coordination state under
`.ivar/features/<feature>/execution/`, surviving sessions. Holds the execution
graph, global status, an append-only journal, directed inboxes, executor cursors,
outboxes, handoffs, blockers and write contracts. Global state is
coordinator-owned; executor artifacts are scoped to their workstream.

**Write contract** — the set of paths a workstream is allowed to write.
Checkable before a write with `ivar feature execute guard-check`.

**Replan** — revising the plan when the divergence is *structural* (the approach
or entities are wrong, or the change crosses workstream boundaries). Advances the
plan's revision and pauses every affected workstream until it acknowledges the new
one. Unaffected workstreams keep running. Design-level, blocking, before code.

**Reconcile** — folding *local* code divergence back into the record when it is
confined to one workstream's operations. Written to the journal; the plan is not
rewritten. The code→plan direction.

**Ack revision** — a paused workstream acknowledging a new plan revision,
unpausing it. The board resumes once every paused workstream has.

## Delivering and finishing

**Delivery preview** — the side-effect-free summary from `ivar feature deliver
--preview`: the feature's plan-gate state, and per promoted repo the local
branch, remote, push refspec, PR action, base branch, ordering and blockers.
Apply is gated on two things — the plan gate being approved, and the preview's
fingerprint, which refuses if state drifted. The gate state is part of the
fingerprinted summary, so crossing it is itself drift.

**`part of`** — how sibling PRs across repos reference each other. Deliberately
not `depends on`: `ivar` models co-belonging, not dependency, and claims nothing
about merge order. Added in a second pass, because a PR's URL does not exist
before it is created.

**Close** — a feature's normal terminal transition. Stops executor sessions,
removes derived board and session state, records `outcome` (`delivered` or
`abandoned`) and `closedAt` in `plan.md`'s frontmatter, and keeps the three plan
files. The hall's git history is the record; there is no separate journal.

**Feature deletion** — the destructive teardown: worktrees, state, plans. A batch
preflight checks write access across every directory in the cleanup tree and
collects all blockers before mutating, so a run that cannot finish does not start.
Feature state is preserved on a runtime failure, making a retry idempotent.

**Feature review** — a VSCode multi-root workspace over a whole feature, written
by `ivar feature review`. Promoted repos appear as their editable feature-branch
worktrees; every other repo as its read-only default-branch checkout, for context.
The unit reviewed is the feature across repos, not one repo or one PR. Needs no
session.

**Feature view** — an interactive terminal dashboard over a feature's promoted
repos: a sidebar of repos, and a live shell per repo in the right panel, lazily
spawned on first focus. A reserved prefix key handles navigation so every other
keystroke reaches the focused shell.

## Health and state

**Hall health** — the hall's operational status, derived rather than stored:
`uninitialized`, `operational`, `stale`, or `degraded`.

**Stale** — structurally valid, but at least one repo has a remote ref its bare
clone does not have. `ivar repo pull` catches up. The hall is fully usable
meanwhile.

**Degraded** — a materialisation capability failed: a clone that did not
complete, a missing worktree. Structural degradation blocks session start;
optional degradation warns. Repair with `ivar sync`.

**Manifest** — `ivar.json` at the hall root. The hall's identity: name, repos
(name, URL, default branch), providers (available and default), and optionally
skill targets and MCP servers. Committed, hand-edited, and parsed strictly — an
unknown key is a hard error naming the key, never a warning.

**MCP** — hall-scoped, per-provider MCP server definitions, part of `ivar.json`
and materialised at the hall root (`.mcp.json` for Claude Code, the OpenCode
equivalent), discovered by walk-up from the view dir. Stores definitions only, and
references secrets through env vars rather than holding them.

**Skill** — a reusable instruction bundle (a folder with a `SKILL.md`) shared
across the hall from `.ivar/skills/` and materialised into each harness's native
location. **External** skills track a ref in another repo; **authored** ones are
local. **Detaching** converts external to authored, one way.

**Workflow Command** — an instruction workflow shipped inside the `ivar` binary
and materialised into each available Provider as `/ivar-<name>`. Workflow
Commands are local derived state: `ivar init`, `ivar provider add`, and
`ivar sync` create or repair them. They are not **Skills** and are not shared
through `.ivar/skills/`.

**Sync** — reconciling the local hall against `ivar.json`: clone missing repos,
materialise per-harness config (managed block, MCP, official workflow
commands), run setup scripts. Idempotent, and it touches no git remote —
refreshing branches is **pull**, a separate verb.

**Migrate** — the explicit, interactive advance of `ivar.json`'s schema version.
The one way a committed file's version moves. See
[Upgrading](guides/upgrading.md).

## Not in ivar

Some terms belong to the hosted product this CLI came out of, and are named here
only so you know they are absent by design rather than missing: **workspace**,
**canonical hall config**, **local customization**, **pending local change**,
**remote deleted**, and the five workspace verbs (`connect`, `push`, `pull`,
`login`, `logout`).

`ivar` is local-only. It has no account, no index and no server, and there is no
configuration that makes it acquire one.
