# Packet brief

Mandatory whenever Phase 3 hands task-packet generation to a child agent. A
child that gets only "write the packets" invents APIs, leaves prose where code
belongs, and "creates" files that exist. Paste this brief into the child's
prompt, filled in; the coordinator keeps ownership of `plan.md` and runs
`ivar plan lint <feature>` on whatever comes back.

## Inputs the child receives

- `../../requirements.md` and `../../analysis.md` — what the packets must cover.
- The **Interfaces** section of `../../plan.md`: the table of signatures each
  packet produces or consumes, locked before any packet is written. A
  packet's `Produces`/`Consumes` copy rows from it; they never invent new ones.
- The session view dir (`$IVAR_SESSION_PATH`): the only tree the child reads
  code from. Never guess at a symbol the view does not show.
- The wave to write, and the packets of earlier waves as read-only context.
  Generate packets wave by wave: wave N+1 starts once wave N's packets lint
  clean, so a consumer cites a signature that is real.

## Rules the child follows

1. **Every API a packet names comes from a `git grep -n` whose output is in the
   packet.** Run it from the repo root, no pathspec, and paste the real output
   in the Readers block (or the command plus
   `no readers outside the declared files`). A symbol with no grep hit is
   either one the packet creates or one that does not exist.
2. **`Create:` only for a file that does not exist; `Modify:` for one that
   does.** Check with `git ls-files <path>`. Write `Modify:` as
   `` `path:123-145` (N bytes) `` using `wc -c`.
3. **A file at or above 20,000 bytes (the commit hook's limit) receives no new
   test.** Put the test in a new file and name it under `Test:`.
4. **List every test helper the change affects** — each fixture, builder or
   shared support module whose behaviour the packet changes or relies on —
   with the `git grep -n` that found its users.
5. **State how the CLI treats warnings.** `ivar` exits `0` when clean, `1` when
   the verb finished with warnings (on stderr; `warnings` under `--json`) and
   `2` when it refused. Each Run/Expected line names the exit code, and a test
   never asserts plain success on a verb that warns. Compiler and clippy
   warnings are errors (`-D warnings`).
6. **Step 1 is test code and Step 3 is code**, as `task-template.md` requires:
   literal source, never a description. `**Sketch:** <reason>` is allowed only
   on Step 3 volume that follows mechanically from a decision shown in code,
   and the reason says why literal code is wrong there.
7. **No placeholders**: `TODO`, `TBD`, `<NNN>`, "as in Interfaces",
   "(run at execution)", "check … during execution". Write the number, the
   signature, the command.
8. **Dependencies are explicit.** A packet that consumes a signature lists the
   packet that produces it in its wave table's `Blocked by`.

## What the child returns

The packet files under `../../tasks/`, and nothing else. It does not edit
`plan.md`, approve gates, or dispatch the reviewer. If a rule cannot be met
(a symbol that has no grep hit, an Interfaces row that is ambiguous), the child
stops and reports it; it does not fill the gap.

## Coordinator check

After each wave: `ivar plan lint <feature>`. Exit `1` lists one problem per
warning, each with a code (`plan.lint.placeholder`, `plan.lint.create_exists`,
`plan.lint.missing_dependency`, …). Fix or regenerate; do not hand the
reviewer a packet the lint refuses.
