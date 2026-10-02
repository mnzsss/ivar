# Child planner dispatch template

Fill every field before dispatching. Every path is absolute. The planner
writes only the child's `plan.md` and `tasks/`; it never runs
`ivar plan approve`, never edits code, and never asks the human directly.

```text
## Context
- Hall root: <absolute hall root>
- Parent feature: <parent>
- Child feature: <child>
- Child feature dir: <hall>/.ivar/features/<child>/
- Child repos and worktrees: <repo>: <absolute worktree, or "not promoted yet">
- Read-only checkout for each repo: <hall>/.ivar/repos/<repo>/<default branch>

## Artifacts (read before writing)
- Parent requirements: <absolute path>
- Parent analysis: <absolute path>
- Parent plan: <absolute path> — this child's row in `### Children`
- Plan template: <absolute path to ivar-plan/references/plan-template.md>
- Task template: <absolute path to ivar-plan/references/task-template.md>
- Current draft: <absolute path to the child's plan.md>
- Answers from the human (empty on the first dispatch):
  <question id>: <answer>

## Boundaries
- Write only <child feature dir>/plan.md and <child feature dir>/tasks/*.md.
- The plan covers exactly this child's row: its repos, the files it owns,
  what it delivers. Anything else is a question, not a plan item.
- `repos:` frontmatter lists only repos this child edits.

## Task
Write the child's plan.md and task packets following both templates. Record
every answer from the human under a `### Decisions` subsection of
`## Approach`, one line per answer, and treat recorded decisions as settled.

When a decision only the human can make blocks the plan — a product choice, a
trade-off the parent plan leaves open, an ambiguity in the slice — stop,
keep the draft written so far, and return needs_input.

Return exactly one of:

status: done
draft: <absolute path to plan.md>
summary: <at most five lines: waves, points, packets>

status: needs_input
draft: <absolute path to plan.md>
questions:
  - id: <short id>
    question: <one question>
    options: [<option>, <option>]   # omit for open questions
    recommended: <index>            # omit when no option is recommended
```
