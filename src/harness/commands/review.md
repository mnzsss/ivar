---
description: Review a feature's or a pull request's changes along two axes — Standards and Spec — reported side by side.
argument-hint: [target]
---

# Review

`/ivar-review` reviews the changes on a feature or a pull request along two
axes: **Standards**, whether the diff follows the conventions the repo
documents, and **Spec**, whether it implements what was asked for. The axes run
as separate subagents so neither pollutes the other's context, and they are
reported side by side.

The findings are the deliverable, and they land in the conversation. This
workflow writes no file.

`ivar feature review <feature>` still writes the VSCode workspace it always
wrote. Step 9 offers it once the review is done.

## Prerequisites

- A target: a feature session with `IVAR_FEATURE` and `IVAR_SESSION_PATH`
  exported, as `/ivar-session-start` and `/ivar-session-connect` instruct, or
  an argument naming one.
- For a pull request target, `gh` must be installed and authenticated. A
  feature target never calls `gh`.
- A subagent this harness can spawn, holding its own context and able to run
  shell commands. Separate contexts are what isolate the axes, so a harness
  with no subagent cannot run this workflow. Concurrency is not required — one
  at a time, in sequence, is enough. This workflow hands subagents the commands
  that produce a diff, never the diff itself.

## Steps

1. **Resolve the target.** The argument is `$ARGUMENTS`. When it is empty
   inside a feature session, the target is `$IVAR_FEATURE` and every repo
   promoted to it. An argument overrides the session: a bare name is a
   feature, and a `<repo>#<number>`
   pair or a `https://github.com/<owner>/<repo>/pull/<number>` URL is a pull
   request. With neither, ask the user which one they mean. Never guess.

2. **Reject a remote that cannot carry a pull request.** For a pull request
   target, read that repo's `url` from the hall's `ivar.json` and derive
   `<owner>/<repo>` from `https://github.com/<owner>/<repo>`, with or without a
   trailing `.git`, or from `git@github.com:<owner>/<repo>.git`. Any other
   value — a filesystem path, a `file://` URL, a host other than `github.com` —
   carries no pull request. Stop, and name the repo and the `url` that decided
   it. Never call `gh` to find out.

3. **Pin the fixed point.** For a feature target, read it from the hall:

   ```bash
   ivar feature status <feature> --json
   ```

   Every entry in `repos[]` carries a `repo` and a `base`, and only promoted
   repos appear. A repo's working directory is `$IVAR_SESSION_PATH/<repo>`,
   which already points at that repo's feature worktree. Per repo:

   ```bash
   git -C $IVAR_SESSION_PATH/<repo> diff <base>...HEAD
   git -C $IVAR_SESSION_PATH/<repo> log <base>..HEAD --oneline
   ```

   The three dots are load-bearing: the comparison is against the merge-base,
   not against everything `<base>` has grown since the branch left it.

   For a pull request target:

   ```bash
   gh pr diff <number> -R <owner>/<repo>
   gh pr view <number> -R <owner>/<repo> --json title,body,baseRefName,url
   ```

   Run these yourself, before spawning anything. Stop and name the check that
   failed when `repos[]` is empty — nothing is promoted, so point at
   `/ivar-promote` — when a `base` is `null`, when a ref does not resolve, or
   when a diff is empty. A bad fixed point must fail here, never inside a
   subagent.

4. **Gather the standards sources, per repo.** Whichever of `ARCHITECTURE.md`,
   `CONTRIBUTING.md`, `CODING_STANDARDS.md`, `AGENTS.md` and `CLAUDE.md` exist
   at that repo's root. A repo that documents none of them is reviewed against
   the smell baseline alone, and its report must say so.

5. **Gather the spec, once for the target.** A feature target uses
   `plans/<feature>/requirements.md` and `plans/<feature>/plan.md` under
   `$IVAR_SESSION_PATH`. A pull request target uses the `title` and `body` read
   in step 3, plus any issue the body references. With neither available, skip
   the Spec subagent and say in the report that no spec was available. Never
   drop the axis silently.

6. **Run the axes.** First, confirm you can isolate them: you hold a subagent
   tool, and it has at least one target able to run shell commands. That is the
   whole requirement. Running the axes at the same time is not part of it, so a
   harness that spawns one subagent at a time passes and runs them in sequence.
   If you hold no such tool, or it has no target, stop here and say so: this
   harness has no subagent to isolate the axes with, and registering one is a
   change to the harness, which `ivar` neither makes nor names. Decide this by
   what you can do, never by an agent's name.

   Then act as the coordinator. Decompose the review using this provider's
   native subagent capabilities: **one Standards subagent per repo, and one
   Spec subagent for the whole target.** Never one per axis per repo.

   The split is asymmetric because the axes are. Standards is a property of a
   repo — each carries its own conventions, and one subagent holding two repos'
   standards at once is the pollution the split exists to prevent. Spec is a
   property of the target — there is one spec, and its most valuable findings,
   a requirement nobody implemented and work nobody asked for, are visible only
   across every repo at once.

   Give each Standards subagent the repo name, its working directory, its diff
   and log commands, the standards sources found for it, and the smell baseline
   below pasted in full — it has no other access to it. Its brief:

   ```
   Report, per file or hunk: every place the diff breaks a documented
   standard, citing the file and the rule it breaks; and every baseline smell
   you find, named and quoted. A documented standard can be a hard violation;
   a baseline smell is always a judgement call. Where the two disagree, the
   documented standard wins. Skip anything tooling already enforces. Under 400
   words.
   ```

   Give the Spec subagent every repo's diff and log command and the spec
   sources. Its brief:

   ```
   Report: requirements the spec asked for that are missing or partial; work
   in the diff nobody asked for; and requirements that look implemented but
   are implemented wrongly. Quote the spec line behind each finding. Under 400
   words.
   ```

7. **Report the axes apart.** Present `## Standards`, one subsection per repo,
   then `## Spec`, in each subagent's own words, verbatim or lightly cleaned.
   Never merge the axes, and never rank a finding on one against a finding on
   the other. Close with one line per axis: how many findings it raised and the
   worst one it raised. Never name a single worst finding across both — that is
   the ranking the split exists to prevent.

8. **Check the change against the hall's relations.** Compare what the diff
   does against the relation context in `HALL.md`, and offer `/ivar-relations`
   only for concrete evidence in the diff that the prose contradicts, extends,
   or obsoletes. Deferring it blocks nothing. This step never writes `HALL.md`;
   `/ivar-relations` is the only writer of the relation region.

9. **Offer the workspace.** For a feature target, offer a visual pass over the
   same changes:

   ```bash
   ivar feature review <feature>
   ```

   It writes `<feature>.code-workspace`, opening every repo in the hall as a
   folder with the promoted ones on the feature branch. Offer it; never run it
   unasked.

## The smell baseline

Two rules sit above this list and outrank every entry in it.

**A documented repo standard always wins.** Where the repo has already decided
— a style guide, a design document, a comment explaining the choice — and this
baseline would flag the very thing the repo endorses, say nothing.

**Nothing here is a violation.** Every entry is a labelled guess — "possible
Feature Envy", never "Feature Envy" — because a diff shows a hunk, not the
codebase around it, and a hunk can look wrong out of context and be right in
it. Skip whatever a linter, a formatter or a type checker already catches; this
baseline exists for what tooling cannot see.

- **Mysterious Name.** A diff adds or renames a symbol, and the reviewer must
  open its body to learn what it does — rename it to say what it does, before
  it merges.
- **Duplicated Code.** A hunk reproduces logic that already exists in the same
  file, or in a sibling file the same diff touches — extract the shared shape
  once a second copy exists, and leave the first copy alone until then.
- **Feature Envy.** A changed method reaches into another file's data and
  accessors more than it touches its own — move it toward the data it uses, or
  ask whether the diff put the logic on the wrong side of a seam.
- **Data Clumps.** The same group of parameters travels together across more
  than one signature or call site in the diff — bundle the group into one type
  and pass that instead.
- **Primitive Obsession.** A new field or parameter is a bare string, integer
  or boolean standing in for a domain concept such as a status, a currency or
  an id — give the concept its own small type once it appears twice.
- **Repeated Switches.** A hunk adds an arm to a switch or an if-chain that
  branches on a tag the diff already branches on elsewhere — reach for
  polymorphism instead of a third arm.
- **Shotgun Surgery.** One conceptual change lands as small scattered hunks
  across many files — the spread is the finding, and consolidating it is the
  next change, not this one.
- **Divergent Change.** One file carries two unrelated hunks in the same diff,
  each there for its own reason — the file already answers to more than one
  purpose, so flag it for a split.
- **Speculative Generality.** The diff adds a parameter, a hook, a base class
  or a configuration knob with exactly one caller — cut it back to what is
  actually called, and let the second caller justify the abstraction when it
  arrives.
- **Message Chains.** A line walks several calls deep to reach a distant object
  — ask the near object for what is needed rather than walking past it.
- **Middle Man.** A new or changed method's body is one call to another object
  and nothing else — call the target directly and drop the wrapper.
- **Refused Bequest.** A subtype overrides an inherited method only to reject
  it or turn it into a no-op — check that the hierarchy is the right one before
  accepting the override as normal.

## Why two axes

A change can pass one axis and fail the other. Code that follows every
convention while implementing the wrong thing passes Standards and fails Spec.
Code that does exactly what was asked while breaking the repo's conventions
passes Spec and fails Standards. Reported together, the passing axis masks the
failing one — so they are reported apart.

## Important

- **The review gates nothing.** `ivar feature deliver` does not consult it, and
  nothing here ever becomes a precondition of delivery. The review informs the
  human; it never blocks them.
- **Every repo is read-only to this workflow.** It reads diffs and documents,
  and writes no file — not in a repo, not in the hall.
- **The fixed point is derived, never asked for.** `ivar feature status --json`
  and `gh pr view` already know it. Ask the user for a target, never for a
  base.
- **A remote that cannot carry a pull request is named, never attempted.**
- **A missing subagent stops the review; it never degrades it.** Never run both
  axes in one context and still report them under `## Standards` and `## Spec`
  — that is the contamination the split exists to prevent, wearing the shape of
  the right artifact. Never hand back a partial review as though it were whole.
- **A capability this harness lacks is never this file's fault.** Name the
  capability and the harness that is missing it. Never report these
  instructions as incomplete, truncated or malformed to explain a stop.

## Credit

The two-axis design — Standards and Spec, isolated subagents, reported without
reranking — is taken from the `code-review` skill in `mattpocock/skills`. The
smell names are Martin Fowler's, from *Refactoring*, chapter 3.
