# Limitations and failure modes

What `ivar` does not do, and what breaks when you push on it. This page exists so
you find these here rather than by hitting them.

`ivar` isolates two things: **the filesystem** and **ports**. Everything below is
something outside those two.

## Shared daemon state — the big one

A `git worktree` gives every feature its own files. It does not give them their own
Postgres.

Two features promoting the same repo get two worktrees, two dev servers, two
distinct ports — and **one shared database**. Run a migration in one and the other
one sees it. Truncate a table in one and the other one's tests fail.

`ivar` does not solve this, and the honest reason is that the only general
solution is a container per environment, which `ivar` rejects by design: the whole
mechanism is real directories on your real filesystem that your real editor and
your real `lazygit` can open.

**What `ivar` gives you instead is the hook.** Each repo can carry a **session
hook** at `.ivar/setups/<repo>.session.sh`, run on every `ivar session start`,
in that repo's worktree, with the session identifier in its environment. The
person who knows how to isolate that repo is the person who wrote it — so `ivar`
hands them the seam rather than guessing.

Note which file this is. The setup script next to it
(`.ivar/setups/<repo>.sh`) is receipt-gated and runs about once per worktree —
right for `pnpm install`, useless for a database that must come up every session.
The hook is ungated and runs every time. A hook that fails warns and the session
still opens.

Two recipes that work:

```sh
# docker compose: one project name per session, so volumes do not collide
export COMPOSE_PROJECT_NAME="myapp-${IVAR_SESSION_ID}"
docker compose up -d db
```

```sh
# sqlite: derive the file from the session
export DATABASE_URL="file:./.data/${IVAR_SESSION_ID}.db"
pnpm db:migrate
```

Neither is imposed. If your repo genuinely cannot be isolated, two features on it
at once will interfere, and that is worth knowing before you try it — not after.

## Your harness may refuse things, and it looks like `ivar` broke

Coding agents run their own sandboxes, and those sandboxes deny things `ivar` set
up correctly. The symptoms are misleading:

| what you see | who actually refused |
| --- | --- |
| `EPERM` binding a port the agent picked | the harness sandbox, not `ivar` |
| a write that "succeeded" but changed nothing | the harness, silently |
| a command that cannot see a directory the view dir clearly contains | the harness's path allowlist |

Before filing this against `ivar`, check the harness's own permission settings.
`ivar doctor` reports what it materialised; if `doctor` is clean and the agent
still cannot act, the refusal is upstream of `ivar`.

## Read-only is a filesystem guarantee, and it covers the worktree root

A repo you have not promoted has its write bits cleared **on the worktree
directory itself** — `mode & ~0o222` on that one path, not on the tree beneath
it. That is the kernel refusing, not a policy an agent can talk its way past,
and it is what stops a non-promoted repo from gaining, losing, or renaming a
top-level entry.

It does not make the files below read-only, and it should not be read as if it
did. Write permission on a directory governs its *entries*, not their contents:
`packages/api/index.ts` in a non-promoted repo is still mode 644, and an agent
that opens it for writing still succeeds.

The obvious hardening — clear the bits recursively — is not available. Package
managers that hardlink out of a content-addressed store (pnpm, bun) leave a
worktree full of files whose inode is shared with the store and with every other
checkout on the machine, and `chmod` acts on the inode, not on the link.
Guarding one worktree recursively would change permissions inside that store and
inside unrelated projects. A per-file guarantee needs a sandbox — Landlock, a
mount namespace — and `ivar` does not run one, which is the sentence the
write-contract section below also ends on.

So the guard is a hard floor at the root; writes to files that already exist are
covered by the two layers below it, not by the kernel.

What it does **not** stop:

- Writing a file that already exists in a non-promoted repo — the guard is the
  worktree root, not the tree under it.
- A tool running as you with `sudo`, or anything that chmods the bits back.
- Reading. Non-promoted repos are fully readable; that is what they are for.
- An agent editing a promoted repo it should have left alone. Promotion is
  per-feature, not per-file — once a repo is writable, it is writable.

Two sessions on the same feature share its worktrees, and both may write them.
Git serialises individual operations; nothing stops two agents editing the same
file.

## The write contract is enforced at two layers, and neither is a sandbox

A workstream's write contract is arbitrated by a `PreToolUse` hook in the
session's harness config. That hook sees the tools that carry a path — `Write`,
`Edit`, `MultiEdit`, `NotebookEdit` — and refuses anything outside the contract
before the write happens.

It does **not** see `Bash`. A shell call carries a command, not a path, and
deciding what a command writes means deciding what a program does. So a
heredoc into `python3`, a formatter run over the repo, a code generator — all
reach the disk without the hook being asked.

The second layer is what catches those: after a workstream's process exits,
`ivar` compares its worktrees against the wave's contracts and fails the
workstream when a path changed that no contract covers. That is **detection,
not prevention** — the bytes are already written when it runs, and nothing is
reverted. The failure and the paths land in the board's journal.

The comparison is against the commit each worktree was on when the run
started, not against its working tree, so an executor's own git actions do not
hide it. What the run committed, amended, rebased or reset onto counts exactly
like what it left uncommitted. This matters more than it sounds: committing is
the *expected* end state — `feature deliver` counts a dirty worktree as a
blocker — so an audit that read only the working tree passed every run that
did what the pipeline asked, stray writes and all.

It reads the difference in both directions. A run does not only add paths:
`git checkout -- .`, `git reset --hard` and `git stash` make divergence
*disappear*, so a run that throws away an uncommitted edit it inherited leaves
a change set that is smaller rather than larger. A path that diverged before
the run, no longer does, and that no contract in the wave covers is reported
the same way a stray write is. Content that was never committed is not
recoverable from the repository, so this is a report, not a repair.

Two things it deliberately does not do:

- **Attribute.** A tick runs a wave of workstreams against the same worktrees,
  and git records that a file changed, not who changed it. So the audit
  measures against the union of the wave's contracts, in both directions. One
  workstream writing inside — or reverting — *another's* contract is invisible
  to it; the hook still refuses the writes for the tools it covers.
- **Revert.** An audit that deleted an agent's work on suspicion would be a
  worse failure than the one it guards against.

## A workstream must have something to show for itself

A clean exit is the executor's claim that it finished, not evidence that it did
anything: a session that misread its prompt, was denied every write, or simply
idled exits zero exactly like one that did the work.

So the same audit asks a second question — did anything change under *this*
workstream's own contract? — and a workstream that has never produced anything
is left `blocked` with a `session.unproductive` journal entry rather than
`done`. Nothing downstream then launches against work that does not exist.

"Never" is the operative word, and it is read from the journal. A workstream
that blocked on a question is relaunched from scratch against a baseline that
already holds what its first run wrote, so its second run can legitimately
change nothing new; the earlier run's `produced` entry is what lets it finish.

A feature with **no promoted repo** is exempt: there is no worktree to read, so
"produced nothing" and "nowhere to produce anything" are the same picture, and
a workstream is never refused for the absence of an oracle.

If you need writes outside the contract to be impossible rather than reported,
that is a filesystem sandbox, and `ivar` does not run one.

## Windows is not supported

The view dir is built entirely out of symlinks, and creating a symlink on Windows
needs Developer Mode or administrator rights. The central mechanism is the part
that does not cross.

**Use WSL.** It consumes the Linux build unchanged — there is no separate path to
maintain and no separate behaviour to learn.

## Rebuilding a view dir has a microscopic window (macOS)

When `ivar` repoints a view-dir symlink — which happens when you promote a repo,
and when `ivar session connect` repairs links another session left stale — it
creates the new link and renames it over the old one. On Linux that swap is
atomic. **On macOS it is not, quite.**

Measured on APFS, hammering 300 replacements against a reader doing nothing else:
roughly 2% of reads in that window saw a transient error — `readlink` failing, and
`stat` and `open` *through* the link failing. `lstat` never failed.

What this means in practice: if an agent happens to read a file through a view-dir
symlink at the exact microsecond that link is being repointed, it can get a
spurious "no such file". Retrying works.

Two things keep it small. `ivar` **skips the swap entirely when the link already
points where it should**, which is the overwhelmingly common case — `connect` is
idempotent and usually changes nothing. And the only time a link genuinely moves
is a promote, when the agent's view of that repo is changing anyway.

It is named here rather than left to be discovered because a one-in-a-thousand
`ENOENT` is otherwise unattributable, and you would reasonably suspect your own
code first.

## Terminal fidelity: two attributes are missing

The session panel renders your shell through a terminal emulator, and that emulator
does not track two SGR attributes: **invisible** (SGR 8) and **strikethrough**
(SGR 9).

In practice: password prompts that mask by making text invisible will show the
text, and a linter that strikes through a deprecated symbol will show it plain.
Colour, bold, italic, underline, dim, inverse, 256-colour, true colour, wide CJK
characters, scrollback and alternate screen all render correctly.

This is a known gap in a dependency with a narrow blast radius, not a design
choice. It is behind a seam and will move when the fix is cheaper than the churn.

## The feature view captures the mouse

A terminal showing the alternate screen sends the **wheel as arrow keys** unless
the application asks for mouse reports. Those arrows are indistinguishable from
typed ones, so without capture every wheel notch is typed into the focused shell:
history recalled at the prompt, or `^[[A` echoed back as text by whatever is
running. `ivar feature view` therefore captures the mouse, and scrolls the panel
with the wheel instead.

Two consequences, both of them the usual ones for a terminal application that
does this:

- **Selecting text needs `shift` held** — the same key `tmux` and `vim` users
  already reach for.
- **The wheel does not reach programs running inside the shell.** A `less` or a
  `vim` with its own mouse handling will not see it; the panel scrolls instead.
  Their own keys (`ctrl+f`, `j`/`k`, `space`) work as always.

## Things that are out of scope, permanently

- **Anything that needs a server.** `ivar` is local-only and has no network client
  beyond talking to GitHub on your behalf when you ask it to. There is no account,
  no index, no telemetry — not even opt-in. `rg` the source.
- **A container per environment.** See the first section.
- **A shared build cache across worktrees.** Worktrees share history, not build
  artifacts. Your build tool's own cache is the answer.
- **Modelling dependencies between repos.** `ivar` models *co-belonging* — these
  repos are part of this feature — not "`web` depends on `api`". Linked pull
  requests say `part of`, never `depends on`.

## When something is wrong

```sh
ivar doctor          # what is materialised, what drifted, what to run
ivar status          # halls, features, sessions
```

`ivar doctor` names the fix rather than the symptom. If it cannot, that is a bug
worth an issue.
