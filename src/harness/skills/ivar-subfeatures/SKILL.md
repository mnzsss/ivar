---
name: ivar-subfeatures
description: Orchestrate a parent feature and its child features from the parent session with subagents — plan each child, route their questions to the human, execute in goal mode, integrate, review, and preview delivery.
argument-hint: [parent-feature]
---

# ivar-subfeatures

Use this when one feature — in one repo or several — breaks into slices that
can be built in parallel. Each slice becomes a **child** feature with its own
branch, cut from the **parent's** branch and integrated back into it. Only
the parent delivers.

You are the **coordinator**, running in the parent's session. You plan the
parent with the human, then drive every child through planning, execution and
integration with subagents, and stop at the parent's delivery preview. No
other terminal or session is launched by hand.

## Ground rules

- **One level of subagents.** Use one level of subagents; a subagent never spawns subagents. You dispatch
  every planner, implementer and reviewer yourself.
- **Return-only results.** You never message a running subagent. Each prompt
  carries everything the subagent needs; it finishes by returning its result.
  A subagent that needs a decision returns `needs_input` (see
  `references/child-planner.md`) and you dispatch a fresh one with the answer.
- **Absolute paths only.** Pass absolute paths in prompts; every path in a prompt is absolute. Resolve child
  worktrees from `ivar feature status <child> --json` (`repos[].worktree`) and
  feature dirs as `<hall>/.ivar/features/<child>/`.
- **State lives on disk, not in this conversation.** Before every step, read
  `ivar feature status <parent> --recursive --json` and pick each child's next
  action from `references/state.md`. Keep only short summaries of subagent
  results.
- **You never edit code or plans yourself.** Subagents make every edit; you
  run `ivar` commands, validation commands, and ask the human.
- **Questions to the human** follow `ivar-execute`'s interactive choice
  convention: the harness's Ask tool when it has one, lettered options
  otherwise, one question per prompt.
- **No AI attribution** in commits, PR titles or PR bodies; pass the rule to
  every subagent that commits.

### Dispatch per provider

| Provider | Dispatch | Concurrency |
| --- | --- | --- |
| Claude Code | the Task tool with a general-purpose subagent | several Task calls in one message |
| OpenCode | the `general` subagent (full tool access); never `explore` for edits | several subagent calls at once |
| omp | the task tool, one task per subagent; do not rely on its nesting or agent messaging | a `tasks` batch |

Run at most three subagents at once unless the human raises the limit.
Plan reviewers run on the smallest model the harness offers, as `ivar-plan`
describes.

## 1. Plan the parent

Run `/ivar-plan <parent>` — full SPDD, with the human at every gate. The
parent plan's **Structure** carries a `### Children` table:

| Child | Repos | Owns (files or dirs) | Depends on | Delivers |
| --- | --- | --- | --- | --- |

Children own disjoint files. The parent plan's waves hold only **Wave 0**:
everything two children would both edit — dependency manifests
(`Cargo.toml`, `package.json`, `pubspec.yaml`), route tables, theme or design
tokens, l10n files, shared registries a child would add one line to. A plan
with no shared edits has no waves.

Promote the parent into every repo a child will touch:

```bash
ivar feature promote <parent> <repo>
```

When the plan has Wave 0, execute it on the parent following `ivar-execute`
with the feature `<parent>` in goal mode, through
`ivar feature execute finish <parent>`, before any child exists.

## 2. Create the children

For each row of the Children table:

```bash
ivar feature create <child> --parent <parent>
ivar plan create <child> plan
```

Promote a child only once every child it depends on is integrated, so its
branch starts from a parent that already holds that work:

```bash
ivar feature promote <child> <repo>
```

Children are one level deep. A slice too big for one child becomes more
siblings in the parent plan, not grandchildren.

## 3. Plan every child

Children take `ivar-plan`'s short path: `plan.md` plus task packets, no
requirements or analysis.

1. Dispatch one planning subagent per child, concurrently, each prompt built
   from `references/child-planner.md`.
2. A result of `status: needs_input` carries questions. Ask the human each
   question, one at a time, then dispatch a fresh planner for that child with
   the answers. It continues from the draft on disk and records the answers
   under `### Decisions`, so no later subagent asks again.
3. A result of `status: done` goes to a plan reviewer subagent with
   `ivar-plan`'s Phase 3 checklist and output format. Issues go back to a fresh
   planner with the review; at most two re-reviews, then the remaining issues
   go to the human with the plan.
4. **Batch approval.** When every child plan is done and reviewed, show the
   human one table — child, repos, waves and points, review status, open
   issues — and ask a single multi-select question: which child plans to
   approve. Run `ivar plan approve <child> plan` for each approved child. A
   child sent back returns to step 1 with the human's note.

Never approve a scaffold, and never execute an unapproved child.

## 4. Execute the children

For each approved child whose repos are promoted, in dependency order —
independent children run concurrently:

```bash
ivar session start <child> --detached --json
```

Keep its `session_id` and `view_dir`. Then coordinate the child exactly as
`ivar-execute` describes, naming the feature `<child>` and goal mode:

```bash
ivar feature execute start <child> --mode goal
```

- Implementer prompts use `ivar-execute/references/subagent.md`, with the
  child's worktree root and the child's `view_dir` as the session.
- Wave validation, `ivar feature execute checkpoint <child> --wave <n>`, the
  Standards and Spec review barrier, and the caps are `ivar-execute`'s.
- Waves of different children may run in the same dispatch batch; waves of
  one child stay in order.
- Finish the child's run with `ivar-execute`'s report:

  ```bash
  ivar feature execute finish <child> --report-json <path> --outcome <succeeded|failed|blocked>
  ```

A `failed` or `blocked` child goes to the human before it is integrated:
**(a) Integrate anyway** with the follow-ups recorded, **(b) Fix and rerun**
(`ivar feature execute start <child> --resume` for a `blocked` run; a
`failed` run is terminal, so `ivar feature execute start <child>` opens a new
one), **(c) Abandon** (`ivar feature close <child> --outcome abandoned`).

## 5. Integrate each child as it finishes

Stop the child's session, then integrate it into the parent:

```bash
ivar session stop <session-id>
ivar feature integrate <child>
```

`integrate` refuses while the child has a session or an active run; a refusal
names its fix command — run it, then integrate again. After each integration,
promote any child whose dependencies are now all integrated and continue at
step 4 for it.

Watch the tree at any time:

```bash
ivar feature status <parent> --recursive
```

## 6. Final review of the parent

When every child is integrated or abandoned:

1. Dispatch two reviewers over the parent's worktrees, concurrently:
   - **Standards:** repository standards and architecture across everything
     the children merged — duplicated helpers, conflicting conventions,
     dead code left by merges.
   - **Spec:** the parent branch against the parent's `requirements.md` and
     `plan.md` and every child's `plan.md`, including what falls between
     children.
2. Run the parent plan's Verification commands yourself.
3. Dispatch blocking findings and failing checks to fix subagents that commit
   on the parent's branch; rerun the reviews. At most three rounds; whatever
   remains is listed for the human.

## 7. Preview the parent's delivery and stop

```bash
ivar feature deliver <parent> --preview
```

Print the preview, the fingerprint, the final review summary and the
follow-ups, then stop: "To deliver: /ivar-deliver". Never apply delivery.

## Resuming

After a compaction, a usage limit or a new session, read
`ivar feature status <parent> --recursive --json` and continue every child
from the action `references/state.md` assigns it. Never redo a step the tree
shows as done.
