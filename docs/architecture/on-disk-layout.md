# On-disk layout

Part of the [architecture overview](../../ARCHITECTURE.md).

## On-disk layout

One dotdir, one manifest, one name everywhere.

```
<hall>/
  ivar.json               the manifest. Committed — it is the identity file and
                          must be visible in review.
  HALL.md                 the canonical standing instructions. Committed, and
                          the sole editable source; the managed block is the
                          only part `ivar` owns.
  CLAUDE.md AGENTS.md     provider root aliases. Committed relative symlinks
                          to `HALL.md` — never sources, never edit targets.
  .ivar/                  everything the tool manages
    state.json            local hall state (gitignored)
    repos/<name>/.bare/   the bare clone; every checkout is a worktree off it
    repos/<name>/<branch>/
    features/<name>/      promotion records, execution/run.json, immutable
                          receipt and legacy-evidence archives, session view dirs
    sessions/<uuid>/      discovery session view dirs
    secrets/              hand-maintained secret material (gitignored)
    setups/<repo>.sh      per-repo setup scripts
    setups/<repo>.session.sh  per-repo session hooks
    skills/               hall-scoped skills (committed)
    skills-local/         personal skills (gitignored)
  agents/<name>.md        committed scheduled agents: recurring (`schedule:`) or
                          Agent Tasks (`at:`).
  agents/done/            committed concluded Agent Tasks, `<name>.<at>.md`.
                          Never read as agents.
  .claude/ .opencode/     harness-dictated, and the TARGET of symlinks, not the source
  .claude/commands/ivar-*.md   derived workflow commands (gitignored)
  .opencode/commands/ivar-*.md derived workflow commands (gitignored)
  .claude/skills/              derived materialised skills (gitignored)
  .opencode/skills/            derived materialised skills (gitignored)
```

A feature-session view dir is a real directory at
`<hall>/.ivar/features/<feature>/sessions/<uuid>/` containing: one symlink per
registered repo (feature worktree if promoted, read-only default otherwise), a
real harness config dir for the session's own provider (`.claude/` or
`.opencode/`, with `commands/` symlinked back to the hall), and the provider's
instruction file (`CLAUDE.md` / `AGENTS.md`) **derived from the canonical
`HALL.md`** — the session bootstrap block followed by the hall's standing
instructions, or the canonical content alone for a discovery session. Working
documents live two levels above the view dir at `.ivar/features/<feature>/`
(`../../` relative to the session), and the instruction file is a per-session
view, never a copy — it dies with the view dir. Every view dir receives the
instruction file, whether or not it is feature-bound, and the file never comes
from the root alias.

`agents/` is hand-edited, and `ivar` does not write to it, with one explicit
exception: when a timer-fired run concludes an Agent Task, `ivar` moves that task's
file from `agents/<name>.md` to `agents/done/`, under a name that never overwrites
an existing file. Nothing else under `agents/` is created, edited or deleted by
the binary (`ivar agent notify telegram setup` edits only the `notify:` line of
agents you choose).

Two traps, both learned the hard way:

- `.gitignore` must be `.ivar/*` plus `!.ivar/skills/` plus `!.ivar/setups/`,
  never `.ivar/`. Both of those children are committed — the skills a team shares
  and the setup script each repo carries. Git does
  not re-include a child of an excluded directory, and the failure is silent.
- **No legacy manifest-name fallback.** The manifest has already been renamed once.
  A fresh implementation should not be born carrying compatibility debt it never
  had.
