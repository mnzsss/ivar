---
description: Generate a multi-root .code-workspace file for a feature opening promoted repos writable and context repos read-only.
argument-hint: <feature-name> [repo...]
---

# Workspace

`/ivar-workspace` generates a multi-root editor workspace (`.code-workspace`)
for a feature. Promoted repos open writable; non-promoted (context) repos open
read-only. Arguments arrive via `$ARGUMENTS`.

## Usage

Generate or regenerate the workspace for a feature:

```bash
ivar feature workspace <feature>
```

Or, when `IVAR_FEATURE` is set:

```bash
ivar feature workspace "$IVAR_FEATURE"
```

Restrict the workspace to specific repos:

```bash
ivar feature workspace <feature> [repo...]
```

Run `ivar feature workspace --help` for the full command options.

## When to use

- You want to open all repos involved in a feature in VSCode or a compatible editor.
- You need to inspect or cross-reference read-only context repos alongside editable feature worktrees without risking accidental edits.
- You want to inspect only a subset of declared repos by naming them explicitly.

## What happens

1. **Path resolution.** Promoted repos resolve to their feature worktree path
   under `.ivar/repos/<repo>/<feature-branch>`. Non-promoted repos resolve to
   their default-branch worktree under `.ivar/repos/<repo>/<default-branch>`.
2. **Read-only enforcement.** Non-promoted folders receive entries in the
   workspace's top-level `settings["files.readonlyInclude"]` keyed by absolute
   path with `/**` suffix, ensuring the editor blocks edits in context folders.
3. **Workspace file generation.** The action writes the `.code-workspace` file
   canonically to `.ivar/features/<feature>/<feature>.code-workspace`.
4. **Console output.** Prints the generated workspace path and summary of
   included folders and read-only protections.
