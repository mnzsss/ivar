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

## Read-only is a filesystem guarantee, and that is all it is

A repo you have not promoted has its write bits cleared. That is the kernel
refusing, not a policy an agent can talk its way past — which is the point.

What it does **not** stop:

- A tool running as you with `sudo`, or anything that chmods the bits back.
- Reading. Non-promoted repos are fully readable; that is what they are for.
- An agent editing a promoted repo it should have left alone. Promotion is
  per-feature, not per-file — once a repo is writable, it is writable.

Two sessions on the same feature share its worktrees, and both may write them.
Git serialises individual operations; nothing stops two agents editing the same
file.

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
