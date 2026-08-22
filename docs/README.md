# ivar documentation

`ivar` mounts the repos a feature spans into one directory — real git worktrees
on the same branch, opened by one agent session.

Start here, in this order:

1. **[Concepts](concepts.md)** — the model, in five terms and a diagram. Read
   this before typing anything. It is short, and everything else assumes it.
2. **[Getting started](getting-started.md)** — two paths: joining a hall someone
   else set up, or creating one.
3. **[Why not just `git worktree` and a script?](why-not-worktree.md)** — the
   fair question. Answered with what breaks, not with adjectives.

Then, by task:

- **[Day to day](guides/day-to-day.md)** — feature, promote, session, deliver.
  The loop you actually spend your week in.
- **[Planning and execution](guides/planning-and-execution.md)** — the SPDD
  artifacts, the three approval gates, and the Run Receipt lifecycle.
- **[Skills](guides/skills.md)** — sharing skills across the hall and
  materialising them per harness.
- **[Upgrading](guides/upgrading.md)** — what moves on its own, what asks you
  first, and what to do when a teammate's `ivar` refuses your hall.

Reference:

- **[Command reference](reference/commands.md)** — every verb, generated from
  the binary.
- **[On-disk format](reference/on-disk-format.md)** — what is committed, what is
  local, and the migration promise.
- **[Limitations](reference/limitations.md)** — what `ivar` does not isolate, in
  writing, before you hit it.
- **[Glossary](glossary.md)** — every other term, once you need it.

Building on `ivar` itself: [ARCHITECTURE.md](../ARCHITECTURE.md) and
[CONTRIBUTING.md](../CONTRIBUTING.md).
